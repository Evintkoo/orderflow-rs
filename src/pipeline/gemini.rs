//! Gemini WebSocket v1 — per-symbol full-depth order book.
//!
//! Endpoint: wss://api.gemini.com/v1/marketdata/{SYMBOL}?heartbeat=true
//!
//! No subscription needed — the full book snapshot arrives as the first message,
//! then incremental "change" events update individual price levels.
//!
//! Protocol:
//!   1. Connect to wss://api.gemini.com/v1/marketdata/BTCUSD?heartbeat=true
//!   2. First message: type="update", socket_sequence=0, events with reason="initial"
//!      — this IS the full snapshot.
//!   3. Subsequent: type="update", events with reason="place"/"cancel"/"fill"/"initial"
//!      remaining="0" means the level is gone.
//!   4. Heartbeat: type="heartbeat" — no response required.
//!   5. Gap: socket_sequence must be consecutive; if not, reconnect.
//!
//! Symbol format: "BTCUSD" (uppercase, no separator).
//! Works in the US without a VPN.

use ordered_float::OrderedFloat;
use serde::Deserialize;

use crate::orderbook::{BookDelta, Side};
use super::realdata::{DataQualityFlags, Event, TsSource};

// ── Message types ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct GeminiMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    /// ms since epoch (only present on update messages)
    pub timestampms: Option<i64>,
    pub socket_sequence: Option<u64>,
    pub events: Option<Vec<GeminiEvent>>,
}

#[derive(Debug, Deserialize)]
pub struct GeminiEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    // -- change --
    pub reason: Option<String>,
    pub price: Option<String>,
    pub remaining: Option<String>,
    pub side: Option<String>,
    // -- trade --
    pub tid: Option<u64>,
    pub amount: Option<String>,
    #[serde(rename = "makerSide")]
    pub maker_side: Option<String>,
}

// ── Sequence state ────────────────────────────────────────────────────────────

pub struct GeminiSeqState {
    pub last_seq: Option<u64>,
}

impl GeminiSeqState {
    pub fn new() -> Self { Self { last_seq: None } }

    /// Returns false if a gap is detected.
    pub fn validate(&mut self, seq: u64) -> bool {
        if let Some(last) = self.last_seq {
            if seq != last + 1 {
                self.last_seq = None;
                return false;
            }
        }
        self.last_seq = Some(seq);
        true
    }

    pub fn reset(&mut self) { self.last_seq = None; }
}

impl Default for GeminiSeqState { fn default() -> Self { Self::new() } }

// ── Parser ────────────────────────────────────────────────────────────────────

pub fn parse_gemini_message(
    text: &str,
    seq_state: &mut GeminiSeqState,
) -> Vec<(Event, DataQualityFlags)> {
    let msg: GeminiMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(_) => return vec![],
    };

    let ts_us = msg.timestampms.unwrap_or(0) * 1_000;

    // Advance sequence counter for ALL message types (heartbeats count toward seq).
    if let Some(seq) = msg.socket_sequence {
        if !seq_state.validate(seq) {
            return vec![(
                Event::Gap { start_ts: ts_us - 1_000, end_ts: ts_us },
                DataQualityFlags {
                    is_imputed: true, gap_flag: true,
                    is_outlier: false, ts_source: TsSource::Exchange,
                },
            )];
        }
    }

    if msg.msg_type == "heartbeat" { return vec![]; }
    if msg.msg_type != "update" { return vec![]; }

    let events = match msg.events {
        Some(e) => e,
        None => return vec![],
    };

    let flags = DataQualityFlags {
        is_imputed: false, gap_flag: false,
        is_outlier: false, ts_source: TsSource::Exchange,
    };

    // Collect all "change" events.  reason="initial" means it's part of the snapshot.
    let mut is_snapshot_batch = false;
    let mut snapshot_bids: Vec<(f64, f64)> = Vec::new();
    let mut snapshot_asks: Vec<(f64, f64)> = Vec::new();
    let mut deltas: Vec<(Event, DataQualityFlags)> = Vec::new();

    for ev in &events {
        match ev.event_type.as_str() {
            "change" => {
                let reason = ev.reason.as_deref().unwrap_or("");
                let price: f64 = match ev.price.as_deref().and_then(|s| s.parse().ok()) {
                    Some(p) if p > 0.0 => p,
                    _ => continue,
                };
                let remaining: f64 = ev.remaining.as_deref()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.0);
                let side = match ev.side.as_deref() {
                    Some("bid") => Side::Bid,
                    Some("ask") => Side::Ask,
                    _ => continue,
                };

                if reason == "initial" {
                    // Part of the initial snapshot batch
                    is_snapshot_batch = true;
                    if remaining > 0.0 {
                        match side {
                            Side::Bid => snapshot_bids.push((price, remaining)),
                            Side::Ask => snapshot_asks.push((price, remaining)),
                        }
                    }
                } else {
                    // Incremental delta
                    deltas.push((
                        Event::BookDelta(BookDelta {
                            price: OrderedFloat(price),
                            qty: remaining,
                            side,
                            exchange_ts: ts_us,
                            recv_ts: ts_us,
                        }),
                        flags.clone(),
                    ));
                }
            }
            "trade" => {
                // We emit trades via the TradeEvent path when amount is available
                // For now, ignored (Gemini trade events can be added later)
            }
            _ => {}
        }
    }

    if is_snapshot_batch {
        vec![(Event::BookSnapshot {
            bids: snapshot_bids,
            asks: snapshot_asks,
            ts: ts_us,
        }, flags)]
    } else {
        deltas
    }
}

