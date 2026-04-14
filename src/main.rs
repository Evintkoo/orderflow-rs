//! orderflow-rs CLI
//!
//! Usage:
//!   orderflow demo
//!   orderflow ingest           <symbol> [output_dir]         — Binance.US (20 levels, live)
//!   orderflow ingest-coinbase  <pair>   [output_dir]         — Coinbase (full depth, live)
//!   orderflow ingest-kraken    <symbol> [depth] [output_dir] — Kraken (up to 1000 levels, live)
//!   orderflow ingest-gemini    <symbol> [output_dir]         — Gemini (full depth, live)
//!   orderflow ingest-alpaca    <symbol> [feed] [output_dir]  — Alpaca US stocks/crypto NBBO (needs API key)
//!   orderflow ingest-okx       <pair>   [output_dir]         — OKX (400 levels, live)
//!   orderflow ingest-bybit     <symbol> [depth] [output_dir] — Bybit (up to 200 levels, live)
//!   orderflow ingest-lobster   <messages.csv> [orderbook.csv] [output_dir]
//!   orderflow ingest-iex       <data.jsonl>   [symbol]        [output_dir]
//!   orderflow ingest-dukascopy <data.csv>     <symbol>        [output_dir]
//!
//! All ingest commands:
//!  - Reconstruct the LOB event-by-event
//!  - Extract all 10 features at each tick
//!  - Attach forward-return labels (5-min delay queue)
//!  - Write to <output_dir>/<source>_<symbol>_features.csv

