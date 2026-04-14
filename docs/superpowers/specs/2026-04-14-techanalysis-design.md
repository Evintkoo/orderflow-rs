# P7 Technical Indicator Correlation Analysis — Design Spec

**Date:** 2026-04-14  
**Status:** Approved  
**Scope:** Post-processing analysis layer over existing Dukascopy + simulation feature CSVs

---

## 1. Goal

Compute ~152 technical indicators from the existing feature CSV data and measure each indicator's Spearman IC (information coefficient) against forward returns at four horizons (r_1s, r_5s, r_30s, r_300s). Produce a per-pair report and a cross-pair summary identifying which indicators have consistent predictive power at microstructure timescales (100ms bars).

---

## 2. Architecture

### 2.1 New files

| File | Purpose |
|------|---------|
| `src/analysis/techind.rs` | All TI computation functions. Pure functions on `&[f64]` slices, return `Vec<Option<f64>>`. No state, no I/O. |
| `src/analysis/techreport.rs` | Report formatter for P7 output. Mirrors style of existing `report.rs`. |

### 2.2 Modified files

| File | Change |
|------|--------|
| `src/analysis/mod.rs` | `pub mod techind; pub mod techreport;` + re-exports |
| `src/main.rs` | New `"techanalysis"` command arm calling `run_techanalysis()` |

### 2.3 No pipeline changes

The TI analysis module is purely a post-processing layer. It reads existing `FeatureRow` CSVs (already written), reconstructs mid-price in memory, computes indicators, and correlates against the existing `r_1s/r_5s/r_30s/r_300s` labels. No changes to ingestion, `FeatureVector`, `LabelQueue`, or CSV schema.

---

## 3. Mid-Price Reconstruction

The existing CSV does not store `mid_price`. Reconstruction from forward returns:

```
p[0] = 1.0  (arbitrary base — scale doesn't affect Spearman IC)
p[i+1] = p[i] × (1 + r_1s[i] × 0.1)
```

Rationale: at 100ms spacing, the period return ≈ r_1s × (100ms / 1000ms). This produces a relative price series accurate enough for computing moving averages, oscillators, and S/R levels. High and Low per bar are approximated as:

```
high[i] = p[i] + spread[i] / 2
low[i]  = p[i] - spread[i] / 2
open[i] = p[i-1]   (previous close)
```

Rows with `r_1s = None` are skipped in reconstruction; the price holds its last known value.

---

## 4. Indicator Catalogue (~152 indicators)

All lookback windows are in bars (1 bar = 100ms). `[V]` = uses `trade_intensity` as volume proxy. `[E]` = uses `spread` as high-low proxy.

### 4.1 Moving Averages (18)

