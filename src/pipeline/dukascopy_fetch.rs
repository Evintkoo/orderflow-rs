//! Dukascopy live HTTP tick-data fetcher.
//!
//! Downloads `.bi5` (LZMA-compressed binary) tick files directly from
//! `https://datafeed.dukascopy.com/datafeed/` — no account or API key needed.
//!
//! ## File format
//!
//! Each `.bi5` file covers one hour of a single FX pair.  After LZMA
//! decompression every tick record is 20 bytes, big-endian:
//!
//!   bytes 0–3  : uint32 — milliseconds from the start of the hour
//!   bytes 4–7  : uint32 — ask × 100 000
//!   bytes 8–11 : uint32 — bid × 100 000
//!   bytes 12–15: float32 — ask volume (millions of base currency)
//!   bytes 16–19: float32 — bid volume
//!
//! ## URL pattern
//!
//!   https://datafeed.dukascopy.com/datafeed/{PAIR}/{YEAR}/{MONTH_0IDX:02}/{DAY:02}/{HOUR:02}h_ticks.bi5
//!
//! Month is **0-indexed** (January = 00, December = 11).
//!
//! ## What this produces
//!
//! Each tick emits a `BookSnapshot` with the real bid/ask prices and sizes.
//! OFI, spread, depth imbalance, and trade intensity features all work at L1.

use anyhow::{bail, Result};

use super::realdata::{DataQualityFlags, Event, TsSource};

// ── Tick record ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DukaTick {
    pub ts_us:   i64,
    pub ask:     f64,
    pub bid:     f64,
    pub ask_vol: f64,
    pub bid_vol: f64,
}

/// Decompress one `.bi5` payload and parse all tick records.
///
/// `hour_start_us` is the Unix timestamp (µs) for the start of the hour the
/// file represents.  Returns `Ok(vec![])` for empty payloads.
pub fn parse_bi5(data: &[u8], hour_start_us: i64) -> Result<Vec<DukaTick>> {
    if data.is_empty() { return Ok(vec![]); }

    let mut decompressed = Vec::new();
    lzma_rs::lzma_decompress(&mut std::io::Cursor::new(data), &mut decompressed)
        .map_err(|e| anyhow::anyhow!("lzma decompress: {e}"))?;

    if decompressed.len() % 20 != 0 {
        bail!("bi5 decompressed size {} not a multiple of 20", decompressed.len());
    }

    let mut ticks = Vec::with_capacity(decompressed.len() / 20);
    for chunk in decompressed.chunks_exact(20) {
        let ms_offset = u32::from_be_bytes([chunk[0],  chunk[1],  chunk[2],  chunk[3]])  as i64;
        let ask_raw   = u32::from_be_bytes([chunk[4],  chunk[5],  chunk[6],  chunk[7]])  as f64;
        let bid_raw   = u32::from_be_bytes([chunk[8],  chunk[9],  chunk[10], chunk[11]]) as f64;
        let ask_vol   = f32::from_be_bytes([chunk[12], chunk[13], chunk[14], chunk[15]]) as f64;
        let bid_vol   = f32::from_be_bytes([chunk[16], chunk[17], chunk[18], chunk[19]]) as f64;

        let ask = ask_raw / 100_000.0;
        let bid = bid_raw / 100_000.0;
        if ask <= 0.0 || bid <= 0.0 { continue; }

        ticks.push(DukaTick {
            ts_us:   hour_start_us + ms_offset * 1_000,
            ask,
            bid,
            ask_vol: ask_vol.max(0.0),
            bid_vol: bid_vol.max(0.0),
        });
    }
    Ok(ticks)
}

/// Convert a tick into a `BookSnapshot` event.
pub fn tick_event(symbol: &str, tick: &DukaTick) -> (String, Event, DataQualityFlags) {
    let min_size = 0.001_f64;
    (
        symbol.to_string(),
        Event::BookSnapshot {
            bids: vec![(tick.bid, tick.bid_vol.max(min_size))],
            asks: vec![(tick.ask, tick.ask_vol.max(min_size))],
            ts:   tick.ts_us,
        },
        DataQualityFlags {
            is_imputed:  false,
            gap_flag:    false,
            is_outlier:  false,
            ts_source:   TsSource::Exchange,
        },
    )
}