use anyhow::Result;
use std::env;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("demo");

    match cmd {
        "ingest" => {
            // Comma-separated symbols: ingest btcusdt,ethusdt,solusdt [output_dir]
            let syms_arg = args.get(2).map(|s| s.as_str()).unwrap_or("btcusdt");
            let symbols: Vec<String> = syms_arg.split(',')
                .map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect();
            let out_dir = args.get(3).map(|s| s.as_str()).unwrap_or("data");
            run_binance_ingest_multi(&symbols, out_dir)?;
        }
        "ingest-lobster" => {
            let messages = args.get(2).ok_or_else(|| {
                anyhow::anyhow!("Usage: orderflow ingest-lobster <messages.csv> [orderbook.csv] [output_dir]")
            })?;
            let orderbook = args.get(3).filter(|s| s.ends_with(".csv")).map(|s| s.as_str());
            let out_dir = args.get(if orderbook.is_some() { 4 } else { 3 })
                .map(|s| s.as_str())
                .unwrap_or("data");
            run_lobster_ingest(messages, orderbook, out_dir)?;
        }
        "ingest-iex" => {
            let file = args.get(2).ok_or_else(|| {
                anyhow::anyhow!("Usage: orderflow ingest-iex <data.jsonl> [symbol] [output_dir]")
            })?;
            let symbol = args.get(3).map(|s| s.as_str()).unwrap_or("IEX");
            let out_dir = args.get(4).map(|s| s.as_str()).unwrap_or("data");
            run_iex_ingest(file, symbol, out_dir)?;
        }
        "ingest-dukascopy" => {
            let file = args.get(2).ok_or_else(|| {
                anyhow::anyhow!("Usage: orderflow ingest-dukascopy <data.csv> <symbol> [output_dir]")
            })?;
            let symbol = args.get(3).ok_or_else(|| {
                anyhow::anyhow!("Usage: orderflow ingest-dukascopy <data.csv> <symbol> [output_dir]")
            })?;
            let out_dir = args.get(4).map(|s| s.as_str()).unwrap_or("data");
            run_dukascopy_ingest(file, symbol, out_dir)?;
        }
        "ingest-coinbase" => {
            let syms_arg = args.get(2).map(|s| s.as_str())
                .unwrap_or("BTC-USD,ETH-USD,SOL-USD,XRP-USD,ADA-USD,AVAX-USD,DOGE-USD,LINK-USD,DOT-USD,LTC-USD");
            let symbols: Vec<String> = syms_arg.split(',')
                .map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            let out_dir = args.get(3).map(|s| s.as_str()).unwrap_or("data");
            run_coinbase_ingest_multi(&symbols, out_dir)?;
        }
        "ingest-kraken" => {
            let syms_arg = args.get(2).map(|s| s.as_str())
                .unwrap_or("BTC/USD,ETH/USD,SOL/USD,XRP/USD,ADA/USD,AVAX/USD,DOGE/USD,LINK/USD,DOT/USD,LTC/USD");
            let symbols: Vec<String> = syms_arg.split(',')
                .map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            let (depth, out_dir) = match args.get(3).map(|s| s.as_str()) {
                Some(s) if s.parse::<u32>().is_ok() => {
                    (s.parse::<u32>().unwrap(), args.get(4).map(|s| s.as_str()).unwrap_or("data"))
                }
                Some(s) => (500u32, s),
                None => (500u32, "data"),
            };
            run_kraken_ingest_multi(&symbols, depth, out_dir)?;
        }
        "ingest-gemini" => {
            let syms_arg = args.get(2).map(|s| s.as_str())
                .unwrap_or("BTCUSD,ETHUSD,SOLUSD,DOGEUSD,LINKUSD,LTCUSD,AAVEUSD,MATICUSD,UNIUSD,XRPUSD");
            let symbols: Vec<String> = syms_arg.split(',')
                .map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            let out_dir = args.get(3).map(|s| s.as_str()).unwrap_or("data");
            run_gemini_ingest_multi(&symbols, out_dir)?;
        }
        "ingest-alpaca" => {
            // Usage: ingest-alpaca AAPL,SPY,QQQ,MSFT [iex|sip|crypto] [output_dir]
            let syms_arg = args.get(2).ok_or_else(||
                anyhow::anyhow!("Usage: orderflow ingest-alpaca AAPL,SPY,QQQ [iex|sip|crypto] [output_dir]")
            )?;
            let symbols: Vec<String> = syms_arg.split(',')
                .map(|s| s.trim().to_uppercase())
                .filter(|s| !s.is_empty())
                .collect();
            let feed_str = args.get(3).map(|s| s.as_str()).unwrap_or("iex");
            let out_dir  = args.get(4).map(|s| s.as_str()).unwrap_or("data");
            run_alpaca_ingest_multi(&symbols, feed_str, out_dir)?;
        }
        "ingest-okx" => {
            let syms_arg = args.get(2).map(|s| s.as_str())
                .unwrap_or("BTC-USDT,ETH-USDT,SOL-USDT,XRP-USDT,ADA-USDT,AVAX-USDT,DOGE-USDT,LINK-USDT,DOT-USDT,LTC-USDT");
            let symbols: Vec<String> = syms_arg.split(',')
                .map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            let out_dir = args.get(3).map(|s| s.as_str()).unwrap_or("data");
            run_okx_ingest_multi(&symbols, out_dir)?;
        }
        "ingest-bybit" => {
            let syms_arg = args.get(2).map(|s| s.as_str())
                .unwrap_or("BTCUSDT,ETHUSDT,SOLUSDT,XRPUSDT,ADAUSDT,AVAXUSDT,DOGEUSDT,LINKUSDT,DOTUSDT,LTCUSDT");
            let symbols: Vec<String> = syms_arg.split(',')
                .map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            let (depth, out_dir) = match args.get(3).map(|s| s.as_str()) {
                Some(s) if s.parse::<u32>().is_ok() => {
                    (s.parse::<u32>().unwrap(), args.get(4).map(|s| s.as_str()).unwrap_or("data"))
                }
                Some(s) => (200u32, s),
                None => (200u32, "data"),
            };
            run_bybit_ingest_multi(&symbols, depth, out_dir)?;
        }
        "ingest-bitstamp" => {
            let syms_arg = args.get(2).map(|s| s.as_str())
                .unwrap_or("BTCUSD,ETHUSD,SOLUSD,XRPUSD,ADAUSD,DOGEUSD,LINKUSD,LTCUSD,UNIUSD,AAVEUSD");
            let symbols: Vec<String> = syms_arg.split(',')
                .map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            let out_dir = args.get(3).map(|s| s.as_str()).unwrap_or("data");
            run_bitstamp_ingest_multi(&symbols, out_dir)?;
        }
        "ingest-polygon" => {
            let syms_arg = args.get(2).ok_or_else(||
                anyhow::anyhow!("Usage: orderflow ingest-polygon AAPL,SPY,QQQ,MSFT [output_dir]\n  Set env: POLYGON_KEY=<key>")
            )?;
            let symbols: Vec<String> = syms_arg.split(',')
                .map(|s| s.trim().to_uppercase()).filter(|s| !s.is_empty()).collect();
            let out_dir = args.get(3).map(|s| s.as_str()).unwrap_or("data");
            run_polygon_ingest_multi(&symbols, out_dir)?;
        }
        "ingest-tiingo" => {
            let syms_arg = args.get(2).ok_or_else(||
                anyhow::anyhow!("Usage: orderflow ingest-tiingo AAPL,SPY,QQQ [output_dir]\n  Set env: TIINGO_KEY=<key>")
            )?;
            let symbols: Vec<String> = syms_arg.split(',')
                .map(|s| s.trim().to_uppercase()).filter(|s| !s.is_empty()).collect();
            let out_dir = args.get(3).map(|s| s.as_str()).unwrap_or("data");
            run_tiingo_ingest_multi(&symbols, out_dir)?;
        }
        "ingest-finnhub" => {
            let syms_arg = args.get(2).ok_or_else(||
                anyhow::anyhow!("Usage: orderflow ingest-finnhub AAPL,SPY,QQQ [output_dir]\n  Set env: FINNHUB_KEY=<key>")
            )?;
            let symbols: Vec<String> = syms_arg.split(',')
                .map(|s| s.trim().to_uppercase()).filter(|s| !s.is_empty()).collect();
            let out_dir = args.get(3).map(|s| s.as_str()).unwrap_or("data");
            run_finnhub_ingest_multi(&symbols, out_dir)?;
        }
        "ingest-yahoo" => {
            // Usage: ingest-yahoo [AAPL,SPY,...] [range] [poll_ms] [output_dir]
            // range:   historical bar range (default "60d"); pass "" to skip backfill
            // poll_ms: live-poll interval in milliseconds (default 2000)
            let syms_arg = args.get(2).map(|s| s.as_str())
                .unwrap_or("AAPL,SPY,QQQ,MSFT,NVDA,TSLA,AMZN,GOOGL,META,JPM");
            let symbols: Vec<String> = syms_arg.split(',')
                .map(|s| s.trim().to_uppercase()).filter(|s| !s.is_empty()).collect();
            let range   = args.get(3).map(|s| s.as_str()).unwrap_or("60d");
            let poll_ms = args.get(4).and_then(|s| s.parse::<u64>().ok()).unwrap_or(2_000);
            let out_dir = args.get(5).map(|s| s.as_str()).unwrap_or("data/yahoo");
            run_yahoo_ingest_multi(&symbols, range, poll_ms, out_dir)?;
        }
        "ingest-binance-rest" => {
            let syms_arg = args.get(2).map(|s| s.as_str())
                .unwrap_or("BTCUSDT,ETHUSDT,SOLUSDT,XRPUSDT,ADAUSDT,AVAXUSDT,DOGEUSDT,LINKUSDT,DOTUSDT,LTCUSDT");
            let symbols: Vec<String> = syms_arg.split(',')
                .map(|s| s.trim().to_uppercase()).filter(|s| !s.is_empty()).collect();
            let days    = args.get(3).and_then(|s| s.parse::<u32>().ok()).unwrap_or(30);
            let poll_ms = args.get(4).and_then(|s| s.parse::<u64>().ok()).unwrap_or(2_000);
            let out_dir = args.get(5).map(|s| s.as_str()).unwrap_or("data/binance-rest");
            run_binance_rest_multi(&symbols, days, poll_ms, out_dir)?;
        }
        "fetch-dukascopy" => {
            let pairs_arg = args.get(2).map(|s| s.as_str())
                .unwrap_or("EURUSD,GBPUSD,USDJPY,USDCHF,AUDUSD,USDCAD");
            let pairs: Vec<String> = pairs_arg.split(',')
                .map(|s| s.trim().to_uppercase()).filter(|s| !s.is_empty()).collect();
            let days    = args.get(3).and_then(|s| s.parse::<u32>().ok()).unwrap_or(30);
            let out_dir = args.get(4).map(|s| s.as_str()).unwrap_or("data/dukascopy");
            run_dukascopy_fetch_multi(&pairs, days, out_dir)?;
        }
        "analyze" => {
            // Usage: analyze [data_dir] [output_dir]
            let data_dir   = args.get(2).map(|s| s.as_str()).unwrap_or("data");
            let output_dir = args.get(3).map(|s| s.as_str()).unwrap_or("reports");
            run_analyze(data_dir, output_dir)?;
        }
        "simulate" => {
            // Usage: simulate [n_paths] [output_dir]
            let n_paths = args.get(2).and_then(|s| s.parse::<usize>().ok()).unwrap_or(10);
            let out_dir = args.get(3).map(|s| s.as_str()).unwrap_or("data/sim");
            run_simulate(n_paths, out_dir)?;
        }
        "backtest" => {
            // Usage: backtest [data_dir] [output_dir] [train_days] [test_days]
            let data_dir   = args.get(2).map(|s| s.as_str()).unwrap_or("data/dukascopy");
            let output_dir = args.get(3).map(|s| s.as_str()).unwrap_or("reports/backtest");
            let train_days = args.get(4).and_then(|s| s.parse::<f64>().ok()).unwrap_or(14.0);
            let test_days  = args.get(5).and_then(|s| s.parse::<f64>().ok()).unwrap_or(7.0);
            run_backtest_cmd(data_dir, output_dir, train_days, test_days)?;
        }
        "techanalysis" => {
            // Usage: techanalysis [data_dir] [reports_dir]
            let data_dir    = args.get(2).map(|s| s.as_str()).unwrap_or("data");
            let reports_dir = args.get(3).map(|s| s.as_str()).unwrap_or("reports/techanalysis");
            orderflow_rs::commands::techanalysis::run(data_dir, reports_dir)?;
        }
        "demo" => demo_orderbook()?,
        _ => print_help(),
    }
    Ok(())
}

