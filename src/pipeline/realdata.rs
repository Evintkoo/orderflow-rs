//! Real-data ingestion pipeline.
//!
//! Defines the `DataSource` trait and implements the Binance L2 WebSocket adapter.
//! Gated behind the `ingest` feature flag.

use crate::orderbook::BookDelta;
use crate::features::trade_pressure::TradeEvent;
use std::fmt;

/// Normalized event emitted by any data source.
#[derive(Debug, Clone)]
pub enum Event {
    BookDelta(BookDelta),
    Trade(TradeEvent),
    /// Full snapshot for resync after a sequence gap.
    BookSnapshot {
        bids: Vec<(f64, f64)>,
        asks: Vec<(f64, f64)>,
        ts: i64,
    },
    /// Sequence gap detected — downstream should emit NaN for this window.
    Gap { start_ts: i64, end_ts: i64 },
    /// Connection closed / reconnecting.
    Disconnected,
}

/// Sequence validation state for Binance diff streams.
#[derive(Debug, Default)]
pub struct BinanceSeqState {
    /// Last accepted update ID (`u` field from last message).
    pub last_update_id: Option<u64>,
    pub synced: bool,
}

impl BinanceSeqState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Validate a new message. Returns true if the message should be applied.
    ///
    /// Binance sequence rule:
    /// Buffer messages until first message where U ≤ (lastUpdateId+1) ≤ u.
    /// After sync, each message must satisfy U = lastUpdateId+1.
    pub fn validate(&mut self, first_id: u64, last_id: u64) -> SequenceResult {
        if !self.synced {
            if let Some(last) = self.last_update_id {
                if first_id <= last + 1 && last + 1 <= last_id {
                    // Perfect sync window found.
                    self.synced = true;
                    self.last_update_id = Some(last_id);
                    return SequenceResult::Accept;
                }
                if last_id <= last {
                    // Message is older than snapshot — discard silently.
                    return SequenceResult::Buffer;
                }
                // first_id > last + 1: we missed messages, can't sync with this snapshot.
                self.last_update_id = None;
                return SequenceResult::Gap;
            }
            // No snapshot yet — keep buffering
            return SequenceResult::Buffer;
        }

        // Synced: check continuity
        if let Some(last) = self.last_update_id {
            if first_id != last + 1 {
                self.synced = false;
                self.last_update_id = None;
                return SequenceResult::Gap;
            }
        }
        self.last_update_id = Some(last_id);
        SequenceResult::Accept
    }

    /// Called when a REST snapshot is received.
    pub fn set_snapshot_id(&mut self, snapshot_last_id: u64) {
        self.last_update_id = Some(snapshot_last_id);
        self.synced = false; // Will sync on next message that overlaps
    }
}

#[derive(Debug, PartialEq)]
pub enum SequenceResult {
    Accept,
    Buffer,
    Gap,
}

/// Data quality flags for a processed event window.
#[derive(Debug, Clone)]
pub struct DataQualityFlags {
    pub is_imputed: bool,
    pub gap_flag: bool,
    pub is_outlier: bool,
    pub ts_source: TsSource,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TsSource {
    Exchange,
    Local,
}

impl fmt::Display for TsSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TsSource::Exchange => write!(f, "exchange"),
            TsSource::Local => write!(f, "local"),
        }
    }
}

/// Parsed Binance depth update message (L2 diff stream).
#[derive(Debug, serde::Deserialize)]
pub struct BinanceDepthMsg {
    #[serde(rename = "E")]
    pub event_ts: i64,
    #[serde(rename = "U")]
    pub first_update_id: u64,
    #[serde(rename = "u")]
    pub last_update_id: u64,
    #[serde(rename = "b")]
    pub bids: Vec<[String; 2]>,
    #[serde(rename = "a")]
    pub asks: Vec<[String; 2]>,
}

/// Parsed Binance aggregated trade message.
#[derive(Debug, serde::Deserialize)]
pub struct BinanceAggTradeMsg {
    #[serde(rename = "T")]
    pub trade_ts: i64,
    #[serde(rename = "p")]
    pub price: String,
    #[serde(rename = "q")]
    pub qty: String,
    #[serde(rename = "m")]
    pub is_buyer_maker: bool,
}

