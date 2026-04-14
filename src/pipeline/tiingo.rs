//! Tiingo IEX WebSocket — real-time NBBO quotes + trades for US stocks.
//!
//! Endpoint: wss://api.tiingo.com/iex
//! Free tier: up to 50 symbols simultaneously.  Requires free API key at tiingo.com.
//!
//! Protocol:
//!   1. Connect
//!   2. Send combined auth+subscribe message with tickers list
//!   3. Server confirms with messageType="I" (info)
//!   4. Data arrives as messageType="A" with array payload
//!
//! Data array positions (0-indexed):
//!   [0]  type string ("T" = IEX update)
//!   [1]  ticker symbol
//!   [2]  date string (ignored)
//!   [3]  last trade price
//!   [4]  last trade size
//!   [5]  last sale timestamp (ignored)
//!   [6]  bid price
//!   [7]  bid size
//!   [8]  ask price
//!   [9]  ask size
//!   [10] quote timestamp (used for ts)

use ordered_float::OrderedFloat;
use serde::Deserialize;

use crate::orderbook::{BookDelta, Side};
use crate::features::trade_pressure::TradeEvent;
use super::realdata::{DataQualityFlags, Event, TsSource};

// ── Message types ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TiingoWsMessage {
    #[serde(rename = "messageType")]
    pub message_type: String,
    pub data: serde_json::Value,
}

// ── Timestamp parser ──────────────────────────────────────────────────────────

/// Parse an RFC 3339 timestamp string like "2023-01-01T15:00:00+00:00"
/// into microseconds since the Unix epoch.
/// Returns 0 if parsing fails.
fn parse_ts_us(s: &str) -> i64 {
    // Manual parse: split on 'T', then parse date + time parts
    // Format: YYYY-MM-DDTHH:MM:SS+00:00 or YYYY-MM-DDTHH:MM:SS.sssZ
    let s = s.trim();
    // Find 'T' separator
    let t_pos = match s.find('T') {
        Some(p) => p,
        None => return 0,
    };
    let date_part = &s[..t_pos];
    let rest = &s[t_pos + 1..];

    // Parse date
    let date_parts: Vec<&str> = date_part.split('-').collect();
    if date_parts.len() < 3 { return 0; }
    let year: i64 = match date_parts[0].parse() { Ok(v) => v, Err(_) => return 0 };
    let month: i64 = match date_parts[1].parse() { Ok(v) => v, Err(_) => return 0 };
    let day: i64 = match date_parts[2].parse() { Ok(v) => v, Err(_) => return 0 };

    // Strip timezone offset / Z suffix from time
    let time_str = rest.split(['+', 'Z']).next().unwrap_or(rest);
    // Also handle negative offset: "15:00:00-05:00" — find last '-' after position 3
    let time_str = if let Some(neg_pos) = time_str[3..].rfind('-') {
        &time_str[..3 + neg_pos]
    } else {
        time_str
    };

    // Parse time HH:MM:SS or HH:MM:SS.fff
    let time_parts: Vec<&str> = time_str.split(':').collect();
    if time_parts.len() < 3 { return 0; }
    let hour: i64 = match time_parts[0].parse() { Ok(v) => v, Err(_) => return 0 };
    let min: i64  = match time_parts[1].parse() { Ok(v) => v, Err(_) => return 0 };
    // seconds may have fractional part
    let sec_str = time_parts[2];
    let (sec_int_str, frac_str) = match sec_str.find('.') {
        Some(p) => (&sec_str[..p], &sec_str[p + 1..]),
        None => (sec_str, ""),
    };
    let sec: i64 = match sec_int_str.parse() { Ok(v) => v, Err(_) => return 0 };
    let frac_us: i64 = if !frac_str.is_empty() {
        let digits: String = frac_str.chars().take(6).collect();
        let padded = format!("{:0<6}", digits);
        padded.parse().unwrap_or(0)
    } else {
        0
    };

    // Days since epoch using a simple algorithm (valid for dates >= 1970-01-01)
    // Using the algorithm from: https://howardhinnant.github.io/date_algorithms.html
    let y = year - if month <= 2 { 1 } else { 0 };
    let m = month + if month <= 2 { 9 } else { -3 };
    let d = day;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days_since_epoch = era * 146097 + doe - 719468;

    let total_s = days_since_epoch * 86400 + hour * 3600 + min * 60 + sec;
    total_s * 1_000_000 + frac_us
}

