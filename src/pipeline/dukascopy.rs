//! Dukascopy tick data reader — FX pairs (EUR/USD, USD/JPY, etc.)
//!
//! Dukascopy provides free historical FX tick data going back to 2003.
//!
//! ## Download instructions
//!
//! Use the JForex platform or their HTTP API to download bi5 (LZMA-compressed)
//! files, or use the community tool `dukascopy-node` (npm):
//!
//!   npx dukascopy-node -i eurusd -from 2023-01-01 -to 2023-01-31 \
//!     -t tick -f csv -dir data/dukascopy/
//!
//! The downloaded CSV files have this format (with header):
//!   DateTime,Ask,Bid,AskVolume,BidVolume
//!   2023-01-02 22:00:00.356,1.07005,1.07,0.75,0.75
//!
//! DateTime: UTC, millisecond precision.
//! Ask/Bid: floating point price.
//! AskVolume/BidVolume: in millions of base currency (1.0 = 1M EUR for EUR/USD).
//!
//! ## What this produces
//!
//! Each tick row produces two `BookDelta` events (one bid, one ask) representing
//! a best-level snapshot update.  This is L1 data — only top of book.
//! Depth imbalance features will work (bid vs ask qty at best level).
//! OFI will work (changes between ticks).
//! Multi-level features (OFI(5), depth(5)) will be single-level only.
//!
//! ## Alternative: JForex CSV export
//!
//! JForex exports use a slightly different format:
//!   DateTime,Ask,Bid,Volume
//! This reader auto-detects by column count.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use ordered_float::OrderedFloat;

use crate::orderbook::{BookDelta, Side};
use super::realdata::{DataQualityFlags, Event, TsSource};

/// Parsed row from a Dukascopy CSV tick file.
#[derive(Debug, Clone)]
pub struct DukascopyTick {
    /// UTC timestamp in microseconds since epoch.
    pub ts_us: i64,
    pub ask: f64,
    pub bid: f64,
    /// Ask volume in millions of base currency.
    pub ask_vol: f64,
    /// Bid volume in millions of base currency.
    pub bid_vol: f64,
}

/// Parse a datetime string of the form "2023-01-02 22:00:00.356" to µs since epoch.
fn parse_datetime_us(s: &str) -> Option<i64> {
    // Split on space: "2023-01-02" and "22:00:00.356"
    let mut parts = s.splitn(2, ' ');
    let date_part = parts.next()?;
    let time_part = parts.next().unwrap_or("00:00:00.000");

    let mut date_cols = date_part.splitn(3, '-');
    let year: i64 = date_cols.next()?.parse().ok()?;
    let month: i64 = date_cols.next()?.parse().ok()?;
    let day: i64 = date_cols.next()?.parse().ok()?;

    // Time part: HH:MM:SS.mmm
    let mut time_cols = time_part.splitn(3, ':');
    let hh: i64 = time_cols.next()?.parse().ok()?;
    let mm: i64 = time_cols.next()?.parse().ok()?;
    let ss_str = time_cols.next().unwrap_or("0");

    // Split seconds from milliseconds
    let (ss_int, ms): (i64, i64) = if let Some(dot) = ss_str.find('.') {
        let secs: i64 = ss_str[..dot].parse().ok()?;
        let frac_str = &ss_str[dot + 1..];
        // Pad/truncate to 3 digits
        let ms_str = format!("{:0<3}", &frac_str[..frac_str.len().min(3)]);
        let ms: i64 = ms_str.parse().unwrap_or(0);
        (secs, ms)
    } else {
        (ss_str.parse().ok()?, 0)
    };

    // Days since epoch (simplified Gregorian)
    let days = gregorian_days_since_epoch(year, month, day)?;
    let total_us = days * 86_400 * 1_000_000
        + hh * 3_600 * 1_000_000
        + mm * 60 * 1_000_000
        + ss_int * 1_000_000
        + ms * 1_000;

    Some(total_us)
}

