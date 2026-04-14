//! OKX WebSocket — order book with 400 levels (books channel).
//!
//! Endpoint: wss://ws.okx.com:8443/ws/v5/public
//! Channel:  books  — 400 levels, 100ms updates, free, no auth required
//!
//! Protocol:
//!   1. Send subscribe message for `books` channel.
//!   2. First message has action="snapshot": replace entire book.
//!   3. Subsequent messages have action="update": apply as deltas.
//!      qty = "0" means remove that price level.
//!   4. Sequence validation: prevSeqId of new message must equal seqId of last.
//!      On mismatch: resubscribe (OKX will re-send snapshot).
//!   5. Checksum validation (CRC32 of top 25 bid/ask levels, optional).
//!   6. OKX sends a heartbeat "ping" every 30s; respond with "pong".
//!
//! Instrument ID format: "BTC-USDT" (spot), "BTC-USDT-SWAP" (perpetual).
//!
//! Channel options:
//!   books        — 400 levels, 100ms
//!   books5       — 5 levels, 100ms (snapshot only, no deltas)
//!   books50-l2-tbt — 50 levels, 10ms tick-by-tick

use ordered_float::OrderedFloat;
use serde::Deserialize;

use crate::orderbook::{BookDelta, Side};
use super::realdata::{DataQualityFlags, Event, TsSource};

// ── Message types ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct OkxWsMessage {
    /// "snapshot" or "update" (or absent for pong/subscribe ack)
    pub action: Option<String>,
    pub arg: Option<OkxArg>,
    pub data: Option<Vec<OkxBookData>>,
    /// "error" for subscription failures
    pub event: Option<String>,
    pub code: Option<String>,
    pub msg: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OkxArg {
    pub channel: String,
    #[serde(rename = "instId")]
    pub inst_id: String,
}

#[derive(Debug, Deserialize)]
pub struct OkxBookData {
    /// Bids as [[price, qty, deprecated, order_count], ...]
    pub bids: Vec<Vec<serde_json::Value>>,
    /// Asks as [[price, qty, deprecated, order_count], ...]
    pub asks: Vec<Vec<serde_json::Value>>,
    /// Exchange timestamp in milliseconds.
    pub ts: String,
    #[serde(rename = "seqId")]
    pub seq_id: Option<i64>,
    #[serde(rename = "prevSeqId")]
    pub prev_seq_id: Option<i64>,
    pub checksum: Option<i64>,
}

/// Sequence state for OKX delta validation.
pub struct OkxSeqState {
    pub last_seq_id: Option<i64>,
}

impl OkxSeqState {
    pub fn new() -> Self { Self { last_seq_id: None } }

    /// Returns true if this data packet is in sequence.
    pub fn validate(&mut self, data: &OkxBookData, is_snapshot: bool) -> bool {
        if is_snapshot {
            self.last_seq_id = data.seq_id;
            return true;
        }
        if let (Some(last), Some(prev)) = (self.last_seq_id, data.prev_seq_id) {
            if prev != last {
                self.last_seq_id = None;
                return false; // Gap
            }
        }
        self.last_seq_id = data.seq_id;
        true
    }

    pub fn reset(&mut self) { self.last_seq_id = None; }
}

impl Default for OkxSeqState {
    fn default() -> Self { Self::new() }
}

