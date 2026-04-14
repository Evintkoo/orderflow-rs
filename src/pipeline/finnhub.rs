//! Finnhub WebSocket — real-time trades for US stocks and crypto.
//!
//! Endpoint: wss://ws.finnhub.io?token=TOKEN
//! Free tier: up to 60 symbols, trades only (no NBBO on free tier).
//! API key available free at finnhub.io.
//!
//! Protocol:
//!   1. Connect (token embedded in URL)
//!   2. Subscribe per symbol: {"type":"subscribe","symbol":"AAPL"}
//!   3. Data: {"type":"trade","data":[{"p":150.23,"s":"AAPL","t":1700000000000,"v":100}]}

use serde::Deserialize;

use crate::features::trade_pressure::TradeEvent;
use super::realdata::{DataQualityFlags, Event, TsSource};

// ── Message types ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct FinnhubMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub data: Option<Vec<FinnhubTrade>>,
}

#[derive(Debug, Deserialize)]
pub struct FinnhubTrade {
    /// Price
    pub p: f64,
    /// Symbol
    pub s: String,
    /// Timestamp in milliseconds
    pub t: i64,
    /// Volume
    pub v: Option<f64>,
}

// ── Parser ────────────────────────────────────────────────────────────────────

/// Parse a raw Finnhub WebSocket text frame into normalized Trade events.
/// Returns `(symbol, event, flags)` tuples.
///
/// Only "trade" messages are emitted; all other types (ping, news, etc.) return empty.
pub fn parse_finnhub_message(text: &str) -> Vec<(String, Event, DataQualityFlags)> {
    let msg: FinnhubMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(_) => return vec![],
    };

    if msg.msg_type != "trade" {
        return vec![];
    }

    let trades = match msg.data {
        Some(t) => t,
        None => return vec![],
    };

    let flags = DataQualityFlags {
        is_imputed: false,
        gap_flag: false,
        is_outlier: false,
        ts_source: TsSource::Exchange,
    };

    trades
        .into_iter()
        .filter(|t| t.p > 0.0)
        .map(|trade| {
            let sym = trade.s.to_uppercase();
            (
                sym,
                Event::Trade(TradeEvent {
                    price: trade.p,
                    qty: trade.v.unwrap_or(1.0),
                    ts: trade.t * 1_000, // ms → µs
                    is_buy: false,       // direction unknown on free tier
                }),
                flags.clone(),
            )
        })
        .collect()
}

/// Build individual subscribe messages — one per symbol.
pub fn subscribe_msgs(symbols: &[&str]) -> Vec<String> {
    symbols
        .iter()
        .map(|s| {
            serde_json::json!({
                "type": "subscribe",
                "symbol": s.to_uppercase()
            })
            .to_string()
        })
        .collect()
}

// ── Live ingester ─────────────────────────────────────────────────────────────

/// Finnhub WebSocket ingester — real-time US stock trades.
#[cfg(feature = "ingest")]
pub struct FinnhubIngester {
    pub symbols: Vec<String>,
    pub api_key: String,
    pub reconnect_delay_ms: u64,
    pub max_reconnect_delay_ms: u64,
}

#[cfg(feature = "ingest")]
impl FinnhubIngester {
    pub fn new(symbols: &[&str], api_key: &str) -> Self {
        Self {
            symbols: symbols.iter().map(|s| s.to_uppercase()).collect(),
            api_key: api_key.to_string(),
            reconnect_delay_ms: 500,
            max_reconnect_delay_ms: 30_000,
        }
    }

    /// Run with a symbol-tagged callback: `FnMut(symbol, Event, DataQualityFlags)`.
    pub async fn run<F>(&mut self, mut on_event: F) -> anyhow::Result<()>
    where
        F: FnMut(String, Event, DataQualityFlags),
    {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest, http::HeaderValue};
        use super::ws::connect_ws;

        let url = format!("wss://ws.finnhub.io/?token={}", self.api_key);
        let mut delay = self.reconnect_delay_ms;

