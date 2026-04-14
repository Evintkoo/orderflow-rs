//! Bybit WebSocket — order book up to 1000 levels.
//!
//! Endpoint: wss://stream.bybit.com/v5/public/spot   (spot)
//!           wss://stream.bybit.com/v5/public/linear  (USDT perpetual)
//!
//! Channel format: "orderbook.{depth}.{symbol}"
//!   Spot/linear depth options: 1 (10ms), 50 (20ms), 200 (100ms), 1000 (200ms)
//!   Options: 25 (20ms), 100 (100ms)
//!
//! Protocol:
//!   1. Subscribe with op="subscribe", args=["orderbook.200.BTCUSDT"]
//!   2. First message: type="snapshot" — full book
//!   3. Subsequent: type="delta" — only changed levels; size "0" = remove
//!   4. Sequence: `u` (update ID) must be prev_u + 1; `seq` is cross-sequence
//!   5. Bybit sends a heartbeat every 20s; respond to server ping with pong.
//!
//! Symbol format: "BTCUSDT" (no separator).

use ordered_float::OrderedFloat;
use serde::Deserialize;

use crate::orderbook::{BookDelta, Side};
use super::realdata::{DataQualityFlags, Event, TsSource};

// ── Message types ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct BybitWsMessage {
    pub topic: Option<String>,
    #[serde(rename = "type")]
    pub msg_type: Option<String>,
    /// Millisecond timestamp.
    pub ts: Option<i64>,
    /// Cross-sequence (for ordering across topics).
    pub cts: Option<i64>,
    pub data: Option<BybitBookData>,
    /// For op responses
    pub op: Option<String>,
    pub success: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct BybitBookData {
    /// Symbol
    pub s: String,
    /// Bids: [[price_str, qty_str], ...]
    pub b: Vec<[String; 2]>,
    /// Asks: [[price_str, qty_str], ...]
    pub a: Vec<[String; 2]>,
    /// Update ID (sequence within this topic).
    pub u: Option<u64>,
    /// Cross-sequence number.
    pub seq: Option<u64>,
}

/// Sequence state for Bybit update ID validation.
pub struct BybitSeqState {
    pub last_u: Option<u64>,
}

impl BybitSeqState {
    pub fn new() -> Self { Self { last_u: None } }

    pub fn validate(&mut self, data: &BybitBookData, is_snapshot: bool) -> bool {
        if is_snapshot {
            self.last_u = data.u;
            return true;
        }
        if let (Some(last), Some(curr)) = (self.last_u, data.u) {
            if curr != last + 1 {
                self.last_u = None;
                return false; // Gap
            }
        }
        self.last_u = data.u;
        true
    }

    pub fn reset(&mut self) { self.last_u = None; }
}

impl Default for BybitSeqState {
    fn default() -> Self { Self::new() }
}

/// Parse [[price, qty], ...] array into BookDeltas.
fn parse_bybit_levels(
    levels: &[[String; 2]],
    side: Side,
    ts_us: i64,
) -> Vec<BookDelta> {
    levels.iter().filter_map(|level| {
        let price: f64 = level[0].parse().ok()?;
        let qty: f64 = level[1].parse().ok()?;
        if price <= 0.0 || !price.is_finite() { return None; }
        Some(BookDelta {
            price: OrderedFloat(price),
            qty,
            side,
            exchange_ts: ts_us,
            recv_ts: ts_us,
        })
    }).collect()
}

