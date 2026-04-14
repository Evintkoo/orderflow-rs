//! Polygon.io WebSocket — real-time and delayed US stock market data.
//!
//! Endpoints:
//!   wss://delayed.polygon.io/stocks  — free tier (15-min delayed trades/quotes)
//!   wss://socket.polygon.io/stocks   — real-time (paid subscription)
//!
//! Protocol:
//!   1. Connect → server sends [{"ev":"status","status":"connected",...}]
//!   2. Authenticate: {"action":"auth","params":"API_KEY"}
//!      Server responds: [{"ev":"status","status":"auth_success",...}]
//!   3. Subscribe: {"action":"subscribe","params":"T.AAPL,Q.AAPL,T.SPY,Q.SPY"}
//!   4. Data: [{"ev":"T",...}] trades,  [{"ev":"Q",...}] NBBO quotes
//!
//! Note: Polygon provides NBBO quotes (best bid/ask only, not full L2).
//! Quotes are emitted as two BookDelta events — one bid, one ask — overwriting
//! the best level. This mirrors the Alpaca top-of-book approach.
//!
//! API key: free at https://polygon.io (delayed data, no paid plan needed).

use ordered_float::OrderedFloat;
use serde::Deserialize;

use crate::orderbook::{BookDelta, Side};
use crate::features::trade_pressure::TradeEvent;
use super::realdata::{DataQualityFlags, Event, TsSource};

// ── Message types ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct PolygonMessage {
    #[serde(rename = "ev")]
    pub event_type: String,

    // Common / trade fields
    #[serde(rename = "sym")]
    pub symbol: Option<String>,
    /// Trade price
    #[serde(rename = "p")]
    pub price: Option<f64>,
    /// Trade size
    #[serde(rename = "s")]
    pub size: Option<f64>,

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

    /// Timestamp in milliseconds since UNIX epoch
    #[serde(rename = "t")]
    pub timestamp_ms: Option<i64>,

    // Status / control
    pub status: Option<String>,
    pub message: Option<String>,
}

// ── Parser ────────────────────────────────────────────────────────────────────

/// Parse a Polygon.io WebSocket message (JSON array of events).
///
/// Returns `(symbol, Event, DataQualityFlags)` tuples.
/// - Status messages are logged if auth fails, then discarded.
/// - Quote (`ev`="Q"): emits bid BookDelta + ask BookDelta (overwrites best level).
/// - Trade (`ev`="T"): emits a `Trade` event.
/// - Aggregates (`ev`="A" / `ev`="AM"): ignored.
pub fn parse_polygon_messages(text: &str) -> Vec<(String, Event, DataQualityFlags)> {
    let msgs: Vec<PolygonMessage> = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(_) => return vec![],
    };

    let mut out = Vec::new();
    for msg in msgs {
        let flags = DataQualityFlags {
            is_imputed: false,
            gap_flag: false,
            is_outlier: false,
            ts_source: TsSource::Exchange,
        };
        let ts_us = msg.timestamp_ms.unwrap_or(0) * 1_000; // ms → µs
        let sym = msg.symbol.clone().unwrap_or_default();

        match msg.event_type.as_str() {
            "status" => {
                // Log auth failures; all other status messages are silently dropped.
                if msg.status.as_deref() == Some("auth_failed") {
                    eprintln!("[POLYGON] auth failed: {:?}", msg.message);
                }
            }
            "Q" => {
                // NBBO quote — emit bid delta then ask delta
                if let (Some(bp), Some(bs)) = (msg.bid_price, msg.bid_size) {
                    if bp > 0.0 {
                        out.push((sym.clone(), Event::BookDelta(BookDelta {
                            price: OrderedFloat(bp),
                            qty: bs.max(0.0),
                            side: Side::Bid,
                            exchange_ts: ts_us,
                            recv_ts: ts_us,
                        }), flags.clone()));
                    }
                }
                if let (Some(ap), Some(as_)) = (msg.ask_price, msg.ask_size) {
                    if ap > 0.0 {
                        out.push((sym, Event::BookDelta(BookDelta {
                            price: OrderedFloat(ap),
                            qty: as_.max(0.0),
                            side: Side::Ask,
                            exchange_ts: ts_us,
                            recv_ts: ts_us,
                        }), flags));
                    }
                }
            }
            "T" => {
                // Trade
                if let (Some(p), Some(s)) = (msg.price, msg.size) {
                    if p > 0.0 && s > 0.0 {
                        out.push((sym, Event::Trade(TradeEvent {
                            price: p,
                            qty: s,
                            ts: ts_us,
                            // Direction unknown from Polygon trade feed
                            is_buy: false,
                        }), flags));
                    }
                }
            }
            // Aggregates (second / minute bars) — ignored for tick-level research
            "A" | "AM" => {}
            _ => {}
        }
    }
    out
}

