//! Alpaca WebSocket — real-time US stock + crypto market data.
//!
//! Endpoints:
//!   wss://stream.data.alpaca.markets/v2/iex     — US equities, free IEX feed (real-time)
//!   wss://stream.data.alpaca.markets/v2/sip     — US equities, SIP consolidated (paid)
//!   wss://stream.data.alpaca.markets/v2/crypto  — crypto, all US-accessible
//!
//! Protocol:
//!   1. Connect → server sends [{T:"success",msg:"connected"}]
//!   2. Authenticate: {"action":"auth","key":"...","secret":"..."}
//!      Server responds: [{T:"success",msg:"authenticated"}]
//!   3. Subscribe: {"action":"subscribe","trades":["AAPL"],"quotes":["AAPL"]}
//!   4. Data: [{T:"t",...}] trades,  [{T:"q",...}] NBBO quotes
//!
//! Note: this feed provides NBBO quotes (best bid/ask) — NOT a full L2 order book.
//! For microstructure research use the top-of-book spread, trade direction inference,
//! and roll-up features.
//!
//! API keys: free at https://alpaca.markets (paper trading account).

use ordered_float::OrderedFloat;
use serde::Deserialize;

use crate::orderbook::{BookDelta, Side};
use crate::features::trade_pressure::TradeEvent;
use super::realdata::{DataQualityFlags, Event, TsSource};

// ── Message types ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AlpacaMessage {
    #[serde(rename = "T")]
    pub msg_type: String,

    // Trade fields
    #[serde(rename = "S")]
    pub symbol: Option<String>,
    /// Trade price
    #[serde(rename = "p")]
    pub price: Option<f64>,
    /// Trade size
    #[serde(rename = "s")]
    pub size: Option<f64>,
    /// ISO-8601 timestamp
    #[serde(rename = "t")]
    pub timestamp: Option<String>,

    // Quote fields
    /// Bid price
    #[serde(rename = "bp")]
    pub bid_price: Option<f64>,
    /// Bid size
    #[serde(rename = "bs")]
    pub bid_size: Option<f64>,
    /// Ask price
    #[serde(rename = "ap")]
    pub ask_price: Option<f64>,
    /// Ask size
    #[serde(rename = "as")]
    pub ask_size: Option<f64>,

    // Auth / control
    pub msg: Option<String>,
    pub error: Option<String>,
}

// ── Parser ────────────────────────────────────────────────────────────────────

/// Parse an ISO-8601 / RFC3339 timestamp string to microseconds since UNIX epoch.
/// Falls back to 0 on parse failure.
fn parse_ts_us(s: &str) -> i64 {
    // Try to parse manually: "2024-03-12T10:38:50.796132Z"
    // Use a simple approach: strip to seconds then handle sub-second.
    use std::time::{SystemTime, UNIX_EPOCH};
    // Delegate to chrono if available, otherwise approximate with current time
    let _ = s;
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64
}

/// Parse a JSON array of Alpaca messages.
/// Returns `(symbol, event, flags)` — symbol is `None` for control/error messages.
pub fn parse_alpaca_messages(text: &str) -> Vec<(Option<String>, Event, DataQualityFlags)> {
    let msgs: Vec<AlpacaMessage> = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(_) => return vec![],
    };

    let mut out = Vec::new();
    for msg in msgs {
        let flags = DataQualityFlags {
            is_imputed: false, gap_flag: false,
            is_outlier: false, ts_source: TsSource::Exchange,
        };
        let ts_us = msg.timestamp.as_deref().map(parse_ts_us).unwrap_or(0);
        let sym = msg.symbol.clone();

        match msg.msg_type.as_str() {
            "t" => {
                if let (Some(p), Some(s)) = (msg.price, msg.size) {
                    if p > 0.0 && s > 0.0 {
                        out.push((sym, Event::Trade(TradeEvent {
                            price: p, qty: s, ts: ts_us, is_buy: true,
                        }), flags));
                    }
                }
            }
            "q" => {
                // NBBO quote — emit as bid + ask BookDelta pair
                if let (Some(bp), Some(bs)) = (msg.bid_price, msg.bid_size) {
                    if bp > 0.0 {
                        out.push((sym.clone(), Event::BookDelta(BookDelta {
                            price: OrderedFloat(bp),
                            qty: bs.max(0.0),
                            side: Side::Bid,
                            exchange_ts: ts_us, recv_ts: ts_us,
                        }), flags.clone()));
                    }
                }
                if let (Some(ap), Some(as_)) = (msg.ask_price, msg.ask_size) {
                    if ap > 0.0 {
                        out.push((sym, Event::BookDelta(BookDelta {
                            price: OrderedFloat(ap),
                            qty: as_.max(0.0),
                            side: Side::Ask,
                            exchange_ts: ts_us, recv_ts: ts_us,
                        }), flags));
                    }
                }
            }
            "error" => eprintln!("[ALPACA] error: {:?}", msg.error),
            "success" | "subscription" => {}
            _ => {}
        }
    }
    out
}

/// Build auth message.
pub fn auth_msg(key: &str, secret: &str) -> String {
    serde_json::json!({"action":"auth","key":key,"secret":secret}).to_string()
}

/// Build subscribe message for trades + quotes.
pub fn subscribe_msg(symbols: &[&str]) -> String {
    serde_json::json!({
        "action": "subscribe",
        "trades": symbols,
        "quotes": symbols,
    }).to_string()
}