fn print_help() {
    println!("orderflow-rs — LOB microstructure research pipeline");
    println!();
    println!("LIVE WEBSOCKET SOURCES (no API key required, US-accessible):");
    println!();
    println!("  ingest <symbol> [output_dir]");
    println!("      Binance.US — 20 levels + REST snapshot (1000 levels).");
    println!("      Example: orderflow ingest btcusdt data/");
    println!();
    println!("  ingest-coinbase <pair> [output_dir]");
    println!("      Coinbase Advanced Trade — full depth order book (all resting levels).");
    println!("      Pair format: BTC-USD, ETH-USD, SOL-USD");
    println!("      Example: orderflow ingest-coinbase BTC-USD data/");
    println!();
    println!("  ingest-kraken <symbol> [depth] [output_dir]");
    println!("      Kraken — up to 1000 levels, US-accessible, no auth required.");
    println!("      depth: 10 | 25 | 100 | 500 | 1000 (default 500)");
    println!("      Symbol format: BTC/USD, ETH/USD, XBT/USD");
    println!("      Example: orderflow ingest-kraken BTC/USD 500 data/kraken/");
    println!();
    println!("  ingest-gemini <symbol> [output_dir]");
    println!("      Gemini — full depth, US-based exchange, no auth required.");
    println!("      Symbol format: BTCUSD, ETHUSD, SOLUSD");
    println!("      Example: orderflow ingest-gemini BTCUSD data/gemini/");
    println!();
    println!("  ingest-bitstamp [symbols] [output_dir]");
    println!("      Bitstamp — full L2 snapshots, US-accessible, no auth required.");
    println!("      Default 10 symbols: BTCUSD,ETHUSD,SOLUSD,XRPUSD,ADAUSD,...");
    println!("      Example: orderflow ingest-bitstamp BTCUSD,ETHUSD data/bitstamp/");
    println!();
    println!("LIVE WEBSOCKET SOURCES (free API key required):");
    println!();
    println!("  ingest-alpaca <SYMBOL> [feed] [output_dir]");
    println!("      Alpaca Markets — US stocks + crypto NBBO quotes + trades.");
    println!("      Set env vars: ALPACA_KEY=<key> ALPACA_SECRET=<secret>");
    println!("      Free account at: https://alpaca.markets (paper trading)");
    println!("      feed: iex (default, free) | sip (paid) | crypto");
    println!("      Example: orderflow ingest-alpaca AAPL,SPY,QQQ,MSFT iex data/alpaca/");
    println!();
    println!("  ingest-polygon <symbols> [output_dir]");
    println!("      Polygon.io — US stocks NBBO + trades (15-min delayed on free tier).");
    println!("      Set env: POLYGON_KEY=<key>  (free account at polygon.io)");
    println!("      Example: orderflow ingest-polygon AAPL,SPY,QQQ,MSFT data/polygon/");
    println!();
    println!("LIVE WEBSOCKET SOURCES (geo-restricted — need VPN outside US):");
    println!();
    println!("  ingest-okx [symbols] [output_dir]");
    println!("      OKX — 400 levels at 100ms. Spot and perpetual.");
    println!("      Default 10 symbols: BTC-USDT,ETH-USDT,SOL-USDT,...");
    println!("      Example: orderflow ingest-okx BTC-USDT,ETH-USDT data/okx/");
    println!();
    println!("  ingest-bybit [symbols] [depth] [output_dir]");
    println!("      Bybit — up to 200 levels (spot), 1000 levels (linear perp).");
    println!("      depth: 1 | 50 | 200 (default 200)");
    println!("      Example: orderflow ingest-bybit BTCUSDT 200 data/");
    println!();
    println!("FREE US EQUITY SOURCES (no API key required):");
    println!();
    println!("  ingest-yahoo [symbols] [range] [poll_ms] [output_dir]");
    println!("      Yahoo Finance — 60-day 1-min historical backfill + live NBBO polling.");
    println!("      No API key required.  Data is free-tier delayed (~15 min for live).");
    println!("      symbols: comma-separated (default: AAPL,SPY,QQQ,MSFT,NVDA,TSLA,AMZN,GOOGL,META,JPM)");
    println!("      range:   historical range (default 60d; pass \"\" to skip backfill)");
    println!("      poll_ms: live poll interval in ms (default 2000)");
    println!("      Example: orderflow ingest-yahoo AAPL,SPY,QQQ 60d 2000 data/yahoo/");
    println!();
    println!("HISTORICAL FILE REPLAY:");
    println!();
    println!("  ingest-lobster <messages.csv> [orderbook.csv] [output_dir]");
    println!("      Replay LOBSTER message file (NASDAQ ITCH reconstruction).");
    println!("      Free sample data at: https://lobsterdata.com");
    println!("      Example:");
    println!("        orderflow ingest-lobster AAPL_message_10.csv AAPL_orderbook_10.csv data/lobster/");
    println!();
    println!("  ingest-iex <data.jsonl> [symbol] [output_dir]");
    println!("      Replay IEX TOPS/DEEP historical JSON file.");
    println!("      Free token at: https://iexcloud.io");
    println!("      Example: orderflow ingest-iex iex_tops_20230103.jsonl AAPL data/iex/");
    println!();
    println!("  ingest-dukascopy <data.csv> <symbol> [output_dir]");
    println!("      Replay Dukascopy FX tick data (free, back to 2003).");
    println!("      Download: npx dukascopy-node -i eurusd -from 2023-01-01 -to 2023-01-31 -t tick -f csv");
    println!("      Example: orderflow ingest-dukascopy EURUSD_2023-01-02.csv EURUSD data/dukascopy/");
}

// ── Shared ingestion logic ────────────────────────────────────────────────────

/// Runs the LOB reconstruction + feature extraction + label queue pipeline
/// over a stream of events.  Writes results to CSV.
fn run_pipeline_from_events<I>(
    events: I,
    symbol: &str,
    exchange: &str,
    data_source: &str,
    out_path: &std::path::Path,
) -> Result<()>
where
    I: Iterator<Item = (orderflow_rs::pipeline::Event, orderflow_rs::pipeline::DataQualityFlags)>,
{
    use orderflow_rs::features::FeatureExtractor;
    use orderflow_rs::orderbook::Orderbook;
    use orderflow_rs::pipeline::labels::LabelQueue;
    use orderflow_rs::pipeline::Event;
    use std::io::{BufWriter, Write};

    let mut book = Orderbook::new();
    let mut fx = FeatureExtractor::new(exchange, symbol, data_source);
    let mut label_q = LabelQueue::new();

    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(out_path)?;
    let mut w = BufWriter::new(file);

    writeln!(w, "ts,ofi_1,ofi_5,ofi_10,depth_imb,microprice_dev,queue_imb,spread,\
                  trade_intensity,price_impact,level_drain,weighted_mid_slope,\
                  r_1s,r_5s,r_30s,r_300s,sign_1s,sign_5s,\
                  exchange,symbol,is_imputed,gap_flag")?;

    let mut total_events = 0u64;
    let mut total_rows = 0u64;

    for (event, flags) in events {
        match event {
            Event::BookDelta(delta) => {
                if let Err(e) = book.apply_delta(&delta) {
                    eprintln!("[WARN] delta rejected: {e}");
                    continue;
                }
                let snap = book.export_snapshot(10);
                let fv = fx.extract(&snap, flags.is_imputed, flags.gap_flag);

                if let (Some(mid), Some(spread)) = (snap.mid(), snap.spread()) {
                    label_q.push(fv, mid, spread);
                    for lfv in label_q.drain_ready() {
                        write_labeled_row(&mut w, &lfv)?;
                        total_rows += 1;
                    }
                }
                total_events += 1;
            }
            Event::Trade(trade) => {
                fx.push_trade(trade);
            }
            Event::BookSnapshot { bids, asks, ts } => {
                book.apply_snapshot_reset(bids.into_iter(), asks.into_iter(), ts);
                eprintln!("[INFO] snapshot applied at ts={ts}");
            }
            Event::Gap { start_ts, end_ts } => {
                eprintln!("[GAP] {start_ts}..{end_ts}");
                fx.reset_on_gap();
            }
            Event::Disconnected => {
                fx.reset_on_gap();
            }
        }

        if total_events % 100_000 == 0 && total_events > 0 {
            eprintln!("  ... {total_events} events processed, {total_rows} labeled rows written");
        }
    }

    // Flush remaining pending labels (end of file)
    label_q.flush();
    for lfv in label_q.drain_ready() {
        write_labeled_row(&mut w, &lfv)?;
        total_rows += 1;
    }

    w.flush()?;
    println!("Done. {total_events} events → {total_rows} labeled rows → {}", out_path.display());
    Ok(())
}

