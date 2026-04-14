//! LOBSTER (Limit Order Book System — The Efficient Reconstructor) file reader.
//!
//! LOBSTER produces two CSV files per ticker per day:
//!   <TICKER>_<DATE>_<START>_<END>_message_<LEVELS>.csv  — order events
//!   <TICKER>_<DATE>_<START>_<END>_orderbook_<LEVELS>.csv — book states
//!
//! This reader uses only the message file to reconstruct the LOB incrementally,
//! producing normalized `Event`s compatible with the rest of the pipeline.
//!
//! Message file columns (no header):
//!   time (s, decimal nanoseconds), type, order_id, size, price (integer, price*10000), direction
//!
//! Event types:
//!   1 = new limit order
//!   2 = partial cancel
//!   3 = full cancel
//!   4 = visible execution (trade)
//!   5 = hidden execution (iceberg trade)
//!   7 = trading halt / auction trade
//!
//! Direction: 1 = buy (bid), -1 = sell (ask)
//!
//! Download from: https://lobsterdata.com  (free academic tier, 10-level data)
//! After registration, download any stock (e.g. AAPL 2012-06-21) and point
//! `--messages` at the _message_ file.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use ordered_float::OrderedFloat;

use crate::orderbook::{BookDelta, Side};
use crate::features::trade_pressure::TradeEvent;
use super::realdata::{DataQualityFlags, Event, TsSource};

/// Parsed row from a LOBSTER message file.
#[derive(Debug, Clone)]
pub struct LobsterMessage {
    /// Timestamp in nanoseconds since midnight.
    pub ts_ns: i64,
    pub event_type: u8,
    pub order_id: i64,
    pub size: f64,
    /// Price in dollars (converted from integer × 10⁻⁴).
    pub price: f64,
    /// 1 = bid, -1 = ask.
    pub direction: i32,
}

/// Convert a LOBSTER price integer (price × 10000) to f64.
fn lobster_price(raw: i64) -> f64 {
    raw as f64 / 10_000.0
}

/// Parse one CSV line from a LOBSTER message file.
///
/// Format: time,type,order_id,size,price,direction
pub fn parse_lobster_line(line: &str) -> Option<LobsterMessage> {
    let mut cols = line.split(',');
    let time_str = cols.next()?.trim();
    let type_str = cols.next()?.trim();
    let oid_str = cols.next()?.trim();
    let size_str = cols.next()?.trim();
    let price_str = cols.next()?.trim();
    let dir_str = cols.next()?.trim();

    // Time is seconds with decimal (e.g. "34200.000000000")
    let time_s: f64 = time_str.parse().ok()?;
    let ts_ns = (time_s * 1_000_000_000.0) as i64;

    let event_type: u8 = type_str.parse().ok()?;
    let order_id: i64 = oid_str.parse().ok()?;
    let size: f64 = size_str.parse::<f64>().ok()?;
    let price_raw: i64 = price_str.parse().ok()?;
    let price = lobster_price(price_raw);
    let direction: i32 = dir_str.parse().ok()?;

    // Type 7 (halt) has price=0 and direction=0 — allow it through
    if event_type != 7 && (!price.is_finite() || price <= 0.0) {
        return None;
    }

    Some(LobsterMessage { ts_ns, event_type, order_id, size, price, direction })
}

/// Convert a `LobsterMessage` to a normalized pipeline `Event`.
///
/// Returns (event, flags) or None for ignored event types (type 7 halt).
pub fn lobster_to_event(msg: &LobsterMessage) -> Option<(Event, DataQualityFlags)> {
    let flags = DataQualityFlags {
        is_imputed: false,
        gap_flag: false,
        is_outlier: false,
        ts_source: TsSource::Exchange,
    };

    // Handle type 7 (halt) before requiring a valid direction
    if msg.event_type == 7 {
        let ts_us = msg.ts_ns / 1000;
        return Some((
            Event::Gap { start_ts: ts_us, end_ts: ts_us },
            DataQualityFlags {
                is_imputed: true,
                gap_flag: true,
                is_outlier: false,
                ts_source: TsSource::Exchange,
            },
        ));
    }

    let side = match msg.direction {
        1 => Side::Bid,
        -1 => Side::Ask,
        _ => return None,
    };

    match msg.event_type {
        // Types 1, 2, 3: add / partial-cancel / full-cancel → book delta
        1 | 2 | 3 => {
            let qty = if msg.event_type == 3 { 0.0 } else { msg.size };
            Some((
                Event::BookDelta(BookDelta {
                    price: OrderedFloat(msg.price),
                    qty,
                    side,
                    exchange_ts: msg.ts_ns / 1000, // ns → µs
                    recv_ts: msg.ts_ns / 1000,
                }),
                flags,
            ))
        }

        // Types 4, 5: execution (visible or iceberg) → trade + delta
        4 | 5 => {
            // Execution removes size from the passive side
            // The aggressive side is the opposite of direction
            let aggressor_is_buy = side == Side::Ask; // Ask side passive → buy aggressor
            let trade = TradeEvent {
                ts: msg.ts_ns / 1000,
                price: msg.price,
                qty: msg.size,
                is_buy: aggressor_is_buy,
            };
            Some((Event::Trade(trade), flags))
        }

        _ => None,
    }
}

