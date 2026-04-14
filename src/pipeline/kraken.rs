//! Kraken WebSocket v2 — public order book up to 1000 levels.
//!
//! Endpoint: wss://ws.kraken.com/v2
//!
//! Channel: "book"
//! Depth options: 10, 25, 100, 500, 1000
//!
//! Protocol:
//!   1. Subscribe: {"method":"subscribe","params":{"channel":"book","symbol":["BTC/USD"],"depth":10}}
//!   2. First message: type="snapshot" — full book
//!   3. Subsequent: type="update" — only changed levels; qty 0.0 = remove
//!   4. No sequence numbers — use checksum for corruption detection
//!   5. Server sends heartbeats every ~10s; no pong required
//!
//! Symbol format: "BTC/USD" (slash-separated).
//! Works in the US without a VPN.

use ordered_float::OrderedFloat;
use serde::Deserialize;

use crate::orderbook::{BookDelta, Side};
use super::realdata::{DataQualityFlags, Event, TsSource};

// ── Message types ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct KrakenV2Message {
    pub channel: Option<String>,
    #[serde(rename = "type")]
    pub msg_type: Option<String>,
    pub data: Option<Vec<KrakenBookData>>,
    // Subscribe response
    pub method: Option<String>,
    pub success: Option<bool>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct KrakenBookData {
    pub symbol: String,
    /// Bids: [{price, qty}]
    pub bids: Vec<KrakenLevel>,
    /// Asks: [{price, qty}]
    pub asks: Vec<KrakenLevel>,
    pub checksum: Option<u32>,
    pub timestamp: Option<String>,
    pub timestamp_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct KrakenLevel {
    pub price: f64,
    pub qty: f64,
}

// ── Parser ────────────────────────────────────────────────────────────────────

fn parse_kraken_levels(levels: &[KrakenLevel], side: Side, ts_us: i64) -> Vec<BookDelta> {
    levels.iter().filter_map(|l| {
        if !l.price.is_finite() || l.price <= 0.0 { return None; }
        Some(BookDelta {
            price: OrderedFloat(l.price),
            qty: l.qty,
            side,
            exchange_ts: ts_us,
            recv_ts: ts_us,
        })
    }).collect()
}

/// Parse a raw Kraken v2 WebSocket text frame into normalized events.
/// Returns `(symbol, event, flags)`.
pub fn parse_kraken_message(text: &str) -> Vec<(String, Event, DataQualityFlags)> {
    let msg: KrakenV2Message = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(_) => return vec![],
    };

    // Ignore subscribe responses and heartbeats
    match msg.channel.as_deref() {
        Some("heartbeat") | Some("status") => return vec![],
        _ => {}
    }
    if msg.method.as_deref() == Some("subscribe") {
        if msg.success == Some(false) {
            eprintln!("[KRAKEN] subscribe failed: {:?}", msg.error);
        }
        return vec![];
    }

    let msg_type = msg.msg_type.as_deref().unwrap_or("");
    let data = match msg.data {
        Some(d) if !d.is_empty() => d,
        _ => return vec![],
    };
    let book = &data[0];

    // Prefer ms timestamp, fall back to 0
    let ts_us = book.timestamp_ms.map(|ms| ms * 1000).unwrap_or(0);

    let flags = DataQualityFlags {
        is_imputed: false, gap_flag: false,
        is_outlier: false, ts_source: TsSource::Exchange,
    };

    let sym = book.symbol.clone();

    if msg_type == "snapshot" {
        let bids: Vec<(f64, f64)> = book.bids.iter()
            .filter(|l| l.price > 0.0 && l.qty > 0.0)
            .map(|l| (l.price, l.qty))
            .collect();
        let asks: Vec<(f64, f64)> = book.asks.iter()
            .filter(|l| l.price > 0.0 && l.qty > 0.0)
            .map(|l| (l.price, l.qty))
            .collect();
        vec![(sym, Event::BookSnapshot { bids, asks, ts: ts_us }, flags)]
    } else {
        let mut out = Vec::new();
        for delta in parse_kraken_levels(&book.bids, Side::Bid, ts_us) {
            out.push((sym.clone(), Event::BookDelta(delta), flags.clone()));
        }
        for delta in parse_kraken_levels(&book.asks, Side::Ask, ts_us) {
            out.push((sym.clone(), Event::BookDelta(delta), flags.clone()));
        }
        out
    }
}

/// Build the Kraken v2 subscribe message for one or more symbols.
pub fn subscribe_msg(symbols: &[&str], depth: u32) -> String {
    serde_json::json!({
        "method": "subscribe",
        "params": {
            "channel": "book",
            "symbol": symbols,
            "depth": depth,
            "snapshot": true
        }
    }).to_string()
}

/// Normalise symbol to Kraken format, e.g. "BTCUSD" → "BTC/USD".
pub fn to_kraken_symbol(s: &str) -> String {
    // If already contains '/', return as-is.
    if s.contains('/') { return s.to_uppercase(); }
    // Common base currencies
    let bases = ["BTC", "ETH", "SOL", "XRP", "ADA", "DOT", "LINK", "LTC", "BCH", "XLM"];
    let upper = s.to_uppercase();
    for base in bases {
        if upper.starts_with(base) {
            let quote = &upper[base.len()..];
            return format!("{base}/{quote}");
        }
    }
    // Fallback: assume last 3 chars are quote
    let (base, quote) = upper.split_at(upper.len().saturating_sub(3));
    format!("{base}/{quote}")
}