fn write_labeled_row<W: std::io::Write + ?Sized>(
    w: &mut W,
    lfv: &orderflow_rs::pipeline::LabeledFeatureVector,
) -> Result<()> {
    let fv = &lfv.fv;
    writeln!(
        w,
        "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
        fv.ts,
        opt(fv.ofi_1), opt(fv.ofi_5), opt(fv.ofi_10),
        opt(fv.depth_imb), opt(fv.microprice_dev),
        opt(fv.queue_imb), opt(fv.spread),
        opt(fv.trade_intensity), opt(fv.price_impact),
        opt(fv.level_drain), opt(fv.weighted_mid_slope),
        opt(lfv.r_1s), opt(lfv.r_5s), opt(lfv.r_30s), opt(lfv.r_300s),
        opt_i8(lfv.sign_1s), opt_i8(lfv.sign_5s),
        fv.exchange, fv.symbol,
        fv.is_imputed as u8, fv.gap_flag as u8,
    )?;
    Ok(())
}

fn opt(v: Option<f64>) -> String {
    v.map(|x| format!("{x:.8}")).unwrap_or_default()
}

fn opt_i8(v: Option<i8>) -> String {
    v.map(|x| x.to_string()).unwrap_or_default()
}

// ── Source-specific runners ───────────────────────────────────────────────────

#[cfg(feature = "ingest")]
fn run_binance_ingest_multi(symbols: &[String], out_dir: &str) -> Result<()> {
    use orderflow_rs::pipeline::realdata::BinanceIngester;
    let syms: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
    run_crypto_multi("binance", symbols, out_dir, move |on_event| async move {
        BinanceIngester::new_multi(&syms).run(on_event).await
    })
}

#[cfg(not(feature = "ingest"))]
fn run_binance_ingest_multi(_s: &[String], _d: &str) -> Result<()> { anyhow::bail!("Rebuild with --features ingest"); }

#[cfg(feature = "ingest")]
fn run_coinbase_ingest_multi(symbols: &[String], out_dir: &str) -> Result<()> {
    use orderflow_rs::pipeline::coinbase::CoinbaseIngester;
    let syms: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
    run_crypto_multi("coinbase", symbols, out_dir, move |on_event| async move {
        CoinbaseIngester::new_multi(&syms).run(on_event).await
    })
}

#[cfg(not(feature = "ingest"))]
fn run_coinbase_ingest_multi(_s: &[String], _d: &str) -> Result<()> { anyhow::bail!("Rebuild with --features ingest"); }

#[cfg(feature = "ingest")]
fn run_kraken_ingest_multi(symbols: &[String], depth: u32, out_dir: &str) -> Result<()> {
    use orderflow_rs::pipeline::kraken::KrakenIngester;
    let syms: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
    run_crypto_multi("kraken", symbols, out_dir, move |on_event| async move {
        KrakenIngester::new_multi(&syms).with_depth(depth).run(on_event).await
    })
}

#[cfg(not(feature = "ingest"))]
fn run_kraken_ingest_multi(_s: &[String], _depth: u32, _d: &str) -> Result<()> { anyhow::bail!("Rebuild with --features ingest"); }

#[cfg(feature = "ingest")]
fn run_gemini_ingest_multi(symbols: &[String], out_dir: &str) -> Result<()> {
    use orderflow_rs::pipeline::gemini::GeminiIngester;
    let syms: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
    run_crypto_multi("gemini", symbols, out_dir, move |on_event| async move {
        GeminiIngester::new_multi(&syms).run(on_event).await
    })
}

#[cfg(not(feature = "ingest"))]
fn run_gemini_ingest_multi(_s: &[String], _d: &str) -> Result<()> { anyhow::bail!("Rebuild with --features ingest"); }

#[cfg(feature = "ingest")]
fn run_okx_ingest_multi(symbols: &[String], out_dir: &str) -> Result<()> {
    use orderflow_rs::pipeline::okx::OkxIngester;
    let syms: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
    run_crypto_multi("okx", symbols, out_dir, move |on_event| async move {
        OkxIngester::new_multi(&syms).run(on_event).await
    })
}

#[cfg(not(feature = "ingest"))]
fn run_okx_ingest_multi(_s: &[String], _d: &str) -> Result<()> { anyhow::bail!("Rebuild with --features ingest"); }

#[cfg(feature = "ingest")]
fn run_bybit_ingest_multi(symbols: &[String], depth: u32, out_dir: &str) -> Result<()> {
    use orderflow_rs::pipeline::bybit::BybitIngester;
    let syms: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
    run_crypto_multi("bybit", symbols, out_dir, move |on_event| async move {
        BybitIngester::new_multi(&syms).with_depth(depth).run(on_event).await
    })
}

#[cfg(not(feature = "ingest"))]
fn run_bybit_ingest_multi(_s: &[String], _depth: u32, _d: &str) -> Result<()> { anyhow::bail!("Rebuild with --features ingest"); }

#[cfg(feature = "ingest")]
fn run_bitstamp_ingest_multi(symbols: &[String], out_dir: &str) -> Result<()> {
    use orderflow_rs::pipeline::bitstamp::BitstampIngester;
    let syms: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
    run_crypto_multi("bitstamp", symbols, out_dir, move |on_event| async move {
        BitstampIngester::new_multi(&syms).run(on_event).await
    })
}

#[cfg(not(feature = "ingest"))]
fn run_bitstamp_ingest_multi(_s: &[String], _d: &str) -> Result<()> { anyhow::bail!("Rebuild with --features ingest"); }

#[cfg(feature = "ingest")]
fn run_polygon_ingest_multi(symbols: &[String], out_dir: &str) -> Result<()> {
    use orderflow_rs::pipeline::polygon::PolygonRestIngester;
    let key = std::env::var("POLYGON_KEY")
        .map_err(|_| anyhow::anyhow!("Set POLYGON_KEY env var (free account at polygon.io)"))?;
    let syms: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
    run_crypto_multi("polygon", symbols, out_dir, move |on_event| async move {
        PolygonRestIngester::new(&syms, &key).run(on_event).await
    })
}

#[cfg(not(feature = "ingest"))]
fn run_polygon_ingest_multi(_s: &[String], _d: &str) -> Result<()> { anyhow::bail!("Rebuild with --features ingest"); }

#[cfg(feature = "ingest")]
fn run_tiingo_ingest_multi(symbols: &[String], out_dir: &str) -> Result<()> {
    use orderflow_rs::pipeline::tiingo::TiingoIngester;
    let key = std::env::var("TIINGO_KEY")
        .map_err(|_| anyhow::anyhow!("Set TIINGO_KEY env var (free at tiingo.com)"))?;
    let syms: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
    run_crypto_multi("tiingo", symbols, out_dir, move |on_event| async move {
        TiingoIngester::new(&syms, &key).run(on_event).await
    })
}

#[cfg(not(feature = "ingest"))]
fn run_tiingo_ingest_multi(_s: &[String], _d: &str) -> Result<()> { anyhow::bail!("Rebuild with --features ingest"); }

#[cfg(feature = "ingest")]
fn run_finnhub_ingest_multi(symbols: &[String], out_dir: &str) -> Result<()> {
    use orderflow_rs::pipeline::finnhub::FinnhubIngester;
    let key = std::env::var("FINNHUB_KEY")
        .map_err(|_| anyhow::anyhow!("Set FINNHUB_KEY env var (free at finnhub.io)"))?;
    let syms: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
    run_crypto_multi("finnhub", symbols, out_dir, move |on_event| async move {
        FinnhubIngester::new(&syms, &key).run(on_event).await
    })
}