/// Stream events from a LOBSTER message file.
///
/// Calls `on_event` for each parsed event. Skips malformed lines with a warning.
pub fn replay_lobster_file<F>(
    messages_path: &Path,
    symbol: &str,
    mut on_event: F,
) -> anyhow::Result<u64>
where
    F: FnMut(Event, DataQualityFlags, &str),
{
    let file = File::open(messages_path)?;
    let reader = BufReader::new(file);
    let mut count = 0u64;
    let mut skipped = 0u64;

    for (line_no, line) in reader.lines().enumerate() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        match parse_lobster_line(line) {
            Some(msg) => {
                if let Some((event, flags)) = lobster_to_event(&msg) {
                    on_event(event, flags, symbol);
                    count += 1;
                }
            }
            None => {
                // Skip header line (some LOBSTER files have one) and malformed rows
                if line_no > 0 {
                    skipped += 1;
                    if skipped <= 5 {
                        eprintln!("[LOBSTER] skipped malformed line {line_no}: {line:.60}");
                    }
                }
            }
        }
    }

    if skipped > 5 {
        eprintln!("[LOBSTER] ... and {} more skipped lines", skipped - 5);
    }

    Ok(count)
}

/// Load the LOBSTER orderbook snapshot file to initialize the book state.
///
/// Orderbook file columns (no header, repeated for each level):
///   ask_price_1, ask_size_1, bid_price_1, bid_size_1, ask_price_2, ask_size_2, ...
///
/// Returns (bids, asks) as (price, qty) vectors, sorted best-first.
pub fn load_lobster_snapshot(
    orderbook_path: &Path,
    levels: usize,
) -> anyhow::Result<(Vec<(f64, f64)>, Vec<(f64, f64)>)> {
    let file = File::open(orderbook_path)?;
    let reader = BufReader::new(file);

    // Take the first non-empty line as the initial state
    for line in reader.lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let vals: Vec<f64> = line
            .split(',')
            .filter_map(|s| s.trim().parse::<f64>().ok())
            .collect();

        // Expected: levels × 4 columns (ask_price, ask_size, bid_price, bid_size)
        let expected = levels * 4;
        if vals.len() < expected {
            anyhow::bail!(
                "LOBSTER orderbook file has {} columns, expected {} for {} levels",
                vals.len(), expected, levels
            );
        }

        let mut bids = Vec::with_capacity(levels);
        let mut asks = Vec::with_capacity(levels);

        for i in 0..levels {
            let base = i * 4;
            let ask_price = lobster_price(vals[base] as i64);
            let ask_size = vals[base + 1];
            let bid_price = lobster_price(vals[base + 2] as i64);
            let bid_size = vals[base + 3];

            if ask_price > 0.0 && ask_size > 0.0 {
                asks.push((ask_price, ask_size));
            }
            if bid_price > 0.0 && bid_size > 0.0 {
                bids.push((bid_price, bid_size));
            }
        }

        // bids already sorted best-first (highest price first) from LOBSTER
        // asks already sorted best-first (lowest price first) from LOBSTER
        return Ok((bids, asks));
    }

    Ok((vec![], vec![]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_new_limit_bid() {
        // Type 1, direction 1 (bid), price 1234567 → $123.4567
        let line = "34200.000000000,1,12345,100,1234567,1";
        let msg = parse_lobster_line(line).unwrap();
        assert_eq!(msg.event_type, 1);
        assert_eq!(msg.direction, 1);
        assert!((msg.price - 123.4567).abs() < 1e-6);
        assert_eq!(msg.size, 100.0);
    }

    #[test]
    fn parse_full_cancel_ask() {
        let line = "34200.100000000,3,99999,50,2000000,-1";
        let msg = parse_lobster_line(line).unwrap();
        assert_eq!(msg.event_type, 3);
        assert_eq!(msg.direction, -1);
    }

    #[test]
    fn full_cancel_emits_zero_qty_delta() {
        let line = "34200.000000000,3,1,100,1000000,1";
        let msg = parse_lobster_line(line).unwrap();
        if let Some((Event::BookDelta(d), _)) = lobster_to_event(&msg) {
            assert!(d.is_delete(), "full cancel should produce qty=0 delta");
        } else {
            panic!("expected BookDelta");
        }
    }

    #[test]
    fn execution_emits_trade() {
        let line = "34200.500000000,4,2,200,1500000,1";
        let msg = parse_lobster_line(line).unwrap();
        if let Some((Event::Trade(t), _)) = lobster_to_event(&msg) {
            assert_eq!(t.qty, 200.0);
            assert!((t.price - 150.0).abs() < 1e-6);
            // direction=1 (bid passive) → aggressor is ask (sell)
            assert!(!t.is_buy);
        } else {
            panic!("expected Trade");
        }
    }

    #[test]
    fn halt_emits_gap() {
        let line = "34200.000000000,7,0,0,0,0";
        let msg = parse_lobster_line(line).unwrap();
        assert!(matches!(lobster_to_event(&msg), Some((Event::Gap { .. }, _))));
    }

    #[test]
    fn partial_cancel_keeps_size() {
        let line = "34200.000000000,2,5,50,2000000,-1";
        let msg = parse_lobster_line(line).unwrap();
        if let Some((Event::BookDelta(d), _)) = lobster_to_event(&msg) {
            assert_eq!(d.qty, 50.0);
        } else {
            panic!("expected BookDelta");
        }
    }

    #[test]
    fn malformed_line_returns_none() {
        assert!(parse_lobster_line("garbage,,not,valid").is_none());
        assert!(parse_lobster_line("").is_none());
    }
}