// ── Date math (no chrono dependency) ─────────────────────────────────────────

/// Decompose a Unix timestamp (seconds) into (year, month 1-based, day, hour).
fn decompose_ts(mut ts_s: i64) -> (i32, u32, u32, u32) {
    // Days since Unix epoch
    let mut days = (ts_s / 86_400) as i32;
    let hour     = ((ts_s % 86_400) / 3_600) as u32;

    // Gregorian calendar conversion (Euclidean / proleptic)
    let z = days + 719_468;
    let era   = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe   = z - era * 146_097;
    let yoe   = (doe - doe/1_460 + doe/36_524 - doe/146_096) / 365;
    let y     = yoe + era * 400;
    let doy   = doe - (365*yoe + yoe/4 - yoe/100);
    let mp    = (5*doy + 2)/153;
    let d     = (doy - (153*mp + 2)/5 + 1) as u32;
    let m     = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y     = if m <= 2 { y + 1 } else { y };

    (y as i32, m, d, hour)
}

/// Build the `.bi5` URL for a given Unix timestamp (seconds, UTC, rounded to hour).
pub fn bi5_url(pair: &str, hour_ts_s: i64) -> String {
    let (year, month, day, hour) = decompose_ts(hour_ts_s);
    let month0 = month - 1; // Dukascopy month is 0-indexed
    format!(
        "https://datafeed.dukascopy.com/datafeed/{pair}/{year}/{month0:02}/{day:02}/{hour:02}h_ticks.bi5"
    )
}

// ── Ingester ──────────────────────────────────────────────────────────────────

/// Default set of major FX pairs available from Dukascopy.
pub const DEFAULT_PAIRS: &[&str] = &[
    "EURUSD", "GBPUSD", "USDJPY", "USDCHF", "AUDUSD", "USDCAD",
];

/// Dukascopy HTTP tick fetcher.
///
/// Downloads the last `backfill_days` of hourly `.bi5` files for each FX pair
/// and feeds them through the standard LOB pipeline callback.
#[cfg(feature = "ingest")]
pub struct DukascopyFetcher {
    pub pairs:         Vec<String>,
    pub backfill_days: u32,
}

#[cfg(feature = "ingest")]
impl DukascopyFetcher {
    pub fn new(pairs: &[&str]) -> Self {
        Self {
            pairs:         pairs.iter().map(|s| s.to_uppercase()).collect(),
            backfill_days: 30,
        }
    }

    pub fn with_backfill_days(mut self, days: u32) -> Self { self.backfill_days = days; self }

    pub async fn run<F>(&mut self, mut on_event: F) -> Result<()>
    where
        F: FnMut(String, Event, DataQualityFlags),
    {
        let client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0")
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        let now_s = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default()
            .as_secs() as i64;

        // Round down to the last completed hour.
        let end_hour_s   = (now_s / 3_600 - 1) * 3_600;
        let start_hour_s = end_hour_s - self.backfill_days as i64 * 86_400;
        let total_hours  = ((end_hour_s - start_hour_s) / 3_600) as usize;

        eprintln!("[dukascopy] fetching {}d ({} hours) of ticks for {} pairs",
            self.backfill_days, total_hours, self.pairs.len());

        let mut total_ticks   = 0usize;
        let mut skipped_hours = 0usize;

        for pair in &self.pairs.clone() {
            let mut ticks_for_pair = 0usize;
            let mut hour_ts = start_hour_s;

            while hour_ts <= end_hour_s {
                let url           = bi5_url(pair, hour_ts);
                let hour_start_us = hour_ts * 1_000_000;

                match client.get(&url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        match resp.bytes().await {
                            Ok(bytes) => {
                                match parse_bi5(&bytes, hour_start_us) {
                                    Ok(ticks) => {
                                        ticks_for_pair += ticks.len();
                                        for tick in &ticks {
                                            let (s, e, f) = tick_event(pair, tick);
                                            on_event(s, e, f);
                                        }
                                    }
                                    Err(e) => eprintln!("[dukascopy] {pair} {hour_ts}: parse: {e}"),
                                }
                            }
                            Err(e) => eprintln!("[dukascopy] {pair} {hour_ts}: body: {e}"),
                        }
                    }
                    Ok(resp) if resp.status().as_u16() == 404 => {
                        skipped_hours += 1; // weekend/holiday — no file
                    }
                    Ok(resp) => eprintln!("[dukascopy] {pair}: HTTP {}", resp.status()),
                    Err(e)   => eprintln!("[dukascopy] {pair}: request: {e}"),
                }

                hour_ts += 3_600;
                // Polite throttle — Dukascopy CDN is public but rate-sensitive.
                tokio::time::sleep(std::time::Duration::from_millis(80)).await;
            }

            total_ticks += ticks_for_pair;
            eprintln!("[dukascopy] {pair}: {ticks_for_pair} ticks replayed");
        }