#[cfg(not(feature = "ingest"))]
fn run_finnhub_ingest_multi(_s: &[String], _d: &str) -> Result<()> { anyhow::bail!("Rebuild with --features ingest"); }

#[cfg(feature = "ingest")]
fn run_yahoo_ingest_multi(symbols: &[String], range: &str, poll_ms: u64, out_dir: &str) -> Result<()> {
    use orderflow_rs::pipeline::yahoo::YahooIngester;
    let syms: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
    let range = range.to_string();
    run_crypto_multi("yahoo", symbols, out_dir, move |on_event| async move {
        YahooIngester::new(&syms)
            .with_range(&range)
            .with_interval_ms(poll_ms)
            .run(on_event)
            .await
    })
}

#[cfg(not(feature = "ingest"))]
fn run_yahoo_ingest_multi(_s: &[String], _range: &str, _poll_ms: u64, _d: &str) -> Result<()> { anyhow::bail!("Rebuild with --features ingest"); }

#[cfg(feature = "ingest")]
fn run_binance_rest_multi(symbols: &[String], days: u32, poll_ms: u64, out_dir: &str) -> Result<()> {
    use orderflow_rs::pipeline::binance_rest::BinanceRestIngester;
    let syms: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
    run_crypto_multi("binance-rest", symbols, out_dir, move |on_event| async move {
        BinanceRestIngester::new(&syms)
            .with_backfill_days(days)
            .with_poll_interval_ms(poll_ms)
            .run(on_event)
            .await
    })
}

#[cfg(not(feature = "ingest"))]
fn run_binance_rest_multi(_s: &[String], _days: u32, _poll_ms: u64, _d: &str) -> Result<()> { anyhow::bail!("Rebuild with --features ingest"); }

#[cfg(feature = "ingest")]
fn run_dukascopy_fetch_multi(pairs: &[String], days: u32, out_dir: &str) -> Result<()> {
    use orderflow_rs::pipeline::dukascopy_fetch::DukascopyFetcher;
    let ps: Vec<&str> = pairs.iter().map(|s| s.as_str()).collect();
    run_crypto_multi("dukascopy", pairs, out_dir, move |on_event| async move {
        DukascopyFetcher::new(&ps)
            .with_backfill_days(days)
            .run(on_event)
            .await
    })
}

#[cfg(not(feature = "ingest"))]
fn run_dukascopy_fetch_multi(_p: &[String], _days: u32, _d: &str) -> Result<()> { anyhow::bail!("Rebuild with --features ingest"); }

/// Shared multi-symbol crypto ingester: one WS connection, per-symbol CSV files.
/// `make_ingester` receives a `FnMut(String, Event, DataQualityFlags)` callback.
#[cfg(feature = "ingest")]
fn run_crypto_multi<F, Fut>(
    exchange: &str,
    symbols: &[String],
    out_dir: &str,
    make_ingester: F,
) -> Result<()>
where
    F: FnOnce(Box<dyn FnMut(String, orderflow_rs::pipeline::Event, orderflow_rs::pipeline::DataQualityFlags) + Send>) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    use orderflow_rs::features::FeatureExtractor;
    use orderflow_rs::orderbook::Orderbook;
    use orderflow_rs::pipeline::labels::LabelQueue;
    use orderflow_rs::pipeline::Event;
    use std::collections::HashMap;
    use std::io::{BufWriter, Write};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    std::fs::create_dir_all(out_dir)?;
    println!("[{exchange}] connecting, {} symbols: {}", symbols.len(), symbols.join(", "));
    println!("Output dir: {out_dir}  (CSV per symbol, rows flush after 300s label delay)");
    println!("Press Ctrl-C to stop.\n");

    type PerSymState = (Orderbook, FeatureExtractor, LabelQueue, BufWriter<std::fs::File>);
    let states: Arc<Mutex<HashMap<String, PerSymState>>> = Arc::new(Mutex::new(HashMap::new()));

    for sym in symbols {
        let sym_file = sym.to_lowercase().replace(['/', '-', '_', '^', '='], "");
        let path = PathBuf::from(out_dir)
            .join(format!("{exchange}_{sym_file}_features.csv"));
        let f = std::fs::OpenOptions::new().create(true).truncate(true).write(true).open(&path)?;
        let mut w = BufWriter::new(f);
        writeln!(w, "ts,ofi_1,ofi_5,ofi_10,depth_imb,microprice_dev,queue_imb,spread,\
                      trade_intensity,price_impact,level_drain,weighted_mid_slope,\
                      r_1s,r_5s,r_30s,r_300s,sign_1s,sign_5s,\
                      exchange,symbol,is_imputed,gap_flag")?;
        states.lock().unwrap().insert(
            sym.to_uppercase(),
            (Orderbook::new(), FeatureExtractor::new(exchange, sym, "live"), LabelQueue::new(), w),
        );
    }

    let last_quote: Arc<Mutex<HashMap<String, (f64, f64)>>> = Arc::new(Mutex::new(HashMap::new()));
    let states2 = Arc::clone(&states);
    let last_quote2 = Arc::clone(&last_quote);
    let exch = exchange.to_string();

    let on_event = Box::new(move |sym: String, event: Event, flags: orderflow_rs::pipeline::DataQualityFlags| {
        let now_us = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_micros() as i64;

        match event {
            Event::BookSnapshot { bids, asks, ts } => {
                let mut map = states2.lock().unwrap();
                if let Some((book, fx, lq, w)) = map.get_mut(&sym) {
                    let is_first = !book.has_data();
                    book.apply_snapshot_reset(bids.into_iter(), asks.into_iter(), ts);
                    if is_first {
                        // Initial/resync snapshot: reset feature state
                        fx.reset_on_gap();
                        eprintln!("\n[{exch}/{sym}] snapshot applied");
                    } else {
                        // Recurring full-snapshot source (e.g. Bitstamp, Yahoo): extract features
                        let snap = book.export_snapshot(20);
                        let fv = fx.extract(&snap, false, false);
                        if let (Some(mid), Some(spread)) = (snap.mid(), snap.spread()) {
                            lq.push(fv, mid, spread);
                            let rows = lq.drain_ready();
                            let wrote = !rows.is_empty();
                            for lfv in rows { let _ = write_labeled_row(w, &lfv); }
                            if wrote { let _ = w.flush(); }
                            if let (Some(bid), Some(ask)) = (snap.best_bid(), snap.best_ask()) {
                                last_quote2.lock().unwrap().insert(sym.clone(), (bid, ask));
                            }
                        }
                    }
                }
            }
            Event::BookDelta(delta) => {
                let mut map = states2.lock().unwrap();
                if let Some((book, fx, lq, w)) = map.get_mut(&sym) {
                    if book.apply_delta(&delta).is_ok() {
                        let snap = book.export_snapshot(20);
                        let fv = fx.extract(&snap, flags.is_imputed, flags.gap_flag);
                        if let (Some(mid), Some(spread)) = (snap.mid(), snap.spread()) {
                            lq.push(fv, mid, spread);
                            let rows = lq.drain_ready();
                            let wrote = !rows.is_empty();
                            for lfv in rows { let _ = write_labeled_row(w, &lfv); }
                            if wrote { let _ = w.flush(); }
                            if let (Some(bid), Some(ask)) = (snap.best_bid(), snap.best_ask()) {
                                last_quote2.lock().unwrap().insert(sym.clone(), (bid, ask));
                            }
                        }
                    }
                }
            }
            Event::Trade(trade) => {
                if let Some((_, fx, _, _)) = states2.lock().unwrap().get_mut(&sym) {
                    fx.push_trade(trade);
                }
            }
            Event::Gap { .. } => {
                if let Some((_, fx, _, _)) = states2.lock().unwrap().get_mut(&sym) { fx.reset_on_gap(); }
            }
            Event::Disconnected => {
                let mut map = states2.lock().unwrap();
                for (_, (_, fx, _, _)) in map.iter_mut() { fx.reset_on_gap(); }
            }
        }

        if now_us % 1_000_000 < 10_000 {
            use std::io::Write as _;
            // Flush all writers so data survives an abrupt Ctrl-C.
            {
                let mut map = states2.lock().unwrap();
                for (_, (_, _, _, w)) in map.iter_mut() { let _ = w.flush(); }
            }
            let q = last_quote.lock().unwrap();
            let mut parts: Vec<String> = q.iter()
                .map(|(s, (b, a))| format!("{s}={b:.2}/{a:.2}")).collect();
            parts.sort();
            print!("\r  [{exch}] {:<160}", parts.join("  "));
            let _ = std::io::stdout().flush();
        }
    });

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move { make_ingester(on_event).await })?;
    Ok(())
}