/// Parse a raw Bybit WebSocket message into normalized events.
pub fn parse_bybit_message(
    text: &str,
    seq_states: &mut std::collections::HashMap<String, BybitSeqState>,
) -> Vec<(String, Event, DataQualityFlags)> {
    let msg: BybitWsMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(_) => return vec![],
    };

    // Op response (subscribe ack)
    if msg.op.is_some() {
        if msg.success == Some(false) {
            eprintln!("[BYBIT] subscribe failed");
        }
        return vec![];
    }

    let msg_type = msg.msg_type.as_deref().unwrap_or("");
    let data = match msg.data {
        Some(d) => d,
        None => return vec![],
    };

    let sym = data.s.clone();
    let ts_us = msg.ts.unwrap_or(0) * 1000; // ms → µs
    let is_snapshot = msg_type == "snapshot";

    let seq_state = seq_states.entry(sym.clone()).or_default();

    if !seq_state.validate(&data, is_snapshot) {
        return vec![(
            sym,
            Event::Gap { start_ts: ts_us - 1000, end_ts: ts_us },
            DataQualityFlags {
                is_imputed: true, gap_flag: true,
                is_outlier: false, ts_source: TsSource::Exchange,
            },
        )];
    }

    if is_snapshot {
        let bids: Vec<(f64, f64)> = data.b.iter().filter_map(|l| {
            let p: f64 = l[0].parse().ok()?;
            let q: f64 = l[1].parse().ok()?;
            if p > 0.0 && q > 0.0 { Some((p, q)) } else { None }
        }).collect();
        let asks: Vec<(f64, f64)> = data.a.iter().filter_map(|l| {
            let p: f64 = l[0].parse().ok()?;
            let q: f64 = l[1].parse().ok()?;
            if p > 0.0 && q > 0.0 { Some((p, q)) } else { None }
        }).collect();

        vec![(
            sym,
            Event::BookSnapshot { bids, asks, ts: ts_us },
            DataQualityFlags {
                is_imputed: false, gap_flag: false,
                is_outlier: false, ts_source: TsSource::Exchange,
            },
        )]
    } else {
        // Delta update
        let flags = DataQualityFlags {
            is_imputed: false, gap_flag: false,
            is_outlier: false, ts_source: TsSource::Exchange,
        };
        let mut out = Vec::new();
        for delta in parse_bybit_levels(&data.b, Side::Bid, ts_us) {
            out.push((data.s.clone(), Event::BookDelta(delta), flags.clone()));
        }
        for delta in parse_bybit_levels(&data.a, Side::Ask, ts_us) {
            out.push((data.s.clone(), Event::BookDelta(delta), flags.clone()));
        }
        out
    }
}

/// Build Bybit subscribe message for multiple symbols.
pub fn subscribe_msg(symbols: &[&str], depth: u32) -> String {
    let topics: Vec<String> = symbols.iter().map(|s| {
        format!("orderbook.{depth}.{}", s.to_uppercase().replace(['-', '/'], ""))
    }).collect();
    serde_json::json!({"op": "subscribe", "args": topics}).to_string()
}

/// Determine the correct Bybit WebSocket URL by market type.
pub fn ws_url(market: BybitMarket) -> &'static str {
    match market {
        BybitMarket::Spot    => "wss://stream.bybit.com/v5/public/spot",
        BybitMarket::Linear  => "wss://stream.bybit.com/v5/public/linear",
        BybitMarket::Inverse => "wss://stream.bybit.com/v5/public/inverse",
        BybitMarket::Option  => "wss://stream.bybit.com/v5/public/option",
    }
}

#[derive(Debug, Clone, Copy)]
pub enum BybitMarket {
    Spot,
    Linear,  // USDT perpetual / USDT futures
    Inverse, // Coin-margined
    Option,
}

/// Bybit WebSocket ingester.
#[cfg(feature = "ingest")]
pub struct BybitIngester {
    pub symbols: Vec<String>,
    pub depth: u32,
    pub market: BybitMarket,
    pub reconnect_delay_ms: u64,
    pub max_reconnect_delay_ms: u64,
}

#[cfg(feature = "ingest")]
impl BybitIngester {
    pub fn new(symbol: &str) -> Self {
        Self::new_multi(&[symbol])
    }

    pub fn new_multi(symbols: &[&str]) -> Self {
        Self {
            symbols: symbols.iter().map(|s| s.to_uppercase().replace(['-', '/'], "")).collect(),
            depth: 200,
            market: BybitMarket::Spot,
            reconnect_delay_ms: 250,
            max_reconnect_delay_ms: 30_000,
        }
    }

    pub fn with_depth(mut self, depth: u32) -> Self {
        self.depth = depth;
        self
    }

    pub fn with_market(mut self, market: BybitMarket) -> Self {
        self.market = market;
        self
    }