/// Parse one side of an OKX book data entry ([[price, qty, ...], ...]).
fn parse_okx_side(
    levels: &[Vec<serde_json::Value>],
    side: Side,
    ts_us: i64,
) -> Vec<BookDelta> {
    levels.iter().filter_map(|entry| {
        let price: f64 = entry.get(0)?.as_str()?.parse().ok()?;
        let qty: f64 = entry.get(1)?.as_str()?.parse().ok()?;
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

/// Parse an OKX WebSocket message into normalized events.
///
/// `seq_states`: per-symbol sequence state map; keyed by instrument ID.
pub fn parse_okx_message(
    text: &str,
    seq_states: &mut std::collections::HashMap<String, OkxSeqState>,
) -> Vec<(String, Event, DataQualityFlags)> {
    // Handle OKX ping/pong (plain text "pong")
    if text.trim() == "pong" {
        return vec![];
    }

    let msg: OkxWsMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(_) => return vec![],
    };

    // Subscription ack or error
    if let Some(event) = &msg.event {
        if event == "error" {
            eprintln!("[OKX] error: code={:?} msg={:?}", msg.code, msg.msg);
        }
        return vec![];
    }

    let inst_id = msg.arg.as_ref().map(|a| a.inst_id.clone()).unwrap_or_default();
    let action = msg.action.as_deref().unwrap_or("");
    let data_list = match msg.data {
        Some(d) => d,
        None => return vec![],
    };

    let mut out = Vec::new();

    for data in &data_list {
        let ts_ms: i64 = data.ts.parse().unwrap_or(0);
        let ts_us = ts_ms * 1000;

        let is_snapshot = action == "snapshot";
        let seq_state = seq_states.entry(inst_id.clone()).or_default();

        if !seq_state.validate(data, is_snapshot) {
            // Sequence gap — emit gap event, caller should resubscribe
            out.push((
                inst_id.clone(),
                Event::Gap { start_ts: ts_us - 1000, end_ts: ts_us },
                DataQualityFlags {
                    is_imputed: true, gap_flag: true,
                    is_outlier: false, ts_source: TsSource::Exchange,
                },
            ));
            continue;
        }

        if is_snapshot {
            // Collect all bids and asks from the full snapshot
            let bids: Vec<(f64, f64)> = data.bids.iter().filter_map(|e| {
                let p: f64 = e.get(0)?.as_str()?.parse().ok()?;
                let q: f64 = e.get(1)?.as_str()?.parse().ok()?;
                if p > 0.0 && q > 0.0 { Some((p, q)) } else { None }
            }).collect();
            let asks: Vec<(f64, f64)> = data.asks.iter().filter_map(|e| {
                let p: f64 = e.get(0)?.as_str()?.parse().ok()?;
                let q: f64 = e.get(1)?.as_str()?.parse().ok()?;
                if p > 0.0 && q > 0.0 { Some((p, q)) } else { None }
            }).collect();

            out.push((
                inst_id.clone(),
                Event::BookSnapshot { bids, asks, ts: ts_us },
                DataQualityFlags {
                    is_imputed: false, gap_flag: false,
                    is_outlier: false, ts_source: TsSource::Exchange,
                },
            ));
        } else {
            // Incremental update
            let flags = DataQualityFlags {
                is_imputed: false, gap_flag: false,
                is_outlier: false, ts_source: TsSource::Exchange,
            };
            for delta in parse_okx_side(&data.bids, Side::Bid, ts_us) {
                out.push((inst_id.clone(), Event::BookDelta(delta), flags.clone()));
            }
            for delta in parse_okx_side(&data.asks, Side::Ask, ts_us) {
                out.push((inst_id.clone(), Event::BookDelta(delta), flags.clone()));
            }
        }
    }

    out
}

/// Build subscribe message for OKX books channel for multiple instruments.
pub fn subscribe_msg(inst_ids: &[&str], channel: &str) -> String {
    let args: Vec<_> = inst_ids.iter().map(|id| {
        serde_json::json!({"channel": channel, "instId": id})
    }).collect();
    serde_json::json!({"op": "subscribe", "args": args}).to_string()
}

/// Convert slash pair to OKX instrument ID.
/// "BTC/USDT" → "BTC-USDT", "BTC-USDT" unchanged.
pub fn to_inst_id(pair: &str) -> String {
    pair.to_uppercase().replace('/', "-")
}

/// OKX WebSocket ingester.
#[cfg(feature = "ingest")]
pub struct OkxIngester {
    pub instruments: Vec<String>,
    /// Channel: "books" (400 levels), "books50-l2-tbt" (50 levels), "books5" (5 levels)
    pub channel: String,
    pub reconnect_delay_ms: u64,
    pub max_reconnect_delay_ms: u64,
}

#[cfg(feature = "ingest")]
impl OkxIngester {
    pub fn new(inst_id: &str) -> Self {
        Self::new_multi(&[inst_id])
    }

    pub fn new_multi(inst_ids: &[&str]) -> Self {
        Self {
            instruments: inst_ids.iter().map(|id| to_inst_id(id)).collect(),
            channel: "books".to_string(), // 400 levels by default
            reconnect_delay_ms: 250,
            max_reconnect_delay_ms: 30_000,
        }
    }

    pub fn with_channel(mut self, channel: &str) -> Self {
        self.channel = channel.to_string();
        self
    }

    pub fn ws_url(&self) -> &'static str {
        "wss://ws.okx.com:8443/ws/v5/public"
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
        let mut seq_states: HashMap<String, OkxSeqState> = HashMap::new();

        loop {
            match connect_ws(self.ws_url()).await {
                Ok((ws, _)) => {
                    delay = self.reconnect_delay_ms;
                    seq_states.clear();
                    let (mut write, mut read) = ws.split();

                    let inst_refs: Vec<&str> = self.instruments.iter().map(|s| s.as_str()).collect();
                    let sub = subscribe_msg(&inst_refs, &self.channel);
                    if write.send(Message::Text(sub.into())).await.is_err() {
                        continue;
                    }

                    while let Some(msg) = read.next().await {
                        match msg {
                            Ok(Message::Text(text)) => {
                                // OKX sends "ping" heartbeats, respond with "pong"
                                if text.trim() == "ping" {
                                    let _ = write.send(Message::Text("pong".into())).await;
                                    continue;
                                }
                                let events = parse_okx_message(&text, &mut seq_states);
                                let has_gap = events.iter().any(|(_, e, _)| matches!(e, Event::Gap { .. }));
                                for (inst_id, event, flags) in events {
                                    on_event(inst_id, event, flags);
                                }
                                // Resubscribe after gap (OKX will resend snapshot)
                                if has_gap {
                                    seq_states.clear();
                                    let inst_refs: Vec<&str> = self.instruments.iter().map(|s| s.as_str()).collect();
                                    let sub = subscribe_msg(&inst_refs, &self.channel);
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
                Err(e) => eprintln!("[OKX] connect failed: {e}"),
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

    fn make_snapshot_msg(bids: &[(&str, &str)], asks: &[(&str, &str)]) -> String {
        let bid_arr: Vec<Vec<serde_json::Value>> = bids.iter().map(|(p, q)| {
            vec![serde_json::Value::String(p.to_string()),
                 serde_json::Value::String(q.to_string()),
                 serde_json::Value::String("0".into()),
                 serde_json::Value::String("1".into())]
        }).collect();
        let ask_arr: Vec<Vec<serde_json::Value>> = asks.iter().map(|(p, q)| {
            vec![serde_json::Value::String(p.to_string()),
                 serde_json::Value::String(q.to_string()),
                 serde_json::Value::String("0".into()),
                 serde_json::Value::String("1".into())]
        }).collect();
        serde_json::json!({
            "action": "snapshot",
            "arg": {"channel": "books", "instId": "BTC-USDT"},
            "data": [{
                "bids": bid_arr,
                "asks": ask_arr,
                "ts": "1687940967466",
                "seqId": 1000,
                "prevSeqId": -1,
                "checksum": 0
            }]
        }).to_string()
    }

    #[test]
    fn parse_snapshot_emits_book_snapshot() {
        let msg = make_snapshot_msg(
            &[("30000.0", "1.5"), ("29999.0", "2.0")],
            &[("30001.0", "0.5"), ("30002.0", "1.0")],
        );
        let events = parse_okx_message(&msg, &mut HashMap::new());
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0].1, Event::BookSnapshot { .. }));
        if let Event::BookSnapshot { bids, asks, .. } = &events[0].1 {
            assert_eq!(bids.len(), 2);
            assert_eq!(asks.len(), 2);
        }
    }

    #[test]
    fn parse_update_emits_deltas() {
        let mut seq_states = HashMap::new();
        // First establish snapshot
        let snap = make_snapshot_msg(&[("30000.0", "1.5")], &[("30001.0", "0.5")]);
        parse_okx_message(&snap, &mut seq_states);

        let update = serde_json::json!({
            "action": "update",
            "arg": {"channel": "books", "instId": "BTC-USDT"},
            "data": [{
                "bids": [["30000.0", "2.0", "0", "1"]],
                "asks": [["30001.0", "0", "0", "0"]],
                "ts": "1687940967566",
                "seqId": 1001,
                "prevSeqId": 1000,
                "checksum": 0
            }]
        }).to_string();

        let events = parse_okx_message(&update, &mut seq_states);
        assert_eq!(events.len(), 2);
        // Second delta (ask qty=0) should be delete
        if let Event::BookDelta(d) = &events[1].1 {
            assert!(d.is_delete());
        }
    }

    #[test]
    fn sequence_gap_emits_gap_event() {
        let mut seq_states = HashMap::new();
        let snap = make_snapshot_msg(&[("30000.0", "1.5")], &[("30001.0", "0.5")]);
        parse_okx_message(&snap, &mut seq_states);

        // Update with wrong prevSeqId (gap)
        let bad_update = serde_json::json!({
            "action": "update",
            "arg": {"channel": "books", "instId": "BTC-USDT"},
            "data": [{
                "bids": [], "asks": [],
                "ts": "1687940967566",
                "seqId": 1005,
                "prevSeqId": 1003,  // Should be 1000 to be in sequence
                "checksum": 0
            }]
        }).to_string();

        let events = parse_okx_message(&bad_update, &mut seq_states);
        assert!(events.iter().any(|(_, e, _)| matches!(e, Event::Gap { .. })));
    }

    #[test]
    fn to_inst_id_conversion() {
        assert_eq!(to_inst_id("BTC/USDT"), "BTC-USDT");
        assert_eq!(to_inst_id("btc-usdt"), "BTC-USDT");
    }

    #[test]
    fn pong_returns_empty() {
        assert!(parse_okx_message("pong", &mut HashMap::new()).is_empty());
    }
}
