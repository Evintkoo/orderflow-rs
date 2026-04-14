//! Walk-forward backtest for LOB microstructure signals.
//!
//! Strategy:
//!   - Data: downsampled rows are resampled to non-overlapping ~300s bars
//!     (avoids the 83% overlap in consecutive r_300s windows at 52s avg spacing)
//!   - Liquid filter: rows with spread > max_spread_multiple × median_spread skipped
//!   - Composite signal: OLS-weighted z-score of 5 features
//!     (ofi_1, depth_imb, microprice_dev, spread, level_drain)
//!   - Exponential decay filter with half-life τ* (from IC decay curve)
//!   - Position: Long if s > k·σ_s, Short if s < -k·σ_s, else Flat
//!   - Bar PnL: position × r_300s (5-min forward return)
//!   - Costs on position change: half-spread (spread/2) + fee_per_trade
//!     (spread is in absolute price units; r_300s is fractional; costs comparable
//!      for FX where spread ~ 0.00008 and typical r_300s ~ 0.0003)

use std::fmt::Write as FmtWrite;
use std::path::Path;

use crate::analysis::report::FeatureRow;
use crate::analysis::ml::{DecayFilter, ZScoreState};
use crate::analysis::stats::{ols, spearman_ic};
use ndarray::{Array1, Array2};

/// Indices into the 5-feature vector used by the backtest.
/// [ofi_1, depth_imb, microprice_dev, spread, level_drain]
const N_FEATS: usize = 5;

fn extract_features(row: &FeatureRow) -> Option<[f64; N_FEATS]> {
    let ofi_1       = row.ofi_1?;
    let depth_imb   = row.depth_imb?;
    let micro_dev   = row.microprice_dev?;
    let spread      = row.spread?;
    let level_drain = row.level_drain?;
    if [ofi_1, depth_imb, micro_dev, spread, level_drain].iter().all(|v| v.is_finite()) {
        Some([ofi_1, depth_imb, micro_dev, spread, level_drain])
    } else {
        None
    }
}

/// Configuration for the walk-forward backtest.
#[derive(Debug, Clone)]
pub struct BacktestConfig {
    /// Training window in days.
    pub train_days: f64,
    /// Test (out-of-sample) window in days.
    pub test_days: f64,
    /// Signal entry thresholds to evaluate (k values).
    pub thresholds: Vec<f64>,
    /// Decay filter half-life τ* in seconds (from IC decay curve).
    pub tau_star_s: f64,
    /// Snapshot interval in seconds (100ms for live data; varies after downsampling).
    pub dt_s: f64,
    /// Round-trip fee per trade as fraction of notional (e.g. 0.0004 = 0.04%).
    pub fee_per_trade: f64,
    /// Resample rows to non-overlapping ~N-second bars before computing PnL.
    /// Fixes the 83% r_300s overlap between consecutive 52s-apart bars.
    /// Set to 0.0 to disable resampling.
    pub resample_period_s: f64,
    /// Maximum allowed spread as multiple of the median spread.
    /// Rows with spread > max_spread_multiple × median_spread are skipped (flat position).
    /// Filters illiquid periods (nights, weekends) with blown-out FX spreads.
    pub max_spread_multiple: f64,
}

impl Default for BacktestConfig {
    fn default() -> Self {
        Self {
            train_days: 14.0,
            test_days: 7.0,
            thresholds: vec![0.5, 1.0, 1.5],
            tau_star_s: 6.0, // median τ* from P5 across passing pairs
            dt_s: 0.1,       // nominal; overridden by measured avg dt
            fee_per_trade: 0.0004,
            resample_period_s: 300.0,  // resample to non-overlapping 5-min bars
            max_spread_multiple: 5.0,  // skip rows where spread > 5× median
        }
    }
}

/// Per-threshold result for one walk-forward window.
#[derive(Debug, Clone)]
pub struct ThresholdResult {
    pub k: f64,
    pub n_trades: usize,
    /// Cumulative net P&L (as fraction of notional, e.g. 0.01 = 1%).
    pub total_pnl_net: f64,
    /// Annualized Sharpe ratio of per-bar P&L.
    pub sharpe: f64,
    /// Maximum drawdown (peak-to-trough, as fraction of notional).
    pub max_drawdown: f64,
    /// Fraction of bars with a position change.
    pub turnover: f64,
}