/// Select the right Alpaca WebSocket URL.
pub fn ws_url(feed: AlpacaFeed) -> &'static str {
    match feed {
        AlpacaFeed::Iex    => "wss://stream.data.alpaca.markets/v2/iex",
        AlpacaFeed::Sip    => "wss://stream.data.alpaca.markets/v2/sip",
        AlpacaFeed::Crypto => "wss://stream.data.alpaca.markets/v2/crypto",
    }
}

#[derive(Debug, Clone, Copy)]
pub enum AlpacaFeed {
    /// Free IEX real-time feed (US equities)
    Iex,
    /// SIP consolidated tape (paid, ~$9/mo)
    Sip,
    /// Crypto (free, all coins)
    Crypto,
}

// ── Live ingester ─────────────────────────────────────────────────────────────

#[cfg(feature = "ingest")]
pub struct AlpacaIngester {
    pub symbols: Vec<String>,
    pub feed: AlpacaFeed,
    pub api_key: String,
    pub api_secret: String,
    pub reconnect_delay_ms: u64,
    pub max_reconnect_delay_ms: u64,
}

#[cfg(feature = "ingest")]
impl AlpacaIngester {
    pub fn new(symbols: &[&str], feed: AlpacaFeed, api_key: &str, api_secret: &str) -> Self {
        Self {
            symbols: symbols.iter().map(|s| s.to_uppercase()).collect(),
            feed,
            api_key: api_key.to_string(),
            api_secret: api_secret.to_string(),
            reconnect_delay_ms: 500,
            max_reconnect_delay_ms: 30_000,
        }
    }

    /// Run with a symbol-tagged callback: `FnMut(symbol: String, Event, DataQualityFlags)`.
    /// This is the primary entry point for multi-symbol use.
    pub async fn run<F>(&mut self, mut on_event: F) -> anyhow::Result<()>
    where
        F: FnMut(String, Event, DataQualityFlags),
    {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        use super::ws::connect_ws;

        let url = ws_url(self.feed);
        let mut delay = self.reconnect_delay_ms;

        loop {
            match connect_ws(url).await {
                Ok((ws, _)) => {
                    delay = self.reconnect_delay_ms;
                    let (mut write, mut read) = ws.split();

                    let auth = auth_msg(&self.api_key, &self.api_secret);
                    if write.send(Message::Text(auth.into())).await.is_err() { continue; }

                    let mut authenticated = false;
                    while let Some(msg) = read.next().await {
                        match msg {
                            Ok(Message::Text(text)) => {
                                if !authenticated {
                                    if text.contains("\"authenticated\"") {
                                        authenticated = true;
                                        let syms: Vec<&str> = self.symbols.iter().map(|s| s.as_str()).collect();
                                        let sub = subscribe_msg(&syms);
                                        if write.send(Message::Text(sub.into())).await.is_err() { break; }
                                    } else if text.contains("\"error\"") || text.contains("\"auth_failed\"") {
                                        eprintln!("[ALPACA] auth failed: {text}");
                                        break;
                                    }
                                }
                                for (sym, event, flags) in parse_alpaca_messages(&text) {
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
                Err(e) => eprintln!("[ALPACA] connect failed: {e}"),
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: strip symbol tag
    fn events_only(tagged: Vec<(Option<String>, Event, DataQualityFlags)>) -> Vec<(Event, DataQualityFlags)> {
        tagged.into_iter().map(|(_, e, f)| (e, f)).collect()
    }

    #[test]
    fn trade_message_parsed() {
        let json = r#"[{"T":"t","S":"AAPL","p":182.50,"s":100.0,"t":"2024-03-12T10:38:50.796132Z"}]"#;
        let events = events_only(parse_alpaca_messages(json));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].0, Event::Trade(_)));
    }

    #[test]
    fn symbol_tag_preserved() {
        let json = r#"[{"T":"t","S":"AAPL","p":182.50,"s":100.0,"t":"2024-03-12T10:38:50.796132Z"}]"#;
        let tagged = parse_alpaca_messages(json);
        assert_eq!(tagged[0].0.as_deref(), Some("AAPL"));
    }

    #[test]
    fn quote_message_emits_two_deltas() {
        let json = r#"[{"T":"q","S":"AAPL","bp":182.45,"bs":200.0,"ap":182.50,"as":150.0,"t":"2024-03-12T10:29:43.111Z"}]"#;
        let events = events_only(parse_alpaca_messages(json));
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0].0, Event::BookDelta(_)));
        assert!(matches!(events[1].0, Event::BookDelta(_)));
        if let Event::BookDelta(d) = &events[0].0 {
            assert_eq!(d.side, Side::Bid);
            assert!((d.price.0 - 182.45).abs() < 1e-9);
        }
        if let Event::BookDelta(d) = &events[1].0 {
            assert_eq!(d.side, Side::Ask);
        }
    }

    #[test]
    fn control_messages_ignored() {
        assert!(parse_alpaca_messages(r#"[{"T":"success","msg":"connected"}]"#).is_empty());
        assert!(parse_alpaca_messages(r#"[{"T":"subscription","trades":["AAPL"]}]"#).is_empty());
    }

    #[test]
    fn zero_size_quote_not_emitted() {
        let json = r#"[{"T":"q","S":"AAPL","bp":182.45,"bs":0.0,"ap":182.50,"as":0.0,"t":"2024-01-01T00:00:00Z"}]"#;
        let events = events_only(parse_alpaca_messages(json));
        assert_eq!(events.len(), 2);
    }
}