// ── Parser ────────────────────────────────────────────────────────────────────

/// Parse a raw Tiingo WebSocket text frame into normalized events.
/// Returns `(symbol, event, flags)` tuples.
///
/// A data ("A") message produces up to 3 events:
///   - BookDelta for the bid side
///   - BookDelta for the ask side
///   - Trade event for the last print
pub fn parse_tiingo_message(text: &str) -> Vec<(String, Event, DataQualityFlags)> {
    let msg: TiingoWsMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(_) => return vec![],
    };

    match msg.message_type.as_str() {
        "I" | "H" => return vec![], // info/heartbeat — ignore
        "A" => {}
        _ => return vec![],
    }

    let arr = match msg.data.as_array() {
        Some(a) => a,
        None => return vec![],
    };

    // Validate type field
    if arr.get(0).and_then(|v| v.as_str()) != Some("T") {
        return vec![];
    }

    let symbol = match arr.get(1).and_then(|v| v.as_str()) {
        Some(s) => s.to_uppercase(),
        None => return vec![],
    };

    let last_price = arr.get(3).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let last_size  = arr.get(4).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let bid_price  = arr.get(6).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let bid_size   = arr.get(7).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let ask_price  = arr.get(8).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let ask_size   = arr.get(9).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let quote_ts   = arr.get(10).and_then(|v| v.as_str()).map(parse_ts_us).unwrap_or(0);

    if bid_price <= 0.0 || ask_price <= 0.0 || last_price <= 0.0 {
        return vec![];
    }

    let flags = DataQualityFlags {
        is_imputed: false,
        gap_flag: false,
        is_outlier: false,
        ts_source: TsSource::Exchange,
    };

    let mut out = Vec::with_capacity(3);

    // Bid BookDelta
    out.push((
        symbol.clone(),
        Event::BookDelta(BookDelta {
            price: OrderedFloat(bid_price),
            qty: bid_size,
            side: Side::Bid,
            exchange_ts: quote_ts,
            recv_ts: quote_ts,
        }),
        flags.clone(),
    ));

    // Ask BookDelta
    out.push((
        symbol.clone(),
        Event::BookDelta(BookDelta {
            price: OrderedFloat(ask_price),
            qty: ask_size,
            side: Side::Ask,
            exchange_ts: quote_ts,
            recv_ts: quote_ts,
        }),
        flags.clone(),
    ));

    // Trade
    out.push((
        symbol,
        Event::Trade(TradeEvent {
            price: last_price,
            qty: last_size,
            ts: quote_ts,
            is_buy: false, // direction unknown on free tier
        }),
        flags,
    ));

    out
}

/// Build the combined auth+subscribe message.
pub fn subscribe_msg(tickers: &[&str], api_key: &str) -> String {
    let tickers_list: Vec<_> = tickers.iter().map(|t| t.to_uppercase()).collect();
    serde_json::json!({
        "eventName": "subscribe",
        "authorization": api_key,
        "eventData": {"tickers": tickers_list}
    })
    .to_string()
}

// ── Live ingester ─────────────────────────────────────────────────────────────

/// Tiingo IEX WebSocket ingester — real-time US stock NBBO quotes + trades.
#[cfg(feature = "ingest")]
pub struct TiingoIngester {
    pub symbols: Vec<String>,
    pub api_key: String,
    pub reconnect_delay_ms: u64,
    pub max_reconnect_delay_ms: u64,
}

#[cfg(feature = "ingest")]
impl TiingoIngester {
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
        use tokio_tungstenite::tungstenite::Message;
        use super::ws::connect_ws;

        let url = "wss://api.tiingo.com/iex";
        let mut delay = self.reconnect_delay_ms;

