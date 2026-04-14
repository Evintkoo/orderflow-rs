//! Yahoo Finance ingester — historical 1-min bars + live NBBO polling.
//!
//! No API key required.
//!
//! ## Historical backfill
//!
//! GET /v8/finance/chart/{SYMBOL}?interval=1m&range=60d
//!
//! Returns up to 60 days of 1-minute OHLCV bars.  Each bar is emitted as:
//!   - Trade event (close price, bar volume)
//!   - BookSnapshot with synthetic NBBO: bid = close − $0.01, ask = close + $0.01
//!
//! Timestamps come from the bar's Unix second timestamp (→ µs).  The label queue
//! uses these historical timestamps, so all 60-day bars are fully labeled before
//! live polling begins.
//!
//! ## Live NBBO polling
//!
//! GET /v7/finance/quote?symbols=AAPL,...&fields=bid,ask,...
//!
//! Fetches real bid/ask/last for all symbols in a single request every
//! `poll_interval_ms` milliseconds (default 2 000 ms).  Emits:
//!   - BookSnapshot with real bid/ask sizes
//!   - Trade event (regularMarketPrice, regularMarketVolume)

use serde::Deserialize;

use crate::features::trade_pressure::TradeEvent;
use super::realdata::{DataQualityFlags, Event, TsSource};

// ── Chart response types ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ChartResponse {
    pub chart: ChartWrapper,
}

#[derive(Debug, Deserialize)]
pub struct ChartWrapper {
    pub result: Option<Vec<ChartResult>>,
    pub error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct ChartResult {
    pub meta: ChartMeta,
    pub timestamp: Option<Vec<Option<i64>>>,
    pub indicators: Option<ChartIndicators>,
}

#[derive(Debug, Deserialize)]
pub struct ChartMeta {
    pub symbol: String,
}

#[derive(Debug, Deserialize)]
pub struct ChartIndicators {
    pub quote: Option<Vec<ChartQuote>>,
}

#[derive(Debug, Deserialize)]
pub struct ChartQuote {
    pub close: Option<Vec<Option<f64>>>,
    pub volume: Option<Vec<Option<f64>>>,
}

// ── Live quote response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct QuoteResponse {
    #[serde(rename = "quoteResponse")]
    pub quote_response: QuoteWrapper,
}

#[derive(Debug, Deserialize)]
pub struct QuoteWrapper {
    pub result: Option<Vec<LiveQuote>>,
    pub error: Option<serde_json::Value>,
}

#[allow(non_snake_case)]
#[derive(Debug, Deserialize)]
pub struct LiveQuote {
    pub symbol: String,
    pub bid: Option<f64>,
    pub ask: Option<f64>,
    /// Bid size in round lots (100 shares each).
    pub bidSize: Option<f64>,
    /// Ask size in round lots (100 shares each).
    pub askSize: Option<f64>,
    pub regularMarketPrice: Option<f64>,
    pub regularMarketVolume: Option<f64>,
    /// Last trade Unix seconds.
    pub regularMarketTime: Option<i64>,
}

// ── URL builders ──────────────────────────────────────────────────────────────

/// Build the chart URL for a historical bar range.
///
/// Yahoo Finance limits intraday intervals by range:
///   - `1m` → max 7 days
///   - `5m` → max 60 days
///   - `1h` → max 730 days
///
/// This function selects the finest interval that fits within `range`.
/// Pass `crumb` if obtained from the Yahoo session; empty string to omit it.
pub fn chart_url(symbol: &str, range: &str, crumb: &str) -> String {
    let crumb_part = if crumb.is_empty() { String::new() } else { format!("&crumb={crumb}") };
    // Parse range into days so we can pick the right interval.
    let days: u32 = if let Some(n) = range.strip_suffix('d') {
        n.parse().unwrap_or(7)
    } else if let Some(n) = range.strip_suffix('y') {
        n.parse::<u32>().unwrap_or(1) * 365
    } else {
        7 // default
    };
    let interval = if days <= 7 { "1m" } else if days <= 60 { "5m" } else { "1h" };
    format!(
        "https://query2.finance.yahoo.com/v8/finance/chart/{symbol}?interval={interval}&range={range}{crumb_part}"
    )
}