| ID | Formula | Params |
|----|---------|--------|
| sma_10/20/50/100/200 | Simple Moving Average | 10, 20, 50, 100, 200 |
| ema_5/12/20/26/50/200 | Exponential MA | 5, 12, 20, 26, 50, 200 |
| wma_14/50 | Linearly Weighted MA | 14, 50 |
| dema_20 | Double EMA = 2×EMA(n) − EMA(EMA(n)) | 20 |
| tema_20 | Triple EMA = 3×EMA − 3×EMA(EMA) + EMA³ | 20 |
| hma_14 | Hull MA = WMA(2×WMA(n/2) − WMA(n), √n) | 14 |
| kama_10 | Kaufman Adaptive MA | 10 |
| zlema_20 | Zero-Lag EMA = EMA(2×p − p[lag]) | 20 |
| smma_14 | Smoothed MA (Wilder's) = (prev×(n-1)+cur)/n | 14 |
| t3_20 | Tillson T3 (6-EMA cascade, v=0.7) | 20 |
| alma_20 | Arnaud Legoux MA (σ=6, offset=0.85) | 20 |
| vwma_20 `[V]` | Volume-Weighted MA | 20 |
| mcginley_14 | McGinley Dynamic | 14 |
| golden_cross | SMA50 > SMA200 → +1, else −1 | — |
| death_cross | SMA50 < SMA200 → +1, else −1 | — |
| ema_cross_signal | EMA12 > EMA26 → +1, else −1 | — |
| ema_cross_dist | (EMA12 − EMA26) / mid | — |
| price_sma50_zscore | (mid − SMA50) / rolling_std(50) | — |

### 4.2 Trend Strength (11)

| ID | Formula | Params |
|----|---------|--------|
| adx_14 `[E]` | Wilder's Average Directional Index | 14 |
| pdi_14 `[E]` | Plus Directional Indicator | 14 |
| mdi_14 `[E]` | Minus Directional Indicator | 14 |
| aroon_up/down/osc_25 | Aroon Up/Down/Oscillator | 25 |
| vortex_pos/neg_14 `[E]` | Vortex +VI / −VI | 14 |
| mass_index_25 `[E]` | Mass Index (EMA ratio sum) | 9/25 |
| adxr_14 `[E]` | ADX Rating = (ADX_t + ADX_{t-14}) / 2 | 14 |
| choppiness_14 `[E]` | CHOP = 100×log10(ΣATRn / (Hn−Ln)) / log10(n) | 14 |
| supertrend `[E]` | ATR trailing stop, flips direction | 10, ×3 |
| chandelier_exit `[E]` | Highest high − ATR multiplier | 22, ×3 |
| trend_mode | ADX>25 AND RSI>50 → +1 uptrend; RSI<50 → −1; else 0 | — |

### 4.3 Momentum Oscillators (20)

| ID | Params |
|----|--------|
| rsi_7/14/21 | RSI, 3 periods |
| stoch_k_14 `[E]` | Fast Stochastic %K |
| stoch_d_14 `[E]` | Slow %D = SMA3(%K) |
| stoch_rsi_14 | RSI fed into Stochastic formula |
| kdj_9 `[E]` | K = %K, D = SMA3(K), J = 3D − 2K |
| macd_line | EMA12 − EMA26 |
| macd_signal | EMA9 of MACD line |
| macd_hist | MACD line − signal |
| ppo | (EMA12 − EMA26) / EMA26 × 100 |
| apo | EMA12 − EMA26 (in price units) |
| cci_20 `[E]` | CCI = (Typical − SMA) / (0.015 × MeanDev) |
| williams_r_14 `[E]` | Williams %R |
| roc_10/20 | Rate of Change |
| momentum_10/20 | Price − Price[n] |
| cmo_14 | Chande Momentum Oscillator |
| tsi_25_13 | True Strength Index (double-smoothed momentum) |
| dpo_20 | Detrended Price Oscillator |
| kst | Know Sure Thing (4 ROC + 4 SMA combo) |
| ultimate_osc `[E]` | Weighted 7/14/28 BP/TR |
| awesome_osc `[E]` | SMA5 − SMA34 of midpoint |
| rvi_14 `[E]` | Relative Vigor Index |
| smi_14 `[E]` | Stochastic Momentum Index |
| qstick_10 | QStick = SMA(close − open) |

### 4.4 Volatility (13)

| ID | Params |
|----|--------|
| bb_width_20 | (Upper − Lower) / mid |
| bb_pct_b_20 | (mid − Lower) / (Upper − Lower) |
| bb_thrust_20 | bb_pct_b > 1 → +1; < 0 → −1; else 0 |
| keltner_width_20 `[E]` | (Upper − Lower) / mid |
| donchian_width_20 | (MaxHigh − MinLow) / mid over 20 bars |
| atr_14 `[E]` | Wilder's ATR |
| natr_14 `[E]` | ATR / mid × 100 |
| realized_vol_20/50 | sqrt(sum(log_returns²) / n) × sqrt(252) |
| parkinson_vol_14 `[E]` | sqrt(sum(ln(H/L)²) / (4ln2×n)) |
| garman_klass_vol_20 `[E]` | OHLC-based estimator |
| rogers_satchell_vol_20 `[E]` | Drift-agnostic OHLC estimator |
| yang_zhang_vol_20 `[E]` | Optimal OHLC estimator |
| vol_ratio | ATR(14) / SMA(ATR(14), 20) |
| squeeze_signal | BB inside Keltner Channel → 1, else 0 |

### 4.5 Volume-Based (11) `[V]`

| ID | Description |
|----|-------------|
| vwap | Cumsum(mid×TI) / Cumsum(TI), reset every 5000 bars |
| twap_50 | SMA50 of mid (equal-time weight) |
| obv | Cumsum(TI × sign(Δmid)) |
| cvd | Cumsum(TI × sign(Δmid)), reset every 5000 bars |
| mfi_14 `[E]` | Money Flow Index |
| force_index_13 | EMA13(Δmid × TI) |
| ease_of_movement_14 `[E]` | ((H+L)/2 − prev(H+L)/2) / (TI/(H−L)) smoothed |
| vpt | Cumsum(TI × Δmid/mid) |
| vroc_14 | (TI − TI[14]) / TI[14] × 100 |
| klinger_34_55 `[E]` | EMA34 − EMA55 of volume force |
| rvol_20 | TI / SMA(TI, 20) |

### 4.6 Support / Resistance (14)

| ID | Description |
|----|-------------|
| dist_high_20/50/200 | (N-bar high − mid) / mid |
| dist_low_20/50/200 | (mid − N-bar low) / mid |
| pivot_std | P = (H+L+C)/3 over last 200 bars |
| r1_dist / s1_dist | Distance to R1=2P−L and S1=2P−H |
| r2_dist / s2_dist | Distance to R2=P+(H−L) and S2=P−(H−L) |
| woodie_pivot_dist | P = (H+L+2C)/4 over 200 bars |
| fib_382_dist | Distance to 38.2% retracement of 200-bar range |
| fib_618_dist | Distance to 61.8% retracement |
| round_number_prox | Distance to nearest 0.001 (1 pip) boundary |
| camarilla_r3_dist | Distance to C + 1.1×(H−L)/4 |
| zigzag_phase | Current position relative to last ZigZag swing (0–1) |
| envelope_pct_b | (mid − lower_env) / (upper_env − lower_env), ±1% MA20 |

### 4.7 Mean Reversion / Statistical (14)

| ID | Description |
|----|-------------|
| hurst_100 | Rescaled range Hurst exponent, 100-bar window |
| autocorr_lag1/lag5 | Rolling 50-bar Pearson autocorrelation of returns |
| half_life_100 | OLS fit ΔP ~ P[t-1], β → half_life = −ln(2)/β |
| fisher_10 | Fisher Transform of price within 10-bar range |
| cog_10 | Center of Gravity Oscillator |
| lr_slope_20/50 | OLS regression slope of mid over N bars |
| lr_r2_20 | R² of rolling OLS fit |
| lr_deviation | mid − OLS forecast |
| zscore_10/20/50/100 | (mid − SMA_N) / rolling_std_N |
| detrended_osc_20 | mid − linear regression value over 20 bars |
| variance_ratio_16 | Lo-MacKinlay VR test statistic (q=4) |
| profit_factor_20 | sum(up_returns) / sum(abs(down_returns)) |

### 4.8 Information-Theoretic / Entropy (6)

| ID | Description | Window |
|----|-------------|--------|
| shannon_entropy_50 | −Σ p_i log p_i of return histogram | 50 bars |
| sample_entropy_50 | SampEn(m=2, r=0.2σ) | 50 bars |
| permutation_entropy_50 | PE(order=3) | 50 bars |
| approx_entropy_50 | ApEn(m=2, r=0.2σ) | 50 bars |
| dfa_slope_100 | Detrended Fluctuation Analysis α exponent | 100 bars |
| fractal_dim_50 | Fractal dimension via box-counting | 50 bars |

### 4.9 Microstructure Extensions (10)

| ID | Description |
|----|-------------|
| amihud_20 `[V]` | mean(\|r\|/TI) over 20 bars |
| roll_spread | 2×sqrt(max(−cov(Δmid_t, Δmid_{t-1}), 0)) |
| kyle_lambda_20 `[V]` | OLS slope of Δmid ~ signed_vol |
| effective_spread | 2×\|mid − trade_mid\| proxy via microprice_dev |
| realized_spread_20 `[V]` | effective_spread − price_impact |
| adverse_selection_20 | price_impact component from Kyle λ |
| book_pressure | (bid_depth − ask_depth) / (bid_depth + ask_depth) proxy via depth_imb |
| spread_change_rate | (spread − spread[1]) / spread[1] |
| spread_osc | EMA5(spread) − EMA20(spread) |
| info_efficiency_ratio | mean(\|Δmid\|) / mean(spread) over 20 bars |

### 4.10 Cycle / Adaptive (8)

| ID | Description |
|----|-------------|
| dominant_cycle_period | Highest-power FFT frequency, 20–100 bar search |
| ehlers_supersmooth_14 | 2-pole Ehlers SuperSmoother |
| ehlers_inst_trendline | Ehlers Instantaneous Trendline (adaptive EMA) |
| sinewave_lead/lag | Sine and cosine of dominant cycle phase |
| cycle_phaser | sin(2π × t / dominant_cycle_period) |
| frama_20 | Fractal Adaptive MA |
| adaptive_rsi_14 | RSI with period scaled by vol_ratio |
| laguerre_rsi_05 | Laguerre RSI (γ=0.5) |

### 4.11 Bill Williams (8)

| ID | Description |
|----|-------------|
| alligator_jaw | SMMA(13, offset 8) of midpoint |
| alligator_teeth | SMMA(8, offset 5) |
| alligator_lips | SMMA(5, offset 3) |
| alligator_open | lips > teeth > jaw → +1; jaw > teeth > lips → −1; else 0 |
| accelerator_osc `[E]` | AO − SMA5(AO) |
| gator_osc `[E]` | \|jaw−teeth\| − \|teeth−lips\|, colored histogram |
| williams_fractal_high/low | 5-bar pattern: bar[2] is highest/lowest of 5 |
| mfi_bw `[V][E]` | Market Facilitation Index = (H−L)/TI |

### 4.12 Cross-Features LOB × TI (9)

| ID | Description |
|----|-------------|
| ofi1_x_rsi14 | ofi_1 × rsi_14 / 100 |
| ofi1_x_adx14 | ofi_1 × adx_14 / 100 |
| ofi1_x_dist_high20 | ofi_1 × dist_high_20 |
| depth_imb_x_bb_width | depth_imb × bb_width_20 |
| depth_imb_x_realized_vol | depth_imb × realized_vol_20 |
| microprice_dev_x_atr | microprice_dev / (atr_14 + ε) |
| queue_imb_x_rsi14 | queue_imb × (rsi_14 − 50) / 50 |
| level_drain_x_vol | level_drain × realized_vol_20 |
| spread_x_rvol | spread × rvol_20 |

### 4.13 Crossover / Pattern Signals (12)

| ID | Description |
|----|-------------|
| rsi_ob | rsi_14 > 70 → +1; < 30 → −1; else 0 |
| bb_breakout | mid > upper_band → +1; < lower_band → −1; else 0 |
| stoch_crossover | %K crosses above %D → +1; below → −1 |
| macd_crossover | MACD crosses above signal → +1; below → −1 |
| sar_flip | Parabolic SAR changes side → ±1 |
| vortex_crossover | +VI crosses −VI → ±1 |
| ema_ribbon | EMA5>EMA12>EMA26>EMA50 → +1; reverse → −1 |
| cci_extreme | CCI > +100 → +1; < −100 → −1; else 0 |
| alligator_signal | alligator_open (from §4.11) |
| squeeze_fire | squeeze_signal transitions 1→0 (energy release) → ±1 |
| supertrend_flip | SuperTrend direction changes → ±1 |
| williams_fractal_signal | fractal_high → −1 resistance; fractal_low → +1 support |

---

## 5. CLI Command

```
orderflow techanalysis <data_dir> <reports_dir>
```

Scans `data_dir` for `*_features.csv` files (same as `analyze` command). Produces one report per pair plus a cross-pair summary.

---

## 6. Report Format (P7)

```
═══════════════════════════════════════════════════════════════
  ORDERFLOW-RS  P7  TECH INDICATOR ANALYSIS  REPORT
═══════════════════════════════════════════════════════════════

SECTION 1: Dataset Summary
  Pair, rows, ts range, mid_price range (reconstructed), avg dt

SECTION 2: IC Table  (Spearman IC | ICIR)  — ~152 indicators × 4 horizons
  Sorted by |IC| at r_300s within each category block

SECTION 3: Top-10 IC Rankings per Horizon
  Which indicators have highest |IC| at r_1s, r_5s, r_30s, r_300s

SECTION 4: Inter-Indicator Correlation Matrix (compact)
  Printed as ASCII heat map: H=|r|>0.7, M=|r|>0.4, L=|r|<0.4
  Lists redundant pairs (|r|>0.8) explicitly

SECTION 5: S/R Proximity Analysis
  For dist_high_20, dist_low_20, round_number_prox:
    IC vs r_1s binned by quartile of proximity
    Tests: "is IC higher when price is near the level?"

SECTION 6: Regime-Conditional IC
  Top-5 TIs sliced by spread quartile (low/mid-low/mid-high/high vol)

SECTION 7: Hypothesis Verdicts
  H1: Any TI achieves IC > 0.05 at ≥1 horizon
  H2: That TI outperforms best existing LOB feature (ofi_1 IC)
  H3: S/R proximity significantly modulates IC (p < 0.05)
  H4: Cross-features (LOB × TI) outperform either component alone
```

Cross-pair summary written to `reports/techanalysis/summary_report.txt`:
- Table: which TIs have IC > 0.05 in ≥3/10 pairs per horizon
- Heatmap: IC by (pair × top-20 TIs)

---

## 7. Module Design

### `src/analysis/techind.rs`

All functions are pure and stateless:

```rust
/// Compute SMA over a price slice. Returns None for warm-up bars.
pub fn sma(prices: &[f64], period: usize) -> Vec<Option<f64>>

/// Compute EMA. alpha = 2/(period+1).
pub fn ema(prices: &[f64], period: usize) -> Vec<Option<f64>>

/// Compute RSI using Wilder's smoothing.
pub fn rsi(prices: &[f64], period: usize) -> Vec<Option<f64>>

// ... one function per indicator family, parameterized
```

A top-level function assembles all indicators into a named map:

```rust
pub struct TechRow {
    pub ts: i64,
    pub indicators: HashMap<&'static str, Option<f64>>,
}

pub fn compute_all(rows: &[FeatureRow]) -> Vec<TechRow>
```

### `src/analysis/techreport.rs`

```rust
pub fn run_techanalysis(data_dir: &str, reports_dir: &str) -> Result<()>
pub fn format_tech_report(symbol: &str, rows: &[TechRow], feature_rows: &[FeatureRow]) -> String
pub fn format_cross_pair_summary(summaries: &[PairSummary]) -> String
```

---

## 8. Error Handling

- Indicators with insufficient data (fewer than `period` rows) return `None` for early bars — same contract as existing features.
- Division by zero in indicators like Amihud, Kyle's λ guarded with `if denom.abs() < f64::EPSILON { None }`.
- Missing `trade_intensity` (None) → volume-based indicators return None for that bar.
- Missing `spread` → `[E]` indicators return None.
- CSV rows with `gap_flag=1` are excluded from IC computation (not from TI rolling windows — gaps are treated as continuations for window state).

---

## 9. Testing

Three tests in `techind.rs`:

1. **`sma_correctness`** — manual 5-bar SMA on known prices, assert exact values.
2. **`rsi_bounds`** — RSI output always in [0, 100] or None for 500 random prices.
3. **`compute_all_no_panic`** — feed 1000 rows of synthetic data through `compute_all`, assert no panic and at least 50% non-None values for core indicators.

One test in `techreport.rs`:

4. **`format_report_contains_sections`** — assert output contains "P7", "Top-10", "S/R Proximity".

---

## 10. Dependencies

No new crate dependencies. All math is `std` + `ndarray` (already in `Cargo.toml`). FFT for dominant cycle period uses a hand-rolled DFT (O(n²), acceptable for ≤100 bins). If this proves slow, can swap in `rustfft` crate.
