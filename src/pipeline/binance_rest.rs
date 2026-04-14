//! Binance REST klines ingester — HTTPS polling, no WebSocket, no API key.
//!
//! Calls `https://api.binance.com/api/v3/klines` to fetch 1-minute OHLCV bars.
//! Useful when the Binance WebSocket (`stream.binance.us:9443`) is blocked by ISP
//! but HTTPS to `api.binance.com:443` is accessible.
//!
//! ## Backfill
//!
//! Paginates backwards from `now` in 1 000-bar chunks until `backfill_days` of
//! history has been downloaded for every symbol.
//!
//! ## Live polling
//!
//! Fetches the latest 2 closed bars every `poll_interval_ms` milliseconds.
//! Only new bars (timestamp > last seen) are emitted.
//!
//! ## Synthetic NBBO
//!
//! From OHLCV only, each bar emits:
//!   - `Event::Trade`        — close price + volume at close_time
//!   - `Event::BookSnapshot` — bid = close − close×0.0001, ask = close + close×0.0001
//!     (1 basis-point half-spread; size = volume / 1 000)

use std::collections::HashMap;
use anyhow::Result;

use crate::features::trade_pressure::TradeEvent;
use super::realdata::{DataQualityFlags, Event, TsSource};

// ── Kline row ─────────────────────────────────────────────────────────────────

/// One kline from Binance REST.  Only the fields we use.
struct Kline {
    open_time_ms:  i64,
    close:         f64,
    volume:        f64,
    close_time_ms: i64,
    is_buy:        bool, // close > open
}

/// Parse the raw JSON array-of-arrays from `GET /api/v3/klines`.
fn parse_klines(json: &str) -> Vec<Kline> {
    let arr: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let rows = match arr.as_array() {
        Some(a) => a,
        None => return vec![],
    };

    rows.iter().filter_map(|row| {
        let r = row.as_array()?;
        let open_time_ms  = r.get(0)?.as_i64()?;
        let open_str      = r.get(1)?.as_str()?;
        let close_str     = r.get(4)?.as_str()?;
        let volume_str    = r.get(5)?.as_str()?;
        let close_time_ms = r.get(6)?.as_i64()?;

        let open:   f64 = open_str.parse().ok()?;
        let close:  f64 = close_str.parse().ok()?;
        let volume: f64 = volume_str.parse().ok()?;

        if close <= 0.0 { return None; }

        Some(Kline {
            open_time_ms,
            close,
            volume,
            close_time_ms,
            is_buy: close >= open,
        })
    }).collect()
}

/// Convert a `Kline` into `(Trade, BookSnapshot)` event pair.
fn kline_events(symbol: &str, k: &Kline) -> [(String, Event, DataQualityFlags); 2] {
    let ts_us = k.close_time_ms * 1_000;
    let half_spread = (k.close * 0.0001_f64).max(0.01);
    let bid = k.close - half_spread;
    let ask = k.close + half_spread;
    let size = (k.volume / 1_000.0).max(1.0);

    let flags = DataQualityFlags {
        is_imputed: false,
        gap_flag: false,
        is_outlier: false,
        ts_source: TsSource::Exchange,
    };

    [
        (
            symbol.to_string(),
            Event::Trade(TradeEvent {
                price:  k.close,
                qty:    k.volume,
                ts:     ts_us,
                is_buy: k.is_buy,
            }),
            flags.clone(),
        ),
        (
            symbol.to_string(),
            Event::BookSnapshot {
                bids: vec![(bid, size)],
                asks: vec![(ask, size)],
                ts:   ts_us,
            },
            flags,
        ),
    ]
}

// ── Ingester ──────────────────────────────────────────────────────────────────

#[cfg(feature = "ingest")]
pub struct BinanceRestIngester {
    pub symbols:          Vec<String>,
    /// How far back to backfill (days).
    pub backfill_days:    u32,
    /// Live-poll interval in milliseconds.
    pub poll_interval_ms: u64,
}

#[cfg(feature = "ingest")]
impl BinanceRestIngester {
    pub fn new(symbols: &[&str]) -> Self {
        Self {
            symbols:          symbols.iter().map(|s| s.to_uppercase()).collect(),
            backfill_days:    30,
            poll_interval_ms: 2_000,
        }
    }