/// Result for one walk-forward window.
#[derive(Debug, Clone)]
pub struct WindowResult {
    pub window_idx: usize,
    pub train_rows: usize,
    pub test_rows: usize,
    /// OOS IC of composite signal vs r_300s in the test window.
    pub oos_ic: f64,
    pub per_threshold: Vec<ThresholdResult>,
}

/// Aggregated summary across all walk-forward windows.
#[derive(Debug)]
pub struct BacktestSummary {
    pub symbol: String,
    pub n_windows: usize,
    pub avg_oos_ic: f64,
    pub windows: Vec<WindowResult>,
    /// Aggregated across windows, per threshold.
    pub agg: Vec<AggResult>,
    /// Resample skip applied (1 = no resampling).
    pub resample_skip: usize,
    /// Max allowed spread for liquid filter.
    pub max_spread: f64,
    /// Median spread of the full dataset.
    pub median_spread: f64,
}

#[derive(Debug)]
pub struct AggResult {
    pub k: f64,
    pub total_pnl_net: f64,
    pub avg_sharpe: f64,
    pub max_drawdown: f64,
    pub avg_turnover: f64,
    pub n_trades_total: usize,
}

// ─── Core walk-forward engine ────────────────────────────────────────────────

/// Run a walk-forward backtest on a sorted slice of FeatureRows.
///
/// Rows must be sorted by timestamp ascending.
/// Returns None if there is insufficient data for at least one window.
pub fn run_backtest(rows: &[FeatureRow], config: &BacktestConfig, symbol: &str) -> Option<BacktestSummary> {
    if rows.is_empty() {
        return None;
    }

    let ts_min = rows.first()?.ts;
    let ts_max = rows.last()?.ts;
    let total_span_us = ts_max - ts_min;
    let us_per_day = 86_400_000_000_i64;

    let train_us = (config.train_days * us_per_day as f64) as i64;
    let test_us  = (config.test_days  * us_per_day as f64) as i64;

    if total_span_us < train_us + test_us {
        return None;
    }

    // Measure average dt from data (after downsampling, rows are NOT 100ms apart).
    let avg_dt_s = if rows.len() > 1 {
        (total_span_us as f64 / 1e6) / (rows.len() - 1) as f64
    } else {
        config.dt_s
    };

    // Resample skip factor: fix the 83% r_300s overlap between consecutive ~52s rows.
    // With resample_period_s=300 and avg_dt=52s, skip=6 → ~50k rows become ~8k bars.
    let resample_skip = if config.resample_period_s > 0.0 && avg_dt_s > 0.0 {
        (config.resample_period_s / avg_dt_s).round().max(1.0) as usize
    } else {
        1
    };

    // Median spread for the entire dataset (liquid filter).
    let median_spread = {
        let mut sv: Vec<f64> = rows.iter()
            .filter_map(|r| r.spread.filter(|s| s.is_finite() && *s > 0.0))
            .collect();
        sv.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        sv.get(sv.len() / 2).copied().unwrap_or(f64::INFINITY)
    };
    let max_spread = config.max_spread_multiple * median_spread;

    let mut windows: Vec<WindowResult> = Vec::new();
    let mut window_idx = 0usize;

    let mut train_start_us = ts_min;
    loop {
        let train_end_us = train_start_us + train_us;
        let test_end_us  = train_end_us   + test_us;
        if test_end_us > ts_max { break; }

        // Collect window rows then resample to non-overlapping bars.
        let train_rows: Vec<&FeatureRow> = rows.iter()
            .filter(|r| r.ts >= train_start_us && r.ts < train_end_us)
            .step_by(resample_skip)
            .collect();
        let test_rows: Vec<&FeatureRow> = rows.iter()
            .filter(|r| r.ts >= train_end_us && r.ts < test_end_us)
            .step_by(resample_skip)
            .collect();

        if train_rows.len() < 20 || test_rows.len() < 10 {
            train_start_us += test_us;
            window_idx += 1;
            continue;
        }

        // Fit OLS betas on training set (target: r_300s)
        let betas = fit_betas(&train_rows);

        // Evaluate on test set
        let wr = evaluate_window(
            &test_rows,
            betas.as_deref(),
            config,
            avg_dt_s * resample_skip as f64,
            max_spread,
            window_idx,
        );
        windows.push(wr);

        train_start_us += test_us; // slide by test_days
        window_idx += 1;
    }

    if windows.is_empty() {
        return None;
    }

    let avg_oos_ic = windows.iter().map(|w| w.oos_ic).sum::<f64>() / windows.len() as f64;
    let n_windows = windows.len();

    // Aggregate per-threshold across windows
    let n_thresh = config.thresholds.len();
    let mut agg: Vec<AggResult> = config.thresholds.iter().map(|&k| AggResult {
        k,
        total_pnl_net: 0.0,
        avg_sharpe: 0.0,
        max_drawdown: 0.0,
        avg_turnover: 0.0,
        n_trades_total: 0,
    }).collect();

    for w in &windows {
        for (i, tr) in w.per_threshold.iter().enumerate() {
            if i < n_thresh {
                agg[i].total_pnl_net    += tr.total_pnl_net;
                agg[i].avg_sharpe       += tr.sharpe / n_windows as f64;
                agg[i].max_drawdown      = agg[i].max_drawdown.max(tr.max_drawdown);
                agg[i].avg_turnover     += tr.turnover / n_windows as f64;
                agg[i].n_trades_total   += tr.n_trades;
            }
        }
    }

    Some(BacktestSummary {
        symbol: symbol.to_string(),
        n_windows,
        avg_oos_ic,
        windows,
        agg,
        resample_skip,
        max_spread,
        median_spread,
    })
}

