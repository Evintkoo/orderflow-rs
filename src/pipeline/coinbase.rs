//! Coinbase Advanced Trade WebSocket — level2 full-depth order book.
//!
//! Endpoint: wss://advanced-trade-ws.coinbase.com
//! Channel:  level2  (full depth, all resting levels, free, no auth required)
//!
//! Protocol:
//!   1. Send subscribe message
//!   2. Receive "snapshot" event: replace entire book with received levels
//!   3. Receive "update" events: each update contains absolute new_quantity
//!      (NOT deltas). new_quantity = "0" means delete the level.
//!   4. Reconnect with exponential backoff on disconnect.
//!
//! Product IDs use Coinbase format: "BTC-USD", "ETH-USD", "BTC-EUR", etc.
//! Convert from standard notation: "BTC/USD" → "BTC-USD"

use ordered_float::OrderedFloat;
use serde::Deserialize;

use crate::orderbook::{BookDelta, Side};
use super::realdata::{DataQualityFlags, Event, TsSource};

// ── Message types ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CoinbaseWsMessage {
    pub channel: Option<String>,
    #[serde(rename = "type")]
    pub msg_type: Option<String>,
    pub events: Option<Vec<CoinbaseEvent>>,
    /// Sequence number for gap detection.
    pub sequence_num: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct CoinbaseEvent {
    #[serde(rename = "type")]
    pub event_type: String, // "snapshot" or "update"
    pub product_id: Option<String>,
    pub updates: Option<Vec<CoinbaseLevelUpdate>>,
}

#[derive(Debug, Deserialize)]
pub struct CoinbaseLevelUpdate {
    pub side: String, // "bid" or "offer" (Coinbase uses "offer" not "ask")
    pub price_level: String,
    pub new_quantity: String,
}

/// Parse a Coinbase level update into a BookDelta.
///
/// new_quantity is absolute (not delta). "0" = delete level.
pub fn parse_coinbase_update(
    update: &CoinbaseLevelUpdate,
    ts_us: i64,
) -> Option<BookDelta> {
    let price: f64 = update.price_level.parse().ok()?;
    let qty: f64 = update.new_quantity.parse().ok()?;

    if price <= 0.0 || !price.is_finite() {
        return None;
    }

    let side = match update.side.as_str() {
        "bid" => Side::Bid,
        "offer" | "ask" => Side::Ask,
        _ => return None,
    };

    Some(BookDelta {
        price: OrderedFloat(price),
        qty,
        side,
        exchange_ts: ts_us,
        recv_ts: ts_us,
    })
}

/// Parse a raw WebSocket message text into normalized events.
/// Returns `(product_id, event, flags)` so multi-symbol subscribers can route correctly.
pub fn parse_coinbase_message(text: &str) -> Vec<(Option<String>, Event, DataQualityFlags)> {
    let msg: CoinbaseWsMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(_) => return vec![],
    };

    let channel = msg.channel.as_deref().unwrap_or("");
    if channel != "l2_data" {
        return vec![];
    }

    let now_us = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64;

    let mut out = Vec::new();

    for event in msg.events.unwrap_or_default() {
        let sym = event.product_id.clone();
        let is_snapshot = event.event_type == "snapshot";

        if is_snapshot {
            let mut bids = Vec::new();
            let mut asks = Vec::new();
            for update in event.updates.unwrap_or_default() {
                let price: f64 = match update.price_level.parse() { Ok(p) => p, Err(_) => continue };
                let qty: f64 = match update.new_quantity.parse() { Ok(q) => q, Err(_) => continue };
                if price <= 0.0 || qty < 0.0 { continue; }
                match update.side.as_str() {
                    "bid" => bids.push((price, qty)),
                    "offer" | "ask" => asks.push((price, qty)),
                    _ => {}
                }
            }
            out.push((sym, Event::BookSnapshot { bids, asks, ts: now_us },
                DataQualityFlags { is_imputed: false, gap_flag: false,
                    is_outlier: false, ts_source: TsSource::Exchange }));
        } else {
            for update in event.updates.unwrap_or_default() {
                if let Some(delta) = parse_coinbase_update(&update, now_us) {
                    out.push((sym.clone(), Event::BookDelta(delta),
                        DataQualityFlags { is_imputed: false, gap_flag: false,
                            is_outlier: false, ts_source: TsSource::Exchange }));
                }
            }
        }
    }

    out
}

/// Build the subscribe JSON message for the level2 channel.
/// `product_ids` may contain multiple products in one subscription.
pub fn subscribe_msg(product_ids: &[&str]) -> String {
    serde_json::json!({
        "type": "subscribe",
        "product_ids": product_ids,
        "channel": "level2"
    })
    .to_string()
}

/// Convert a slash-separated pair to Coinbase product ID format.
///
/// "BTC/USD" → "BTC-USD", "btcusd" → "BTC-USD" (best-effort)
pub fn to_product_id(pair: &str) -> String {
    let upper = pair.to_uppercase();
    if upper.contains('-') {
        return upper;
    }
    if upper.contains('/') {
        return upper.replace('/', "-");
    }
    // Common 6/7 char pairs: BTCUSDT → BTC-USDT, ETHUSD → ETH-USD
    let known_bases = ["BTC", "ETH", "SOL", "XRP", "ADA", "DOGE", "MATIC", "AVAX", "LINK", "UNI"];
    for base in &known_bases {
        if upper.starts_with(base) {
            let quote = &upper[base.len()..];
            return format!("{base}-{quote}");
        }
    }
    upper
}