// ── Symbol helper ─────────────────────────────────────────────────────────────

/// Build the subscribe `params` string for trades + quotes.
///
/// ```
/// # use orderflow_rs::pipeline::polygon::subscribe_params;
/// assert_eq!(
///     subscribe_params(&["AAPL", "SPY"]),
///     "T.AAPL,Q.AAPL,T.SPY,Q.SPY"
/// );
/// ```
pub fn subscribe_params(symbols: &[&str]) -> String {
    symbols
        .iter()
        .flat_map(|s| {
            let up = s.to_uppercase();
            [format!("T.{up}"), format!("Q.{up}")]
        })
        .collect::<Vec<_>>()
        .join(",")
}

// ── REST snapshot structs ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct PolygonSnapshotResponse {
    pub status: String,
    #[serde(default)]
    pub tickers: Vec<PolygonTickerSnapshot>,
}

#[derive(Debug, Deserialize)]
pub struct PolygonTickerSnapshot {
    pub ticker: String,
    #[serde(rename = "lastQuote")]
    pub last_quote: Option<PolygonLastQuote>,
    #[serde(rename = "lastTrade")]
    pub last_trade: Option<PolygonLastTrade>,
}

#[derive(Debug, Deserialize)]
pub struct PolygonLastQuote {
    #[serde(rename = "P")]
    pub ask_price: Option<f64>,
    #[serde(rename = "S")]
    pub ask_size: Option<f64>,
    #[serde(rename = "p")]
    pub bid_price: Option<f64>,
    #[serde(rename = "s")]
    pub bid_size: Option<f64>,
    /// Nanoseconds since epoch
    #[serde(rename = "t")]
    pub ts_ns: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct PolygonLastTrade {
    #[serde(rename = "p")]
    pub price: Option<f64>,
    #[serde(rename = "s")]
    pub size: Option<f64>,
    #[serde(rename = "t")]
    pub ts_ns: Option<i64>,
}

// ── REST snapshot parser ──────────────────────────────────────────────────────

/// Parse a Polygon REST snapshot response into (symbol, event, flags) tuples.
///
/// For each ticker:
/// - Emits bid BookDelta + ask BookDelta from `lastQuote` (if present and prices > 0).
/// - Emits Trade from `lastTrade` (if present and price > 0).
/// - Timestamps are converted: ns / 1000 → microseconds.
pub fn parse_polygon_snapshot(response: &PolygonSnapshotResponse) -> Vec<(String, Event, DataQualityFlags)> {
    let mut out = Vec::new();

    for ticker in &response.tickers {
        let sym = ticker.ticker.clone();
        let flags = DataQualityFlags {
            is_imputed: false,
            gap_flag: false,
            is_outlier: false,
            ts_source: TsSource::Exchange,
        };

        if let Some(q) = &ticker.last_quote {
            let ts_us = q.ts_ns.unwrap_or(0) / 1_000; // ns → µs

            if let (Some(bp), Some(bs)) = (q.bid_price, q.bid_size) {
                if bp > 0.0 {
                    out.push((sym.clone(), Event::BookDelta(BookDelta {
                        price: OrderedFloat(bp),
                        qty: bs.max(0.0),
                        side: Side::Bid,
                        exchange_ts: ts_us,
                        recv_ts: ts_us,
                    }), flags.clone()));
                }
            }

            if let (Some(ap), Some(as_)) = (q.ask_price, q.ask_size) {
                if ap > 0.0 {
                    out.push((sym.clone(), Event::BookDelta(BookDelta {
                        price: OrderedFloat(ap),
                        qty: as_.max(0.0),
                        side: Side::Ask,
                        exchange_ts: ts_us,
                        recv_ts: ts_us,
                    }), flags.clone()));
                }
            }
        }

        if let Some(t) = &ticker.last_trade {
            let ts_us = t.ts_ns.unwrap_or(0) / 1_000; // ns → µs
            if let (Some(p), Some(s)) = (t.price, t.size) {
                if p > 0.0 {
                    out.push((sym, Event::Trade(TradeEvent {
                        price: p,
                        qty: s,
                        ts: ts_us,
                        is_buy: false,
                    }), flags));
                }
            }
        }
    }

    out
}

// ── Live ingester ─────────────────────────────────────────────────────────────

/// Polygon.io WebSocket ingester for US stock market data.
///
/// Defaults to the free delayed endpoint (`wss://delayed.polygon.io/stocks`).
/// Call `.with_realtime()` to switch to the paid real-time endpoint.
#[cfg(feature = "ingest")]
pub struct PolygonIngester {
    pub symbols: Vec<String>,
    pub api_key: String,
    /// WebSocket endpoint URL.
    pub endpoint: String,
    pub reconnect_delay_ms: u64,
    pub max_reconnect_delay_ms: u64,
}

#[cfg(feature = "ingest")]
impl PolygonIngester {
    /// Create a new ingester using the **free delayed** endpoint.
    pub fn new(symbols: &[&str], api_key: &str) -> Self {
        Self {
            symbols: symbols.iter().map(|s| s.to_uppercase()).collect(),
            api_key: api_key.to_string(),
            endpoint: "wss://delayed.polygon.io/stocks".to_string(),
            reconnect_delay_ms: 500,
            max_reconnect_delay_ms: 30_000,
        }
    }