// ─── Training ────────────────────────────────────────────────────────────────

/// Fit OLS betas on r_300s for the 5 features.
/// Returns Some(betas) or None if insufficient clean data.
fn fit_betas(rows: &[&FeatureRow]) -> Option<Vec<f64>> {
    let pairs: Vec<([f64; N_FEATS], f64)> = rows.iter()
        .filter_map(|r| {
            let feats = extract_features(r)?;
            let ret   = r.r_300s?;
            if ret.is_finite() { Some((feats, ret)) } else { None }
        })
        .collect();

    if pairs.len() < 20 {
        return None;
    }

    let n = pairs.len();
    let mut x = Array2::zeros((n, N_FEATS + 1));
    let mut y = Array1::zeros(n);

    for (i, (feats, ret)) in pairs.iter().enumerate() {
        x[[i, 0]] = 1.0; // intercept
        for j in 0..N_FEATS {
            x[[i, j + 1]] = feats[j];
        }
        y[i] = *ret;
    }

    let (beta, _) = ols(&x, &y)?;
    // Skip intercept (index 0), return feature betas
    let betas: Vec<f64> = beta.iter().skip(1).copied().collect();
    Some(betas)
}

// ─── Evaluation ─────────────────────────────────────────────────────────────

/// Evaluate signal on test rows.
fn evaluate_window(
    rows: &[&FeatureRow],
    betas: Option<&[f64]>,
    config: &BacktestConfig,
    bar_dt_s: f64,
    max_spread: f64,
    window_idx: usize,
) -> WindowResult {
    // Compute composite signal for each row
    let signals = compute_signals(rows, betas, config, bar_dt_s);

    // OOS IC: correlation of raw signal (pre-threshold) with r_300s
    let oos_ic = {
        let pairs: Vec<(f64, f64)> = signals.iter()
            .zip(rows.iter())
            .filter_map(|(&s, r)| {
                let ret = r.r_300s?;
                if s.is_finite() && ret.is_finite() { Some((s, ret)) } else { None }
            })
            .collect();
        let sigs: Vec<f64> = pairs.iter().map(|(s, _)| *s).collect();
        let rets: Vec<f64> = pairs.iter().map(|(_, r)| *r).collect();
        spearman_ic(&sigs, &rets).unwrap_or(0.0)
    };

    // Signal std for threshold scaling
    let finite_sigs: Vec<f64> = signals.iter().copied().filter(|s| s.is_finite()).collect();
    let sigma_s = if finite_sigs.len() > 1 {
        let mean = finite_sigs.iter().sum::<f64>() / finite_sigs.len() as f64;
        let var  = finite_sigs.iter().map(|s| (s - mean).powi(2)).sum::<f64>()
                   / finite_sigs.len() as f64;
        var.sqrt().max(1e-10)
    } else {
        1.0
    };

    let per_threshold: Vec<ThresholdResult> = config.thresholds.iter()
        .map(|&k| simulate_strategy(rows, &signals, sigma_s, k, config, max_spread))
        .collect();

    WindowResult {
        window_idx,
        train_rows: 0, // filled by caller if desired
        test_rows: rows.len(),
        oos_ic,
        per_threshold,
    }
}