        loop {
            match connect_ws(url).await {
                Ok((ws, _)) => {
                    delay = self.reconnect_delay_ms;
                    let (mut write, mut read) = ws.split();

                    let syms: Vec<&str> = self.symbols.iter().map(|s| s.as_str()).collect();
                    let sub = subscribe_msg(&syms, &self.api_key);
                    if write.send(Message::Text(sub.into())).await.is_err() {
                        continue;
                    }

                    eprintln!("\n[tiingo] subscribed to {} symbols", self.symbols.len());

                    while let Some(msg) = read.next().await {
                        match msg {
                            Ok(Message::Text(text)) => {
                                for (sym, event, flags) in parse_tiingo_message(&text) {
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
                Err(e) => eprintln!("[TIINGO] connect failed: {e}"),
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
    fn parse_data_message_emits_nbbo_and_trade() {
        let text = r#"{
            "messageType": "A",
            "data": [
                "T",
                "AAPL",
                "2023-01-01T00:00:00+00:00",
                150.23,
                100.0,
                "2023-01-01T15:00:00+00:00",
                150.20,
                200.0,
                150.25,
                100.0,
                "2023-01-01T15:00:00+00:00"
            ]
        }"#;
        let events = parse_tiingo_message(text);
        assert_eq!(events.len(), 3, "expected 2 BookDeltas + 1 Trade");

        let (ref sym0, ref ev0, _) = events[0];
        assert_eq!(sym0, "AAPL");
        if let Event::BookDelta(d) = ev0 {
            assert_eq!(d.side, Side::Bid);
            assert!((d.price.0 - 150.20).abs() < 1e-9);
            assert!((d.qty - 200.0).abs() < 1e-9);
        } else {
            panic!("expected BookDelta for bid");
        }

        let (ref sym1, ref ev1, _) = events[1];
        assert_eq!(sym1, "AAPL");
        if let Event::BookDelta(d) = ev1 {
            assert_eq!(d.side, Side::Ask);
            assert!((d.price.0 - 150.25).abs() < 1e-9);
            assert!((d.qty - 100.0).abs() < 1e-9);
        } else {
            panic!("expected BookDelta for ask");
        }

        let (ref sym2, ref ev2, _) = events[2];
        assert_eq!(sym2, "AAPL");
        if let Event::Trade(t) = ev2 {
            assert!((t.price - 150.23).abs() < 1e-9);
            assert!((t.qty - 100.0).abs() < 1e-9);
            assert!(!t.is_buy);
        } else {
            panic!("expected Trade");
        }
    }

    #[test]
    fn info_message_ignored() {
        let text = r#"{"messageType":"I","data":"You've successfully connected and are subscribed to: AAPL"}"#;
        assert!(parse_tiingo_message(text).is_empty());
    }

    #[test]
    fn heartbeat_message_ignored() {
        let text = r#"{"messageType":"H","data":{}}"#;
        assert!(parse_tiingo_message(text).is_empty());
    }

    #[test]
    fn subscribe_msg_format() {
        let msg = subscribe_msg(&["AAPL", "spy"], "test_key");
        let v: serde_json::Value = serde_json::from_str(&msg).expect("valid JSON");
        assert_eq!(v["eventName"], "subscribe");
        assert_eq!(v["authorization"], "test_key");
        let tickers = v["eventData"]["tickers"].as_array().unwrap();
        assert_eq!(tickers.len(), 2);
        assert_eq!(tickers[0], "AAPL");
        assert_eq!(tickers[1], "SPY");
    }

    #[test]
    fn zero_price_filtered() {
        // bid_price = 0 → should return empty
        let text = r#"{
            "messageType": "A",
            "data": ["T","AAPL","2023-01-01T00:00:00+00:00",150.0,100.0,"",0.0,0.0,150.25,50.0,"2023-01-01T15:00:00+00:00"]
        }"#;
        assert!(parse_tiingo_message(text).is_empty());
    }

    #[test]
    fn parse_ts_us_basic() {
        // 2023-01-01T15:00:00+00:00 should parse correctly (non-zero)
        let ts = parse_ts_us("2023-01-01T15:00:00+00:00");
        assert!(ts > 0, "timestamp should be positive, got {ts}");
        // 1672585200 seconds since epoch
        assert_eq!(ts, 1_672_585_200_000_000_i64);
    }
}