/// Multi-symbol Alpaca ingester — single WS connection, per-symbol CSV output.
#[cfg(feature = "ingest")]
fn run_alpaca_ingest_multi(symbols: &[String], feed_str: &str, out_dir: &str) -> Result<()> {
    use orderflow_rs::features::FeatureExtractor;
    use orderflow_rs::orderbook::Orderbook;
    use orderflow_rs::pipeline::alpaca::{AlpacaFeed, AlpacaIngester};
    use orderflow_rs::pipeline::labels::LabelQueue;
    use orderflow_rs::pipeline::Event;
    use std::collections::HashMap;
    use std::io::{BufWriter, Write};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    let key    = std::env::var("ALPACA_KEY")
        .map_err(|_| anyhow::anyhow!("Set ALPACA_KEY env var (free account at alpaca.markets)"))?;
    let secret = std::env::var("ALPACA_SECRET")
        .map_err(|_| anyhow::anyhow!("Set ALPACA_SECRET env var"))?;

    let feed = match feed_str {
        "sip"    => AlpacaFeed::Sip,
        "crypto" => AlpacaFeed::Crypto,
        _        => AlpacaFeed::Iex,
    };

    std::fs::create_dir_all(out_dir)?;
    println!("[alpaca] Connecting ({feed_str} feed) for {} symbols: {}",
        symbols.len(), symbols.join(", "));
    println!("Output dir: {out_dir}");
    println!("Press Ctrl-C to stop.\n");

    // Per-symbol state
    type SymState = (Orderbook, FeatureExtractor, LabelQueue, BufWriter<std::fs::File>);
    let states: Arc<Mutex<HashMap<String, SymState>>> = Arc::new(Mutex::new(HashMap::new()));

    for sym in symbols {
        let csv_path = PathBuf::from(out_dir)
            .join(format!("alpaca_{}_features.csv", sym.to_lowercase().replace('/', "_")));
        let f = std::fs::OpenOptions::new()
            .create(true).truncate(true).write(true)
            .open(&csv_path)?;
        let mut w = BufWriter::new(f);
        writeln!(w, "ts,ofi_1,ofi_5,ofi_10,depth_imb,microprice_dev,queue_imb,spread,\
                      trade_intensity,price_impact,level_drain,weighted_mid_slope,\
                      r_1s,r_5s,r_30s,r_300s,sign_1s,sign_5s,\
                      exchange,symbol,is_imputed,gap_flag")?;
        let book = Orderbook::new();
        let fx   = FeatureExtractor::new("alpaca", sym, "live");
        let lq   = LabelQueue::new();
        states.lock().unwrap().insert(sym.clone(), (book, fx, lq, w));
    }

    // Last known bid/ask per symbol for the status line
    let last_quote: Arc<Mutex<HashMap<String, (f64, f64)>>> = Arc::new(Mutex::new(HashMap::new()));

    let syms_owned = symbols.to_vec();
    let key2 = key.clone();
    let secret2 = secret.clone();

    let states2 = Arc::clone(&states);
    let last_quote2 = Arc::clone(&last_quote);

    let on_event = move |sym: String, event: Event, flags: orderflow_rs::pipeline::DataQualityFlags| {
        let now_us = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as i64;

        match event {
            Event::BookDelta(delta) => {
                let mut map = states2.lock().unwrap();
                if let Some((book, fx, lq, w)) = map.get_mut(&sym) {
                    if book.apply_delta(&delta).is_ok() {
                        let snap = book.export_snapshot(10);
                        let fv = fx.extract(&snap, flags.is_imputed, flags.gap_flag);
                        if let (Some(mid), Some(spread)) = (snap.mid(), snap.spread()) {
                            lq.push(fv, mid, spread);
                            for lfv in lq.drain_ready() {
                                let _ = write_labeled_row(w, &lfv);
                            }
                            if let (Some(bid), Some(ask)) = (snap.best_bid(), snap.best_ask()) {
                                last_quote2.lock().unwrap().insert(sym.clone(), (bid, ask));
                            }
                        }
                    }
                }
            }
            Event::Trade(trade) => {
                let mut map = states2.lock().unwrap();
                if let Some((_, fx, _, _)) = map.get_mut(&sym) {
                    fx.push_trade(trade);
                }
            }
            Event::Gap { .. } | Event::BookSnapshot { .. } => {
                let mut map = states2.lock().unwrap();
                if let Some((_, fx, _, _)) = map.get_mut(&sym) { fx.reset_on_gap(); }
            }
            Event::Disconnected => {
                let mut map = states2.lock().unwrap();
                for (_, (_, fx, _, _)) in map.iter_mut() { fx.reset_on_gap(); }
            }
        }

        // Status line: show all symbols' last quote every ~1s
        if now_us % 1_000_000 < 10_000 {
            use std::io::Write as _;
            let quotes = last_quote.lock().unwrap();
            let mut parts: Vec<String> = quotes.iter()
                .map(|(s, (b, a))| format!("{s}={b:.2}/{a:.2}"))
                .collect();
            parts.sort();
            print!("\r  {:<140}", parts.join("  "));
            let _ = std::io::stdout().flush();
        }
    };

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        let sym_refs: Vec<&str> = syms_owned.iter().map(|s| s.as_str()).collect();
        AlpacaIngester::new(&sym_refs, feed, &key2, &secret2)
            .run(on_event).await.unwrap();
    });
    Ok(())
}

#[cfg(not(feature = "ingest"))]
fn run_alpaca_ingest_multi(_symbols: &[String], _feed: &str, _out_dir: &str) -> Result<()> {
    anyhow::bail!("Rebuild with --features ingest");
}