/// Build the batch live-quote URL for multiple symbols.
/// Pass `crumb` if obtained from the Yahoo session; empty string to omit it.
pub fn quote_url(symbols: &[&str], crumb: &str) -> String {
    let crumb_part = if crumb.is_empty() { String::new() } else { format!("&crumb={crumb}") };
    format!(
        "https://query2.finance.yahoo.com/v7/finance/quote?symbols={}&fields=bid,ask,bidSize,askSize,regularMarketPrice,regularMarketVolume,regularMarketTime&formatted=false{crumb_part}",
        symbols.join(",")
    )
}

// ── Parsers ───────────────────────────────────────────────────────────────────

/// Parse a Yahoo Finance chart JSON response into a stream of (Trade, BookSnapshot) event pairs.
///
/// Each valid bar produces:
///   1. `Event::Trade`         — close price + bar volume at bar timestamp
///   2. `Event::BookSnapshot`  — synthetic NBBO: bid = close − $0.01, ask = close + $0.01
///
/// The Trade event is emitted first so trade intensity is captured in the same tick's features.
pub fn parse_chart_response(json: &str) -> Vec<(String, Event, DataQualityFlags)> {
    let resp: ChartResponse = match serde_json::from_str(json) {
        Ok(r) => r,
        Err(_) => return vec![],
    };

    let results = match resp.chart.result {
        Some(r) if !r.is_empty() => r,
        _ => return vec![],
    };
    let result = &results[0];
    let symbol = result.meta.symbol.to_uppercase();

    let timestamps = match &result.timestamp {
        Some(t) => t,
        None => return vec![],
    };

    let quote = result
        .indicators
        .as_ref()
        .and_then(|i| i.quote.as_ref())
        .and_then(|q| q.first());

    let closes: &[Option<f64>] = quote
        .and_then(|q| q.close.as_deref())
        .unwrap_or(&[]);
    let volumes: &[Option<f64>] = quote
        .and_then(|q| q.volume.as_deref())
        .unwrap_or(&[]);

    let flags = DataQualityFlags {
        is_imputed: false,
        gap_flag: false,
        is_outlier: false,
        ts_source: TsSource::Exchange,
    };

    let mut out = Vec::with_capacity(timestamps.len() * 2);

    for (i, ts_opt) in timestamps.iter().enumerate() {
        let ts_s = match ts_opt {
            Some(t) => *t,
            None => continue,
        };
        let ts_us = ts_s * 1_000_000;

        let close = match closes.get(i).copied().flatten() {
            Some(c) if c > 0.0 => c,
            _ => continue,
        };
        let volume = volumes.get(i).copied().flatten().unwrap_or(0.0);

        // Synthetic NBBO: ±$0.01 around close, 1000-share size each side.
        let bid = close - 0.01;
        let ask = close + 0.01;
        let size = 1000.0_f64;

        // 1. Trade event first — so trade intensity is captured in the same snapshot tick.
        out.push((
            symbol.clone(),
            Event::Trade(TradeEvent {
                price: close,
                qty: volume,
                ts: ts_us,
                is_buy: false,
            }),
            flags.clone(),
        ));

        // 2. BookSnapshot — replaces the single-level synthetic book each bar.
        out.push((
            symbol.clone(),
            Event::BookSnapshot {
                bids: vec![(bid, size)],
                asks: vec![(ask, size)],
                ts: ts_us,
            },
            flags.clone(),
        ));
    }

    out
}