// ── Live ingester ─────────────────────────────────────────────────────────────

#[cfg(feature = "ingest")]
pub struct KrakenIngester {
    pub symbols: Vec<String>,
    pub depth: u32,
    pub reconnect_delay_ms: u64,
    pub max_reconnect_delay_ms: u64,
}

#[cfg(feature = "ingest")]
impl KrakenIngester {
    pub fn new(symbol: &str) -> Self {
        Self {
            symbols: vec![to_kraken_symbol(symbol)],
            depth: 500,
            reconnect_delay_ms: 250,
            max_reconnect_delay_ms: 30_000,
        }
    }

    pub fn new_multi(symbols: &[&str]) -> Self {
        Self {
            symbols: symbols.iter().map(|s| to_kraken_symbol(s)).collect(),
            depth: 500,
            reconnect_delay_ms: 250,
            max_reconnect_delay_ms: 30_000,
        }
    }

    pub fn with_depth(mut self, depth: u32) -> Self { self.depth = depth; self }

    /// Run with a symbol-tagged callback: `FnMut(symbol, Event, DataQualityFlags)`.
    pub async fn run<F>(&mut self, mut on_event: F) -> anyhow::Result<()>
    where
        F: FnMut(String, Event, DataQualityFlags),
    {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        use super::ws::connect_ws;

        let url = "wss://ws.kraken.com/v2";
        let mut delay = self.reconnect_delay_ms;

        loop {
            match connect_ws(url).await {
                Ok((ws, _)) => {
                    delay = self.reconnect_delay_ms;
                    let (mut write, mut read) = ws.split();

                    let syms: Vec<&str> = self.symbols.iter().map(|s| s.as_str()).collect();
                    let sub = subscribe_msg(&syms, self.depth);
                    if write.send(Message::Text(sub.into())).await.is_err() { continue; }

                    while let Some(msg) = read.next().await {
                        match msg {
                            Ok(Message::Text(text)) => {
                                for (sym, event, flags) in parse_kraken_message(&text) {
                                    on_event(sym, event, flags);
                                }
                            }
                            Ok(Message::Ping(p)) => { let _ = write.send(Message::Pong(p)).await; }
                            Ok(Message::Close(_)) | Err(_) => break,
                            _ => {}
                        }
                    }
                }
                Err(e) => eprintln!("[KRAKEN] connect failed: {e}"),
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

    fn snapshot_msg_json(bids: &[(f64, f64)], asks: &[(f64, f64)]) -> String {
        let b: Vec<_> = bids.iter().map(|(p,q)| serde_json::json!({"price": p, "qty": q})).collect();
        let a: Vec<_> = asks.iter().map(|(p,q)| serde_json::json!({"price": p, "qty": q})).collect();
        serde_json::json!({
            "channel": "book",
            "type": "snapshot",
            "data": [{"symbol": "BTC/USD", "bids": b, "asks": a, "checksum": 0, "timestamp_ms": 1_700_000_000_000_i64}]
        }).to_string()
    }

    fn update_msg_json(bids: &[(f64, f64)], asks: &[(f64, f64)]) -> String {
        let b: Vec<_> = bids.iter().map(|(p,q)| serde_json::json!({"price": p, "qty": q})).collect();
        let a: Vec<_> = asks.iter().map(|(p,q)| serde_json::json!({"price": p, "qty": q})).collect();
        serde_json::json!({
            "channel": "book",
            "type": "update",
            "data": [{"symbol": "BTC/USD", "bids": b, "asks": a, "checksum": 0, "timestamp_ms": 1_700_000_001_000_i64}]
        }).to_string()
    }

    #[test]
    fn snapshot_emits_book_snapshot() {
        let events = parse_kraken_message(&snapshot_msg_json(
            &[(67000.0, 1.5), (66999.0, 2.0)],
            &[(67001.0, 0.5)],
        ));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].1, Event::BookSnapshot { .. }));
        if let Event::BookSnapshot { bids, asks, .. } = &events[0].1 {
            assert_eq!(bids.len(), 2);
            assert_eq!(asks.len(), 1);
        }
    }

    #[test]
    fn update_emits_deltas() {
        let events = parse_kraken_message(&update_msg_json(
            &[(67000.0, 2.0)],
            &[(67001.0, 0.0)],
        ));
        assert_eq!(events.len(), 2);
        if let Event::BookDelta(d) = &events[1].1 {
            assert!(d.is_delete()); // qty=0
        }
    }

    #[test]
    fn heartbeat_ignored() {
        let hb = r#"{"channel":"heartbeat","type":"heartbeat","data":[{"timestamp":"2024-01-01T00:00:00Z"}]}"#;
        assert!(parse_kraken_message(hb).is_empty());
    }

    #[test]
    fn symbol_normalisation() {
        assert_eq!(to_kraken_symbol("BTCUSD"), "BTC/USD");
        assert_eq!(to_kraken_symbol("ETHUSD"), "ETH/USD");
        assert_eq!(to_kraken_symbol("BTC/USD"), "BTC/USD");
    }
}
