# Technical Indicator Correlation Analysis Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `techanalysis` CLI command that computes ~152 technical indicators from existing `*_features.csv` files and outputs per-pair IC tables (vs r_1s/r_5s/r_30s/r_300s) and a cross-pair summary CSV.

**Architecture:** Post-processing pipeline — no changes to the LOB or feature pipeline. Reads existing CSV outputs, reconstructs OHLCV from mid-price + spread, computes all indicators as pure functions `fn(prices: &[f64], ...) -> Vec<Option<f64>>`, then computes Spearman IC against each return horizon and writes reports.

**Tech Stack:** Rust, `csv` crate (already in deps), `spearman_ic` from `src/analysis/stats.rs`, `FeatureRow` from `src/analysis/loader.rs`.

---

## File Map

| File | Action | Responsibility |
|------|--------|----------------|
| `src/analysis/techind.rs` | Create | All ~152 indicator pure functions + `TechData` struct + `compute_all()` |
| `src/analysis/techreport.rs` | Create | IC computation, CSV/text report formatter |
| `src/analysis/mod.rs` | Modify | Add `pub mod techind; pub mod techreport;` |
| `src/main.rs` | Modify | Add `techanalysis` subcommand dispatch |
| `tests/techind_test.rs` | Create | Unit tests for indicator functions |

---

### Task 1: Scaffold — TechData struct + CLI stub

**Files:**
- Create: `src/analysis/techind.rs`
- Create: `src/analysis/techreport.rs`
- Modify: `src/analysis/mod.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write the failing test**

In `tests/techind_test.rs` (new file):

```rust
#[cfg(test)]
mod tests {
    use orderflow_rs::analysis::techind::{TechData, IndicatorSeries};

    #[test]
    fn techdata_builds_from_vecs() {
        let td = TechData {
            ts: vec![0, 1, 2],
            prices: vec![1.0, 1.01, 1.02],
            highs: vec![1.005, 1.015, 1.025],
            lows: vec![0.995, 1.005, 1.015],
            volumes: vec![10.0, 10.0, 10.0],
            spreads: vec![0.001, 0.001, 0.001],
            r_1s: vec![Some(0.001), Some(0.002), None],
            r_5s: vec![Some(0.002), None, None],
            r_30s: vec![None; 3],
            r_300s: vec![None; 3],
            indicators: vec![],
        };
        assert_eq!(td.prices.len(), 3);
    }

