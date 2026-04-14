//! Bitstamp WebSocket — full L2 order book snapshots.
//!
//! Endpoint: wss://ws.bitstamp.net
//!
//! Channel: "order_book_{pair}" (e.g. "order_book_btcusd")
//!
//! Protocol:
//!   1. Subscribe: {"event":"bts:subscribe","data":{"channel":"order_book_btcusd"}}
//!   2. Server ack: {"event":"bts:subscription_succeeded","channel":"order_book_btcusd","data":{}}
//!   3. Data pushed every ~1s: full L2 snapshot via event == "data"
//!   4. Server heartbeats: {"event":"bts:heartbeat"} — no response needed
//!   5. Keep-alive: send {"event":"bts:heartbeat"} every 8s
//!
//! Since order_book_ pushes full snapshots, every "data" message emits Event::BookSnapshot.

use ordered_float::OrderedFloat;
use serde::Deserialize;

use super::realdata::{DataQualityFlags, Event, TsSource};

// ── Message types ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct BitstampWsMessage {
    pub event: String,
    pub channel: Option<String>,
    pub data: Option<BitstampData>,
}

#[derive(Debug, Deserialize)]
pub struct BitstampData {
    pub timestamp: Option<String>,
    pub microtimestamp: Option<String>,
    #[serde(default)]
    pub bids: Vec<[String; 2]>,
    #[serde(default)]
    pub asks: Vec<[String; 2]>,
}

// ── Symbol helpers ────────────────────────────────────────────────────────────

/// "BTC/USD" or "BTCUSD" → "order_book_btcusd"
pub fn to_channel(pair: &str) -> String {
    let clean = pair.to_lowercase().replace(['/', '-', '_'], "");
    format!("order_book_{clean}")
}

/// "order_book_btcusd" → "BTCUSD"
pub fn channel_to_symbol(channel: &str) -> String {
    channel.trim_start_matches("order_book_").to_uppercase()
}

// ── Parser ────────────────────────────────────────────────────────────────────

fn parse_levels(levels: &[[String; 2]]) -> Vec<(f64, f64)> {
    levels.iter().filter_map(|entry| {
        let price: f64 = entry[0].parse().ok()?;
        let qty: f64 = entry[1].parse().ok()?;
        if price <= 0.0 || qty < 0.0 { return None; }
        Some((price, qty))
    }).collect()
}

/// Parse a raw Bitstamp WS text frame.
/// Returns (symbol, event, flags) — symbol extracted from channel name.
pub fn parse_bitstamp_message(text: &str) -> Vec<(String, Event, DataQualityFlags)> {
    let msg: BitstampWsMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(_) => return vec![],
    };

    // Ignore non-data events
    match msg.event.as_str() {
        "bts:subscription_succeeded" | "bts:heartbeat" | "bts:error" => return vec![],
        "data" => {}
        _ => return vec![],
    }

    let channel = match msg.channel.as_deref() {
        Some(c) if !c.is_empty() => c,
        _ => return vec![],
    };

    let symbol = channel_to_symbol(channel);

    let data = match msg.data {
        Some(d) => d,
        None => return vec![],
    };

    // Resolve timestamp: prefer microtimestamp (µs), fall back to timestamp (s) * 1_000_000
    let ts_us: i64 = if let Some(ref mus) = data.microtimestamp {
        mus.parse().unwrap_or(0)
    } else if let Some(ref ts) = data.timestamp {
        ts.parse::<i64>().unwrap_or(0) * 1_000_000
    } else {
        0
    };

    let bids = parse_levels(&data.bids);
    let asks = parse_levels(&data.asks);

    let flags = DataQualityFlags {
        is_imputed: false,
        gap_flag: false,
        is_outlier: false,
        ts_source: TsSource::Exchange,
    };

    vec![(symbol, Event::BookSnapshot { bids, asks, ts: ts_us }, flags)]
}

// ── Live ingester ─────────────────────────────────────────────────────────────

#[cfg(feature = "ingest")]
pub struct BitstampIngester {
    pub symbols: Vec<String>,       // channel names like "order_book_btcusd"
    pub reconnect_delay_ms: u64,
    pub max_reconnect_delay_ms: u64,
}

#[cfg(feature = "ingest")]
impl BitstampIngester {
    pub fn new(pair: &str) -> Self {
        Self {
            symbols: vec![to_channel(pair)],
            reconnect_delay_ms: 250,
            max_reconnect_delay_ms: 30_000,
        }
    }