/// Parse a Yahoo Finance live-quote JSON response into event pairs per symbol.
///
/// Each symbol with a valid bid+ask+last produces:
///   1. `Event::Trade`        — regularMarketPrice + regularMarketVolume
///   2. `Event::BookSnapshot` — real bid/ask/size from the NBBO snapshot
pub fn parse_quote_response(json: &str) -> Vec<(String, Event, DataQualityFlags)> {
    let resp: QuoteResponse = match serde_json::from_str(json) {
        Ok(r) => r,
        Err(_) => return vec![],
    };

    let results = match resp.quote_response.result {
        Some(r) if !r.is_empty() => r,
        _ => return vec![],
    };

    let flags = DataQualityFlags {
        is_imputed: false,
        gap_flag: false,
        is_outlier: false,
        ts_source: TsSource::Exchange,
    };

    let mut out = Vec::with_capacity(results.len() * 2);

    for q in results {
        let bid = match q.bid.filter(|&b| b > 0.0) {
            Some(b) => b,
            None => continue,
        };
        let ask = match q.ask.filter(|&a| a > 0.0) {
            Some(a) => a,
            None => continue,
        };
        let last = match q.regularMarketPrice.filter(|&p| p > 0.0) {
            Some(p) => p,
            None => continue,
        };

        let bid_size = q.bidSize.unwrap_or(1.0) * 100.0;   // lots → shares
        let ask_size = q.askSize.unwrap_or(1.0) * 100.0;
        let volume   = q.regularMarketVolume.unwrap_or(0.0);
        let ts_us    = q.regularMarketTime.unwrap_or(0) * 1_000_000;
        let symbol   = q.symbol.to_uppercase();

        // 1. Trade event.
        out.push((
            symbol.clone(),
            Event::Trade(TradeEvent {
                price: last,
                qty: volume,
                ts: ts_us,
                is_buy: false,
            }),
            flags.clone(),
        ));

        // 2. BookSnapshot with real NBBO.
        out.push((
            symbol,
            Event::BookSnapshot {
                bids: vec![(bid, bid_size)],
                asks: vec![(ask, ask_size)],
                ts: ts_us,
            },
            flags.clone(),
        ));
    }

    out
}

// ── Live ingester ─────────────────────────────────────────────────────────────

/// Yahoo Finance ingester.
///
/// Phase 1 — historical backfill: fetches `range` of 1-minute bars per symbol
/// (sequential HTTP GETs).
///
/// Phase 2 — live polling: polls the batch NBBO quote endpoint every
/// `poll_interval_ms` ms, indefinitely.
#[cfg(feature = "ingest")]
pub struct YahooIngester {
    pub symbols: Vec<String>,
    /// Historical range string, e.g. `"60d"`.  Set to `""` to skip backfill.
    pub range: String,
    /// Live-poll interval in milliseconds.
    pub poll_interval_ms: u64,
}

#[cfg(feature = "ingest")]
impl YahooIngester {
    pub fn new(symbols: &[&str]) -> Self {
        Self {
            symbols: symbols.iter().map(|s| s.to_uppercase()).collect(),
            range: "60d".to_string(),
            poll_interval_ms: 2_000,
        }
    }

    pub fn with_range(mut self, range: &str) -> Self {
        self.range = range.to_string();
        self
    }

    pub fn with_interval_ms(mut self, ms: u64) -> Self {
        self.poll_interval_ms = ms;
        self
    }