    pub async fn run<F>(&mut self, mut on_event: F) -> anyhow::Result<()>
    where
        F: FnMut(String, Event, DataQualityFlags),
    {
        use std::collections::HashMap;
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        use super::ws::connect_ws;

        let mut delay = self.reconnect_delay_ms;
        let mut seq_states: HashMap<String, BybitSeqState> = HashMap::new();

        loop {
            let url = ws_url(self.market);
            match connect_ws(url).await {
                Ok((ws, _)) => {
                    delay = self.reconnect_delay_ms;
                    seq_states.clear();
                    let (mut write, mut read) = ws.split();

                    let sym_refs: Vec<&str> = self.symbols.iter().map(|s| s.as_str()).collect();
                    let sub = subscribe_msg(&sym_refs, self.depth);
                    if write.send(Message::Text(sub.into())).await.is_err() {
                        continue;
                    }

                    // Bybit heartbeat: send {"op":"ping"} every 20s
                    let mut last_ping = tokio::time::Instant::now();

                    while let Some(msg) = read.next().await {
                        // Send heartbeat if due
                        if last_ping.elapsed().as_secs() >= 20 {
                            let _ = write.send(Message::Text(
                                r#"{"op":"ping"}"#.into()
                            )).await;
                            last_ping = tokio::time::Instant::now();
                        }

                        match msg {
                            Ok(Message::Text(text)) => {
                                let events = parse_bybit_message(&text, &mut seq_states);
                                let has_gap = events.iter().any(|(_, e, _)| matches!(e, Event::Gap { .. }));
                                for (sym, event, flags) in events {
                                    on_event(sym, event, flags);
                                }
                                if has_gap {
                                    // Resubscribe to get a fresh snapshot
                                    seq_states.clear();
                                    let sym_refs: Vec<&str> = self.symbols.iter().map(|s| s.as_str()).collect();
                                    let sub = subscribe_msg(&sym_refs, self.depth);
                                    let _ = write.send(Message::Text(sub.into())).await;
                                }
                            }
                            Ok(Message::Ping(p)) => {
                                let _ = write.send(Message::Pong(p)).await;
                            }
                            Ok(Message::Close(_)) | Err(_) => break,
                            _ => {}
                        }
                    }
                }
                Err(e) => eprintln!("[BYBIT] connect failed: {e}"),
            }

            on_event(
                String::new(),
                Event::Disconnected,
                DataQualityFlags {
                    is_imputed: true, gap_flag: true,
                    is_outlier: false, ts_source: TsSource::Local,
                },
            );
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            delay = (delay * 2).min(self.max_reconnect_delay_ms);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn snapshot_msg(bids: &[(&str, &str)], asks: &[(&str, &str)], u: u64) -> String {
        let b: Vec<[String; 2]> = bids.iter().map(|(p, q)| [p.to_string(), q.to_string()]).collect();
        let a: Vec<[String; 2]> = asks.iter().map(|(p, q)| [p.to_string(), q.to_string()]).collect();
        serde_json::json!({
            "topic": "orderbook.50.BTCUSDT",
            "type": "snapshot",
            "ts": 1672304484978_i64,
            "data": { "s": "BTCUSDT", "b": b, "a": a, "u": u, "seq": u }
        }).to_string()
    }

    fn delta_msg(bids: &[(&str, &str)], asks: &[(&str, &str)], u: u64) -> String {
        let b: Vec<[String; 2]> = bids.iter().map(|(p, q)| [p.to_string(), q.to_string()]).collect();
        let a: Vec<[String; 2]> = asks.iter().map(|(p, q)| [p.to_string(), q.to_string()]).collect();
        serde_json::json!({
            "topic": "orderbook.50.BTCUSDT",
            "type": "delta",
            "ts": 1672304485000_i64,
            "data": { "s": "BTCUSDT", "b": b, "a": a, "u": u, "seq": u }
        }).to_string()
    }

    #[test]
    fn snapshot_produces_book_snapshot_event() {
        let msg = snapshot_msg(&[("30000.0", "1.5"), ("29999.0", "2.0")],
                               &[("30001.0", "0.5")], 1);
        let events = parse_bybit_message(&msg, &mut HashMap::new());
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0].1, Event::BookSnapshot { .. }));
        if let Event::BookSnapshot { bids, asks, .. } = &events[0].1 {
            assert_eq!(bids.len(), 2);
            assert_eq!(asks.len(), 1);
        }
    }

    #[test]
    fn delta_produces_book_deltas() {
        let mut seq_states = HashMap::new();
        parse_bybit_message(&snapshot_msg(&[("30000.0", "1.5")], &[("30001.0", "0.5")], 1), &mut seq_states);
        let events = parse_bybit_message(&delta_msg(&[("30000.0", "2.0")], &[("30001.0", "0")], 2), &mut seq_states);
        assert_eq!(events.len(), 2);
        if let Event::BookDelta(d) = &events[1].1 {
            assert!(d.is_delete()); // ask qty=0
        }
    }

    #[test]
    fn gap_in_sequence_emits_gap_event() {
        let mut seq_states = HashMap::new();
        parse_bybit_message(&snapshot_msg(&[("30000.0", "1.0")], &[("30001.0", "1.0")], 10), &mut seq_states);
        // u should be 11, but we send 13
        let events = parse_bybit_message(&delta_msg(&[], &[], 13), &mut seq_states);
        assert!(events.iter().any(|(_, e, _)| matches!(e, Event::Gap { .. })));
    }

    #[test]
    fn subscribe_msg_format() {
        let msg: serde_json::Value = serde_json::from_str(&subscribe_msg(&["BTCUSDT"], 200)).unwrap();
        assert_eq!(msg["op"], "subscribe");
        assert!(msg["args"][0].as_str().unwrap().contains("200"));
    }
}
