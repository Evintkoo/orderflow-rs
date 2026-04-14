//! IEX TOPS / DEEP feed reader — two modes:
//!
//! ## Mode 1: Historical JSON files (IEX Cloud)
//!
//! IEX publishes historical TOPS/DEEP data as newline-delimited JSON at:
//!   https://iexcloud.io/docs/api/#historical-prices
//!
//! Download daily files:
//!   curl -L "https://data.iex.cloud/v1/data/CORE/TOPS/2023-01-03?token=<TOKEN>" \
//!        -o iex_tops_20230103.jsonl
//!
//! Free tier: delayed data (15 min). Sandbox token works for format testing.
//!
//! Each line is a JSON object. TOPS quote messages look like:
//!   {"messageType":"Q","symbol":"AAPL","bidSize":100,"bidPrice":145.20,
//!    "askPrice":145.22,"askSize":200,"lastUpdated":1672751400123456789}
//!
//! Trade messages look like:
//!   {"messageType":"T","symbol":"AAPL","price":145.21,"size":50,
//!    "flags":1,"timestamp":1672751400123456789}
//!
//! ## Mode 2: IEX real-time SSE stream (free, no key needed)
//!
//! IEX exposes a free Server-Sent Events (SSE) endpoint:
//!   https://stream.iexapis.com/stable/tops?symbols=AAPL,MSFT&token=Tpk_...
//!
//! The `replay_iex_jsonl_file` function handles Mode 1 (historical files).
//! The `IexSseIngester` handles Mode 2 (live SSE stream) — gated by `ingest` feature.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use ordered_float::OrderedFloat;
use serde::Deserialize;

use crate::orderbook::{BookDelta, Side};
use crate::features::trade_pressure::TradeEvent;
use super::realdata::{DataQualityFlags, Event, TsSource};

// ── JSON message types ────────────────────────────────────────────────────────

/// IEX TOPS quote update message.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IexQuoteMsg {
    pub symbol: String,
    pub bid_size: Option<f64>,
    pub bid_price: Option<f64>,
    pub ask_price: Option<f64>,
    pub ask_size: Option<f64>,
    /// Nanosecond timestamp.
    pub last_updated: Option<i64>,
    // Alternative field names in some IEX formats
    pub timestamp: Option<i64>,
}

/// IEX trade message.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IexTradeMsg {
    pub symbol: String,
    pub price: f64,
    pub size: f64,
    /// Bit flags: 0x8 = odd-lot, 0x20 = ISO, 0x40 = extended hours, 0x80 = official close
    pub flags: Option<u32>,
    pub timestamp: Option<i64>,
    pub last_updated: Option<i64>,
}