    /// Run the two-phase ingester.
    ///
    /// `on_event` receives `(symbol, event, flags)` — same interface as every
    /// other ingester in this pipeline.
    pub async fn run<F>(&mut self, mut on_event: F) -> anyhow::Result<()>
    where
        F: FnMut(String, Event, DataQualityFlags),
    {
        let client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
            .cookie_store(true)
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        // ── Obtain Yahoo session cookie + crumb ───────────────────────────────
        // Yahoo Finance requires a crumb token (bound to a session cookie) in
        // all API requests.  The cookie is set by visiting the homepage; the
        // crumb is then fetched from a dedicated endpoint.
        let crumb = self.fetch_crumb(&client).await;
        if crumb.is_empty() {
            eprintln!("[yahoo] warning: could not obtain crumb — requests may fail");
        } else {
            eprintln!("[yahoo] crumb obtained (len={})", crumb.len());
        }

        // ── Phase 1: historical backfill ──────────────────────────────────────
        if !self.range.is_empty() {
            eprintln!("[yahoo] fetching {} of 1-min bars for {} symbols…",
                self.range, self.symbols.len());

            let mut total_bars = 0usize;
            for sym in &self.symbols {
                let url = chart_url(sym, &self.range, &crumb);
                match client.get(&url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        match resp.text().await {
                            Ok(body) => {
                                let events = parse_chart_response(&body);
                                let n = events.iter()
                                    .filter(|(_, e, _)| matches!(e, Event::BookSnapshot { .. }))
                                    .count();
                                total_bars += n;
                                for (s, event, flags) in events {
                                    on_event(s, event, flags);
                                }
                                eprintln!("[yahoo] {sym}: {n} bars replayed");
                            }
                            Err(e) => eprintln!("[yahoo] {sym}: body read error: {e}"),
                        }
                    }
                    Ok(resp) => {
                        eprintln!("[yahoo] {sym}: HTTP {} — skipping backfill", resp.status());
                    }
                    Err(e) => eprintln!("[yahoo] {sym}: request error: {e} — skipping backfill"),
                }
                // Brief pause between per-symbol GETs to stay under rate limits.
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            }
            eprintln!("[yahoo] historical backfill complete: {total_bars} total bars");
        }

        // ── Phase 2: live NBBO polling ────────────────────────────────────────
        let syms: Vec<&str> = self.symbols.iter().map(|s| s.as_str()).collect();
        eprintln!("[yahoo] starting live poll every {}ms for: {}",
            self.poll_interval_ms, self.symbols.join(", "));

        // Re-fetch crumb periodically (Yahoo crumbs expire after a few hours).
        let mut crumb = crumb;
        let mut polls_since_crumb_refresh = 0u32;
        const CRUMB_REFRESH_INTERVAL: u32 = 1_800; // ~1 h at 2 s/poll