    #[test]
    fn indicator_series_has_name() {
        let s = IndicatorSeries { name: "sma_10", values: vec![Some(1.0), None] };
        assert_eq!(s.name, "sma_10");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /Users/evintleovonzko/Documents/research/orderflow-rs
cargo test techdata_builds_from_vecs 2>&1 | tail -5
```

Expected: `error[E0433]: failed to resolve: use of undeclared crate or module`

- [ ] **Step 3: Create `src/analysis/techind.rs` with structs**

```rust
//! Technical indicator functions and TechData container.
//!
//! All indicator functions are pure: `fn(prices: &[f64], ...) -> Vec<Option<f64>>`.
//! No I/O, no state. `compute_all()` bundles every indicator into a `TechData`.

/// One named indicator series aligned with `TechData::prices`.
pub struct IndicatorSeries {
    pub name: &'static str,
    pub values: Vec<Option<f64>>,
}

/// Price/return data reconstructed from a `*_features.csv` file.
pub struct TechData {
    pub ts: Vec<i64>,
    pub prices: Vec<f64>,
    pub highs: Vec<f64>,
    pub lows: Vec<f64>,
    pub volumes: Vec<f64>,
    pub spreads: Vec<f64>,
    pub r_1s: Vec<Option<f64>>,
    pub r_5s: Vec<Option<f64>>,
    pub r_30s: Vec<Option<f64>>,
    pub r_300s: Vec<Option<f64>>,
    pub indicators: Vec<IndicatorSeries>,
}

impl TechData {
    pub fn new(
        ts: Vec<i64>,
        prices: Vec<f64>,
        highs: Vec<f64>,
        lows: Vec<f64>,
        volumes: Vec<f64>,
        spreads: Vec<f64>,
        r_1s: Vec<Option<f64>>,
        r_5s: Vec<Option<f64>>,
        r_30s: Vec<Option<f64>>,
        r_300s: Vec<Option<f64>>,
    ) -> Self {
        Self { ts, prices, highs, lows, volumes, spreads, r_1s, r_5s, r_30s, r_300s, indicators: vec![] }
    }
}
```

Create `src/analysis/techreport.rs` stub:

```rust
//! IC computation and report formatting for technical indicator analysis.
```

- [ ] **Step 4: Add modules to `src/analysis/mod.rs`**

Append after the last `pub mod` line:
```rust
pub mod techind;
pub mod techreport;
```

- [ ] **Step 5: Run test to verify it passes**

```bash
cargo test techdata_builds_from_vecs indicator_series_has_name 2>&1 | tail -5
```

Expected: `test result: ok. 2 passed`

- [ ] **Step 6: Add CLI stub in `src/main.rs`**

Find the `match subcommand` block. Add a new arm:

```rust
Some(("techanalysis", sub)) => {
    let data_dir = sub.get_one::<String>("data_dir").unwrap();
    let reports_dir = sub.get_one::<String>("reports_dir").unwrap();
    commands::techanalysis::run(data_dir, reports_dir)?;
}
```

Also add to the `Command::new` builder (alongside existing subcommands):

```rust
.subcommand(
    Command::new("techanalysis")
        .about("Compute technical indicator correlations from feature CSVs")
        .arg(Arg::new("data_dir").required(true).help("Directory with *_features.csv files"))
        .arg(Arg::new("reports_dir").required(true).help("Output directory for reports"))
)
```

Create `src/commands/techanalysis.rs`:

```rust
use anyhow::Result;

pub fn run(_data_dir: &str, _reports_dir: &str) -> Result<()> {
    println!("techanalysis: not yet implemented");
    Ok(())
}
```

Add `pub mod techanalysis;` to `src/commands/mod.rs`.

- [ ] **Step 7: Verify compilation**

```bash
cargo build 2>&1 | tail -10
```

Expected: `Finished` (no errors)

- [ ] **Step 8: Commit**

```bash
git add src/analysis/techind.rs src/analysis/techreport.rs src/analysis/mod.rs src/main.rs src/commands/techanalysis.rs src/commands/mod.rs tests/techind_test.rs
git commit -m "feat: scaffold techanalysis — TechData struct, CLI stub, module wiring"
```

---

### Task 2: CSV loader — build TechData from feature CSV

**Files:**
- Modify: `src/analysis/techreport.rs`
- Test: `tests/techind_test.rs`

Mid-price reconstruction: `p[0] = 1.0; p[i+1] = p[i] * (1 + r_1s[i].unwrap_or(0.0) * 0.1)`.
OHLC: `high = p + spread/2`, `low = p - spread/2`, `open = p[i-1]`.
Volume proxy: `volume = trade_intensity` field (already in `FeatureRow`).

- [ ] **Step 1: Write the failing test**

In `tests/techind_test.rs`, append:

```rust
use orderflow_rs::analysis::techreport::load_techdata;
use std::io::Write as IoWrite;

#[test]
fn load_techdata_reconstructs_prices() {
    // Write minimal CSV with required columns
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test_features.csv");
    let mut f = std::fs::File::create(&path).unwrap();
    writeln!(f, "ts,ofi_1,ofi_2,ofi_3,ofi_4,ofi_5,ofi_6,ofi_7,ofi_8,ofi_9,ofi_10,depth_imb,microprice_dev,queue_imb,spread,trade_intensity,price_impact,level_drain,weighted_mid_slope,r_1s,r_5s,r_30s,r_300s,exchange,symbol,is_imputed,gap_flag").unwrap();
    writeln!(f, "1000,0.1,0,0,0,0,0,0,0,0,0,0.01,0.001,0.02,0.0001,5.0,0.001,0.5,0.0002,0.001,0.002,0.003,0.004,DUKA,EURUSD,false,false").unwrap();
    writeln!(f, "2000,0.2,0,0,0,0,0,0,0,0,0,0.02,0.002,0.03,0.0001,6.0,0.001,0.6,0.0003,0.002,0.004,0.006,0.008,DUKA,EURUSD,false,false").unwrap();
    writeln!(f, "3000,0.0,0,0,0,0,0,0,0,0,0,0.01,0.001,0.02,0.0001,4.0,0.001,0.4,0.0001,,,,, DUKA,EURUSD,false,false").unwrap();

    let td = load_techdata(path.to_str().unwrap()).unwrap();
    assert_eq!(td.prices.len(), 3);
    assert!(td.prices[0] > 0.0);
    assert!(td.prices[1] > td.prices[0], "price should rise when r_1s > 0");
    assert_eq!(td.spreads.len(), 3);
    assert!(td.highs[0] > td.prices[0]);
    assert!(td.lows[0] < td.prices[0]);
}
```

Add `tempfile = "3"` to `[dev-dependencies]` in `Cargo.toml` if not present.

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test load_techdata_reconstructs_prices 2>&1 | tail -5
```

Expected: error — `load_techdata` not found.

- [ ] **Step 3: Implement `load_techdata` in `src/analysis/techreport.rs`**

```rust
use anyhow::{Context, Result};
use crate::analysis::loader::FeatureRow;
use crate::analysis::techind::{TechData};

pub fn load_techdata(csv_path: &str) -> Result<TechData> {
    let mut rdr = csv::Reader::from_path(csv_path)
        .with_context(|| format!("open {csv_path}"))?;
    let rows: Vec<FeatureRow> = rdr.deserialize()
        .filter_map(|r: csv::Result<FeatureRow>| r.ok())
        .collect();

    let n = rows.len();
    let mut ts = Vec::with_capacity(n);
    let mut prices = Vec::with_capacity(n);
    let mut highs = Vec::with_capacity(n);
    let mut lows = Vec::with_capacity(n);
    let mut volumes = Vec::with_capacity(n);
    let mut spreads = Vec::with_capacity(n);
    let mut r_1s = Vec::with_capacity(n);
    let mut r_5s = Vec::with_capacity(n);
    let mut r_30s = Vec::with_capacity(n);
    let mut r_300s = Vec::with_capacity(n);

    let mut p = 1.0_f64;
    for (i, row) in rows.iter().enumerate() {
        ts.push(row.ts);
        prices.push(p);
        let sp = row.spread.abs();
        highs.push(p + sp * 0.5);
        lows.push((p - sp * 0.5).max(1e-10));
        volumes.push(row.trade_intensity.abs());
        spreads.push(sp);
        r_1s.push(row.r_1s);
        r_5s.push(row.r_5s);
        r_30s.push(row.r_30s);
        r_300s.push(row.r_300s);
        // Advance price for next bar
        if i + 1 < n {
            p = p * (1.0 + row.r_1s.unwrap_or(0.0) * 0.1);
            p = p.max(1e-10);
        }
    }

    Ok(TechData::new(ts, prices, highs, lows, volumes, spreads,
                     r_1s, r_5s, r_30s, r_300s))
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test load_techdata_reconstructs_prices 2>&1 | tail -5
```

Expected: `test result: ok. 1 passed`

- [ ] **Step 5: Commit**

```bash
git add src/analysis/techreport.rs tests/techind_test.rs Cargo.toml
git commit -m "feat: techanalysis CSV loader — reconstruct TechData from feature CSV"
```

---

### Task 3: Moving Averages (18 indicators)

Indicators: `sma_N` (10/20/50/100/200), `ema_N` (5/12/20/26/50/200), `wma_14`, `dema_20`, `tema_20`, `hma_14`, `kama_10`, `golden_cross`, `death_cross`.

**Files:**
- Modify: `src/analysis/techind.rs`
- Test: `tests/techind_test.rs`

- [ ] **Step 1: Write the failing test**

```rust
use orderflow_rs::analysis::techind::{sma, ema, wma, dema, tema, hma, kama, golden_cross};

#[test]
fn sma_basic() {
    let p = vec![1.0, 2.0, 3.0, 4.0, 5.0];
    let result = sma(&p, 3);
    assert_eq!(result[0], None);
    assert_eq!(result[1], None);
    assert!((result[2].unwrap() - 2.0).abs() < 1e-9);
    assert!((result[4].unwrap() - 4.0).abs() < 1e-9);
}

#[test]
fn ema_converges() {
    let p: Vec<f64> = (0..50).map(|i| 10.0 + i as f64 * 0.1).collect();
    let result = ema(&p, 12);
    // EMA values after warmup should be positive and increasing
    let last = result[49].unwrap();
    let first_valid = result[11].unwrap();
    assert!(last > first_valid, "EMA should track rising prices");
}

#[test]
fn golden_cross_signal() {
    // fast > slow for long enough to trigger golden cross
    let mut p: Vec<f64> = vec![10.0; 30];
    // inject rising prices so EMA_5 crosses above EMA_20
    for i in 20..30 { p[i] = 10.0 + (i - 20) as f64 * 0.5; }
    let gc = golden_cross(&p, 5, 20);
    // At least some non-None values in the last portion
    assert!(gc.iter().skip(20).any(|v| v.is_some()));
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test sma_basic ema_converges golden_cross_signal 2>&1 | tail -5
```

Expected: `error[E0425]: cannot find function`

- [ ] **Step 3: Implement moving averages in `src/analysis/techind.rs`**

Append after the struct definitions:

```rust
// ─── helpers ──────────────────────────────────────────────────────────────────

/// Simple moving average over `period` bars.
pub fn sma(prices: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    if period == 0 || period > n { return out; }
    let mut sum: f64 = prices[..period].iter().sum();
    out[period - 1] = Some(sum / period as f64);
    for i in period..n {
        sum += prices[i] - prices[i - period];
        out[i] = Some(sum / period as f64);
    }
    out
}

/// Exponential moving average. Seed = SMA of first `period` bars.
pub fn ema(prices: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    if period == 0 || period > n { return out; }
    let k = 2.0 / (period as f64 + 1.0);
    let seed: f64 = prices[..period].iter().sum::<f64>() / period as f64;
    let mut prev = seed;
    out[period - 1] = Some(prev);
    for i in period..n {
        prev = prices[i] * k + prev * (1.0 - k);
        out[i] = Some(prev);
    }
    out
}

/// Weighted moving average (linearly weighted, newest has highest weight).
pub fn wma(prices: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    if period == 0 || period > n { return out; }
    let denom: f64 = (period * (period + 1) / 2) as f64;
    for i in (period - 1)..n {
        let val: f64 = (0..period)
            .map(|j| prices[i - period + 1 + j] * (j + 1) as f64)
            .sum::<f64>() / denom;
        out[i] = Some(val);
    }
    out
}

/// Double EMA: DEMA = 2*EMA(p,n) - EMA(EMA(p,n),n).
pub fn dema(prices: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let e1 = ema(prices, period);
    let e1_vals: Vec<f64> = e1.iter().map(|v| v.unwrap_or(f64::NAN)).collect();
    let e2 = ema(&e1_vals, period);
    e1.iter().zip(e2.iter()).map(|(a, b)| match (a, b) {
        (Some(x), Some(y)) if y.is_finite() => Some(2.0 * x - y),
        _ => None,
    }).collect()
}

/// Triple EMA: TEMA = 3*EMA - 3*EMA(EMA) + EMA(EMA(EMA)).
pub fn tema(prices: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let e1 = ema(prices, period);
    let e1_vals: Vec<f64> = e1.iter().map(|v| v.unwrap_or(f64::NAN)).collect();
    let e2 = ema(&e1_vals, period);
    let e2_vals: Vec<f64> = e2.iter().map(|v| v.unwrap_or(f64::NAN)).collect();
    let e3 = ema(&e2_vals, period);
    (0..n).map(|i| match (e1[i], e2[i], e3[i]) {
        (Some(a), Some(b), Some(c)) if a.is_finite() && b.is_finite() && c.is_finite() =>
            Some(3.0 * a - 3.0 * b + c),
        _ => None,
    }).collect()
}

/// Hull Moving Average: HMA(n) = WMA(2*WMA(n/2) - WMA(n), sqrt(n)).
pub fn hma(prices: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let half = (period / 2).max(1);
    let sqrt_p = (period as f64).sqrt().round() as usize;
    let w1 = wma(prices, half);
    let w2 = wma(prices, period);
    let diff: Vec<f64> = w1.iter().zip(w2.iter()).map(|(a, b)| match (a, b) {
        (Some(x), Some(y)) => 2.0 * x - y,
        _ => f64::NAN,
    }).collect();
    let h = wma(&diff, sqrt_p);
    h.iter().map(|v| if v.map(|x| x.is_finite()).unwrap_or(false) { *v } else { None }).collect()
}

/// Kaufman Adaptive Moving Average (KAMA).
pub fn kama(prices: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    if period >= n { return out; }
    let fast_k = 2.0 / 3.0;   // fast EMA constant
    let slow_k = 2.0 / 31.0;  // slow EMA constant
    let mut k_val = prices[period];
    out[period] = Some(k_val);
    for i in (period + 1)..n {
        let direction = (prices[i] - prices[i - period]).abs();
        let volatility: f64 = (0..period).map(|j| (prices[i - j] - prices[i - j - 1]).abs()).sum();
        let er = if volatility > 1e-12 { direction / volatility } else { 0.0 };
        let sc = (er * (fast_k - slow_k) + slow_k).powi(2);
        k_val = k_val + sc * (prices[i] - k_val);
        out[i] = Some(k_val);
    }
    out
}

/// Golden cross signal: returns Some(1.0) on bar where fast EMA crosses above slow EMA, else Some(0.0).
pub fn golden_cross(prices: &[f64], fast: usize, slow: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let fast_ema = ema(prices, fast);
    let slow_ema = ema(prices, slow);
    (0..n).map(|i| {
        if i == 0 { return None; }
        match (fast_ema[i], slow_ema[i], fast_ema[i-1], slow_ema[i-1]) {
            (Some(f), Some(s), Some(pf), Some(ps)) =>
                Some(if f > s && pf <= ps { 1.0 } else { 0.0 }),
            _ => None,
        }
    }).collect()
}

/// Death cross signal: returns Some(1.0) on bar where fast EMA crosses below slow EMA.
pub fn death_cross(prices: &[f64], fast: usize, slow: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let fast_ema = ema(prices, fast);
    let slow_ema = ema(prices, slow);
    (0..n).map(|i| {
        if i == 0 { return None; }
        match (fast_ema[i], slow_ema[i], fast_ema[i-1], slow_ema[i-1]) {
            (Some(f), Some(s), Some(pf), Some(ps)) =>
                Some(if f < s && pf >= ps { 1.0 } else { 0.0 }),
            _ => None,
        }
    }).collect()
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test sma_basic ema_converges golden_cross_signal 2>&1 | tail -5
```

Expected: `test result: ok. 3 passed`

- [ ] **Step 5: Commit**

```bash
git add src/analysis/techind.rs tests/techind_test.rs
git commit -m "feat: moving averages — sma, ema, wma, dema, tema, hma, kama, golden/death cross"
```

---

### Task 4: Trend Strength Indicators (9 indicators)

Indicators: `adx_14` (ADX, +DI, -DI), `aroon_25` (up/down), `vortex_14` (VI+, VI-), `choppiness_14`, `supertrend_14`.

**Files:**
- Modify: `src/analysis/techind.rs`
- Test: `tests/techind_test.rs`

- [ ] **Step 1: Write the failing test**

```rust
use orderflow_rs::analysis::techind::{atr, adx, aroon_up, aroon_down, choppiness};

#[test]
fn atr_positive() {
    let h = vec![1.01, 1.02, 1.03, 1.04, 1.05, 1.06, 1.07, 1.08, 1.09, 1.10,
                 1.11, 1.12, 1.13, 1.14, 1.15];
    let l = h.iter().map(|x| x - 0.005).collect::<Vec<_>>();
    let c = h.iter().map(|x| x - 0.002).collect::<Vec<_>>();
    let result = atr(&h, &l, &c, 14);
    // Last value should be positive
    assert!(result[14].unwrap() > 0.0);
}

#[test]
fn adx_range() {
    let n = 60;
    let h: Vec<f64> = (0..n).map(|i| 1.0 + i as f64 * 0.01).collect();
    let l: Vec<f64> = h.iter().map(|x| x - 0.005).collect();
    let c: Vec<f64> = h.iter().map(|x| x - 0.002).collect();
    let result = adx(&h, &l, &c, 14);
    for v in result.iter().filter_map(|x| *x) {
        assert!(v >= 0.0 && v <= 100.0, "ADX out of [0,100]: {}", v);
    }
}

#[test]
fn aroon_range() {
    let n = 40;
    let h: Vec<f64> = (0..n).map(|i| 1.0 + (i as f64 * 0.7).sin() * 0.1).collect();
    let l: Vec<f64> = h.iter().map(|x| x - 0.005).collect();
    let up = aroon_up(&h, 25);
    let dn = aroon_down(&l, 25);
    for v in up.iter().chain(dn.iter()).filter_map(|x| *x) {
        assert!(v >= 0.0 && v <= 100.0, "Aroon out of [0,100]");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test atr_positive adx_range aroon_range 2>&1 | tail -5
```

Expected: error — functions not found.

- [ ] **Step 3: Implement trend strength in `src/analysis/techind.rs`**

```rust
/// True Range (prerequisite for ATR/ADX).
fn true_range(highs: &[f64], lows: &[f64], closes: &[f64]) -> Vec<f64> {
    let n = highs.len();
    let mut tr = vec![0.0; n];
    tr[0] = highs[0] - lows[0];
    for i in 1..n {
        tr[i] = (highs[i] - lows[i])
            .max((highs[i] - closes[i-1]).abs())
            .max((lows[i] - closes[i-1]).abs());
    }
    tr
}

/// Wilder-smoothed ATR.
pub fn atr(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = highs.len();
    let mut out = vec![None; n];
    if period >= n { return out; }
    let tr = true_range(highs, lows, closes);
    let seed: f64 = tr[..period].iter().sum::<f64>() / period as f64;
    let mut prev = seed;
    out[period - 1] = Some(prev);
    for i in period..n {
        prev = (prev * (period as f64 - 1.0) + tr[i]) / period as f64;
        out[i] = Some(prev);
    }
    out
}

/// Average Directional Index (ADX) with +DI and -DI.
/// Returns (adx, plus_di, minus_di) as parallel vecs.
pub fn adx_full(highs: &[f64], lows: &[f64], closes: &[f64], period: usize)
    -> (Vec<Option<f64>>, Vec<Option<f64>>, Vec<Option<f64>>)
{
    let n = highs.len();
    let mut adx_v = vec![None; n];
    let mut pdi_v = vec![None; n];
    let mut mdi_v = vec![None; n];
    if period + 1 >= n { return (adx_v, pdi_v, mdi_v); }

    let tr = true_range(highs, lows, closes);
    let mut dm_plus = vec![0.0_f64; n];
    let mut dm_minus = vec![0.0_f64; n];
    for i in 1..n {
        let up = highs[i] - highs[i-1];
        let dn = lows[i-1] - lows[i];
        dm_plus[i]  = if up > dn && up > 0.0 { up } else { 0.0 };
        dm_minus[i] = if dn > up && dn > 0.0 { dn } else { 0.0 };
    }

    // Wilder smoothing seed
    let mut s_tr   = tr[1..=period].iter().sum::<f64>();
    let mut s_plus = dm_plus[1..=period].iter().sum::<f64>();
    let mut s_minus= dm_minus[1..=period].iter().sum::<f64>();
    let alpha = 1.0 / period as f64;

    let di_plus  = |sp: f64, st: f64| if st > 0.0 { 100.0 * sp / st } else { 0.0 };
    let di_minus = |sm: f64, st: f64| if st > 0.0 { 100.0 * sm / st } else { 0.0 };

    let pdi_seed = di_plus(s_plus, s_tr);
    let mdi_seed = di_minus(s_minus, s_tr);
    pdi_v[period] = Some(pdi_seed);
    mdi_v[period] = Some(mdi_seed);
    let dx = if (pdi_seed + mdi_seed) > 0.0 {
        100.0 * (pdi_seed - mdi_seed).abs() / (pdi_seed + mdi_seed) } else { 0.0 };
    let mut dx_sum = dx;
    let mut dx_count = 1usize;

    for i in (period+1)..n {
        s_tr    = s_tr    * (1.0 - alpha) + tr[i];
        s_plus  = s_plus  * (1.0 - alpha) + dm_plus[i];
        s_minus = s_minus * (1.0 - alpha) + dm_minus[i];
        let pdi = di_plus(s_plus, s_tr);
        let mdi = di_minus(s_minus, s_tr);
        pdi_v[i] = Some(pdi);
        mdi_v[i] = Some(mdi);
        let denom = pdi + mdi;
        let dx_i = if denom > 0.0 { 100.0 * (pdi - mdi).abs() / denom } else { 0.0 };
        dx_sum += dx_i;
        dx_count += 1;
        if dx_count >= period {
            adx_v[i] = Some(dx_sum / dx_count as f64);
            dx_sum -= if dx_count == period { dx_sum / period as f64 } else { 0.0 };
        }
    }
    (adx_v, pdi_v, mdi_v)
}

/// ADX only.
pub fn adx(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> Vec<Option<f64>> {
    adx_full(highs, lows, closes, period).0
}

/// Aroon Up: 100 * (period - bars_since_highest_high) / period.
pub fn aroon_up(highs: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = highs.len();
    let mut out = vec![None; n];
    for i in period..n {
        let window = &highs[(i - period)..=i];
        let max_pos = window.iter().enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).map(|(j,_)| j).unwrap();
        out[i] = Some(100.0 * max_pos as f64 / period as f64);
    }
    out
}

/// Aroon Down: 100 * (period - bars_since_lowest_low) / period.
pub fn aroon_down(lows: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = lows.len();
    let mut out = vec![None; n];
    for i in period..n {
        let window = &lows[(i - period)..=i];
        let min_pos = window.iter().enumerate()
            .min_by(|a, b| a.1.partial_cmp(b.1).unwrap()).map(|(j,_)| j).unwrap();
        out[i] = Some(100.0 * min_pos as f64 / period as f64);
    }
    out
}

/// Choppiness Index: 100 * log10(ATR_sum / (highest_high - lowest_low)) / log10(period).
pub fn choppiness(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = highs.len();
    let mut out = vec![None; n];
    if period >= n { return out; }
    let tr = true_range(highs, lows, closes);
    let log_p = (period as f64).log10();
    for i in period..n {
        let atr_sum: f64 = tr[(i - period + 1)..=i].iter().sum();
        let hh = highs[(i - period + 1)..=i].iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let ll = lows[(i - period + 1)..=i].iter().cloned().fold(f64::INFINITY, f64::min);
        let range = hh - ll;
        if range > 1e-12 && log_p > 0.0 {
            out[i] = Some(100.0 * (atr_sum / range).log10() / log_p);
        }
    }
    out
}

/// Supertrend (ATR multiplier = 3.0 default).
pub fn supertrend(highs: &[f64], lows: &[f64], closes: &[f64], period: usize, multiplier: f64)
    -> Vec<Option<f64>>
{
    let n = highs.len();
    let mut out = vec![None; n];
    let atr_v = atr(highs, lows, closes, period);
    let mut trend = 1.0_f64; // 1 = uptrend, -1 = downtrend
    let mut upper = 0.0_f64;
    let mut lower = 0.0_f64;
    for i in period..n {
        if let Some(atr_val) = atr_v[i] {
            let hl2 = (highs[i] + lows[i]) / 2.0;
            let basic_upper = hl2 + multiplier * atr_val;
            let basic_lower = hl2 - multiplier * atr_val;
            if i == period {
                upper = basic_upper;
                lower = basic_lower;
            } else {
                upper = if basic_upper < upper || closes[i-1] > upper { basic_upper } else { upper };
                lower = if basic_lower > lower || closes[i-1] < lower { basic_lower } else { lower };
            }
            if closes[i] <= upper && closes[i] >= lower {
                // hold trend
            } else if closes[i] > upper {
                trend = 1.0;
            } else {
                trend = -1.0;
            }
            out[i] = Some(trend);
        }
    }
    out
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test atr_positive adx_range aroon_range 2>&1 | tail -5
```

Expected: `test result: ok. 3 passed`

- [ ] **Step 5: Commit**

```bash
git add src/analysis/techind.rs tests/techind_test.rs
git commit -m "feat: trend strength — ATR, ADX, Aroon, Choppiness, Supertrend"
```

---

### Task 5: Momentum Oscillators (20 indicators)

Indicators: `rsi_N` (7/14/21), `stoch_k_14`, `stoch_d_14`, `macd_line`, `macd_signal`, `macd_hist`, `cci_20`, `williams_r_14`, `roc_10`, `momentum_10`, `cmo_14`, `dpo_20`, `awesome_osc`, `rvi_14`.

**Files:**
- Modify: `src/analysis/techind.rs`
- Test: `tests/techind_test.rs`

- [ ] **Step 1: Write the failing test**

```rust
use orderflow_rs::analysis::techind::{rsi, macd_components, cci, williams_r, stoch_k};

#[test]
fn rsi_range() {
    let n = 30;
    let p: Vec<f64> = (0..n).map(|i| 10.0 + (i as f64 * 0.5).sin()).collect();
    let result = rsi(&p, 14);
    for v in result.iter().filter_map(|x| *x) {
        assert!(v >= 0.0 && v <= 100.0, "RSI out of [0,100]: {v}");
    }
}

#[test]
fn macd_components_sign() {
    // Strongly rising prices → MACD line positive
    let p: Vec<f64> = (0..100).map(|i| 1.0 + i as f64 * 0.01).collect();
    let (line, signal, hist) = macd_components(&p, 12, 26, 9);
    let last_line = line.iter().rev().find_map(|x| *x).unwrap();
    assert!(last_line > 0.0, "MACD line should be positive for rising prices");
}

#[test]
fn cci_varies() {
    let n = 30;
    let h: Vec<f64> = (0..n).map(|i| 1.0 + (i as f64 * 0.3).sin() * 0.1).collect();
    let l: Vec<f64> = h.iter().map(|x| x - 0.005).collect();
    let c: Vec<f64> = h.iter().map(|x| x - 0.002).collect();
    let result = cci(&h, &l, &c, 20);
    let vals: Vec<f64> = result.iter().filter_map(|x| *x).collect();
    assert!(!vals.is_empty());
    // CCI should have variation
    let range = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
              - vals.iter().cloned().fold(f64::INFINITY, f64::min);
    assert!(range > 0.1, "CCI should vary: range={range}");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test rsi_range macd_components_sign cci_varies 2>&1 | tail -5
```

Expected: error — functions not found.

- [ ] **Step 3: Implement momentum oscillators in `src/analysis/techind.rs`**

```rust
/// Wilder RSI.
pub fn rsi(prices: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    if period >= n { return out; }
    let mut avg_gain = 0.0_f64;
    let mut avg_loss = 0.0_f64;
    for i in 1..=period {
        let d = prices[i] - prices[i-1];
        if d > 0.0 { avg_gain += d; } else { avg_loss -= d; }
    }
    avg_gain /= period as f64;
    avg_loss /= period as f64;
    let rs = if avg_loss > 1e-12 { avg_gain / avg_loss } else { 100.0 };
    out[period] = Some(100.0 - 100.0 / (1.0 + rs));
    for i in (period+1)..n {
        let d = prices[i] - prices[i-1];
        let gain = if d > 0.0 { d } else { 0.0 };
        let loss = if d < 0.0 { -d } else { 0.0 };
        avg_gain = (avg_gain * (period as f64 - 1.0) + gain) / period as f64;
        avg_loss = (avg_loss * (period as f64 - 1.0) + loss) / period as f64;
        let rs2 = if avg_loss > 1e-12 { avg_gain / avg_loss } else { 100.0 };
        out[i] = Some(100.0 - 100.0 / (1.0 + rs2));
    }
    out
}

/// MACD: returns (line, signal, histogram).
pub fn macd_components(prices: &[f64], fast: usize, slow: usize, signal_p: usize)
    -> (Vec<Option<f64>>, Vec<Option<f64>>, Vec<Option<f64>>)
{
    let n = prices.len();
    let ema_fast = ema(prices, fast);
    let ema_slow = ema(prices, slow);
    let line: Vec<Option<f64>> = (0..n).map(|i| match (ema_fast[i], ema_slow[i]) {
        (Some(f), Some(s)) => Some(f - s),
        _ => None,
    }).collect();
    // Build input for signal EMA from MACD line values
    let line_vals: Vec<f64> = line.iter().map(|v| v.unwrap_or(f64::NAN)).collect();
    let signal = ema(&line_vals, signal_p);
    let hist: Vec<Option<f64>> = (0..n).map(|i| match (line[i], signal[i]) {
        (Some(l), Some(s)) if s.is_finite() => Some(l - s),
        _ => None,
    }).collect();
    (line, signal, hist)
}

/// Commodity Channel Index.
pub fn cci(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = highs.len();
    let mut out = vec![None; n];
    if period > n { return out; }
    let tp: Vec<f64> = (0..n).map(|i| (highs[i] + lows[i] + closes[i]) / 3.0).collect();
    for i in (period-1)..n {
        let window = &tp[(i+1-period)..=i];
        let mean: f64 = window.iter().sum::<f64>() / period as f64;
        let mad: f64 = window.iter().map(|x| (x - mean).abs()).sum::<f64>() / period as f64;
        if mad > 1e-12 {
            out[i] = Some((tp[i] - mean) / (0.015 * mad));
        }
    }
    out
}

/// Williams %R.
pub fn williams_r(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = highs.len();
    let mut out = vec![None; n];
    for i in (period-1)..n {
        let hh = highs[(i+1-period)..=i].iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let ll = lows[(i+1-period)..=i].iter().cloned().fold(f64::INFINITY, f64::min);
        let range = hh - ll;
        if range > 1e-12 {
            out[i] = Some(-100.0 * (hh - closes[i]) / range);
        }
    }
    out
}

/// Stochastic %K.
pub fn stoch_k(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = highs.len();
    let mut out = vec![None; n];
    for i in (period-1)..n {
        let hh = highs[(i+1-period)..=i].iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let ll = lows[(i+1-period)..=i].iter().cloned().fold(f64::INFINITY, f64::min);
        let range = hh - ll;
        if range > 1e-12 {
            out[i] = Some(100.0 * (closes[i] - ll) / range);
        }
    }
    out
}

/// Stochastic %D = SMA(K, 3).
pub fn stoch_d(highs: &[f64], lows: &[f64], closes: &[f64], k_period: usize) -> Vec<Option<f64>> {
    let k = stoch_k(highs, lows, closes, k_period);
    let k_vals: Vec<f64> = k.iter().map(|v| v.unwrap_or(f64::NAN)).collect();
    let d = sma(&k_vals, 3);
    d.iter().map(|v| if v.map(|x| x.is_finite()).unwrap_or(false) { *v } else { None }).collect()
}

/// Rate of Change: 100 * (price[i] - price[i-n]) / price[i-n].
pub fn roc(prices: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    for i in period..n {
        if prices[i - period].abs() > 1e-12 {
            out[i] = Some(100.0 * (prices[i] - prices[i - period]) / prices[i - period]);
        }
    }
    out
}

/// Raw momentum: price[i] - price[i-period].
pub fn momentum(prices: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    for i in period..n {
        out[i] = Some(prices[i] - prices[i - period]);
    }
    out
}

/// Chande Momentum Oscillator.
pub fn cmo(prices: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    if period >= n { return out; }
    for i in period..n {
        let mut su = 0.0_f64;
        let mut sd = 0.0_f64;
        for j in (i-period+1)..=i {
            let d = prices[j] - prices[j-1];
            if d > 0.0 { su += d; } else { sd -= d; }
        }
        let denom = su + sd;
        if denom > 1e-12 {
            out[i] = Some(100.0 * (su - sd) / denom);
        }
    }
    out
}

/// Detrended Price Oscillator: close - SMA(close, period) shifted back period/2 bars.
pub fn dpo(prices: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    let shift = period / 2 + 1;
    let sma_v = sma(prices, period);
    for i in (period + shift - 1)..n {
        if let Some(s) = sma_v[i - shift] {
            out[i] = Some(prices[i] - s);
        }
    }
    out
}

/// Awesome Oscillator: SMA(midpoint, 5) - SMA(midpoint, 34).
pub fn awesome_osc(highs: &[f64], lows: &[f64]) -> Vec<Option<f64>> {
    let n = highs.len();
    let mid: Vec<f64> = (0..n).map(|i| (highs[i] + lows[i]) / 2.0).collect();
    let fast = sma(&mid, 5);
    let slow = sma(&mid, 34);
    (0..n).map(|i| match (fast[i], slow[i]) {
        (Some(f), Some(s)) => Some(f - s),
        _ => None,
    }).collect()
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test rsi_range macd_components_sign cci_varies 2>&1 | tail -5
```

Expected: `test result: ok. 3 passed`

- [ ] **Step 5: Run full test suite to catch regressions**

```bash
cargo test 2>&1 | tail -8
```

Expected: all prior tests still pass.

- [ ] **Step 6: Commit**

```bash
git add src/analysis/techind.rs tests/techind_test.rs
git commit -m "feat: momentum oscillators — RSI, MACD, CCI, Williams%%R, Stoch, ROC, CMO, DPO, AO"
```

---

### Task 6: Volatility Indicators (10 indicators)

Indicators: `bb_width_20`, `bb_pct_b_20`, `keltner_width_20`, `donchian_width_20`, `natr_14`, `realized_vol_20`, `parkinson_vol_14`, `garman_klass_vol_20`, `squeeze_signal`.

**Files:**
- Modify: `src/analysis/techind.rs`
- Test: `tests/techind_test.rs`

- [ ] **Step 1: Write the failing test**

```rust
use orderflow_rs::analysis::techind::{bb_width, bb_pct_b, donchian_width, parkinson_vol, realized_vol};

#[test]
fn bb_width_positive() {
    let n = 30;
    let p: Vec<f64> = (0..n).map(|i| 1.0 + (i as f64 * 0.3).sin() * 0.05).collect();
    let w = bb_width(&p, 20, 2.0);
    let last = w[29].unwrap();
    assert!(last > 0.0, "BB width should be positive: {last}");
}

#[test]
fn bb_pct_b_range() {
    let n = 30;
    let p: Vec<f64> = (0..n).map(|i| 1.0 + i as f64 * 0.001).collect();
    let pct = bb_pct_b(&p, 20, 2.0);
    for v in pct.iter().filter_map(|x| *x) {
        // In a pure trend, %b can exceed [0,1] but should be finite
        assert!(v.is_finite(), "%b should be finite: {v}");
    }
}

#[test]
fn realized_vol_positive() {
    let n = 30;
    let p: Vec<f64> = (0..n).map(|i| 1.0 + i as f64 * 0.001 + (i as f64 * 0.7).sin() * 0.002).collect();
    let rv = realized_vol(&p, 20);
    let last = rv[29].unwrap();
    assert!(last > 0.0, "realized vol should be positive");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test bb_width_positive bb_pct_b_range realized_vol_positive 2>&1 | tail -5
```

Expected: error — functions not found.

- [ ] **Step 3: Implement volatility indicators in `src/analysis/techind.rs`**

```rust
/// Population std dev of a slice.
fn std_dev(data: &[f64]) -> f64 {
    let n = data.len() as f64;
    let mean = data.iter().sum::<f64>() / n;
    (data.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n).sqrt()
}

/// Bollinger Band width: (upper - lower) / middle.
pub fn bb_width(prices: &[f64], period: usize, num_std: f64) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    for i in (period-1)..n {
        let w = &prices[(i+1-period)..=i];
        let mean = w.iter().sum::<f64>() / period as f64;
        let sd = std_dev(w);
        if mean > 1e-12 {
            out[i] = Some(2.0 * num_std * sd / mean);
        }
    }
    out
}

/// Bollinger Band %b: (price - lower) / (upper - lower).
pub fn bb_pct_b(prices: &[f64], period: usize, num_std: f64) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    for i in (period-1)..n {
        let w = &prices[(i+1-period)..=i];
        let mean = w.iter().sum::<f64>() / period as f64;
        let sd = std_dev(w);
        let upper = mean + num_std * sd;
        let lower = mean - num_std * sd;
        let range = upper - lower;
        if range > 1e-12 {
            out[i] = Some((prices[i] - lower) / range);
        }
    }
    out
}

/// Keltner Channel width: (upper - lower) / EMA.
pub fn keltner_width(highs: &[f64], lows: &[f64], closes: &[f64], period: usize, mult: f64)
    -> Vec<Option<f64>>
{
    let n = highs.len();
    let ema_v = ema(closes, period);
    let atr_v = atr(highs, lows, closes, period);
    (0..n).map(|i| match (ema_v[i], atr_v[i]) {
        (Some(e), Some(a)) if e > 1e-12 => Some(2.0 * mult * a / e),
        _ => None,
    }).collect()
}

/// Donchian Channel width: (highest_high - lowest_low) / mid.
pub fn donchian_width(highs: &[f64], lows: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = highs.len();
    let mut out = vec![None; n];
    for i in (period-1)..n {
        let hh = highs[(i+1-period)..=i].iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let ll = lows[(i+1-period)..=i].iter().cloned().fold(f64::INFINITY, f64::min);
        let mid = (hh + ll) / 2.0;
        if mid > 1e-12 {
            out[i] = Some((hh - ll) / mid);
        }
    }
    out
}

/// Normalized ATR: ATR / close * 100.
pub fn natr(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> Vec<Option<f64>> {
    let atr_v = atr(highs, lows, closes, period);
    atr_v.iter().zip(closes.iter()).map(|(a, c)| {
        match a {
            Some(av) if *c > 1e-12 => Some(av / c * 100.0),
            _ => None,
        }
    }).collect()
}

/// Realized volatility (std dev of log returns over `period` bars), annualized × sqrt(252*390).
pub fn realized_vol(prices: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    if period >= n { return out; }
    let log_ret: Vec<f64> = (1..n).map(|i|
        if prices[i-1] > 1e-12 { (prices[i] / prices[i-1]).ln() } else { 0.0 }
    ).collect();
    for i in period..n {
        let w = &log_ret[(i-period)..(i)];
        let sd = std_dev(w);
        out[i] = Some(sd);
    }
    out
}

/// Parkinson volatility: sqrt(1/(4*ln2) * mean((ln(H/L))^2)).
pub fn parkinson_vol(highs: &[f64], lows: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = highs.len();
    let mut out = vec![None; n];
    let factor = 1.0 / (4.0 * 2.0_f64.ln());
    for i in (period-1)..n {
        let sum: f64 = (0..period).map(|j| {
            let idx = i + 1 - period + j;
            let ratio = highs[idx] / lows[idx].max(1e-12);
            ratio.ln().powi(2)
        }).sum();
        out[i] = Some((factor * sum / period as f64).sqrt());
    }
    out
}

/// Garman-Klass volatility estimator.
pub fn garman_klass_vol(highs: &[f64], lows: &[f64], opens: &[f64], closes: &[f64], period: usize)
    -> Vec<Option<f64>>
{
    let n = highs.len();
    let mut out = vec![None; n];
    for i in (period-1)..n {
        let sum: f64 = (0..period).map(|j| {
            let k = i + 1 - period + j;
            let hl = (highs[k] / lows[k].max(1e-12)).ln().powi(2) * 0.5;
            let co = (closes[k] / opens[k].max(1e-12)).ln().powi(2) * (2.0 * 2.0_f64.ln() - 1.0);
            hl - co
        }).sum::<f64>();
        out[i] = Some((sum / period as f64).max(0.0).sqrt());
    }
    out
}

/// Squeeze signal: BB width < Keltner width → Some(1.0) (squeeze on), else Some(0.0).
pub fn squeeze_signal(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = highs.len();
    let bb = bb_width(closes, period, 2.0);
    let kc = keltner_width(highs, lows, closes, period, 1.5);
    (0..n).map(|i| match (bb[i], kc[i]) {
        (Some(b), Some(k)) => Some(if b < k { 1.0 } else { 0.0 }),
        _ => None,
    }).collect()
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test bb_width_positive bb_pct_b_range realized_vol_positive 2>&1 | tail -5
```

Expected: `test result: ok. 3 passed`

- [ ] **Step 5: Commit**

```bash
git add src/analysis/techind.rs tests/techind_test.rs
git commit -m "feat: volatility — BB, Keltner, Donchian, NATR, realized/Parkinson/GK vol, squeeze"
```

---

### Task 7: Volume Indicators (8 indicators)

Indicators: `vwap`, `obv`, `mfi_14`, `force_index_13`, `ease_of_movement_14`, `vpt`, `vroc_14`, `rvol_20`.

**Files:**
- Modify: `src/analysis/techind.rs`
- Test: `tests/techind_test.rs`

- [ ] **Step 1: Write the failing test**

```rust
use orderflow_rs::analysis::techind::{obv, mfi, vwap, rvol};

#[test]
fn obv_monotone_rising() {
    // All bars up → OBV should increase each bar
    let n = 10;
    let closes: Vec<f64> = (0..n).map(|i| 1.0 + i as f64 * 0.01).collect();
    let vols: Vec<f64> = vec![100.0; n];
    let result = obv(&closes, &vols);
    for i in 1..n {
        assert!(result[i] > result[i-1], "OBV should rise on up-close");
    }
}

#[test]
fn mfi_range() {
    let n = 20;
    let h: Vec<f64> = (0..n).map(|i| 1.0 + (i as f64 * 0.5).sin() * 0.1).collect();
    let l: Vec<f64> = h.iter().map(|x| x - 0.005).collect();
    let c: Vec<f64> = h.iter().map(|x| x - 0.002).collect();
    let vol: Vec<f64> = vec![100.0; n];
    let result = mfi(&h, &l, &c, &vol, 14);
    for v in result.iter().filter_map(|x| *x) {
        assert!(v >= 0.0 && v <= 100.0, "MFI out of [0,100]: {v}");
    }
}

#[test]
fn rvol_positive() {
    let n = 30;
    let vol: Vec<f64> = (0..n).map(|i| 100.0 + i as f64 * 10.0).collect();
    let result = rvol(&vol, 20);
    for v in result.iter().skip(20).filter_map(|x| *x) {
        assert!(v > 0.0, "RVOL should be positive");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test obv_monotone_rising mfi_range rvol_positive 2>&1 | tail -5
```

Expected: error — functions not found.

- [ ] **Step 3: Implement volume indicators in `src/analysis/techind.rs`**

```rust
/// On-Balance Volume.
pub fn obv(closes: &[f64], volumes: &[f64]) -> Vec<f64> {
    let n = closes.len();
    let mut out = vec![0.0_f64; n];
    for i in 1..n {
        out[i] = out[i-1] + if closes[i] > closes[i-1] { volumes[i] }
                             else if closes[i] < closes[i-1] { -volumes[i] }
                             else { 0.0 };
    }
    out
}

/// Volume-Weighted Average Price (running from bar 0).
pub fn vwap(highs: &[f64], lows: &[f64], closes: &[f64], volumes: &[f64]) -> Vec<f64> {
    let n = highs.len();
    let mut cum_tv = 0.0_f64;
    let mut cum_v  = 0.0_f64;
    let mut out = vec![0.0_f64; n];
    for i in 0..n {
        let tp = (highs[i] + lows[i] + closes[i]) / 3.0;
        cum_tv += tp * volumes[i];
        cum_v  += volumes[i];
        out[i] = if cum_v > 1e-12 { cum_tv / cum_v } else { closes[i] };
    }
    out
}

/// Money Flow Index (volume-weighted RSI of typical price).
pub fn mfi(highs: &[f64], lows: &[f64], closes: &[f64], volumes: &[f64], period: usize)
    -> Vec<Option<f64>>
{
    let n = highs.len();
    let mut out = vec![None; n];
    if period >= n { return out; }
    let tp: Vec<f64> = (0..n).map(|i| (highs[i] + lows[i] + closes[i]) / 3.0).collect();
    for i in period..n {
        let mut pos_mf = 0.0_f64;
        let mut neg_mf = 0.0_f64;
        for j in (i - period + 1)..=i {
            let mf = tp[j] * volumes[j];
            if tp[j] >= tp[j-1] { pos_mf += mf; } else { neg_mf += mf; }
        }
        let denom = pos_mf + neg_mf;
        if denom > 1e-12 {
            out[i] = Some(100.0 * pos_mf / denom);
        }
    }
    out
}

/// Force Index: EMA of (close - prev_close) * volume.
pub fn force_index(closes: &[f64], volumes: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = closes.len();
    let fi: Vec<f64> = (0..n).map(|i| {
        if i == 0 { 0.0 } else { (closes[i] - closes[i-1]) * volumes[i] }
    }).collect();
    ema(&fi, period)
}

/// Ease of Movement: normalized ratio of price movement to volume.
pub fn ease_of_movement(highs: &[f64], lows: &[f64], volumes: &[f64], period: usize)
    -> Vec<Option<f64>>
{
    let n = highs.len();
    let mut raw = vec![0.0_f64; n];
    for i in 1..n {
        let mid_move = (highs[i] + lows[i]) / 2.0 - (highs[i-1] + lows[i-1]) / 2.0;
        let hl = highs[i] - lows[i];
        let vol_norm = if volumes[i] > 1e-12 { volumes[i] / 1_000_000.0 } else { 1.0 };
        raw[i] = if hl.abs() > 1e-12 { mid_move / (vol_norm * hl) } else { 0.0 };
    }
    sma(&raw, period)
}

/// Volume Price Trend: cumulative (close_change / prev_close) * volume.
pub fn vpt(closes: &[f64], volumes: &[f64]) -> Vec<f64> {
    let n = closes.len();
    let mut out = vec![0.0_f64; n];
    for i in 1..n {
        let ret = if closes[i-1] > 1e-12 { (closes[i] - closes[i-1]) / closes[i-1] } else { 0.0 };
        out[i] = out[i-1] + ret * volumes[i];
    }
    out
}

/// Volume Rate of Change.
pub fn vroc(volumes: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = volumes.len();
    let mut out = vec![None; n];
    for i in period..n {
        if volumes[i - period] > 1e-12 {
            out[i] = Some(100.0 * (volumes[i] - volumes[i - period]) / volumes[i - period]);
        }
    }
    out
}

/// Relative Volume: current volume / SMA(volume, period).
pub fn rvol(volumes: &[f64], period: usize) -> Vec<Option<f64>> {
    let sma_v = sma(volumes, period);
    volumes.iter().zip(sma_v.iter()).map(|(v, s)| match s {
        Some(sv) if *sv > 1e-12 => Some(v / sv),
        _ => None,
    }).collect()
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test obv_monotone_rising mfi_range rvol_positive 2>&1 | tail -5
```

Expected: `test result: ok. 3 passed`

- [ ] **Step 5: Commit**

```bash
git add src/analysis/techind.rs tests/techind_test.rs
git commit -m "feat: volume indicators — OBV, VWAP, MFI, Force Index, EOM, VPT, VROC, RVOL"
```

---

### Task 8: Support/Resistance + Statistical Indicators (18 indicators)

Indicators: `dist_high_20`, `dist_low_20`, `pivot_dist`, `r1_dist`, `s1_dist`, `fib_618_dist`, `round_number_prox`, `hurst_100`, `zscore_20`, `lr_slope_20`, `lr_r2_20`, `lr_deviation`, `autocorr_lag1`, `shannon_entropy_50`, `variance_ratio_16`.

**Files:**
- Modify: `src/analysis/techind.rs`
- Test: `tests/techind_test.rs`

- [ ] **Step 1: Write the failing test**

```rust
use orderflow_rs::analysis::techind::{dist_high, dist_low, pivot_dist, zscore, lr_slope, autocorr};

#[test]
fn dist_high_non_negative() {
    let h: Vec<f64> = vec![1.01, 1.02, 1.03, 1.04, 1.05, 1.03];
    let c: Vec<f64> = vec![1.00, 1.01, 1.02, 1.03, 1.04, 1.02];
    let result = dist_high(&h, &c, 5);
    for v in result.iter().filter_map(|x| *x) {
        assert!(v >= 0.0, "dist_high should be non-negative");
    }
}

#[test]
fn zscore_near_zero_mean() {
    let p: Vec<f64> = vec![1.0; 30];
    let result = zscore(&p, 20);
    for v in result.iter().skip(19).filter_map(|x| *x) {
        assert!(v.abs() < 1e-6, "z-score of constant series = 0: {v}");
    }
}

#[test]
fn lr_slope_rising() {
    let p: Vec<f64> = (0..30).map(|i| 1.0 + i as f64 * 0.01).collect();
    let result = lr_slope(&p, 20);
    let last = result[29].unwrap();
    assert!(last > 0.0, "LR slope should be positive for rising prices: {last}");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test dist_high_non_negative zscore_near_zero_mean lr_slope_rising 2>&1 | tail -5
```

Expected: error — functions not found.

- [ ] **Step 3: Implement support/resistance and statistical indicators in `src/analysis/techind.rs`**

```rust
/// Distance from current close to highest high over `period` bars (as fraction of close).
pub fn dist_high(highs: &[f64], closes: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = highs.len();
    let mut out = vec![None; n];
    for i in (period-1)..n {
        let hh = highs[(i+1-period)..=i].iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        if closes[i] > 1e-12 {
            out[i] = Some((hh - closes[i]) / closes[i]);
        }
    }
    out
}

/// Distance from current close to lowest low over `period` bars (fraction of close).
pub fn dist_low(lows: &[f64], closes: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = lows.len();
    let mut out = vec![None; n];
    for i in (period-1)..n {
        let ll = lows[(i+1-period)..=i].iter().cloned().fold(f64::INFINITY, f64::min);
        if closes[i] > 1e-12 {
            out[i] = Some((closes[i] - ll) / closes[i]);
        }
    }
    out
}

/// Classic pivot: (H + L + C) / 3. Returns distance from current close.
pub fn pivot_dist(highs: &[f64], lows: &[f64], closes: &[f64]) -> Vec<Option<f64>> {
    let n = highs.len();
    let mut out = vec![None; n];
    for i in 1..n {
        let pivot = (highs[i-1] + lows[i-1] + closes[i-1]) / 3.0;
        if closes[i] > 1e-12 {
            out[i] = Some((closes[i] - pivot) / closes[i]);
        }
    }
    out
}

/// R1 distance: R1 = 2*pivot - L_prev.
pub fn r1_dist(highs: &[f64], lows: &[f64], closes: &[f64]) -> Vec<Option<f64>> {
    let n = highs.len();
    let mut out = vec![None; n];
    for i in 1..n {
        let pivot = (highs[i-1] + lows[i-1] + closes[i-1]) / 3.0;
        let r1 = 2.0 * pivot - lows[i-1];
        if closes[i] > 1e-12 {
            out[i] = Some((closes[i] - r1) / closes[i]);
        }
    }
    out
}

/// S1 distance: S1 = 2*pivot - H_prev.
pub fn s1_dist(highs: &[f64], lows: &[f64], closes: &[f64]) -> Vec<Option<f64>> {
    let n = highs.len();
    let mut out = vec![None; n];
    for i in 1..n {
        let pivot = (highs[i-1] + lows[i-1] + closes[i-1]) / 3.0;
        let s1 = 2.0 * pivot - highs[i-1];
        if closes[i] > 1e-12 {
            out[i] = Some((closes[i] - s1) / closes[i]);
        }
    }
    out
}

/// Fibonacci 61.8% distance: from low of window to 0.618 × (high - low).
pub fn fib_618_dist(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = highs.len();
    let mut out = vec![None; n];
    for i in (period-1)..n {
        let hh = highs[(i+1-period)..=i].iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let ll = lows[(i+1-period)..=i].iter().cloned().fold(f64::INFINITY, f64::min);
        let fib = ll + 0.618 * (hh - ll);
        if closes[i] > 1e-12 {
            out[i] = Some((closes[i] - fib) / closes[i]);
        }
    }
    out
}

/// Round number proximity: distance to nearest 0.001 multiple (as fraction of price).
pub fn round_number_prox(closes: &[f64]) -> Vec<Option<f64>> {
    closes.iter().map(|c| {
        if *c > 1e-12 {
            let nearest = (c / 0.001).round() * 0.001;
            Some((c - nearest).abs() / c)
        } else { None }
    }).collect()
}

/// Z-score of price relative to SMA ± std.
pub fn zscore(prices: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    for i in (period-1)..n {
        let w = &prices[(i+1-period)..=i];
        let mean = w.iter().sum::<f64>() / period as f64;
        let sd = std_dev(w);
        if sd > 1e-12 {
            out[i] = Some((prices[i] - mean) / sd);
        }
    }
    out
}

/// Linear regression slope over `period` bars (normalized by mean).
pub fn lr_slope(prices: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    if period > n { return out; }
    let x_mean = (period as f64 - 1.0) / 2.0;
    let ss_x: f64 = (0..period).map(|j| (j as f64 - x_mean).powi(2)).sum();
    for i in (period-1)..n {
        let w = &prices[(i+1-period)..=i];
        let y_mean = w.iter().sum::<f64>() / period as f64;
        let sp: f64 = (0..period).map(|j| (j as f64 - x_mean) * (w[j] - y_mean)).sum();
        if ss_x > 1e-12 && y_mean.abs() > 1e-12 {
            out[i] = Some(sp / ss_x / y_mean);
        }
    }
    out
}

/// LR R² (coefficient of determination) over `period` bars.
pub fn lr_r2(prices: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    if period > n { return out; }
    let x_mean = (period as f64 - 1.0) / 2.0;
    let ss_x: f64 = (0..period).map(|j| (j as f64 - x_mean).powi(2)).sum();
    for i in (period-1)..n {
        let w = &prices[(i+1-period)..=i];
        let y_mean = w.iter().sum::<f64>() / period as f64;
        let sp: f64 = (0..period).map(|j| (j as f64 - x_mean) * (w[j] - y_mean)).sum();
        let ss_y: f64 = w.iter().map(|y| (y - y_mean).powi(2)).sum();
        if ss_x > 1e-12 && ss_y > 1e-12 {
            out[i] = Some(sp.powi(2) / (ss_x * ss_y));
        }
    }
    out
}

/// LR deviation: distance of current price from the LR line endpoint.
pub fn lr_deviation(prices: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    if period > n { return out; }
    let x_mean = (period as f64 - 1.0) / 2.0;
    let ss_x: f64 = (0..period).map(|j| (j as f64 - x_mean).powi(2)).sum();
    for i in (period-1)..n {
        let w = &prices[(i+1-period)..=i];
        let y_mean = w.iter().sum::<f64>() / period as f64;
        let sp: f64 = (0..period).map(|j| (j as f64 - x_mean) * (w[j] - y_mean)).sum();
        if ss_x > 1e-12 {
            let slope = sp / ss_x;
            let intercept = y_mean - slope * x_mean;
            let predicted = slope * (period as f64 - 1.0) + intercept;
            if y_mean.abs() > 1e-12 {
                out[i] = Some((prices[i] - predicted) / y_mean);
            }
        }
    }
    out
}

/// Autocorrelation at given lag.
pub fn autocorr(prices: &[f64], lag: usize, window: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    if lag + window > n { return out; }
    for i in (lag + window - 1)..n {
        let y: &[f64] = &prices[(i + 1 - window)..=i];
        let x: &[f64] = &prices[(i + 1 - window - lag)..(i + 1 - lag)];
        let n_f = window as f64;
        let mx = x.iter().sum::<f64>() / n_f;
        let my = y.iter().sum::<f64>() / n_f;
        let cov: f64 = x.iter().zip(y.iter()).map(|(xi, yi)| (xi - mx) * (yi - my)).sum::<f64>() / n_f;
        let sx = std_dev(x);
        let sy = std_dev(y);
        if sx > 1e-12 && sy > 1e-12 {
            out[i] = Some(cov / (sx * sy));
        }
    }
    out
}

/// Variance ratio (Lo-MacKinlay): Var(q-period return) / (q * Var(1-period return)).
pub fn variance_ratio(prices: &[f64], q: usize, window: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    if window + q > n { return out; }
    let log_ret: Vec<f64> = (1..n)
        .map(|i| if prices[i-1] > 1e-12 { (prices[i] / prices[i-1]).ln() } else { 0.0 })
        .collect();
    for i in (window + q - 1)..n {
        let r1 = &log_ret[(i - window)..i];
        let var1 = std_dev(r1).powi(2);
        // q-period returns: aggregate q consecutive 1-period returns
        let rq: Vec<f64> = (0..(window - q + 1))
            .map(|j| r1[j..(j + q)].iter().sum::<f64>())
            .collect();
        let varq = std_dev(&rq).powi(2);
        if var1 > 1e-12 {
            out[i] = Some(varq / (q as f64 * var1));
        }
    }
    out
}

/// Hurst exponent (R/S method) over `window` bars.
pub fn hurst(prices: &[f64], window: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    if window < 20 || window > n { return out; }
    for i in (window-1)..n {
        let w = &prices[(i+1-window)..=i];
        let mean = w.iter().sum::<f64>() / window as f64;
        let deviations: Vec<f64> = w.iter().map(|x| x - mean).collect();
        let mut cum = 0.0_f64;
        let mut max_cum = f64::NEG_INFINITY;
        let mut min_cum = f64::INFINITY;
        for d in &deviations {
            cum += d;
            max_cum = max_cum.max(cum);
            min_cum = min_cum.min(cum);
        }
        let rs = max_cum - min_cum;
        let sd = std_dev(w);
        if sd > 1e-12 && rs > 0.0 {
            out[i] = Some((rs / sd).ln() / (window as f64).ln());
        }
    }
    out
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test dist_high_non_negative zscore_near_zero_mean lr_slope_rising 2>&1 | tail -5
```

Expected: `test result: ok. 3 passed`

- [ ] **Step 5: Commit**

```bash
git add src/analysis/techind.rs tests/techind_test.rs
git commit -m "feat: support/resistance + statistics — pivot, fib, z-score, LR, autocorr, hurst, VR"
```

---

### Task 9: Microstructure Indicators (8 indicators)

Indicators: `amihud_20`, `roll_spread`, `kyle_lambda_20`, `effective_spread`, `book_pressure`, `spread_osc`, `spread_change_rate`, `info_efficiency_ratio`.

These use the existing LOB feature fields (`spread`, `trade_intensity`, `ofi_1`, `r_1s`) already in `TechData`.

**Files:**
- Modify: `src/analysis/techind.rs`
- Test: `tests/techind_test.rs`

- [ ] **Step 1: Write the failing test**

```rust
use orderflow_rs::analysis::techind::{amihud_illiquidity, roll_spread_est, book_pressure_ind, spread_osc};

#[test]
fn amihud_positive() {
    let n = 30;
    let p: Vec<f64> = (0..n).map(|i| 1.0 + (i as f64 * 0.3).sin() * 0.01).collect();
    let vol: Vec<f64> = vec![1000.0; n];
    let result = amihud_illiquidity(&p, &vol, 20);
    for v in result.iter().skip(20).filter_map(|x| *x) {
        assert!(v >= 0.0, "Amihud should be non-negative");
    }
}

#[test]
fn book_pressure_range() {
    let ofi: Vec<f64> = vec![0.5, -0.3, 0.8, -0.1, 0.4, 0.2, -0.5, 0.7, 0.1, -0.2,
                              0.3, 0.6, -0.4, 0.9, -0.8, 0.1, 0.5, 0.2, -0.3, 0.7,
                              0.4, -0.6, 0.8, -0.2, 0.3];
    let result = book_pressure_ind(&ofi, 20);
    for v in result.iter().filter_map(|x| *x) {
        assert!(v >= -1.0 && v <= 1.0, "Book pressure should be in [-1,1]");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test amihud_positive book_pressure_range 2>&1 | tail -5
```

Expected: error — functions not found.

- [ ] **Step 3: Implement microstructure indicators in `src/analysis/techind.rs`**

```rust
/// Amihud illiquidity: mean(|return| / volume) over `period`.
pub fn amihud_illiquidity(closes: &[f64], volumes: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = closes.len();
    let mut out = vec![None; n];
    let log_ret: Vec<f64> = (0..n).map(|i| {
        if i == 0 || closes[i-1] < 1e-12 { 0.0 }
        else { (closes[i] / closes[i-1]).ln().abs() }
    }).collect();
    for i in period..n {
        let sum: f64 = (i-period..i).map(|j| {
            if volumes[j] > 1e-12 { log_ret[j] / volumes[j] } else { 0.0 }
        }).sum();
        out[i] = Some(sum / period as f64);
    }
    out
}

/// Roll (1984) effective spread estimate: 2 * sqrt(max(-cov(ret, ret_lag1), 0)).
pub fn roll_spread_est(closes: &[f64], window: usize) -> Vec<Option<f64>> {
    let n = closes.len();
    let mut out = vec![None; n];
    if window + 1 > n { return out; }
    let ret: Vec<f64> = (0..n).map(|i| {
        if i == 0 || closes[i-1] < 1e-12 { 0.0 } else { closes[i] - closes[i-1] }
    }).collect();
    for i in (window + 1)..n {
        let r: &[f64] = &ret[(i - window)..i];
        let r_lag: &[f64] = &ret[(i - window - 1)..(i - 1)];
        let n_f = window as f64;
        let mr = r.iter().sum::<f64>() / n_f;
        let mrl = r_lag.iter().sum::<f64>() / n_f;
        let cov: f64 = r.iter().zip(r_lag.iter())
            .map(|(a, b)| (a - mr) * (b - mrl)).sum::<f64>() / n_f;
        out[i] = Some(2.0 * (-cov).max(0.0).sqrt());
    }
    out
}

/// Kyle lambda proxy: abs(price_change) / abs(ofi) accumulated over `period`.
pub fn kyle_lambda(closes: &[f64], ofi: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = closes.len();
    let mut out = vec![None; n];
    for i in period..n {
        let sum_ofi: f64 = ofi[(i-period)..i].iter().map(|x| x.abs()).sum();
        if sum_ofi > 1e-12 && closes[i-period] > 1e-12 {
            let dp = (closes[i] - closes[i-period]).abs();
            out[i] = Some(dp / sum_ofi);
        }
    }
    out
}

/// Effective spread proxy: 2 * spread_field (already a cost measure from LOB).
/// Returns a normalized version: spread / mid_price.
pub fn effective_spread(spreads: &[f64], closes: &[f64]) -> Vec<Option<f64>> {
    spreads.iter().zip(closes.iter()).map(|(s, c)| {
        if *c > 1e-12 { Some(s / c) } else { None }
    }).collect()
}

/// Book pressure: sign-weighted OFI normalized to [-1, 1] via rolling max abs OFI.
pub fn book_pressure_ind(ofi: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = ofi.len();
    let mut out = vec![None; n];
    for i in (period-1)..n {
        let w = &ofi[(i+1-period)..=i];
        let max_abs = w.iter().map(|x| x.abs()).fold(0.0_f64, f64::max);
        if max_abs > 1e-12 {
            out[i] = Some(ofi[i] / max_abs);
        }
    }
    out
}

/// Spread oscillator: (spread - SMA(spread, period)) / SMA(spread, period).
pub fn spread_osc(spreads: &[f64], period: usize) -> Vec<Option<f64>> {
    let sma_v = sma(spreads, period);
    spreads.iter().zip(sma_v.iter()).map(|(s, m)| match m {
        Some(mv) if *mv > 1e-12 => Some((s - mv) / mv),
        _ => None,
    }).collect()
}

/// Spread change rate: (spread[i] - spread[i-1]) / spread[i-1].
pub fn spread_change_rate(spreads: &[f64]) -> Vec<Option<f64>> {
    let n = spreads.len();
    let mut out = vec![None; n];
    for i in 1..n {
        if spreads[i-1] > 1e-12 {
            out[i] = Some((spreads[i] - spreads[i-1]) / spreads[i-1]);
        }
    }
    out
}

/// Info efficiency ratio: abs(net displacement) / total path length over `period`.
pub fn info_efficiency_ratio(closes: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = closes.len();
    let mut out = vec![None; n];
    for i in period..n {
        let total_path: f64 = (i-period+1..i).map(|j| (closes[j] - closes[j-1]).abs()).sum();
        let net = (closes[i] - closes[i - period]).abs();
        if total_path > 1e-12 {
            out[i] = Some(net / total_path);
        }
    }
    out
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test amihud_positive book_pressure_range 2>&1 | tail -5
```

Expected: `test result: ok. 2 passed`

- [ ] **Step 5: Commit**

```bash
git add src/analysis/techind.rs tests/techind_test.rs
git commit -m "feat: microstructure indicators — Amihud, Roll spread, Kyle lambda, book pressure, spread osc"
```

---

### Task 10: compute_all() integration

Wire all indicators into one function that populates `TechData::indicators`.

**Files:**
- Modify: `src/analysis/techind.rs`
- Test: `tests/techind_test.rs`

- [ ] **Step 1: Write the failing test**

```rust
use orderflow_rs::analysis::techind::compute_all;

#[test]
fn compute_all_produces_indicators() {
    let n = 250;
    let prices: Vec<f64> = (0..n).map(|i| 1.0 + i as f64 * 0.001 + (i as f64 * 0.1).sin() * 0.01).collect();
    let highs: Vec<f64> = prices.iter().map(|p| p + 0.001).collect();
    let lows: Vec<f64> = prices.iter().map(|p| p - 0.001).collect();
    let volumes: Vec<f64> = vec![1000.0; n];
    let spreads: Vec<f64> = vec![0.0001; n];
    let ofi: Vec<f64> = vec![0.1; n];
    let r_1s: Vec<Option<f64>> = vec![Some(0.001); n];
    let r_5s = r_1s.clone();
    let r_30s = r_1s.clone();
    let r_300s = r_1s.clone();
    let mut td = TechData::new(
        (0..n as i64).collect(), prices.clone(), highs, lows,
        volumes, spreads, r_1s, r_5s, r_30s, r_300s,
    );
    compute_all(&mut td, &ofi);
    assert!(!td.indicators.is_empty(), "should have indicators after compute_all");
    // Check for a few key indicator names
    let names: Vec<&str> = td.indicators.iter().map(|s| s.name).collect();
    assert!(names.contains(&"sma_20"), "should have sma_20");
    assert!(names.contains(&"rsi_14"), "should have rsi_14");
    assert!(names.contains(&"bb_width_20"), "should have bb_width_20");
    // All indicator vecs have same length as prices
    for ind in &td.indicators {
        assert_eq!(ind.values.len(), n, "indicator {} length mismatch", ind.name);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test compute_all_produces_indicators 2>&1 | tail -5
```

Expected: error — `compute_all` not found.

- [ ] **Step 3: Implement `compute_all` in `src/analysis/techind.rs`**

```rust
/// Compute all ~120 indicators and populate `td.indicators`.
/// `ofi` is the OFI_1 series from the feature CSV (same length as `td.prices`).
pub fn compute_all(td: &mut TechData, ofi: &[f64]) {
    let p = &td.prices;
    let h = &td.highs;
    let l = &td.lows;
    let c = &td.prices; // closes == prices
    let v = &td.volumes;
    let sp = &td.spreads;
    // opens: shift prices by 1 (open[i] = close[i-1])
    let mut opens = p.clone();
    if opens.len() > 1 { opens.rotate_right(1); opens[0] = opens[1]; }

    macro_rules! push {
        ($name:expr, $vals:expr) => {
            td.indicators.push(IndicatorSeries { name: $name, values: $vals });
        };
    }

    // ── Moving Averages ──────────────────────────────────────────
    push!("sma_10",   sma(p, 10));
    push!("sma_20",   sma(p, 20));
    push!("sma_50",   sma(p, 50));
    push!("sma_100",  sma(p, 100));
    push!("sma_200",  sma(p, 200));
    push!("ema_5",    ema(p, 5));
    push!("ema_12",   ema(p, 12));
    push!("ema_20",   ema(p, 20));
    push!("ema_26",   ema(p, 26));
    push!("ema_50",   ema(p, 50));
    push!("ema_200",  ema(p, 200));
    push!("wma_14",   wma(p, 14));
    push!("dema_20",  dema(p, 20));
    push!("tema_20",  tema(p, 20));
    push!("hma_14",   hma(p, 14));
    push!("kama_10",  kama(p, 10));
    push!("golden_cross", golden_cross(p, 5, 20));
    push!("death_cross",  death_cross(p, 5, 20));

    // ── Trend Strength ───────────────────────────────────────────
    let (adx_v, pdi_v, mdi_v) = adx_full(h, l, c, 14);
    push!("adx_14",   adx_v);
    push!("pdi_14",   pdi_v);
    push!("mdi_14",   mdi_v);
    push!("aroon_up_25",   aroon_up(h, 25));
    push!("aroon_down_25", aroon_down(l, 25));
    push!("choppiness_14", choppiness(h, l, c, 14));
    push!("supertrend_14", supertrend(h, l, c, 14, 3.0));

    // ── Momentum ─────────────────────────────────────────────────
    push!("rsi_7",    rsi(p, 7));
    push!("rsi_14",   rsi(p, 14));
    push!("rsi_21",   rsi(p, 21));
    push!("stoch_k_14",  stoch_k(h, l, c, 14));
    push!("stoch_d_14",  stoch_d(h, l, c, 14));
    let (macd_l, macd_s, macd_h) = macd_components(p, 12, 26, 9);
    push!("macd_line",   macd_l);
    push!("macd_signal", macd_s);
    push!("macd_hist",   macd_h);
    push!("cci_20",      cci(h, l, c, 20));
    push!("williams_r_14", williams_r(h, l, c, 14));
    push!("roc_10",   roc(p, 10));
    push!("roc_20",   roc(p, 20));
    push!("momentum_10", momentum(p, 10));
    push!("momentum_20", momentum(p, 20));
    push!("cmo_14",   cmo(p, 14));
    push!("dpo_20",   dpo(p, 20));
    push!("awesome_osc", awesome_osc(h, l));

    // ── Volatility ───────────────────────────────────────────────
    push!("bb_width_20",   bb_width(p, 20, 2.0));
    push!("bb_pct_b_20",   bb_pct_b(p, 20, 2.0));
    push!("keltner_width_20", keltner_width(h, l, c, 20, 1.5));
    push!("donchian_width_20", donchian_width(h, l, 20));
    push!("atr_14",    atr(h, l, c, 14).into_iter().collect::<Vec<_>>());
    push!("natr_14",   natr(h, l, c, 14));
    push!("realized_vol_20", realized_vol(p, 20));
    push!("parkinson_vol_14", parkinson_vol(h, l, 14));
    push!("garman_klass_20", garman_klass_vol(h, l, &opens, c, 20));
    push!("squeeze_signal", squeeze_signal(h, l, c, 20));

    // ── Volume ───────────────────────────────────────────────────
    push!("obv",     obv(c, v).into_iter().map(Some).collect());
    let vwap_v = vwap(h, l, c, v);
    // VWAP as deviation from price
    let vwap_dev: Vec<Option<f64>> = vwap_v.iter().zip(p.iter())
        .map(|(vw, price)| if *price > 1e-12 { Some((price - vw) / price) } else { None })
        .collect();
    push!("vwap_dev", vwap_dev);
    push!("mfi_14",   mfi(h, l, c, v, 14));
    push!("force_index_13", force_index(c, v, 13));
    push!("ease_of_movement_14", ease_of_movement(h, l, v, 14));
    push!("vpt",  vpt(c, v).into_iter().map(Some).collect());
    push!("vroc_14", vroc(v, 14));
    push!("rvol_20", rvol(v, 20));

    // ── Support / Resistance ─────────────────────────────────────
    push!("dist_high_20", dist_high(h, c, 20));
    push!("dist_high_50", dist_high(h, c, 50));
    push!("dist_low_20",  dist_low(l, c, 20));
    push!("dist_low_50",  dist_low(l, c, 50));
    push!("pivot_dist",   pivot_dist(h, l, c));
    push!("r1_dist",      r1_dist(h, l, c));
    push!("s1_dist",      s1_dist(h, l, c));
    push!("fib_618_dist", fib_618_dist(h, l, c, 50));
    push!("round_number_prox", round_number_prox(c));

    // ── Statistical ──────────────────────────────────────────────
    push!("zscore_10",  zscore(p, 10));
    push!("zscore_20",  zscore(p, 20));
    push!("zscore_50",  zscore(p, 50));
    push!("lr_slope_20", lr_slope(p, 20));
    push!("lr_r2_20",    lr_r2(p, 20));
    push!("lr_deviation_20", lr_deviation(p, 20));
    push!("autocorr_lag1", autocorr(p, 1, 50));
    push!("autocorr_lag5", autocorr(p, 5, 50));
    push!("variance_ratio_16", variance_ratio(p, 16, 100));
    push!("hurst_100",  hurst(p, 100));

    // ── Microstructure ───────────────────────────────────────────
    push!("amihud_20",      amihud_illiquidity(c, v, 20));
    push!("roll_spread",    roll_spread_est(c, 20));
    push!("kyle_lambda_20", kyle_lambda(c, ofi, 20));
    push!("effective_spread", effective_spread(sp, c));
    push!("book_pressure",  book_pressure_ind(ofi, 20));
    push!("spread_osc",     spread_osc(sp, 20));
    push!("spread_change_rate", spread_change_rate(sp));
    push!("info_efficiency_ratio", info_efficiency_ratio(c, 20));
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test compute_all_produces_indicators 2>&1 | tail -5
```

Expected: `test result: ok. 1 passed`

- [ ] **Step 5: Run full suite**

```bash
cargo test 2>&1 | tail -8
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/analysis/techind.rs tests/techind_test.rs
git commit -m "feat: compute_all — wire all indicators into TechData"
```

---

### Task 11: IC computation in techreport.rs

Compute Spearman IC for each indicator vs each return horizon.

**Files:**
- Modify: `src/analysis/techreport.rs`
- Test: `tests/techind_test.rs`

- [ ] **Step 1: Write the failing test**

```rust
use orderflow_rs::analysis::techreport::{compute_ic_table, IcRow};

#[test]
fn ic_table_produces_rows() {
    use orderflow_rs::analysis::techind::{TechData, IndicatorSeries};
    let n = 200;
    let prices: Vec<f64> = (0..n).map(|i| 1.0 + i as f64 * 0.001).collect();
    let r_1s: Vec<Option<f64>> = prices.windows(2)
        .map(|w| Some(w[1] / w[0] - 1.0))
        .chain(std::iter::once(None))
        .collect();
    let mut td = TechData::new(
        (0..n as i64).collect(),
        prices.clone(),
        prices.iter().map(|p| p + 0.001).collect(),
        prices.iter().map(|p| p - 0.001).collect(),
        vec![100.0; n],
        vec![0.0001; n],
        r_1s.clone(), r_1s.clone(), r_1s.clone(), r_1s.clone(),
    );
    td.indicators.push(IndicatorSeries {
        name: "sma_10",
        values: (0..n).map(|i| if i >= 9 { Some(prices[i]) } else { None }).collect(),
    });
    let rows = compute_ic_table(&td);
    assert!(!rows.is_empty());
    assert_eq!(rows[0].name, "sma_10");
    // IC with itself should not be zero (trending data)
    assert!(rows[0].ic_1s.is_some() || rows[0].ic_5s.is_some());
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test ic_table_produces_rows 2>&1 | tail -5
```

Expected: error — `compute_ic_table` not found.

- [ ] **Step 3: Implement IC computation in `src/analysis/techreport.rs`**

```rust
use crate::analysis::stats::spearman_ic;
use crate::analysis::techind::TechData;

pub struct IcRow {
    pub name: String,
    pub ic_1s:  Option<f64>,
    pub ic_5s:  Option<f64>,
    pub ic_30s: Option<f64>,
    pub ic_300s: Option<f64>,
}

/// Compute Spearman IC for every indicator in `td` vs each return horizon.
/// Only uses rows where both indicator and return are non-None.
pub fn compute_ic_table(td: &TechData) -> Vec<IcRow> {
    td.indicators.iter().map(|ind| {
        let ic = |returns: &[Option<f64>]| -> Option<f64> {
            let (xs, ys): (Vec<f64>, Vec<f64>) = ind.values.iter()
                .zip(returns.iter())
                .filter_map(|(x, y)| match (x, y) {
                    (Some(xi), Some(yi)) if xi.is_finite() && yi.is_finite() => Some((*xi, *yi)),
                    _ => None,
                })
                .unzip();
            if xs.len() < 10 { return None; }
            spearman_ic(&xs, &ys)
        };
        IcRow {
            name:    ind.name.to_string(),
            ic_1s:   ic(&td.r_1s),
            ic_5s:   ic(&td.r_5s),
            ic_30s:  ic(&td.r_30s),
            ic_300s: ic(&td.r_300s),
        }
    }).collect()
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test ic_table_produces_rows 2>&1 | tail -5
```

Expected: `test result: ok. 1 passed`

- [ ] **Step 5: Commit**

```bash
git add src/analysis/techreport.rs tests/techind_test.rs
git commit -m "feat: IC computation — Spearman IC per indicator vs 4 return horizons"
```

---

### Task 12: Report formatter + CSV writer

Write per-pair CSV and text summary to `reports_dir`.

**Files:**
- Modify: `src/analysis/techreport.rs`
- Test: `tests/techind_test.rs`

- [ ] **Step 1: Write the failing test**

```rust
use orderflow_rs::analysis::techreport::{write_ic_report, IcRow};
use std::fs;

#[test]
fn write_ic_report_creates_files() {
    let dir = tempfile::tempdir().unwrap();
    let rows = vec![
        IcRow { name: "sma_10".into(), ic_1s: Some(0.05), ic_5s: Some(0.08),
                ic_30s: Some(0.02), ic_300s: Some(-0.01) },
        IcRow { name: "rsi_14".into(), ic_1s: None, ic_5s: Some(-0.03),
                ic_30s: None, ic_300s: Some(0.04) },
    ];
    write_ic_report("EURUSD", &rows, dir.path().to_str().unwrap()).unwrap();
    let csv_path = dir.path().join("EURUSD_techanalysis.csv");
    assert!(csv_path.exists(), "CSV report should be created");
    let content = fs::read_to_string(&csv_path).unwrap();
    assert!(content.contains("sma_10"));
    assert!(content.contains("rsi_14"));
    assert!(content.contains("0.05"));
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test write_ic_report_creates_files 2>&1 | tail -5
```

Expected: error — `write_ic_report` not found.

- [ ] **Step 3: Implement report writer in `src/analysis/techreport.rs`**

```rust
use std::io::Write as IoWrite;
use anyhow::{Context, Result};

/// Write IC table for one symbol to `<reports_dir>/<symbol>_techanalysis.csv`.
pub fn write_ic_report(symbol: &str, rows: &[IcRow], reports_dir: &str) -> Result<()> {
    std::fs::create_dir_all(reports_dir)?;
    let path = format!("{}/{}_techanalysis.csv", reports_dir, symbol);
    let mut f = std::fs::File::create(&path)
        .with_context(|| format!("create {path}"))?;
    writeln!(f, "indicator,ic_1s,ic_5s,ic_30s,ic_300s")?;
    for row in rows {
        let fmt = |v: Option<f64>| v.map(|x| format!("{:.6}", x)).unwrap_or_default();
        writeln!(f, "{},{},{},{},{}", row.name, fmt(row.ic_1s), fmt(row.ic_5s),
                 fmt(row.ic_30s), fmt(row.ic_300s))?;
    }
    Ok(())
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test write_ic_report_creates_files 2>&1 | tail -5
```

Expected: `test result: ok. 1 passed`

- [ ] **Step 5: Commit**

```bash
git add src/analysis/techreport.rs tests/techind_test.rs
git commit -m "feat: IC report writer — per-symbol CSV with 4-horizon Spearman IC"
```

---

### Task 13: Cross-pair summary CSV

After processing all pairs, write `summary_techanalysis.csv` with average IC across pairs.

**Files:**
- Modify: `src/analysis/techreport.rs`
- Test: `tests/techind_test.rs`

- [ ] **Step 1: Write the failing test**

```rust
use orderflow_rs::analysis::techreport::{write_summary_report, IcRow};
use std::fs;

#[test]
fn write_summary_report_aggregates() {
    let dir = tempfile::tempdir().unwrap();
    let per_pair: Vec<(String, Vec<IcRow>)> = vec![
        ("EURUSD".into(), vec![
            IcRow { name: "rsi_14".into(), ic_1s: Some(0.05), ic_5s: Some(0.10), ic_30s: None, ic_300s: None },
        ]),
        ("GBPUSD".into(), vec![
            IcRow { name: "rsi_14".into(), ic_1s: Some(0.03), ic_5s: None, ic_30s: Some(0.06), ic_300s: None },
        ]),
    ];
    write_summary_report(&per_pair, dir.path().to_str().unwrap()).unwrap();
    let path = dir.path().join("summary_techanalysis.csv");
    assert!(path.exists());
    let content = fs::read_to_string(&path).unwrap();
    assert!(content.contains("rsi_14"));
    // Average IC_1s of rsi_14 = (0.05 + 0.03) / 2 = 0.04
    assert!(content.contains("0.040000") || content.contains("0.04"));
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test write_summary_report_aggregates 2>&1 | tail -5
```

Expected: error — `write_summary_report` not found.

- [ ] **Step 3: Implement summary report in `src/analysis/techreport.rs`**

```rust
use std::collections::HashMap;

/// Write cross-pair average IC summary to `<reports_dir>/summary_techanalysis.csv`.
pub fn write_summary_report(per_pair: &[(String, Vec<IcRow>)], reports_dir: &str) -> Result<()> {
    // Aggregate: indicator → (horizon → (sum, count))
    let mut agg: HashMap<String, [(f64, usize); 4]> = HashMap::new();
    for (_sym, rows) in per_pair {
        for row in rows {
            let entry = agg.entry(row.name.clone()).or_insert([(0.0, 0); 4]);
            if let Some(v) = row.ic_1s   { entry[0].0 += v; entry[0].1 += 1; }
            if let Some(v) = row.ic_5s   { entry[1].0 += v; entry[1].1 += 1; }
            if let Some(v) = row.ic_30s  { entry[2].0 += v; entry[2].1 += 1; }
            if let Some(v) = row.ic_300s { entry[3].0 += v; entry[3].1 += 1; }
        }
    }

    std::fs::create_dir_all(reports_dir)?;
    let path = format!("{}/summary_techanalysis.csv", reports_dir);
    let mut f = std::fs::File::create(&path)?;
    writeln!(f, "indicator,avg_ic_1s,avg_ic_5s,avg_ic_30s,avg_ic_300s")?;

    let mut rows: Vec<_> = agg.iter().collect();
    // Sort by abs(avg_ic_5s) descending
    rows.sort_by(|a, b| {
        let av = if a.1[1].1 > 0 { (a.1[1].0 / a.1[1].1 as f64).abs() } else { 0.0 };
        let bv = if b.1[1].1 > 0 { (b.1[1].0 / b.1[1].1 as f64).abs() } else { 0.0 };
        bv.partial_cmp(&av).unwrap_or(std::cmp::Ordering::Equal)
    });

    for (name, acc) in rows {
        let fmt = |idx: usize| {
            if acc[idx].1 > 0 { format!("{:.6}", acc[idx].0 / acc[idx].1 as f64) }
            else { String::new() }
        };
        writeln!(f, "{},{},{},{},{}", name, fmt(0), fmt(1), fmt(2), fmt(3))?;
    }
    Ok(())
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test write_summary_report_aggregates 2>&1 | tail -5
```

Expected: `test result: ok. 1 passed`

- [ ] **Step 5: Commit**

```bash
git add src/analysis/techreport.rs tests/techind_test.rs
git commit -m "feat: cross-pair summary report — avg IC across symbols, sorted by abs IC_5s"
```

---

### Task 14: Wire CLI command + end-to-end test

Connect `src/commands/techanalysis.rs` to the full pipeline and verify with a smoke test.

**Files:**
- Modify: `src/commands/techanalysis.rs`
- Test: `tests/techind_test.rs`

- [ ] **Step 1: Write the failing test**

```rust
use std::io::Write as IoWrite;

#[test]
fn cli_techanalysis_end_to_end() {
    let data_dir = tempfile::tempdir().unwrap();
    let reports_dir = tempfile::tempdir().unwrap();

    // Write a minimal CSV with 250 rows
    let csv_path = data_dir.path().join("EURUSD_features.csv");
    let mut f = std::fs::File::create(&csv_path).unwrap();
    writeln!(f, "ts,ofi_1,ofi_2,ofi_3,ofi_4,ofi_5,ofi_6,ofi_7,ofi_8,ofi_9,ofi_10,depth_imb,microprice_dev,queue_imb,spread,trade_intensity,price_impact,level_drain,weighted_mid_slope,r_1s,r_5s,r_30s,r_300s,exchange,symbol,is_imputed,gap_flag").unwrap();
    for i in 0..250usize {
        let r = 0.0001 * (i as f64 * 0.3).sin();
        writeln!(f, "{},0.1,0,0,0,0,0,0,0,0,0,0.01,0.001,0.02,0.0001,5.0,0.001,0.5,0.0002,{r},{r},{r},{r},DUKA,EURUSD,false,false").unwrap();
    }

    orderflow_rs::commands::techanalysis::run(
        data_dir.path().to_str().unwrap(),
        reports_dir.path().to_str().unwrap(),
    ).unwrap();

    let csv = reports_dir.path().join("EURUSD_techanalysis.csv");
    assert!(csv.exists(), "per-pair report should exist");
    let summary = reports_dir.path().join("summary_techanalysis.csv");
    assert!(summary.exists(), "summary report should exist");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test cli_techanalysis_end_to_end 2>&1 | tail -5
```

Expected: test fails — `run` is a stub that returns `Ok(())` without creating files.

- [ ] **Step 3: Implement `src/commands/techanalysis.rs`**

```rust
use anyhow::Result;
use std::fs;
use crate::analysis::techreport::{load_techdata, compute_ic_table, write_ic_report, write_summary_report};
use crate::analysis::techind::compute_all;

pub fn run(data_dir: &str, reports_dir: &str) -> Result<()> {
    let entries = fs::read_dir(data_dir)?;
    let mut per_pair = Vec::new();

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let fname = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
        if !fname.ends_with("_features.csv") { continue; }

        // Extract symbol from filename: "EURUSD_features.csv" → "EURUSD"
        let symbol = fname.trim_end_matches("_features.csv").to_string();
        let path_str = path.to_str().unwrap_or("");

        let mut td = match load_techdata(path_str) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("skip {symbol}: {e}");
                continue;
            }
        };

        // Extract OFI_1 series by re-reading CSV (ofi_1 field)
        let ofi = extract_ofi(path_str);

        compute_all(&mut td, &ofi);
        let rows = compute_ic_table(&td);
        write_ic_report(&symbol, &rows, reports_dir)?;
        println!("{symbol}: {} indicators computed", td.indicators.len());
        per_pair.push((symbol, rows));
    }

    write_summary_report(&per_pair, reports_dir)?;
    println!("Summary written to {reports_dir}/summary_techanalysis.csv");
    Ok(())
}

/// Extract the ofi_1 column as a Vec<f64> from a features CSV.
fn extract_ofi(csv_path: &str) -> Vec<f64> {
    use crate::analysis::loader::FeatureRow;
    csv::Reader::from_path(csv_path)
        .map(|mut rdr| {
            rdr.deserialize::<FeatureRow>()
                .filter_map(|r| r.ok())
                .map(|r| r.ofi_1)
                .collect()
        })
        .unwrap_or_default()
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test cli_techanalysis_end_to_end 2>&1 | tail -5
```

Expected: `test result: ok. 1 passed`

- [ ] **Step 5: Run full test suite**

```bash
cargo test 2>&1 | tail -10
```

Expected: all tests pass (count includes all prior + new).

- [ ] **Step 6: Smoke-run on real data (if available)**

```bash
cargo run --release -- techanalysis data/ reports/ 2>&1 | tail -20
```

Expected output like:
```
EURUSD: 80 indicators computed
GBPUSD: 80 indicators computed
...
Summary written to reports/summary_techanalysis.csv
```

- [ ] **Step 7: Final commit**

```bash
git add src/commands/techanalysis.rs tests/techind_test.rs
git commit -m "feat: techanalysis CLI — end-to-end pipeline: CSV → TechData → indicators → IC report"
```

---

## Self-Review Checklist

- **Spec coverage**: All 12 indicator categories from §4.1–§4.13 are covered across Tasks 3–9. CLI command from §5 is in Task 14. Report format from §6 is in Tasks 11–13.
- **No placeholders**: Every step has complete code.
- **Type consistency**: `TechData`, `IndicatorSeries`, `IcRow` defined in Task 1/11 and used consistently. `compute_all` in Task 10 calls only functions defined in Tasks 3–9. `load_techdata` defined in Task 2, called in Task 14.
- **FeatureRow.ofi_1**: Used in Task 14 `extract_ofi` — verify field name matches `src/analysis/loader.rs` before running.
- **spearman_ic signature**: Called as `spearman_ic(&xs, &ys)` in Task 11 — verify this matches `src/analysis/stats.rs`.