/// Shared boilerplate for live WebSocket ingesters (single-symbol, legacy).
#[allow(dead_code)]
#[cfg(feature = "ingest")]
fn run_ws_ingest<F, Fut>(
    symbol: &str,
    exchange: &str,
    out_dir: &str,
    make_ingester: F,
) -> Result<()>
where
    F: FnOnce(String, Box<dyn FnMut(orderflow_rs::pipeline::Event, orderflow_rs::pipeline::DataQualityFlags) + Send>) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    use orderflow_rs::features::FeatureExtractor;
    use orderflow_rs::orderbook::Orderbook;
    use orderflow_rs::pipeline::labels::LabelQueue;
    use orderflow_rs::pipeline::Event;
    use std::io::{BufWriter, Write};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    std::fs::create_dir_all(out_dir)?;
    let sym_clean = symbol.replace('/', "_").replace('-', "_").to_lowercase();
    let csv_path = PathBuf::from(out_dir).join(format!("{exchange}_{sym_clean}_features.csv"));

    println!("[{exchange}] Connecting for {symbol} ...");
    println!("Output: {}", csv_path.display());
    println!("Press Ctrl-C to stop.\n");

    let book = Arc::new(Mutex::new(Orderbook::new()));
    let fx = Arc::new(Mutex::new(FeatureExtractor::new(exchange, symbol, "live")));
    let label_q = Arc::new(Mutex::new(LabelQueue::new()));

    let csv_file = std::fs::OpenOptions::new()
        .create(true).truncate(true).write(true)
        .open(&csv_path)?;
    let csv_w = Arc::new(Mutex::new(BufWriter::new(csv_file)));
    {
        let mut w = csv_w.lock().unwrap();
        writeln!(w, "ts,ofi_1,ofi_5,ofi_10,depth_imb,microprice_dev,queue_imb,spread,\
                      trade_intensity,price_impact,level_drain,weighted_mid_slope,\
                      r_1s,r_5s,r_30s,r_300s,sign_1s,sign_5s,\
                      exchange,symbol,is_imputed,gap_flag")?;
    }

    let on_event = {
        let book = Arc::clone(&book);
        let fx = Arc::clone(&fx);
        let label_q = Arc::clone(&label_q);
        let csv_w = Arc::clone(&csv_w);
        let sym = symbol.to_string();
        let exch = exchange.to_string();

        Box::new(move |event: Event, flags: orderflow_rs::pipeline::DataQualityFlags| {
            match event {
                Event::BookDelta(delta) => {
                    let mut b = book.lock().unwrap();
                    if let Err(e) = b.apply_delta(&delta) {
                        eprintln!("[{exch}] delta rejected: {e}");
                        return;
                    }
                    let snap = b.export_snapshot(20);
                    drop(b);

                    let mut f = fx.lock().unwrap();
                    let fv = f.extract(&snap, flags.is_imputed, flags.gap_flag);
                    drop(f);

                    if let (Some(mid), Some(spread)) = (snap.mid(), snap.spread()) {
                        let mut lq = label_q.lock().unwrap();
                        lq.push(fv.clone(), mid, spread);
                        let ready = lq.drain_ready();
                        drop(lq);

                        if !ready.is_empty() {
                            let mut w = csv_w.lock().unwrap();
                            for lfv in &ready {
                                let _ = write_labeled_row(&mut *w, lfv);
                            }
                        }
                    }

                    // Live status line
                    use std::io::Write as _;
                    if snap.ts % 1_000_000 < 10_000 {
                        if let (Some(bid), Some(ask)) = (snap.best_bid(), snap.best_ask()) {
                            print!("\r  [{exch}] {sym} bid={bid:.2} ask={ask:.2} spread={:.4} ofi1={:>8.1}   ",
                                ask - bid, fv.ofi_1.unwrap_or(0.0));
                            let _ = std::io::stdout().flush();
                        }
                    }
                }
                Event::Trade(trade) => {
                    fx.lock().unwrap().push_trade(trade);
                }
                Event::BookSnapshot { bids, asks, ts } => {
                    let mut b = book.lock().unwrap();
                    b.apply_snapshot_reset(bids.into_iter(), asks.into_iter(), ts);
                    let levels = b.export_snapshot(1000);
                    let bid_count = levels.bids.len();
                    let ask_count = levels.asks.len();
                    eprintln!("\n[{exch}] snapshot: {bid_count} bid levels, {ask_count} ask levels");
                    fx.lock().unwrap().reset_on_gap();
                }
                Event::Gap { start_ts, end_ts } => {
                    eprintln!("\n[{exch}] gap {start_ts}..{end_ts}, resyncing");
                    fx.lock().unwrap().reset_on_gap();
                }
                Event::Disconnected => {
                    eprintln!("\n[{exch}] disconnected, reconnecting...");
                    fx.lock().unwrap().reset_on_gap();
                }
            }
        }) as Box<dyn FnMut(Event, orderflow_rs::pipeline::DataQualityFlags) + Send>
    };

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(make_ingester(symbol.to_string(), on_event))?;
    Ok(())
}

fn run_lobster_ingest(
    messages_path: &str,
    orderbook_path: Option<&str>,
    out_dir: &str,
) -> Result<()> {
    use orderflow_rs::pipeline::lobster::{
        load_lobster_snapshot, parse_lobster_line, lobster_to_event,
    };
    use orderflow_rs::pipeline::Event;
    use std::fs::File;
    use std::io::{BufRead, BufReader};

    // Infer symbol from filename: AAPL_2012-06-21_..._message_10.csv → AAPL
    let symbol = std::path::Path::new(messages_path)
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.split('_').next())
        .unwrap_or("UNKNOWN");

    println!("LOBSTER replay: {messages_path}");
    println!("Symbol: {symbol}");

    // Load initial orderbook snapshot if provided
    let mut init_bids = vec![];
    let mut init_asks = vec![];
    if let Some(ob_path) = orderbook_path {
        let levels = std::path::Path::new(messages_path)
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| {
                // filename ends with _message_10.csv → extract 10
                n.split('_').last()?.split('.').next()?.parse::<usize>().ok()
            })
            .unwrap_or(10);

        (init_bids, init_asks) = load_lobster_snapshot(
            std::path::Path::new(ob_path),
            levels,
        )?;
        println!("Initial snapshot: {} bid levels, {} ask levels", init_bids.len(), init_asks.len());
    }

    // Build event iterator from file
    let messages_path_owned = messages_path.to_string();
    let file = File::open(&messages_path_owned)?;
    let reader = BufReader::new(file);

    let init_snapshot_event: Option<(Event, orderflow_rs::pipeline::DataQualityFlags)> =
        if !init_bids.is_empty() || !init_asks.is_empty() {
            Some((
                Event::BookSnapshot { bids: init_bids, asks: init_asks, ts: 0 },
                orderflow_rs::pipeline::DataQualityFlags {
                    is_imputed: false,
                    gap_flag: false,
                    is_outlier: false,
                    ts_source: orderflow_rs::pipeline::realdata::TsSource::Exchange,
                },
            ))
        } else {
            None
        };

    let line_events = reader.lines().filter_map(|line| {
        let line = line.ok()?;
        let msg = parse_lobster_line(line.trim())?;
        lobster_to_event(&msg)
    });

    let all_events = init_snapshot_event.into_iter().chain(line_events);

    let out_path = std::path::PathBuf::from(out_dir)
        .join(format!("{symbol}_lobster_features.csv"));
    run_pipeline_from_events(all_events, symbol, "lobster", "lobster", &out_path)?;
    Ok(())
}

fn run_iex_ingest(file_path: &str, symbol: &str, out_dir: &str) -> Result<()> {
    use orderflow_rs::pipeline::iex::parse_iex_line;
    use std::fs::File;
    use std::io::{BufRead, BufReader};

    println!("IEX replay: {file_path}");
    println!("Symbol filter: {symbol}");

    let file = File::open(file_path)?;
    let reader = BufReader::new(file);
    let symbol_owned = symbol.to_string();

    let events = reader.lines().filter_map(move |line| {
        let line = line.ok()?;
        let line = line.trim().to_string();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        // Symbol filter on raw string before full parse
        if !line.contains(&symbol_owned) && symbol_owned != "IEX" {
            return None;
        }
        let evts = parse_iex_line(&line);
        evts.into_iter().next() // take first event per line (simplification)
    });

    let out_path = std::path::PathBuf::from(out_dir)
        .join(format!("{}_iex_features.csv", symbol.replace('/', "_")));
    run_pipeline_from_events(events, symbol, "iex", "iex", &out_path)?;
    Ok(())
}