    pub fn new_multi(pairs: &[&str]) -> Self {
        Self {
            symbols: pairs.iter().map(|p| to_channel(p)).collect(),
            reconnect_delay_ms: 250,
            max_reconnect_delay_ms: 30_000,
        }
    }

    /// Run with a symbol-tagged callback: `FnMut(symbol, Event, DataQualityFlags)`.
    pub async fn run<F>(&mut self, mut on_event: F) -> anyhow::Result<()>
    where
        F: FnMut(String, Event, DataQualityFlags),
    {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        use super::ws::connect_ws;
        use tokio::time::{interval, Duration};

        let url = "wss://ws.bitstamp.net";
        let mut delay = self.reconnect_delay_ms;

        loop {
            match connect_ws(url).await {
                Ok((ws, _)) => {
                    delay = self.reconnect_delay_ms;
                    let (mut write, mut read) = ws.split();

                    // Subscribe to each channel separately
                    for channel in &self.symbols {
                        let sub = serde_json::json!({
                            "event": "bts:subscribe",
                            "data": { "channel": channel }
                        }).to_string();
                        if write.send(Message::Text(sub.into())).await.is_err() {
                            break;
                        }
                    }

                    let heartbeat_msg = r#"{"event":"bts:heartbeat"}"#;
                    let mut heartbeat_interval = interval(Duration::from_secs(8));
                    // Consume the first immediate tick so we don't send a heartbeat right away
                    heartbeat_interval.tick().await;

                    loop {
                        tokio::select! {
                            msg = read.next() => {
                                match msg {
                                    Some(Ok(Message::Text(text))) => {
                                        for (sym, event, flags) in parse_bitstamp_message(&text) {
                                            on_event(sym, event, flags);
                                        }
                                    }
                                    Some(Ok(Message::Ping(p))) => {
                                        let _ = write.send(Message::Pong(p)).await;
                                    }
                                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                                    _ => {}
                                }
                            }
                            _ = heartbeat_interval.tick() => {
                                if write.send(Message::Text(heartbeat_msg.into())).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                }
                Err(e) => eprintln!("[BITSTAMP] connect failed: {e}"),
            }

            on_event(String::new(), Event::Disconnected, DataQualityFlags {
                is_imputed: true,
                gap_flag: true,
                is_outlier: false,
                ts_source: TsSource::Local,
            });
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            delay = (delay * 2).min(self.max_reconnect_delay_ms);
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_data_message_emits_snapshot() {
        let text = r#"{
            "event": "data",
            "channel": "order_book_btcusd",
            "data": {
                "timestamp": "1700000000",
                "microtimestamp": "1700000000000000",
                "bids": [["67000.00", "1.5"], ["66999.00", "2.0"]],
                "asks": [["67001.00", "0.5"], ["67002.00", "1.0"]]
            }
        }"#;
        let events = parse_bitstamp_message(text);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "BTCUSD");
        if let Event::BookSnapshot { bids, asks, ts } = &events[0].1 {
            assert_eq!(bids.len(), 2);
            assert_eq!(asks.len(), 2);
            assert_eq!(*ts, 1_700_000_000_000_000_i64);
            assert!((bids[0].0 - 67000.0).abs() < 1e-8);
            assert!((asks[0].0 - 67001.0).abs() < 1e-8);
        } else {
            panic!("expected BookSnapshot");
        }
    }

    #[test]
    fn subscription_ack_ignored() {
        let text = r#"{"event":"bts:subscription_succeeded","channel":"order_book_btcusd","data":{}}"#;
        assert!(parse_bitstamp_message(text).is_empty());
    }

    #[test]
    fn heartbeat_ignored() {
        let text = r#"{"event":"bts:heartbeat","channel":"","data":{}}"#;
        assert!(parse_bitstamp_message(text).is_empty());
    }

    #[test]
    fn to_channel_normalizes_symbol() {
        assert_eq!(to_channel("BTC/USD"), "order_book_btcusd");
        assert_eq!(to_channel("ETHUSD"), "order_book_ethusd");
        assert_eq!(to_channel("ETH/USD"), "order_book_ethusd");
        assert_eq!(to_channel("btc-usd"), "order_book_btcusd");
    }
}