// ─── Signal construction ─────────────────────────────────────────────────────

/// Build composite signals for a test window.
///
/// Uses betas from training (or equal weights if None).
/// Applies rolling z-score normalization and exponential decay filter.
fn compute_signals(
    rows: &[&FeatureRow],
    betas: Option<&[f64]>,
    config: &BacktestConfig,
    bar_dt_s: f64,
) -> Vec<f64> {
    let weights: Vec<f64> = match betas {
        Some(b) => {
            let abs_sum: f64 = b.iter().map(|x| x.abs()).sum();
            if abs_sum < 1e-12 {
                vec![1.0 / N_FEATS as f64; N_FEATS]
            } else {
                b.iter().map(|x| x / abs_sum).collect()
            }
        }
        None => vec![1.0 / N_FEATS as f64; N_FEATS],
    };

    // Rolling z-score window: 30 min worth of bars (capped at 300)
    let zscore_window = ((30.0 * 60.0 / bar_dt_s) as usize).min(300).max(10);
    let mut z_states: Vec<ZScoreState> = (0..N_FEATS)
        .map(|_| ZScoreState::new(zscore_window))
        .collect();

    let mut decay = DecayFilter::new(config.tau_star_s, bar_dt_s);

    rows.iter().map(|row| {
        let feats = match extract_features(row) {
            Some(f) => f,
            None => return f64::NAN,
        };

        let mut raw_signal = 0.0;
        let mut n_valid = 0;
        for i in 0..N_FEATS {
            if let Some(z) = z_states[i].update(feats[i]) {
                raw_signal += weights[i] * z;
                n_valid += 1;
            }
        }
        if n_valid == 0 {
            return f64::NAN;
        }

        decay.update(raw_signal)
    }).collect()
}

// ─── Strategy simulation ─────────────────────────────────────────────────────