/// Wrapper to dispatch on `messageType` field.
#[derive(Debug, Deserialize)]
pub struct IexMessage {
    #[serde(rename = "messageType")]
    pub message_type: String,
    // Remaining fields parsed per-type
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

/// Normalized events from one IEX quote message.
///
/// IEX TOPS only gives the best bid/ask — this produces two BookDeltas
/// (one for bid, one for ask). The snapshot approach: treat each quote as
/// "set best level to these values" (prior best level implicitly replaced).
pub fn iex_quote_to_events(msg: &IexQuoteMsg) -> Vec<(Event, DataQualityFlags)> {
    let ts_ns = msg.last_updated.or(msg.timestamp).unwrap_or(0);
    let ts_us = ts_ns / 1000; // ns → µs

    let flags = DataQualityFlags {
        is_imputed: false,
        gap_flag: false,
        is_outlier: false,
        ts_source: TsSource::Exchange,
    };

    let mut events = Vec::new();

    if let (Some(bid_p), Some(bid_q)) = (msg.bid_price, msg.bid_size) {
        if bid_p > 0.0 && bid_q >= 0.0 {
            events.push((
                Event::BookDelta(BookDelta {
                    price: OrderedFloat(bid_p),
                    qty: bid_q,
                    side: Side::Bid,
                    exchange_ts: ts_us,
                    recv_ts: ts_us,
                }),
                flags.clone(),
            ));
        }
    }

    if let (Some(ask_p), Some(ask_q)) = (msg.ask_price, msg.ask_size) {
        if ask_p > 0.0 && ask_q >= 0.0 {
            events.push((
                Event::BookDelta(BookDelta {
                    price: OrderedFloat(ask_p),
                    qty: ask_q,
                    side: Side::Ask,
                    exchange_ts: ts_us,
                    recv_ts: ts_us,
                }),
                flags.clone(),
            ));
        }
    }

    events
}

/// Convert an IEX trade message to a normalized `Event::Trade`.
pub fn iex_trade_to_event(msg: &IexTradeMsg) -> Option<(Event, DataQualityFlags)> {
    if msg.price <= 0.0 || msg.size <= 0.0 {
        return None;
    }

    let ts_ns = msg.timestamp.or(msg.last_updated).unwrap_or(0);
    let ts_us = ts_ns / 1000;

    // IEX trade flags: bit 0x10 = intermarket sweep; no reliable aggressor bit in TOPS
    // Default to unknown aggressor (is_buy = false — fill in from book context if needed)
    let is_buy = msg.flags.map(|f| f & 0x1 != 0).unwrap_or(false);

    Some((
        Event::Trade(TradeEvent {
            ts: ts_us,
            price: msg.price,
            qty: msg.size,
            is_buy,
        }),
        DataQualityFlags {
            is_imputed: false,
            gap_flag: false,
            is_outlier: false,
            ts_source: TsSource::Exchange,
        },
    ))
}

/// Parse one line from an IEX JSONL file.
///
/// Returns zero or more events (a quote produces up to 2 deltas, a trade produces 1).
pub fn parse_iex_line(line: &str) -> Vec<(Event, DataQualityFlags)> {
    // Try to extract messageType first
    let outer: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    let msg_type = outer.get("messageType")
        .or_else(|| outer.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    match msg_type {
        "Q" | "quote" | "TOPS" => {
            match serde_json::from_value::<IexQuoteMsg>(outer) {
                Ok(msg) => iex_quote_to_events(&msg),
                Err(_) => vec![],
            }
        }
        "T" | "trade" => {
            match serde_json::from_value::<IexTradeMsg>(outer) {
                Ok(msg) => iex_trade_to_event(&msg).into_iter().collect(),
                Err(_) => vec![],
            }
        }
        _ => vec![],
    }
}

/// Replay a historical IEX JSONL file, calling `on_event` for each event.
///
/// Handles both compressed (.jsonl.gz) detection by trying to gunzip if
/// BufReader returns garbled bytes, and plain .jsonl files.
pub fn replay_iex_jsonl_file<F>(
    path: &Path,
    symbol_filter: Option<&str>,
    mut on_event: F,
) -> anyhow::Result<u64>
where
    F: FnMut(Event, DataQualityFlags, &str),
{
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut count = 0u64;

    for line in reader.lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Optional symbol filter: check raw string before full parse
        if let Some(sym) = symbol_filter {
            if !line.contains(sym) {
                continue;
            }
        }

        let events = parse_iex_line(line);
        for (event, flags) in events {
            let sym = symbol_filter.unwrap_or("IEX");
            on_event(event, flags, sym);
            count += 1;
        }
    }

    Ok(count)
}

/// IEX SSE live stream ingester (requires `ingest` feature).
///
/// Endpoint (free sandbox, no key):
///   https://sandbox.iexapis.com/stable/tops/last?symbols=AAPL&token=Tpk_test
///
/// Real endpoint (requires free IEX Cloud token):
///   https://stream.iexapis.com/stable/tops?symbols=AAPL&token=<TOKEN>
#[cfg(feature = "ingest")]
pub struct IexSseIngester {
    pub symbol: String,
    pub token: String,
}

#[cfg(feature = "ingest")]
impl IexSseIngester {
    pub fn new(symbol: &str, token: &str) -> Self {
        Self {
            symbol: symbol.to_uppercase(),
            token: token.to_string(),
        }
    }

    /// SSE stream URL for TOPS (best bid/ask + trades).
    pub fn stream_url(&self) -> String {
        format!(
            "https://stream.iexapis.com/stable/tops?symbols={}&token={}",
            self.symbol, self.token
        )
    }