        eprintln!("[dukascopy] complete: {total_ticks} ticks  ({skipped_hours} weekend/holiday hours skipped)");
        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn pack_tick(ms: u32, ask_raw: u32, bid_raw: u32, ask_vol: f32, bid_vol: f32) -> [u8; 20] {
        let mut buf = [0u8; 20];
        buf[0..4].copy_from_slice(&ms.to_be_bytes());
        buf[4..8].copy_from_slice(&ask_raw.to_be_bytes());
        buf[8..12].copy_from_slice(&bid_raw.to_be_bytes());
        buf[12..16].copy_from_slice(&ask_vol.to_be_bytes());
        buf[16..20].copy_from_slice(&bid_vol.to_be_bytes());
        buf
    }

    fn make_bi5(ticks: &[[u8; 20]]) -> Vec<u8> {
        let raw: Vec<u8> = ticks.iter().flat_map(|t| t.iter().copied()).collect();
        let mut out = Vec::new();
        lzma_rs::lzma_compress(&mut std::io::Cursor::new(&raw), &mut out).unwrap();
        out
    }

    #[test]
    fn parse_two_ticks() {
        let tick0 = pack_tick(0,   107145, 107100, 0.75, 0.50);
        let tick1 = pack_tick(500, 107150, 107105, 1.00, 0.75);
        let data  = make_bi5(&[tick0, tick1]);

        let hour_start = 1_700_000_000_000_000i64;
        let ticks = parse_bi5(&data, hour_start).unwrap();
        assert_eq!(ticks.len(), 2);
        assert!((ticks[0].ask - 1.07145).abs() < 1e-5);
        assert!((ticks[0].bid - 1.07100).abs() < 1e-5);
        assert_eq!(ticks[0].ts_us, hour_start);
        assert_eq!(ticks[1].ts_us, hour_start + 500_000);
    }

    #[test]
    fn empty_bi5_is_ok() {
        assert!(parse_bi5(&[], 0).unwrap().is_empty());
    }

    #[test]
    fn bi5_url_december() {
        // 2024-12-15 10:00 UTC
        let ts = 1734256800i64; // 2024-12-15T10:00:00Z
        let url = bi5_url("EURUSD", ts);
        // December = month 11 (0-indexed)
        assert!(url.contains("/2024/11/15/10h_ticks.bi5"), "url={url}");
    }

    #[test]
    fn decompose_known_dates() {
        // 2024-01-01 00:00:00 UTC = 1704067200
        let (y, m, d, h) = decompose_ts(1704067200);
        assert_eq!((y, m, d, h), (2024, 1, 1, 0));

        // 2024-12-15 10:00:00 UTC = 1734256800
        let (y, m, d, h) = decompose_ts(1734256800);
        assert_eq!((y, m, d, h), (2024, 12, 15, 10));
    }
}