fn run_dukascopy_ingest(file_path: &str, symbol: &str, out_dir: &str) -> Result<()> {
    use orderflow_rs::pipeline::dukascopy::{parse_dukascopy_line, dukascopy_to_events};
    use std::fs::File;
    use std::io::{BufRead, BufReader};

    println!("Dukascopy replay: {file_path}");
    println!("Symbol: {symbol}");

    let file = File::open(file_path)?;
    let reader = BufReader::new(file);

    let events = reader.lines().enumerate().filter_map(|(i, line)| {
        let line = line.ok()?;
        let line = line.trim().to_string();
        // Skip header
        if i == 0 && (line.to_lowercase().starts_with("datetime")
            || line.to_lowercase().starts_with("gmt")
            || line.to_lowercase().starts_with("time")) {
            return None;
        }
        if line.is_empty() { return None; }
        let tick = parse_dukascopy_line(&line)?;
        let evts = dukascopy_to_events(&tick);
        // Return all events flattened — we can only return one per iteration,
        // so flatten by returning a vec that gets chained below
        Some(evts)
    }).flatten();

    let out_path = std::path::PathBuf::from(out_dir)
        .join(format!("{}_dukascopy_features.csv", symbol.replace('/', "_")));
    run_pipeline_from_events(events, symbol, "dukascopy", "dukascopy", &out_path)?;
    Ok(())
}


fn demo_orderbook() -> Result<()> {
    use orderflow_rs::features::FeatureExtractor;
    use orderflow_rs::orderbook::{BookDelta, Orderbook, Side};
    use ordered_float::OrderedFloat;

    let mut book = Orderbook::new();
    let mut fx = FeatureExtractor::new("demo", "BTC/USDT", "demo");

    for i in 0..5 {
        let bid_price = 100.0 - i as f64 * 0.1;
        let ask_price = 100.2 + i as f64 * 0.1;
        book.apply_delta(&BookDelta {
            price: OrderedFloat(bid_price), qty: 10.0 + i as f64,
            side: Side::Bid, exchange_ts: 1_000_000, recv_ts: 1_000_001,
        })?;
        book.apply_delta(&BookDelta {
            price: OrderedFloat(ask_price), qty: 8.0 + i as f64,
            side: Side::Ask, exchange_ts: 1_000_000, recv_ts: 1_000_001,
        })?;
    }

    let snap1 = book.export_snapshot(10);
    println!("Best bid: {:.2}, Best ask: {:.2}, Spread: {:.4}",
        snap1.best_bid().unwrap_or(f64::NAN),
        snap1.best_ask().unwrap_or(f64::NAN),
        snap1.spread().unwrap_or(f64::NAN));

    let fv1 = fx.extract(&snap1, false, false);
    println!("Tick 1 — OFI(1): {:?}, depth_imb: {:?}", fv1.ofi_1, fv1.depth_imb);

    book.apply_delta(&BookDelta {
        price: OrderedFloat(100.0), qty: 20.0,
        side: Side::Bid, exchange_ts: 1_100_000, recv_ts: 1_100_001,
    })?;

    let snap2 = book.export_snapshot(10);
    let fv2 = fx.extract(&snap2, false, false);
    println!("Tick 2 — OFI(1): {:?}, depth_imb: {:?}", fv2.ofi_1, fv2.depth_imb);

    Ok(())
}

// ── Analysis & Simulation runners ────────────────────────────────────────────

fn run_analyze(data_dir: &str, output_dir: &str) -> Result<()> {
    use orderflow_rs::analysis::report::analyze_directory;
    use std::path::Path;
    std::fs::create_dir_all(output_dir)?;
    println!("[analyze] scanning {data_dir} for feature CSV files...");
    analyze_directory(Path::new(data_dir), Path::new(output_dir))?;
    println!("[analyze] reports written to {output_dir}/");
    Ok(())
}

#[cfg(feature = "sim")]
fn run_simulate(n_paths: usize, out_dir: &str) -> Result<()> {
    use orderflow_rs::simulator::lob_sim::{SimConfig, run_simulation};
    use orderflow_rs::pipeline::labels::LabelQueue;
    use rayon::prelude::*;
    use rand::SeedableRng;
    use std::io::{BufWriter, Write};

    std::fs::create_dir_all(out_dir)?;
    let config = SimConfig::default();
    println!("[simulate] running {n_paths} path(s) × {:.0}h (parallel)...",
        config.duration_s / 3600.0);

    // Run all paths in parallel; collect labeled rows per path.
    let all_paths: Vec<Vec<orderflow_rs::pipeline::labels::LabeledFeatureVector>> = (0..n_paths)
        .into_par_iter()
        .map(|p| {
            let mut rng = rand::rngs::SmallRng::seed_from_u64(p as u64 * 6364136223846793005 + 1442695040888963407);
            let output = run_simulation(&mut rng, &config);

            // mid_prices[i] and feature_vectors[i] are 1:1 (both emitted every snap_every steps)
            let mut lq = LabelQueue::new();
            for (fv, (_, mid)) in output.feature_vectors.iter()
                                        .zip(output.mid_prices.iter())
            {
                let spread = fv.spread.unwrap_or(0.001 * mid);
                lq.push(fv.clone(), *mid, spread);
            }
            lq.flush();
            lq.drain_ready()
        })
        .collect();

    let total_paths = all_paths.len();
    let total_rows: usize = all_paths.iter().map(|p| p.len()).sum();
    println!("[simulate] {total_paths} paths done, {total_rows} labeled rows total");

    // Write a single combined feature CSV (same schema as real-data ingestion)
    let path_out = std::path::Path::new(out_dir).join("sim_features.csv");
    let file = std::fs::OpenOptions::new()
        .create(true).truncate(true).write(true).open(&path_out)?;
    let mut w = BufWriter::new(file);
    writeln!(w, "ts,ofi_1,ofi_5,ofi_10,depth_imb,microprice_dev,queue_imb,spread,\
                  trade_intensity,price_impact,level_drain,weighted_mid_slope,\
                  r_1s,r_5s,r_30s,r_300s,sign_1s,sign_5s,\
                  exchange,symbol,is_imputed,gap_flag")?;

    let mut written = 0usize;
    for labeled_rows in &all_paths {
        for lfv in labeled_rows {
            write_labeled_row(&mut w, lfv)?;
            written += 1;
        }
    }
    w.flush()?;
    println!("[simulate] wrote {written} rows to {}", path_out.display());
    Ok(())
}

#[cfg(not(feature = "sim"))]
fn run_simulate(_n: usize, _d: &str) -> Result<()> { anyhow::bail!("Rebuild with --features sim"); }

fn run_backtest_cmd(data_dir: &str, output_dir: &str, train_days: f64, test_days: f64) -> Result<()> {
    use orderflow_rs::analysis::backtest::{BacktestConfig, backtest_directory};
    use std::path::Path;
    let config = BacktestConfig {
        train_days,
        test_days,
        ..BacktestConfig::default()
    };
    println!("[backtest] scanning {data_dir} (train={train_days}d test={test_days}d)...");
    backtest_directory(Path::new(data_dir), Path::new(output_dir), &config)?;
    println!("[backtest] reports written to {output_dir}/");
    Ok(())
}