        loop {
            let mut req = url.as_str().into_client_request().expect("valid url");
            let hdrs = req.headers_mut();
            hdrs.insert("Origin",      HeaderValue::from_static("https://finnhub.io"));
            hdrs.insert("User-Agent",  HeaderValue::from_static("Mozilla/5.0"));
            hdrs.insert("Cache-Control", HeaderValue::from_static("no-cache"));
            hdrs.insert("Pragma",      HeaderValue::from_static("no-cache"));
            match connect_ws(req).await {
                Ok((ws, _)) => {
                    delay = self.reconnect_delay_ms;
                    let (mut write, mut read) = ws.split();

                    // Subscribe to each symbol individually
                    let syms: Vec<&str> = self.symbols.iter().map(|s| s.as_str()).collect();
                    for sub in subscribe_msgs(&syms) {
                        if write.send(Message::Text(sub.into())).await.is_err() {
                            break;
                        }
                    }

                    eprintln!("\n[finnhub] subscribed to {} symbols", self.symbols.len());

                    while let Some(msg) = read.next().await {
                        match msg {
                            Ok(Message::Text(text)) => {
                                for (sym, event, flags) in parse_finnhub_message(&text) {
                                    on_event(sym, event, flags);
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
                Err(e) => eprintln!("[FINNHUB] connect failed: {e}"),
            }

            on_event(
                String::new(),
                Event::Disconnected,
                DataQualityFlags {
                    is_imputed: true,
                    gap_flag: true,
                    is_outlier: false,
                    ts_source: TsSource::Local,
                },
            );
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
    fn trade_message_emits_trade_events() {
        let text = r#"{
            "type": "trade",
            "data": [
                {"p": 150.23, "s": "AAPL", "t": 1700000000000, "v": 100.0},
                {"p": 151.00, "s": "AAPL", "t": 1700000001000, "v": 50.0}
            ]
        }"#;
        let events = parse_finnhub_message(text);
        assert_eq!(events.len(), 2);

        let (ref sym0, ref ev0, _) = events[0];
        assert_eq!(sym0, "AAPL");
        if let Event::Trade(t) = ev0 {
            assert!((t.price - 150.23).abs() < 1e-9);
            assert!((t.qty - 100.0).abs() < 1e-9);
            assert_eq!(t.ts, 1_700_000_000_000_i64 * 1_000);
            assert!(!t.is_buy);
        } else {
            panic!("expected Trade event");
        }

        let (ref sym1, ref ev1, _) = events[1];
        assert_eq!(sym1, "AAPL");
        if let Event::Trade(t) = ev1 {
            assert!((t.price - 151.00).abs() < 1e-9);
        } else {
            panic!("expected Trade event");
        }
    }

    #[test]
    fn non_trade_message_ignored() {
        // ping message
        let ping = r#"{"type":"ping"}"#;
        assert!(parse_finnhub_message(ping).is_empty());

        // news message
        let news = r#"{"type":"news","data":[]}"#;
        assert!(parse_finnhub_message(news).is_empty());
    }

    #[test]
    fn subscribe_msgs_format() {
        let msgs = subscribe_msgs(&["AAPL", "spy", "QQQ"]);
        assert_eq!(msgs.len(), 3);
        let v: serde_json::Value = serde_json::from_str(&msgs[0]).expect("valid JSON");
        assert_eq!(v["type"], "subscribe");
        assert_eq!(v["symbol"], "AAPL");
        let v1: serde_json::Value = serde_json::from_str(&msgs[1]).expect("valid JSON");
        assert_eq!(v1["symbol"], "SPY");
    }

    #[test]
    fn volume_defaults_to_one_when_missing() {
        let text = r#"{"type":"trade","data":[{"p":100.0,"s":"MSFT","t":1700000000000}]}"#;
        let events = parse_finnhub_message(text);
        assert_eq!(events.len(), 1);
        if let Event::Trade(t) = &events[0].1 {
            assert!((t.qty - 1.0).abs() < 1e-9);
        }
    }
}