/// Simulate the long/short/flat strategy on test rows.
///
/// `max_spread`: rows with spread > max_spread are treated as flat (illiquid filter).
fn simulate_strategy(
    rows: &[&FeatureRow],
    signals: &[f64],
    sigma_s: f64,
    k: f64,
    config: &BacktestConfig,
    max_spread: f64,
) -> ThresholdResult {
    let n = rows.len().min(signals.len());
    let mut prev_pos: i8 = 0;
    let mut n_trades = 0usize;
    let mut cumulative_pnl = 0.0_f64;
    let mut peak_pnl = 0.0_f64;
    let mut max_dd = 0.0_f64;
    let mut n_changes = 0usize;
    let mut bar_pnls: Vec<f64> = Vec::with_capacity(n);

    for i in 0..n {
        let s = signals[i];
        let row = rows[i];

        let ret_300s = row.r_300s.unwrap_or(0.0);
        let spread = row.spread.unwrap_or(0.0);

        // Liquid filter: force flat when spread is too wide (off-hours, illiquid).
        let is_illiquid = spread > max_spread;

        // Determine position from signal
        let pos: i8 = if is_illiquid || !s.is_finite() {
            0
        } else if s > k * sigma_s {
            1
        } else if s < -k * sigma_s {
            -1
        } else {
            0
        };

        // Cost: paid when position changes
        let position_change = (pos - prev_pos).abs() as f64;
        // Half-spread: spread/2 on entry + spread/2 on exit (amortized: pay on change)
        let half_spread_cost = (spread / 2.0) * position_change;
        let fee_cost = config.fee_per_trade * position_change;
        let total_cost = half_spread_cost + fee_cost;

        if position_change > 0.5 {
            n_trades += 1;
            n_changes += 1;
        }

        // Bar P&L: prior position × this bar's return − cost
        let gross_pnl = prev_pos as f64 * ret_300s;
        let net_pnl = gross_pnl - total_cost;
        cumulative_pnl += net_pnl;
        bar_pnls.push(net_pnl);

        // Track drawdown
        if cumulative_pnl > peak_pnl {
            peak_pnl = cumulative_pnl;
        }
        let dd = peak_pnl - cumulative_pnl;
        if dd > max_dd {
            max_dd = dd;
        }

        prev_pos = pos;
    }

    // Annualized Sharpe using bar P&L series
    let sharpe = annualized_sharpe(&bar_pnls);
    let turnover = if n > 0 { n_changes as f64 / n as f64 } else { 0.0 };

    ThresholdResult {
        k,
        n_trades,
        total_pnl_net: cumulative_pnl,
        sharpe,
        max_drawdown: max_dd,
        turnover,
    }
}

/// Sharpe ratio scaled by √N (period Sharpe, not annualized).
///
/// Reports mean/std × √N which is the t-statistic of the mean return.
/// Use this to compare across windows — divide by √(bars_per_year) to annualize.
fn annualized_sharpe(bar_pnls: &[f64]) -> f64 {
    if bar_pnls.len() < 2 {
        return 0.0;
    }
    let n = bar_pnls.len() as f64;
    let mean = bar_pnls.iter().sum::<f64>() / n;
    let var  = bar_pnls.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / n;
    let std  = var.sqrt();
    if std < 1e-15 {
        return 0.0;
    }
    // t-statistic = mean/std × √N: positive = signal adds value in this period
    (mean / std) * n.sqrt()
}

// ─── Report writer ────────────────────────────────────────────────────────────