/// Coinbase WebSocket ingester — supports multiple products on one connection.
#[cfg(feature = "ingest")]
pub struct CoinbaseIngester {
    pub product_ids: Vec<String>,
    pub reconnect_delay_ms: u64,
    pub max_reconnect_delay_ms: u64,
}

#[cfg(feature = "ingest")]
impl CoinbaseIngester {
    pub fn new(product_id: &str) -> Self {
        Self {
            product_ids: vec![to_product_id(product_id)],
            reconnect_delay_ms: 250,
            max_reconnect_delay_ms: 30_000,
        }
    }

    pub fn new_multi(pairs: &[&str]) -> Self {
        Self {
            product_ids: pairs.iter().map(|p| to_product_id(p)).collect(),
            reconnect_delay_ms: 250,
            max_reconnect_delay_ms: 30_000,
        }
    }

    /// Run with a symbol-tagged callback: `FnMut(product_id, Event, DataQualityFlags)`.
    pub async fn run<F>(&mut self, mut on_event: F) -> anyhow::Result<()>
    where
        F: FnMut(String, Event, DataQualityFlags),
    {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        use super::ws::connect_ws;

        let mut delay = self.reconnect_delay_ms;
        let url = "wss://advanced-trade-ws.coinbase.com";

        loop {
            match connect_ws(url).await {
                Ok((ws, _)) => {
                    delay = self.reconnect_delay_ms;
                    let (mut write, mut read) = ws.split();

                    let ids: Vec<&str> = self.product_ids.iter().map(|s| s.as_str()).collect();
                    let sub = subscribe_msg(&ids);
                    if write.send(Message::Text(sub.into())).await.is_err() { continue; }

                    eprintln!("\n[coinbase] snapshot: subscribing to {} products", self.product_ids.len());

                    while let Some(msg) = read.next().await {
                        match msg {
                            Ok(Message::Text(text)) => {
                                for (sym, event, flags) in parse_coinbase_message(&text) {
                                    let sym = sym.unwrap_or_default();
                                    on_event(sym, event, flags);
                                }
                            }
                            Ok(Message::Ping(p)) => { let _ = write.send(Message::Pong(p)).await; }
                            Ok(Message::Close(_)) | Err(_) => break,
                            _ => {}
                        }
                    }
                }
                Err(e) => eprintln!("[COINBASE] connect failed: {e}"),
            }

            on_event(String::new(), Event::Disconnected, DataQualityFlags {
                is_imputed: true, gap_flag: true,
                is_outlier: false, ts_source: TsSource::Local,
            });
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            delay = (delay * 2).min(self.max_reconnect_delay_ms);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_product_id_slash() {
        assert_eq!(to_product_id("BTC/USD"), "BTC-USD");
        assert_eq!(to_product_id("eth/usd"), "ETH-USD");
    }

    #[test]
    fn to_product_id_no_sep() {
        assert_eq!(to_product_id("BTCUSD"), "BTC-USD");
        assert_eq!(to_product_id("ETHUSD"), "ETH-USD");
    }

    #[test]
    fn parse_snapshot_produces_book_snapshot_event() {
        let text = r#"{
            "channel": "l2_data",
            "sequence_num": 0,
            "events": [{
                "type": "snapshot",
                "product_id": "BTC-USD",
                "updates": [
                    {"side": "bid", "price_level": "29000.00", "new_quantity": "1.5"},
                    {"side": "offer", "price_level": "29001.00", "new_quantity": "2.0"}
                ]
            }]
        }"#;
        let events = parse_coinbase_message(text);
        assert_eq!(events.len(), 1);
        if let Event::BookSnapshot { bids, asks, .. } = &events[0].1 {
            assert_eq!(bids.len(), 1);
            assert_eq!(asks.len(), 1);
            assert!((bids[0].0 - 29000.0).abs() < 1e-8);
        } else {
            panic!("expected BookSnapshot");
        }
    }

    #[test]
    fn parse_update_produces_deltas() {
        let text = r#"{
            "channel": "l2_data",
            "sequence_num": 1,
            "events": [{
                "type": "update",
                "product_id": "BTC-USD",
                "updates": [
                    {"side": "bid", "price_level": "29000.00", "new_quantity": "3.0"},
                    {"side": "offer", "price_level": "29001.00", "new_quantity": "0"}
                ]
            }]
        }"#;
        let events = parse_coinbase_message(text);
        assert_eq!(events.len(), 2);
        // Second event (ask qty=0) should be a delete
        if let Event::BookDelta(d) = &events[1].1 {
            assert!(d.is_delete());
            assert_eq!(d.side, Side::Ask);
        } else {
            panic!("expected BookDelta");
        }
    }

    #[test]
    fn non_l2_channel_ignored() {
        let text = r#"{"channel":"heartbeats","events":[]}"#;
        assert!(parse_coinbase_message(text).is_empty());
    }

    #[test]
    fn offer_side_maps_to_ask() {
        let update = CoinbaseLevelUpdate {
            side: "offer".into(),
            price_level: "100.0".into(),
            new_quantity: "5.0".into(),
        };
        let d = parse_coinbase_update(&update, 0).unwrap();
        assert_eq!(d.side, Side::Ask);
    }
}