    /// Switch to the **real-time** (paid) endpoint.
    pub fn with_realtime(mut self) -> Self {
        self.endpoint = "wss://socket.polygon.io/stocks".to_string();
        self
    }

    /// Run the ingestion loop, dispatching events to `on_event`.
    ///
    /// Connects, authenticates, subscribes, and loops on messages.
    /// On disconnect emits `Event::Disconnected` and reconnects with
    /// exponential backoff up to `max_reconnect_delay_ms`.
    pub async fn run<F>(&mut self, mut on_event: F) -> anyhow::Result<()>
    where
        F: FnMut(String, Event, DataQualityFlags),
    {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        use super::ws::connect_ws;

        let mut delay = self.reconnect_delay_ms;

        loop {
            match connect_ws(self.endpoint.as_str()).await {
                Ok((ws, _)) => {
                    delay = self.reconnect_delay_ms;
                    let (mut write, mut read) = ws.split();

                    // Send auth immediately (Polygon accepts it before the connected message)
                    let auth = serde_json::json!({
                        "action": "auth",
                        "params": self.api_key,
                    })
                    .to_string();
                    if write.send(Message::Text(auth.into())).await.is_err() {
                        continue;
                    }

                    let mut authenticated = false;

                    while let Some(msg) = read.next().await {
                        match msg {
                            Ok(Message::Text(text)) => {
                                if !authenticated {
                                    if text.contains("auth_success") {
                                        authenticated = true;
                                        // Subscribe to trades + quotes for all symbols
                                        let syms: Vec<&str> =
                                            self.symbols.iter().map(|s| s.as_str()).collect();
                                        let sub = serde_json::json!({
                                            "action": "subscribe",
                                            "params": subscribe_params(&syms),
                                        })
                                        .to_string();
                                        if write.send(Message::Text(sub.into())).await.is_err() {
                                            break;
                                        }
                                    } else if text.contains("auth_failed") {
                                        eprintln!("[POLYGON] auth failed: {text}");
                                        break;
                                    }
                                    // Also dispatch any data in the pre-auth burst
                                }
                                for (sym, event, flags) in parse_polygon_messages(&text) {
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
                Err(e) => eprintln!("[POLYGON] connect failed: {e}"),
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

// ── REST ingester ─────────────────────────────────────────────────────────────

/// Polygon.io REST polling ingester for US stock market data.
///
/// Uses the snapshot endpoint to get current NBBO quotes and last trade for all
/// symbols every `poll_interval_secs` seconds (default 15). Compatible with the
/// free Polygon.io API key tier (no WebSocket access required).
#[cfg(feature = "ingest")]
pub struct PolygonRestIngester {
    pub symbols: Vec<String>,
    pub api_key: String,
    /// Polling interval in seconds (default 15).
    pub poll_interval_secs: u64,
}

#[cfg(feature = "ingest")]
impl PolygonRestIngester {
    pub fn new(symbols: &[&str], api_key: &str) -> Self {
        Self {
            symbols: symbols.iter().map(|s| s.to_uppercase()).collect(),
            api_key: api_key.to_string(),
            poll_interval_secs: 15,
        }
    }

    pub async fn run<F>(&mut self, mut on_event: F) -> anyhow::Result<()>
    where
        F: FnMut(String, Event, DataQualityFlags),
    {
        use reqwest::Client;
        let client = Client::new();
        let mut interval = tokio::time::interval(
            std::time::Duration::from_secs(self.poll_interval_secs)
        );

        loop {
            interval.tick().await;

            let tickers = self.symbols.join(",");
            let url = format!(
                "https://api.polygon.io/v2/snapshot/locale/us/markets/stocks/tickers?tickers={}&apiKey={}",
                tickers, self.api_key
            );

            match client.get(&url).send().await {
                Ok(resp) => {
                    match resp.json::<PolygonSnapshotResponse>().await {
                        Ok(data) => {
                            for (sym, event, flags) in parse_polygon_snapshot(&data) {
                                on_event(sym, event, flags);
                            }
                        }
                        Err(e) => eprintln!("[POLYGON/REST] parse error: {e}"),
                    }
                }
                Err(e) => eprintln!("[POLYGON/REST] request failed: {e}"),
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_quote_emits_two_deltas() {
        let json = r#"[{"ev":"Q","sym":"AAPL","bp":150.20,"bs":200.0,"ap":150.25,"as":100.0,"t":1700000000000}]"#;
        let events = parse_polygon_messages(json);
        assert_eq!(events.len(), 2, "quote should emit exactly 2 BookDelta events");

        let (sym0, ref ev0, _) = &events[0];
        let (sym1, ref ev1, _) = &events[1];

        assert_eq!(sym0, "AAPL");
        assert_eq!(sym1, "AAPL");

        if let Event::BookDelta(d) = ev0 {
            assert_eq!(d.side, Side::Bid);
            assert!((d.price.0 - 150.20).abs() < 1e-9);
            assert_eq!(d.qty, 200.0);
            assert_eq!(d.exchange_ts, 1_700_000_000_000_000); // ms * 1000
        } else {
            panic!("expected BookDelta for bid, got {:?}", ev0);
        }

        if let Event::BookDelta(d) = ev1 {
            assert_eq!(d.side, Side::Ask);
            assert!((d.price.0 - 150.25).abs() < 1e-9);
            assert_eq!(d.qty, 100.0);
        } else {
            panic!("expected BookDelta for ask, got {:?}", ev1);
        }
    }

    #[test]
    fn parse_trade_emits_trade_event() {
        let json = r#"[{"ev":"T","sym":"AAPL","p":150.23,"s":100.0,"t":1700000000000,"c":[14,41]}]"#;
        let events = parse_polygon_messages(json);
        assert_eq!(events.len(), 1, "trade should emit exactly 1 event");

        let (sym, ref ev, _) = &events[0];
        assert_eq!(sym, "AAPL");

        if let Event::Trade(t) = ev {
            assert!((t.price - 150.23).abs() < 1e-9);
            assert_eq!(t.qty, 100.0);
            assert_eq!(t.ts, 1_700_000_000_000_000);
        } else {
            panic!("expected Trade, got {:?}", ev);
        }
    }

    #[test]
    fn status_message_ignored() {
        // Connected status
        let connected = r#"[{"ev":"status","status":"connected","message":"Connected Successfully"}]"#;
        assert!(parse_polygon_messages(connected).is_empty(), "connected status should return empty");

        // Auth success
        let auth_ok = r#"[{"ev":"status","status":"auth_success","message":"authenticated"}]"#;
        assert!(parse_polygon_messages(auth_ok).is_empty(), "auth_success should return empty");

        // Unrecognised status
        let other = r#"[{"ev":"status","status":"something_else","message":"..."}]"#;
        assert!(parse_polygon_messages(other).is_empty(), "unknown status should return empty");
    }

    #[test]
    fn subscribe_params_format() {
        let params = subscribe_params(&["AAPL", "SPY"]);
        assert_eq!(params, "T.AAPL,Q.AAPL,T.SPY,Q.SPY");
    }

    #[test]
    fn subscribe_params_uppercases() {
        let params = subscribe_params(&["aapl", "qqq"]);
        assert_eq!(params, "T.AAPL,Q.AAPL,T.QQQ,Q.QQQ");
    }

    #[test]
    fn aggregate_messages_ignored() {
        let agg = r#"[{"ev":"A","sym":"AAPL","o":150.0,"c":151.0,"h":152.0,"l":149.0,"v":1000000,"t":1700000000000}]"#;
        assert!(parse_polygon_messages(agg).is_empty());

        let agg_min = r#"[{"ev":"AM","sym":"SPY","o":440.0,"c":441.0,"h":442.0,"l":439.0,"v":5000000,"t":1700000000000}]"#;
        assert!(parse_polygon_messages(agg_min).is_empty());
    }

    #[test]
    fn malformed_json_returns_empty() {
        assert!(parse_polygon_messages("not json").is_empty());
        assert!(parse_polygon_messages("").is_empty());
    }

    #[test]
    fn snapshot_quote_emits_two_book_deltas() {
        let response = PolygonSnapshotResponse {
            status: "OK".to_string(),
            tickers: vec![PolygonTickerSnapshot {
                ticker: "AAPL".to_string(),
                last_quote: Some(PolygonLastQuote {
                    ask_price: Some(150.25),
                    ask_size: Some(200.0),
                    bid_price: Some(150.20),
                    bid_size: Some(100.0),
                    ts_ns: Some(1_700_000_000_000_000_000),
                }),
                last_trade: None,
            }],
        };

        let events = parse_polygon_snapshot(&response);
        assert_eq!(events.len(), 2, "quote should emit exactly 2 BookDelta events");

        let (sym0, ref ev0, _) = &events[0];
        let (sym1, ref ev1, _) = &events[1];

        assert_eq!(sym0, "AAPL");
        assert_eq!(sym1, "AAPL");

        if let Event::BookDelta(d) = ev0 {
            assert_eq!(d.side, Side::Bid);
            assert!((d.price.0 - 150.20).abs() < 1e-9);
            assert_eq!(d.qty, 100.0);
            // ns / 1000 = µs
            assert_eq!(d.exchange_ts, 1_700_000_000_000_000);
        } else {
            panic!("expected BookDelta for bid, got {:?}", ev0);
        }

        if let Event::BookDelta(d) = ev1 {
            assert_eq!(d.side, Side::Ask);
            assert!((d.price.0 - 150.25).abs() < 1e-9);
            assert_eq!(d.qty, 200.0);
        } else {
            panic!("expected BookDelta for ask, got {:?}", ev1);
        }
    }

    #[test]
    fn snapshot_trade_emits_trade_event() {
        let response = PolygonSnapshotResponse {
            status: "OK".to_string(),
            tickers: vec![PolygonTickerSnapshot {
                ticker: "SPY".to_string(),
                last_quote: None,
                last_trade: Some(PolygonLastTrade {
                    price: Some(440.10),
                    size: Some(50.0),
                    ts_ns: Some(1_700_000_000_000_000_000),
                }),
            }],
        };

        let events = parse_polygon_snapshot(&response);
        assert_eq!(events.len(), 1, "trade should emit exactly 1 Trade event");

        let (sym, ref ev, _) = &events[0];
        assert_eq!(sym, "SPY");

        if let Event::Trade(t) = ev {
            assert!((t.price - 440.10).abs() < 1e-9);
            assert_eq!(t.qty, 50.0);
            assert_eq!(t.ts, 1_700_000_000_000_000); // ns / 1000
        } else {
            panic!("expected Trade, got {:?}", ev);
        }
    }

    #[test]
    fn multi_event_array() {
        // Array with a trade and a quote together
        let json = r#"[
            {"ev":"T","sym":"SPY","p":440.10,"s":50.0,"t":1700000000001},
            {"ev":"Q","sym":"SPY","bp":440.05,"bs":300.0,"ap":440.15,"as":200.0,"t":1700000000002}
        ]"#;
        let events = parse_polygon_messages(json);
        // 1 trade + 2 book deltas
        assert_eq!(events.len(), 3);
        assert!(matches!(events[0].1, Event::Trade(_)));
        assert!(matches!(events[1].1, Event::BookDelta(_)));
        assert!(matches!(events[2].1, Event::BookDelta(_)));
    }
}