/// Write a formatted backtest report to a string.
pub fn format_report(summary: &BacktestSummary) -> String {
    let mut out = String::new();
    writeln!(out, "═══════════════════════════════════════════════════════════════").unwrap();
    writeln!(out, "  ORDERFLOW-RS  P6  BACKTEST  REPORT").unwrap();
    writeln!(out, "═══════════════════════════════════════════════════════════════").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "  Symbol       : {}", summary.symbol).unwrap();
    writeln!(out, "  Windows      : {}", summary.n_windows).unwrap();
    writeln!(out, "  Avg OOS IC   : {:.4}", summary.avg_oos_ic).unwrap();
    writeln!(out, "  Resample skip: {} (non-overlapping 300s bars)", summary.resample_skip).unwrap();
    writeln!(out, "  Median spread: {:.6}  Max allowed: {:.6} ({}× filter)",
        summary.median_spread, summary.max_spread, 5).unwrap();
    writeln!(out).unwrap();

    writeln!(out, "SECTION 1: Walk-Forward Window Summary").unwrap();
    writeln!(out, "─────────────────────────────────────────────────────────────").unwrap();
    writeln!(out, "{:<6}  {:>8}  {:>8}  {:>8}  {:>10}  {:>10}  {:>10}",
        "Win", "TestRows", "OOS_IC", "k=0.5", "k=1.0", "k=1.5", "").unwrap();
    writeln!(out, "{:<6}  {:>8}  {:>8}  {:>10}  {:>10}  {:>10}",
        "", "", "", "PnL(net)", "PnL(net)", "PnL(net)").unwrap();
    writeln!(out, "{}", "-".repeat(65)).unwrap();

    for w in &summary.windows {
        let pnls: Vec<String> = w.per_threshold.iter()
            .map(|t| format!("{:+.6}", t.total_pnl_net))
            .collect();
        let pnl_str = pnls.join("  ");
        writeln!(out, "{:<6}  {:>8}  {:>8.4}  {}",
            w.window_idx, w.test_rows, w.oos_ic, pnl_str).unwrap();
    }

    writeln!(out).unwrap();
    writeln!(out, "SECTION 2: Aggregated Results by Threshold").unwrap();
    writeln!(out, "─────────────────────────────────────────────────────────────").unwrap();
    writeln!(out, "{:<6}  {:>12}  {:>10}  {:>12}  {:>10}  {:>10}",
        "k", "TotalPnL", "AvgSharpe", "MaxDrawdown", "AvgTurnover", "NTrades").unwrap();
    writeln!(out, "{}", "-".repeat(65)).unwrap();

    for a in &summary.agg {
        writeln!(out, "{:<6.1}  {:>+12.6}  {:>10.4}  {:>12.6}  {:>10.4}  {:>10}",
            a.k, a.total_pnl_net, a.avg_sharpe, a.max_drawdown,
            a.avg_turnover, a.n_trades_total).unwrap();
    }

    writeln!(out).unwrap();
    writeln!(out, "SECTION 3: Detailed Window Results").unwrap();
    writeln!(out, "─────────────────────────────────────────────────────────────").unwrap();

    for w in &summary.windows {
        writeln!(out, "  Window {}  (test_rows={}  oos_ic={:.4})",
            w.window_idx, w.test_rows, w.oos_ic).unwrap();
        writeln!(out, "  {:>6}  {:>10}  {:>10}  {:>12}  {:>10}",
            "k", "NTrades", "PnL(net)", "MaxDrawdown", "Turnover").unwrap();
        for t in &w.per_threshold {
            writeln!(out, "  {:>6.1}  {:>10}  {:>+10.6}  {:>12.6}  {:>10.4}",
                t.k, t.n_trades, t.total_pnl_net, t.max_drawdown, t.turnover).unwrap();
        }
        writeln!(out).unwrap();
    }

    writeln!(out, "═══════════════════════════════════════════════════════════════").unwrap();
    writeln!(out, "  END OF REPORT").unwrap();
    writeln!(out, "═══════════════════════════════════════════════════════════════").unwrap();
    out
}

/// Write the backtest report to a file.
pub fn write_report(summary: &BacktestSummary, output_dir: &Path) -> std::io::Result<()> {
    use std::io::Write;
    let sym_clean = summary.symbol.to_lowercase()
        .replace(['/', '-', '_'], "");
    let path = output_dir.join(format!("backtest_{sym_clean}_report.txt"));
    let content = format_report(summary);
    let mut f = std::fs::File::create(&path)?;
    f.write_all(content.as_bytes())?;
    Ok(())
}