    pub fn with_backfill_days(mut self, days: u32) -> Self { self.backfill_days = days; self }
    pub fn with_poll_interval_ms(mut self, ms: u64) -> Self { self.poll_interval_ms = ms; self }

    /// Build a klines URL.
    fn klines_url(symbol: &str, limit: u32, end_time_ms: Option<i64>, start_time_ms: Option<i64>) -> String {
        let mut url = format!(
            "https://api.binance.com/api/v3/klines?symbol={symbol}&interval=1m&limit={limit}"
        );
        if let Some(et) = end_time_ms   { url.push_str(&format!("&endTime={et}")); }
        if let Some(st) = start_time_ms { url.push_str(&format!("&startTime={st}")); }
        url
    }

    pub async fn run<F>(&mut self, mut on_event: F) -> Result<()>
    where
        F: FnMut(String, Event, DataQualityFlags),
    {
        let client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0")
            .timeout(std::time::Duration::from_secs(20))
            .build()?;

        // ── Phase 1: historical backfill ──────────────────────────────────────
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default()
            .as_millis() as i64;
        let start_ms = now_ms - self.backfill_days as i64 * 86_400_000;

        eprintln!("[binance-rest] fetching {}d backfill for {} symbols",
            self.backfill_days, self.symbols.len());

        let mut total_bars = 0usize;
        for sym in &self.symbols.clone() {
            let mut page_end = now_ms;
            let mut pages: Vec<Vec<Kline>> = Vec::new();

            loop {
                let url = Self::klines_url(sym, 1000, Some(page_end), None);
                match client.get(&url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        match resp.text().await {
                            Ok(body) => {
                                let bars = parse_klines(&body);
                                if bars.is_empty() { break; }
                                let oldest_ms = bars[0].open_time_ms;
                                pages.push(bars);
                                if oldest_ms <= start_ms { break; }
                                page_end = oldest_ms - 1;
                            }
                            Err(e) => { eprintln!("[binance-rest] {sym}: body error: {e}"); break; }
                        }
                    }
                    Ok(resp) => { eprintln!("[binance-rest] {sym}: HTTP {}", resp.status()); break; }
                    Err(e)   => { eprintln!("[binance-rest] {sym}: request error: {e}"); break; }
                }
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            }

            // Emit pages in chronological order (they were fetched newest-first).
            pages.reverse();
            let mut n = 0usize;
            for page in pages {
                for k in &page {
                    if k.open_time_ms < start_ms { continue; }
                    for ev in kline_events(sym, k) {
                        let (s, e, f) = ev;
                        on_event(s, e, f);
                    }
                    n += 1;
                }
            }
            total_bars += n;
            eprintln!("[binance-rest] {sym}: {n} bars replayed");
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        eprintln!("[binance-rest] backfill complete: {total_bars} total bars");

        // ── Phase 2: live polling ─────────────────────────────────────────────
        let syms: Vec<String> = self.symbols.clone();
        let poll_ms = self.poll_interval_ms;
        eprintln!("[binance-rest] starting live poll every {poll_ms}ms for: {}",
            syms.join(", "));

        // Track last-seen close_time per symbol to avoid re-emitting.
        let mut last_ts: HashMap<String, i64> = HashMap::new();
        for s in &syms { last_ts.insert(s.clone(), 0); }

        loop {
            tokio::time::sleep(std::time::Duration::from_millis(poll_ms)).await;

            for sym in &syms {
                let url = Self::klines_url(sym, 2, None, None);
                match client.get(&url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        if let Ok(body) = resp.text().await {
                            let bars = parse_klines(&body);
                            let prev = *last_ts.get(sym).unwrap_or(&0);
                            for k in &bars {
                                if k.close_time_ms <= prev { continue; }
                                for ev in kline_events(sym, k) {
                                    let (s, e, f) = ev;
                                    on_event(s, e, f);
                                }
                            }
                            if let Some(last_bar) = bars.last() {
                                last_ts.insert(sym.clone(), last_bar.close_time_ms);
                            }
                        }
                    }
                    Ok(resp) => eprintln!("[binance-rest] {sym}: HTTP {}", resp.status()),
                    Err(e)   => eprintln!("[binance-rest] {sym}: poll error: {e}"),
                }
            }
        }
    }
}