/// Parse a Binance depth message into BookDelta events.
///
/// Prices are parsed from strings (not computed) to preserve tick precision.
pub fn parse_binance_depth(
    msg: &BinanceDepthMsg,
    recv_ts: i64,
) -> Vec<BookDelta> {
    use crate::orderbook::Side;
    use ordered_float::OrderedFloat;

    let mut deltas = Vec::with_capacity(msg.bids.len() + msg.asks.len());

    for level in &msg.bids {
        if let (Ok(price), Ok(qty)) = (level[0].parse::<f64>(), level[1].parse::<f64>()) {
            if price > 0.0 {
                deltas.push(BookDelta {
                    price: OrderedFloat(price),
                    qty,
                    side: Side::Bid,
                    exchange_ts: msg.event_ts * 1000, // ms → µs
                    recv_ts,
                });
            }
        }
    }

    for level in &msg.asks {
        if let (Ok(price), Ok(qty)) = (level[0].parse::<f64>(), level[1].parse::<f64>()) {
            if price > 0.0 {
                deltas.push(BookDelta {
                    price: OrderedFloat(price),
                    qty,
                    side: Side::Ask,
                    exchange_ts: msg.event_ts * 1000,
                    recv_ts,
                });
            }
        }
    }

    deltas
}

/// Parse a Binance agg-trade message into a TradeEvent.
pub fn parse_binance_trade(
    msg: &BinanceAggTradeMsg,
    _recv_ts: i64,
) -> Option<TradeEvent> {
    let price = msg.price.parse::<f64>().ok()?;
    let qty = msg.qty.parse::<f64>().ok()?;
    if price <= 0.0 || qty <= 0.0 {
        return None;
    }
    Some(TradeEvent {
        ts: msg.trade_ts * 1000, // ms → µs
        price,
        qty,
        // is_buyer_maker=true means the buyer is the passive side → taker is seller
        is_buy: !msg.is_buyer_maker,
    })
}

/// Binance WebSocket ingestion runner.
///
/// Connects to `wss://stream.binance.com:9443/stream?streams=<symbol>@depth@100ms/<symbol>@aggTrade`
/// and drives the provided callbacks on each event.
#[cfg(feature = "ingest")]
pub struct BinanceIngester {
    pub symbols: Vec<String>,
    /// Per-symbol sequence state (keyed by lowercase symbol)
    pub seq_states: std::collections::HashMap<String, BinanceSeqState>,
    pub reconnect_delay_ms: u64,
    pub max_reconnect_delay_ms: u64,
}

#[cfg(feature = "ingest")]
impl BinanceIngester {
    pub fn new(symbol: &str) -> Self {
        let sym = symbol.to_lowercase();
        let mut seq_states = std::collections::HashMap::new();
        seq_states.insert(sym.clone(), BinanceSeqState::new());
        Self {
            symbols: vec![sym],
            seq_states,
            reconnect_delay_ms: 250,
            max_reconnect_delay_ms: 30_000,
        }
    }

    pub fn new_multi(symbols: &[&str]) -> Self {
        let syms: Vec<String> = symbols.iter().map(|s| s.to_lowercase()).collect();
        let seq_states = syms.iter().map(|s| (s.clone(), BinanceSeqState::new())).collect();
        Self { symbols: syms, seq_states, reconnect_delay_ms: 250, max_reconnect_delay_ms: 30_000 }
    }

    /// Build the combined-stream URL for all symbols.
    pub fn ws_url(&self) -> String {
        let streams: Vec<String> = self.symbols.iter()
            .flat_map(|s| [format!("{s}@depth@100ms"), format!("{s}@aggTrade")])
            .collect();
        format!("wss://stream.binance.us:9443/stream?streams={}", streams.join("/")

        )
    }

    /// Build the REST URL for a depth snapshot for a specific symbol.
    pub fn rest_snapshot_url(&self, symbol: &str, limit: u32) -> String {
        format!(
            "https://api.binance.us/api/v3/depth?symbol={}&limit={}",
            symbol.to_uppercase(),
            limit
        )
    }