        loop {
            if polls_since_crumb_refresh >= CRUMB_REFRESH_INTERVAL {
                let fresh = self.fetch_crumb(&client).await;
                if !fresh.is_empty() {
                    crumb = fresh;
                    eprintln!("[yahoo] crumb refreshed");
                }
                polls_since_crumb_refresh = 0;
            }

            let url = quote_url(&syms, &crumb);
            match client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    match resp.text().await {
                        Ok(body) => {
                            for (sym, event, flags) in parse_quote_response(&body) {
                                on_event(sym, event, flags);
                            }
                        }
                        Err(e) => eprintln!("[yahoo] live poll body error: {e}"),
                    }
                }
                Ok(resp) => {
                    eprintln!("[yahoo] live poll HTTP {} — will retry", resp.status());
                    // On auth failure, try refreshing the crumb immediately.
                    if resp.status() == 401 || resp.status() == 403 {
                        let fresh = self.fetch_crumb(&client).await;
                        if !fresh.is_empty() { crumb = fresh; }
                    }
                }
                Err(e) => eprintln!("[yahoo] live poll error: {e}"),
            }

            polls_since_crumb_refresh += 1;
            tokio::time::sleep(std::time::Duration::from_millis(self.poll_interval_ms)).await;
        }
    }

    /// Fetch a Yahoo Finance crumb token by:
    ///   1. Priming session cookies via the homepage.
    ///   2. Calling the getcrumb endpoint.
    ///
    /// Returns an empty string on failure so the caller can proceed without it.
    async fn fetch_crumb(&self, client: &reqwest::Client) -> String {
        // Prime cookies.
        let _ = client.get("https://finance.yahoo.com/")
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
            .send().await;

        // Small pause so cookies settle.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // Fetch crumb.
        match client.get("https://query2.finance.yahoo.com/v1/test/getcrumb")
            .header("Accept", "text/plain")
            .send().await
        {
            Ok(resp) if resp.status().is_success() => {
                resp.text().await.unwrap_or_default().trim().to_string()
            }
            Ok(resp) => {
                eprintln!("[yahoo] getcrumb HTTP {}", resp.status());
                String::new()
            }
            Err(e) => {
                eprintln!("[yahoo] getcrumb error: {e}");
                String::new()
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn chart_json(symbol: &str, timestamps: &[i64], closes: &[f64], volumes: &[f64]) -> String {
        let ts: Vec<serde_json::Value> = timestamps.iter().map(|&t| serde_json::json!(t)).collect();
        let cl: Vec<serde_json::Value> = closes.iter().map(|&c| serde_json::json!(c)).collect();
        let vo: Vec<serde_json::Value> = volumes.iter().map(|&v| serde_json::json!(v)).collect();
        serde_json::json!({
            "chart": {
                "result": [{
                    "meta": { "symbol": symbol },
                    "timestamp": ts,
                    "indicators": {
                        "quote": [{ "close": cl, "volume": vo }]
                    }
                }],
                "error": null
            }
        }).to_string()
    }

    fn quote_json(symbol: &str, bid: f64, ask: f64, last: f64, ts: i64) -> String {
        serde_json::json!({
            "quoteResponse": {
                "result": [{
                    "symbol": symbol,
                    "bid": bid,
                    "ask": ask,
                    "bidSize": 3.0,
                    "askSize": 2.0,
                    "regularMarketPrice": last,
                    "regularMarketVolume": 1_000_000.0,
                    "regularMarketTime": ts
                }],
                "error": null
            }
        }).to_string()
    }

    #[test]
    fn chart_emits_trade_then_snapshot_per_bar() {
        let json = chart_json("AAPL",
            &[1_700_000_000, 1_700_000_060],
            &[170.50, 170.75],
            &[100_000.0, 80_000.0],
        );
        let events = parse_chart_response(&json);
        // 2 bars × 2 events (Trade + BookSnapshot) = 4
        assert_eq!(events.len(), 4);

        // Bar 0 — Trade comes first
        assert!(matches!(events[0].1, Event::Trade(_)));
        if let Event::Trade(ref t) = events[0].1 {
            assert!((t.price - 170.50).abs() < 1e-9);
            assert!((t.qty - 100_000.0).abs() < 1e-9);
            assert_eq!(t.ts, 1_700_000_000 * 1_000_000);
        }

        // Bar 0 — BookSnapshot second
        assert!(matches!(events[1].1, Event::BookSnapshot { .. }));
        if let Event::BookSnapshot { ref bids, ref asks, ts } = events[1].1 {
            assert_eq!(bids.len(), 1);
            assert_eq!(asks.len(), 1);
            assert!((bids[0].0 - (170.50 - 0.01)).abs() < 1e-9, "bid = close - 0.01");
            assert!((asks[0].0 - (170.50 + 0.01)).abs() < 1e-9, "ask = close + 0.01");
            assert!((bids[0].1 - 1000.0).abs() < 1e-9);
            assert_eq!(ts, 1_700_000_000 * 1_000_000);
        }

        // Bar 1 events
        assert!(matches!(events[2].1, Event::Trade(_)));
        assert!(matches!(events[3].1, Event::BookSnapshot { .. }));
    }

    #[test]
    fn chart_symbol_uppercased() {
        let json = chart_json("aapl", &[1_700_000_000], &[170.0], &[100.0]);
        let events = parse_chart_response(&json);
        assert_eq!(events[0].0, "AAPL");
    }

    #[test]
    fn chart_null_close_skipped() {
        // Bar with null close should be omitted
        let json = serde_json::json!({
            "chart": {
                "result": [{
                    "meta": { "symbol": "SPY" },
                    "timestamp": [1_700_000_000_i64, 1_700_000_060_i64],
                    "indicators": {
                        "quote": [{ "close": [null, 460.0], "volume": [null, 50_000.0] }]
                    }
                }],
                "error": null
            }
        }).to_string();
        let events = parse_chart_response(&json);
        // Only bar 1 is valid → 2 events
        assert_eq!(events.len(), 2);
        if let Event::Trade(ref t) = events[0].1 {
            assert!((t.price - 460.0).abs() < 1e-9);
        }
    }

    #[test]
    fn quote_emits_trade_then_snapshot() {
        let json = quote_json("MSFT", 380.00, 380.05, 380.02, 1_712_000_000);
        let events = parse_quote_response(&json);
        assert_eq!(events.len(), 2);

        // Trade first
        assert_eq!(events[0].0, "MSFT");
        if let Event::Trade(ref t) = events[0].1 {
            assert!((t.price - 380.02).abs() < 1e-9);
            assert!((t.qty - 1_000_000.0).abs() < 1e-9);
            assert_eq!(t.ts, 1_712_000_000 * 1_000_000);
        } else {
            panic!("expected Trade");
        }

        // Snapshot second
        assert_eq!(events[1].0, "MSFT");
        if let Event::BookSnapshot { ref bids, ref asks, ts } = events[1].1 {
            assert!((bids[0].0 - 380.00).abs() < 1e-9);
            assert!((asks[0].0 - 380.05).abs() < 1e-9);
            // bidSize = 3.0 lots × 100 = 300 shares
            assert!((bids[0].1 - 300.0).abs() < 1e-9);
            assert!((asks[0].1 - 200.0).abs() < 1e-9);
            assert_eq!(ts, 1_712_000_000 * 1_000_000);
        } else {
            panic!("expected BookSnapshot");
        }
    }

    #[test]
    fn quote_zero_bid_skipped() {
        let json = serde_json::json!({
            "quoteResponse": {
                "result": [
                    {
                        "symbol": "AAPL",
                        "bid": 0.0,
                        "ask": 170.55,
                        "regularMarketPrice": 170.52,
                        "regularMarketTime": 1_712_000_000_i64
                    }
                ],
                "error": null
            }
        }).to_string();
        // bid = 0 → skip
        assert!(parse_quote_response(&json).is_empty());
    }

    #[test]
    fn quote_url_encodes_multiple_symbols() {
        let url = quote_url(&["AAPL", "SPY", "QQQ"], "");
        assert!(url.contains("AAPL,SPY,QQQ"));
        assert!(url.contains("v7/finance/quote"));
    }

    #[test]
    fn quote_url_includes_crumb_when_provided() {
        let url = quote_url(&["AAPL"], "testcrumb123");
        assert!(url.contains("crumb=testcrumb123"));
    }

    #[test]
    fn chart_url_uses_1m_for_short_range() {
        let url = chart_url("AAPL", "7d", "");
        assert!(url.contains("AAPL"));
        assert!(url.contains("interval=1m"));
        assert!(url.contains("range=7d"));
    }

    #[test]
    fn chart_url_uses_5m_for_medium_range() {
        let url = chart_url("AAPL", "60d", "");
        assert!(url.contains("interval=5m"), "60d should use 5m interval, got: {url}");
        assert!(url.contains("range=60d"));
    }

    #[test]
    fn chart_url_uses_1h_for_long_range() {
        let url = chart_url("AAPL", "365d", "");
        assert!(url.contains("interval=1h"), "365d should use 1h interval, got: {url}");
    }

    #[test]
    fn chart_url_includes_crumb_when_provided() {
        let url = chart_url("AAPL", "60d", "mycrumb");
        assert!(url.contains("crumb=mycrumb"));
    }
}