/// Normalise symbol to Gemini format: "BTC/USD" or "BTC-USD" → "BTCUSD".
pub fn to_gemini_symbol(s: &str) -> String {
    s.to_uppercase().replace(['/', '-', '_'], "")
}

// ── Live ingester ─────────────────────────────────────────────────────────────

#[cfg(feature = "ingest")]
pub struct GeminiIngester {
    pub symbols: Vec<String>,
    pub reconnect_delay_ms: u64,
    pub max_reconnect_delay_ms: u64,
}

#[cfg(feature = "ingest")]
impl GeminiIngester {
    pub fn new(symbol: &str) -> Self {
        Self {
            symbols: vec![to_gemini_symbol(symbol)],
            reconnect_delay_ms: 250,
            max_reconnect_delay_ms: 30_000,
        }
    }

    pub fn new_multi(symbols: &[&str]) -> Self {
        Self {
            symbols: symbols.iter().map(|s| to_gemini_symbol(s)).collect(),
            reconnect_delay_ms: 250,
            max_reconnect_delay_ms: 30_000,
        }
    }

    /// Run one WS task per symbol (Gemini has per-symbol endpoints).
    /// Callback: `FnMut(symbol, Event, DataQualityFlags)`.
    pub async fn run<F>(&mut self, on_event: F) -> anyhow::Result<()>
    where
        F: FnMut(String, Event, DataQualityFlags) + Send + 'static,
    {
        use futures_util::StreamExt;
        use std::sync::{Arc, Mutex};
        use tokio_tungstenite::tungstenite::Message;
        use super::ws::connect_ws;

        // Wrap callback in Arc<Mutex<>> so it can be shared across per-symbol tasks.
        let cb = Arc::new(Mutex::new(on_event));

        // Spawn a task per symbol; run them concurrently.
        let mut handles = Vec::new();
        for sym in self.symbols.clone() {
            let delay_init = self.reconnect_delay_ms;
            let max_delay = self.max_reconnect_delay_ms;
            let cb = Arc::clone(&cb);

            handles.push(tokio::spawn(async move {
                let mut delay = delay_init;
                loop {
                    let url = format!("wss://api.gemini.com/v1/marketdata/{}?heartbeat=true", sym);
                    match connect_ws(&url).await {
                        Ok((ws, _)) => {
                            delay = delay_init;
                            let mut seq = GeminiSeqState::new();
                            let (_, mut read) = ws.split();

                            while let Some(msg) = read.next().await {
                                match msg {
                                    Ok(Message::Text(text)) => {
                                        let events = parse_gemini_message(&text, &mut seq);
                                        let has_gap = events.iter().any(|(e, _)| matches!(e, Event::Gap { .. }));
                                        for (event, flags) in events {
                                            cb.lock().unwrap()(sym.clone(), event, flags);
                                        }
                                        if has_gap { break; }
                                    }
                                    Ok(Message::Close(_)) | Err(_) => break,
                                    _ => {}
                                }
                            }
                        }
                        Err(e) => eprintln!("[GEMINI/{sym}] connect failed: {e}"),
                    }

                    cb.lock().unwrap()(sym.clone(), Event::Disconnected, DataQualityFlags {
                        is_imputed: true, gap_flag: true,
                        is_outlier: false, ts_source: TsSource::Local,
                    });
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    delay = (delay * 2).min(max_delay);
                }
            }));
        }

        // Wait for all tasks (they run forever)
        for h in handles { let _ = h.await; }
        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn initial_update(bids: &[(&str, &str)], asks: &[(&str, &str)], seq: u64) -> String {
        let mut events = Vec::new();
        for (p, r) in bids {
            events.push(serde_json::json!({"type":"change","reason":"initial","price":p,"remaining":r,"side":"bid"}));
        }
        for (p, r) in asks {
            events.push(serde_json::json!({"type":"change","reason":"initial","price":p,"remaining":r,"side":"ask"}));
        }
        serde_json::json!({
            "type": "update",
            "timestampms": 1_700_000_000_000_i64,
            "socket_sequence": seq,
            "events": events
        }).to_string()
    }

    fn delta_update(bids: &[(&str, &str)], asks: &[(&str, &str)], seq: u64) -> String {
        let mut events = Vec::new();
        for (p, r) in bids {
            events.push(serde_json::json!({"type":"change","reason":"place","price":p,"remaining":r,"side":"bid"}));
        }
        for (p, r) in asks {
            events.push(serde_json::json!({"type":"change","reason":"cancel","price":p,"remaining":r,"side":"ask"}));
        }
        serde_json::json!({
            "type": "update",
            "timestampms": 1_700_000_001_000_i64,
            "socket_sequence": seq,
            "events": events
        }).to_string()
    }

    #[test]
    fn initial_batch_emits_snapshot() {
        let mut seq = GeminiSeqState::new();
        let events = parse_gemini_message(
            &initial_update(&[("67000.00", "1.5"), ("66999.00", "2.0")], &[("67001.00", "0.5")], 0),
            &mut seq,
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].0, Event::BookSnapshot { .. }));
        if let Event::BookSnapshot { bids, asks, .. } = &events[0].0 {
            assert_eq!(bids.len(), 2);
            assert_eq!(asks.len(), 1);
        }
    }

    #[test]
    fn delta_emits_book_deltas() {
        let mut seq = GeminiSeqState::new();
        // snapshot
        parse_gemini_message(&initial_update(&[("67000.00", "1.5")], &[("67001.00", "0.5")], 0), &mut seq);
        // delta
        let events = parse_gemini_message(
            &delta_update(&[("67000.00", "2.0")], &[("67001.00", "0")], 1),
            &mut seq,
        );
        assert!(events.iter().any(|(e, _)| matches!(e, Event::BookDelta(_))));
        if let Event::BookDelta(d) = &events.last().unwrap().0 {
            assert!(d.is_delete()); // remaining=0
        }
    }

    #[test]
    fn sequence_gap_emits_gap_event() {
        let mut seq = GeminiSeqState::new();
        parse_gemini_message(&initial_update(&[("67000.00", "1.0")], &[], 0), &mut seq);
        // seq 2 instead of 1 → gap
        let events = parse_gemini_message(&delta_update(&[], &[], 2), &mut seq);
        assert!(events.iter().any(|(e, _)| matches!(e, Event::Gap { .. })));
    }

    #[test]
    fn heartbeat_ignored() {
        let mut seq = GeminiSeqState::new();
        let hb = r#"{"type":"heartbeat","socket_sequence":0}"#;
        assert!(parse_gemini_message(hb, &mut seq).is_empty());
    }

    #[test]
    fn symbol_normalisation() {
        assert_eq!(to_gemini_symbol("BTC/USD"), "BTCUSD");
        assert_eq!(to_gemini_symbol("btc-usd"), "BTCUSD");
        assert_eq!(to_gemini_symbol("BTCUSD"), "BTCUSD");
    }
}