    /// Run the ingestion loop with proper Binance sequence sync protocol:
    ///
    /// 1. Open WebSocket and start buffering depth messages.
    /// 2. Fetch REST snapshot (1000 levels) to initialize the book.
    /// 3. Discard buffered messages older than snapshot's lastUpdateId.
    /// 4. Apply first message where U ≤ lastUpdateId+1 ≤ u, then continue.
    /// 5. Reconnect with exponential backoff on disconnect or sequence gap.
    /// Run with a symbol-tagged callback: `FnMut(symbol, Event, DataQualityFlags)`.
    /// Fetches a REST snapshot for each symbol, then processes the combined WS stream.
    pub async fn run<F>(&mut self, mut on_event: F) -> anyhow::Result<()>
    where
        F: FnMut(String, Event, DataQualityFlags),
    {
        use futures_util::StreamExt;
        use tokio_tungstenite::tungstenite::Message;
        use super::ws::connect_ws;

        let mut delay = self.reconnect_delay_ms;

        loop {
            let (ws_stream, _) = match connect_ws(self.ws_url()).await {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("[BINANCE] connect failed: {e}");
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    delay = (delay * 2).min(self.max_reconnect_delay_ms);
                    continue;
                }
            };
            delay = self.reconnect_delay_ms;

            // Reset all per-symbol seq states
            for state in self.seq_states.values_mut() { *state = BinanceSeqState::new(); }

            let (_, mut read) = ws_stream.split();

            // Fetch REST snapshots for all symbols concurrently, buffering WS frames meanwhile.
            let mut buffer: Vec<String> = Vec::with_capacity(200);
            let syms = self.symbols.clone();
            let snapshot_futs: Vec<_> = syms.iter().map(|sym| {
                let url = self.rest_snapshot_url(sym, 1000);
                async move {
                    let json: Option<serde_json::Value> = match reqwest::get(&url).await {
                        Ok(r) => r.json().await.ok(),
                        Err(e) => { eprintln!("[BINANCE/{sym}] REST failed: {e}"); None }
                    };
                    (sym.clone(), json)
                }
            }).collect();
            let snapshots: Vec<(String, Option<serde_json::Value>)> =
                futures_util::future::join_all(snapshot_futs).await;

            // Drain brief WS buffer
            let drain_deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(600);
            loop {
                let remaining = drain_deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() { break; }
                match tokio::time::timeout(remaining, read.next()).await {
                    Ok(Some(Ok(Message::Text(t)))) => {
                        buffer.push(t.to_string());
                        if buffer.len() > 500 { buffer.remove(0); }
                    }
                    _ => break,
                }
            }

            // Apply snapshots and set seq states
            for (sym, snap_opt) in snapshots {
                if let Some(snap) = snap_opt {
                    let last_id = snap["lastUpdateId"].as_u64().unwrap_or(0);
                    if let Some(state) = self.seq_states.get_mut(&sym) {
                        state.set_snapshot_id(last_id);
                    }
                    let bids: Vec<(f64, f64)> = snap["bids"].as_array().unwrap_or(&vec![])
                        .iter().filter_map(|l| {
                            let p: f64 = l[0].as_str()?.parse().ok()?;
                            let q: f64 = l[1].as_str()?.parse().ok()?;
                            if p > 0.0 && q > 0.0 { Some((p, q)) } else { None }
                        }).collect();
                    let asks: Vec<(f64, f64)> = snap["asks"].as_array().unwrap_or(&vec![])
                        .iter().filter_map(|l| {
                            let p: f64 = l[0].as_str()?.parse().ok()?;
                            let q: f64 = l[1].as_str()?.parse().ok()?;
                            if p > 0.0 && q > 0.0 { Some((p, q)) } else { None }
                        }).collect();
                    let n = bids.len() + asks.len();
                    on_event(sym.clone(), Event::BookSnapshot { bids, asks, ts: 0 },
                        DataQualityFlags { is_imputed: false, gap_flag: false,
                            is_outlier: false, ts_source: TsSource::Exchange });
                    eprintln!("[BINANCE/{sym}] snapshot: {n} levels, lastUpdateId={last_id}");
                }
            }

            // Replay buffer then process live stream
            let mut gap_sym: Option<String> = None;
            for text in buffer.drain(..) {
                if let Some(sym) = self.handle_message_tagged(&text, &mut on_event) {
                    gap_sym = Some(sym); break;
                }
            }

            if gap_sym.is_none() {
                while let Some(msg) = read.next().await {
                    match msg {
                        Ok(Message::Text(text)) => {
                            if let Some(sym) = self.handle_message_tagged(&text, &mut on_event) {
                                gap_sym = Some(sym);
                                break;
                            }
                        }
                        Ok(Message::Ping(_)) => {}
                        Ok(Message::Close(_)) | Err(_) => break,
                        _ => {}
                    }
                }
            }

            // Emit Disconnected for all symbols
            for sym in &self.symbols {
                on_event(sym.clone(), Event::Disconnected, DataQualityFlags {
                    is_imputed: true, gap_flag: true,
                    is_outlier: false, ts_source: TsSource::Local,
                });
            }

            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            delay = (delay * 2).min(self.max_reconnect_delay_ms);
        }
    }

    /// Returns `Some(symbol)` if a gap was detected (caller should reconnect).
    fn handle_message_tagged<F>(&mut self, text: &str, on_event: &mut F) -> Option<String>
    where
        F: FnMut(String, Event, DataQualityFlags),
    {
        let outer: serde_json::Value = serde_json::from_str(text).ok()?;
        let stream = outer["stream"].as_str().unwrap_or("");
        let data = &outer["data"];
        let recv_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as i64;

        // Extract symbol from stream name: "btcusdt@depth@100ms" → "btcusdt"
        let sym_lower = stream.split('@').next().unwrap_or("").to_string();

        if stream.ends_with("@depth@100ms") {
            if let Ok(msg) = serde_json::from_value::<BinanceDepthMsg>(data.clone()) {
                let seq_result = self.seq_states
                    .entry(sym_lower.clone())
                    .or_insert_with(BinanceSeqState::new)
                    .validate(msg.first_update_id, msg.last_update_id);
                match seq_result {
                    SequenceResult::Accept => {
                        let sym_upper = sym_lower.to_uppercase();
                        let ok_flags = DataQualityFlags { is_imputed: false, gap_flag: false,
                            is_outlier: false, ts_source: TsSource::Exchange };
                        for delta in parse_binance_depth(&msg, recv_ts) {
                            on_event(sym_upper.clone(), Event::BookDelta(delta), ok_flags.clone());
                        }
                    }
                    SequenceResult::Gap => {
                        eprintln!("[BINANCE/{sym_lower}] gap — reconnecting");
                        on_event(sym_lower.to_uppercase(),
                            Event::Gap { start_ts: recv_ts - 500_000, end_ts: recv_ts },
                            DataQualityFlags { is_imputed: true, gap_flag: true,
                                is_outlier: false, ts_source: TsSource::Local });
                        return Some(sym_lower);
                    }
                    SequenceResult::Buffer => {}
                }
            }
        } else if stream.ends_with("@aggTrade") {
            if let Ok(msg) = serde_json::from_value::<BinanceAggTradeMsg>(data.clone()) {
                if let Some(trade) = parse_binance_trade(&msg, recv_ts) {
                    on_event(sym_lower.to_uppercase(), Event::Trade(trade),
                        DataQualityFlags { is_imputed: false, gap_flag: false,
                            is_outlier: false, ts_source: TsSource::Exchange });
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binance_seq_initial_sync() {
        let mut state = BinanceSeqState::new();
        state.set_snapshot_id(100);
        // First message that overlaps snapshot
        assert_eq!(state.validate(99, 105), SequenceResult::Accept);
        assert!(state.synced);
    }

    #[test]
    fn binance_seq_detects_gap() {
        let mut state = BinanceSeqState::new();
        state.set_snapshot_id(100);
        state.validate(99, 105); // Sync
        // Gap: next message should start at 106, but starts at 108
        assert_eq!(state.validate(108, 110), SequenceResult::Gap);
        assert!(!state.synced);
    }

    #[test]
    fn binance_seq_continuous_accepts() {
        let mut state = BinanceSeqState::new();
        state.set_snapshot_id(100);
        state.validate(99, 105);
        assert_eq!(state.validate(106, 110), SequenceResult::Accept);
        assert_eq!(state.validate(111, 115), SequenceResult::Accept);
    }

    #[test]
    fn parse_binance_depth_parses_from_string() {
        let msg = BinanceDepthMsg {
            event_ts: 1_700_000_000_000,
            first_update_id: 1,
            last_update_id: 5,
            bids: vec![["100.5".into(), "10.0".into()]],
            asks: vec![["101.0".into(), "5.5".into()], ["101.5".into(), "0.0".into()]],
        };
        let deltas = parse_binance_depth(&msg, 0);
        assert_eq!(deltas.len(), 3);
        // First delta is the bid
        assert!((deltas[0].price.0 - 100.5).abs() < f64::EPSILON);
        assert_eq!(deltas[0].qty, 10.0);
        // Third delta has qty=0 (delete)
        assert!(deltas[2].is_delete());
    }

    #[test]
    fn parse_binance_trade_direction() {
        let msg = BinanceAggTradeMsg {
            trade_ts: 1_700_000_000_000,
            price: "50000.0".into(),
            qty: "0.5".into(),
            is_buyer_maker: false, // taker is buyer → is_buy = true
        };
        let trade = parse_binance_trade(&msg, 0).unwrap();
        assert!(trade.is_buy);
    }
}