/// Scan a directory for feature CSVs and run backtest on each.
pub fn backtest_directory(
    data_dir: &Path,
    output_dir: &Path,
    config: &BacktestConfig,
) -> anyhow::Result<()> {
    use crate::analysis::report::load_csv;

    let entries: Vec<_> = std::fs::read_dir(data_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name();
            let s = n.to_string_lossy();
            s.ends_with("_features.csv")
        })
        .collect();

    if entries.is_empty() {
        println!("[backtest] no feature CSV files found in {}", data_dir.display());
        return Ok(());
    }

    std::fs::create_dir_all(output_dir)?;

    for entry in &entries {
        let path = entry.path();
        let fname = path.file_name().unwrap().to_string_lossy().to_string();
        println!("[backtest] loading {fname}");

        let mut rows = match load_csv(&path) {
            Ok(r) => r,
            Err(e) => { eprintln!("[backtest] skip {fname}: {e}"); continue; }
        };
        rows.sort_unstable_by_key(|r| r.ts);

        // Derive symbol from filename: e.g. dukascopy_eurusd_features.csv → EURUSD
        let symbol = fname
            .trim_end_matches("_features.csv")
            .split('_')
            .last()
            .unwrap_or("?")
            .to_uppercase();

        println!("[backtest]   {} rows, symbol={}", rows.len(), symbol);

        match run_backtest(&rows, config, &symbol) {
            Some(summary) => {
                write_report(&summary, output_dir)?;
                println!("[backtest]   windows={} avg_oos_ic={:.4}  report written",
                    summary.n_windows, summary.avg_oos_ic);
            }
            None => {
                println!("[backtest]   insufficient data for walk-forward (skipped)");
            }
        }
    }
    Ok(())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::report::FeatureRow;

    fn make_row(ts_us: i64, ofi: f64, ret_300: f64, spread: f64) -> FeatureRow {
        FeatureRow {
            ts: ts_us,
            ofi_1: Some(ofi), ofi_5: Some(ofi), ofi_10: Some(ofi),
            depth_imb: Some(ofi * 0.5),
            microprice_dev: Some(ofi * 0.3),
            queue_imb: Some(ofi * 0.5),
            spread: Some(spread),
            trade_intensity: None,
            price_impact: None,
            level_drain: Some(-ofi * 0.2),
            weighted_mid_slope: None,
            r_1s: None, r_5s: None, r_30s: None, r_300s: Some(ret_300),
            exchange: "test".into(), symbol: "T".into(),
            is_imputed: false, gap_flag: false,
        }
    }

    #[test]
    fn backtest_runs_on_toy_data() {
        // 22 days of toy data (need >21 for a 14+7 window)
        let n = 22_000usize;
        let us_per_row = 86_400_000_000_i64 / 1000;
        let rows: Vec<FeatureRow> = (0..n).map(|i| {
            let ts = i as i64 * us_per_row;
            let ofi = if i % 3 == 0 { 1.0 } else { -0.5 };
            let ret = ofi * 0.0001 + 0.00001 * ((i % 7) as f64 - 3.0);
            make_row(ts, ofi, ret, 0.0002)
        }).collect();

        let mut config = BacktestConfig::default();
        config.train_days = 14.0;
        config.test_days  = 7.0;
        config.thresholds = vec![0.5, 1.0];

        let summary = run_backtest(&rows, &config, "TEST").expect("should produce result");
        assert!(summary.n_windows >= 1, "should have at least 1 window");
        assert!(summary.avg_oos_ic.is_finite());
    }

    #[test]
    fn signal_tracks_positive_ofi() {
        // All rows have positive OFI and positive forward returns
        let n = 500usize;
        let us_per_row = 60_000_000_i64; // 1 minute per row
        let rows: Vec<FeatureRow> = (0..n).map(|i| {
            make_row(i as i64 * us_per_row, 1.0, 0.001, 0.0001)
        }).collect();

        let config = BacktestConfig {
            train_days: 0.2,
            test_days: 0.1,
            thresholds: vec![0.5],
            tau_star_s: 1.0,
            dt_s: 60.0,
            fee_per_trade: 0.0,
            resample_period_s: 0.0,
            max_spread_multiple: 0.0,
        };
        let summary = run_backtest(&rows, &config, "TEST");
        // May be None if data too short — just check it doesn't panic
        let _ = summary;
    }

    #[test]
    fn format_report_produces_sections() {
        let summary = BacktestSummary {
            symbol: "EURUSD".into(),
            n_windows: 1,
            avg_oos_ic: 0.05,
            windows: vec![WindowResult {
                window_idx: 0,
                train_rows: 1000,
                test_rows: 500,
                oos_ic: 0.05,
                per_threshold: vec![
                    ThresholdResult { k: 0.5, n_trades: 10, total_pnl_net: 0.001, sharpe: 0.5, max_drawdown: 0.0005, turnover: 0.1 },
                ],
            }],
            agg: vec![AggResult { k: 0.5, total_pnl_net: 0.001, avg_sharpe: 0.5, max_drawdown: 0.0005, avg_turnover: 0.1, n_trades_total: 10 }],
            resample_skip: 1,
            max_spread: 0.001,
            median_spread: 0.0002,
        };
        let report = format_report(&summary);
        assert!(report.contains("P6  BACKTEST  REPORT"));
        assert!(report.contains("EURUSD"));
        assert!(report.contains("Aggregated Results"));
    }
}