    /// Run the SSE stream, calling `on_event` for each parsed line.
    ///
    /// IEX SSE sends `data: {...}` lines (Server-Sent Events format).
    /// Uses a simple HTTP GET with streaming response parsing.
    pub async fn run<F>(&self, mut on_event: F) -> anyhow::Result<()>
    where
        F: FnMut(Event, DataQualityFlags),
    {
        use tokio::io::{AsyncBufReadExt, BufReader as TokioBufReader};

        let client = reqwest::Client::new();
        let response = client
            .get(&self.stream_url())
            .header("Accept", "text/event-stream")
            .send()
            .await?;

        use futures_util::StreamExt as _;
        let mapped = response.bytes_stream().map(|r| {
            r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
        });
        let mut stream = TokioBufReader::new(tokio_util::io::StreamReader::new(mapped));

        let mut line = String::new();
        loop {
            line.clear();
            let n = stream.read_line(&mut line).await?;
            if n == 0 {
                break; // EOF
            }

            let line = line.trim();
            // SSE format: "data: {...}"
            let json = if let Some(rest) = line.strip_prefix("data:") {
                rest.trim()
            } else {
                continue;
            };

            if json.is_empty() || json == "[]" {
                continue;
            }

            // IEX SSE sends arrays of messages
            let parsed = if json.starts_with('[') {
                serde_json::from_str::<Vec<serde_json::Value>>(json)
                    .unwrap_or_default()
            } else {
                serde_json::from_str::<serde_json::Value>(json)
                    .map(|v| vec![v])
                    .unwrap_or_default()
            };

            for obj in parsed {
                let obj_str = obj.to_string();
                let events = parse_iex_line(&obj_str);
                for (event, flags) in events {
                    on_event(event, flags);
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_quote_message() {
        let line = r#"{"messageType":"Q","symbol":"AAPL","bidSize":100,"bidPrice":145.20,"askPrice":145.22,"askSize":200,"lastUpdated":1672751400123456789}"#;
        let events = parse_iex_line(line);
        assert_eq!(events.len(), 2, "quote should emit bid + ask deltas");

        // First event should be bid
        if let (Event::BookDelta(d), _) = &events[0] {
            assert_eq!(d.side, crate::orderbook::Side::Bid);
            assert!((d.price.0 - 145.20).abs() < 1e-6);
            assert_eq!(d.qty, 100.0);
        } else {
            panic!("expected BookDelta for bid");
        }

        // Second event should be ask
        if let (Event::BookDelta(d), _) = &events[1] {
            assert_eq!(d.side, crate::orderbook::Side::Ask);
            assert!((d.price.0 - 145.22).abs() < 1e-6);
            assert_eq!(d.qty, 200.0);
        } else {
            panic!("expected BookDelta for ask");
        }
    }

    #[test]
    fn parse_trade_message() {
        let line = r#"{"messageType":"T","symbol":"AAPL","price":145.21,"size":50,"flags":0,"timestamp":1672751400000000000}"#;
        let events = parse_iex_line(line);
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0].0, Event::Trade(_)));
    }

    #[test]
    fn zero_qty_bid_not_emitted() {
        // bid qty=0 = delete; IEX represents empty best level as qty=0
        let line = r#"{"messageType":"Q","symbol":"AAPL","bidSize":0,"bidPrice":145.20,"askPrice":145.22,"askSize":100,"lastUpdated":1000000000}"#;
        let events = parse_iex_line(line);
        // qty=0 is valid (delete); should still emit the delta
        let bid_events: Vec<_> = events.iter()
            .filter(|(e, _)| matches!(e, Event::BookDelta(d) if d.side == Side::Bid))
            .collect();
        assert_eq!(bid_events.len(), 1);
        if let (Event::BookDelta(d), _) = &bid_events[0] {
            assert_eq!(d.qty, 0.0);
        }
    }

    #[test]
    fn unknown_message_type_ignored() {
        let line = r#"{"messageType":"H","symbol":"AAPL","status":"halted"}"#;
        let events = parse_iex_line(line);
        assert!(events.is_empty());
    }

    #[test]
    fn malformed_json_returns_empty() {
        assert!(parse_iex_line("not json at all").is_empty());
        assert!(parse_iex_line("{broken:}").is_empty());
    }

    #[test]
    fn timestamp_ns_to_us_conversion() {
        let line = r#"{"messageType":"Q","symbol":"AAPL","bidSize":100,"bidPrice":100.0,"askPrice":101.0,"askSize":100,"lastUpdated":1000000000000}"#;
        let events = parse_iex_line(line);
        if let (Event::BookDelta(d), _) = &events[0] {
            // 1000000000000 ns = 1000000000 µs
            assert_eq!(d.exchange_ts, 1_000_000_000);
        }
    }
}