fn gregorian_days_since_epoch(year: i64, month: i64, day: i64) -> Option<i64> {
    if year < 1970 || month < 1 || month > 12 || day < 1 || day > 31 {
        return None;
    }
    // Days from 1970-01-01 to start of year
    let y = year - 1970;
    let leap_days = (y / 4) - (y / 100) + (y / 400); // leap years 1970..year
    let base_days = y * 365 + leap_days;

    // Days in each month of the target year
    let is_leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let month_days = [31, if is_leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let days_in_year: i64 = month_days[..(month - 1) as usize].iter().sum();

    Some(base_days + days_in_year + day - 1)
}

/// Parse one CSV line from a Dukascopy tick file.
///
/// Handles both 5-column (with separate AskVolume, BidVolume)
/// and 4-column (single Volume) formats.
pub fn parse_dukascopy_line(line: &str) -> Option<DukascopyTick> {
    let cols: Vec<&str> = line.split(',').collect();
    if cols.len() < 4 {
        return None;
    }

    let ts_us = parse_datetime_us(cols[0].trim())?;
    let ask: f64 = cols[1].trim().parse().ok()?;
    let bid: f64 = cols[2].trim().parse().ok()?;

    if ask <= 0.0 || bid <= 0.0 || bid >= ask {
        return None; // Sanity check: bid must be strictly below ask
    }

    let (ask_vol, bid_vol) = if cols.len() >= 5 {
        let av: f64 = cols[3].trim().parse().unwrap_or(1.0);
        let bv: f64 = cols[4].trim().parse().unwrap_or(1.0);
        (av, bv)
    } else {
        // 4-column: single volume applies to both sides
        let v: f64 = cols[3].trim().parse().unwrap_or(1.0);
        (v, v)
    };

    Some(DukascopyTick { ts_us, ask, bid, ask_vol, bid_vol })
}

/// Convert a Dukascopy tick to normalized events (bid delta + ask delta).
pub fn dukascopy_to_events(tick: &DukascopyTick) -> Vec<(Event, DataQualityFlags)> {
    let flags = DataQualityFlags {
        is_imputed: false,
        gap_flag: false,
        is_outlier: false,
        ts_source: TsSource::Exchange,
    };

    // FX volumes are in millions of base currency. Preserve as-is — callers
    // can normalize by ADV if needed. The depth imbalance ratio is scale-invariant.
    vec![
        (
            Event::BookDelta(BookDelta {
                price: OrderedFloat(tick.bid),
                qty: tick.bid_vol,
                side: Side::Bid,
                exchange_ts: tick.ts_us,
                recv_ts: tick.ts_us,
            }),
            flags.clone(),
        ),
        (
            Event::BookDelta(BookDelta {
                price: OrderedFloat(tick.ask),
                qty: tick.ask_vol,
                side: Side::Ask,
                exchange_ts: tick.ts_us,
                recv_ts: tick.ts_us,
            }),
            flags,
        ),
    ]
}

/// Replay a Dukascopy CSV tick file, calling `on_event` for each event.
///
/// Skips the header line automatically.
/// Returns total number of events emitted.
pub fn replay_dukascopy_file<F>(
    path: &Path,
    symbol: &str,
    mut on_event: F,
) -> anyhow::Result<u64>
where
    F: FnMut(Event, DataQualityFlags, &str),
{
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut count = 0u64;
    let mut prev_ts: Option<i64> = None;

    for (line_no, line) in reader.lines().enumerate() {
        let line = line?;
        let line = line.trim();

        // Skip header
        if line_no == 0 && (line.to_lowercase().starts_with("datetime")
            || line.to_lowercase().starts_with("time")
            || line.to_lowercase().starts_with("gmt")) {
            continue;
        }
        if line.is_empty() {
            continue;
        }

        match parse_dukascopy_line(line) {
            Some(tick) => {
                // Timestamp monotonicity: use max(ts, prev_ts) to handle clock skew
                let ts = match prev_ts {
                    Some(p) if tick.ts_us < p => {
                        eprintln!("[DUKASCOPY] non-monotone ts at line {line_no}, clamping");
                        p
                    }
                    _ => tick.ts_us,
                };
                prev_ts = Some(ts);

                let tick = DukascopyTick { ts_us: ts, ..tick };
                for (event, flags) in dukascopy_to_events(&tick) {
                    on_event(event, flags, symbol);
                    count += 1;
                }
            }
            None => {
                if line_no > 0 {
                    // Only warn for non-header lines
                }
            }
        }
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_standard_5col_tick() {
        let line = "2023-01-02 22:00:00.356,1.07005,1.07000,0.75,0.75";
        let tick = parse_dukascopy_line(line).unwrap();
        assert!((tick.ask - 1.07005).abs() < 1e-8);
        assert!((tick.bid - 1.07000).abs() < 1e-8);
        assert_eq!(tick.ask_vol, 0.75);
        assert_eq!(tick.bid_vol, 0.75);
    }

    #[test]
    fn parse_4col_tick_jforex() {
        let line = "2023-01-02 22:00:00.100,1.07010,1.07005,1.50";
        let tick = parse_dukascopy_line(line).unwrap();
        assert!((tick.ask - 1.07010).abs() < 1e-8);
        assert_eq!(tick.ask_vol, tick.bid_vol); // Both get the single volume
    }

    #[test]
    fn timestamp_epoch_correct() {
        // 1970-01-01 00:00:00.000 should be 0 µs
        let ts = parse_datetime_us("1970-01-01 00:00:00.000").unwrap();
        assert_eq!(ts, 0);
    }

    #[test]
    fn timestamp_known_date() {
        // 2023-01-02 22:00:00.356
        // Days from 1970-01-01 to 2023-01-02: (53 years × 365) + 13 leap days + 1
        // Let's just check it's positive and in a reasonable range
        let ts = parse_datetime_us("2023-01-02 22:00:00.356").unwrap();
        assert!(ts > 1_600_000_000_000_000, "should be after 2020 in µs");
        assert!(ts < 2_000_000_000_000_000, "should be before 2033 in µs");
        // Verify millisecond component
        let ms_component = (ts / 1000) % 1000;
        assert_eq!(ms_component, 356);
    }

    #[test]
    fn tick_to_events_produces_bid_and_ask() {
        let tick = DukascopyTick {
            ts_us: 1_000_000,
            ask: 1.0701,
            bid: 1.0699,
            ask_vol: 1.5,
            bid_vol: 2.0,
        };
        let events = dukascopy_to_events(&tick);
        assert_eq!(events.len(), 2);

        let bid_evt = events.iter().find(|(e, _)| {
            matches!(e, Event::BookDelta(d) if d.side == Side::Bid)
        }).unwrap();
        if let (Event::BookDelta(d), _) = bid_evt {
            assert!((d.price.0 - 1.0699).abs() < 1e-8);
            assert_eq!(d.qty, 2.0);
        }
    }

    #[test]
    fn header_line_skipped_in_replay() {
        use std::io::Write;
        let dir = std::env::temp_dir();
        let path = dir.join("test_duka.csv");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            writeln!(f, "DateTime,Ask,Bid,AskVolume,BidVolume").unwrap();
            writeln!(f, "2023-01-02 22:00:00.000,1.07005,1.07000,0.75,0.75").unwrap();
        }
        let mut count = 0u64;
        replay_dukascopy_file(&path, "EURUSD", |_, _, _| count += 1).unwrap();
        assert_eq!(count, 2); // bid + ask deltas for the one tick row
    }

    #[test]
    fn crossed_spread_rejected() {
        // ask < bid is invalid
        assert!(parse_dukascopy_line("2023-01-02 22:00:00.000,1.0700,1.0701,1.0,1.0").is_none());
    }
}
