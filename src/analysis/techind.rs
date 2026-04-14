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

// ─── Moving Average Helpers ───────────────────────────────────────────────────

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
            .map(|j| prices[i + 1 + j - period] * (j + 1) as f64)
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
    (0..n).map(|i| match (e1[i], e2[i]) {
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
    let diff: Vec<f64> = (0..n).map(|i| match (w1[i], w2[i]) {
        (Some(x), Some(y)) => 2.0 * x - y,
        _ => f64::NAN,
    }).collect();
    let h = wma(&diff, sqrt_p);
    h.iter().map(|v| match v {
        Some(x) if x.is_finite() => Some(*x),
        _ => None,
    }).collect()
}

/// Kaufman Adaptive Moving Average (KAMA).
pub fn kama(prices: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    if period >= n { return out; }
    let fast_k = 2.0 / 3.0;
    let slow_k = 2.0 / 31.0;
    let mut k_val = prices[period];
    out[period] = Some(k_val);
    for i in (period + 1)..n {
        let direction = (prices[i] - prices[i - period]).abs();
        let volatility: f64 = (0..period)
            .map(|j| (prices[i - j] - prices[i - j - 1]).abs())
            .sum();
        let er = if volatility > 1e-12 { direction / volatility } else { 0.0 };
        let sc = (er * (fast_k - slow_k) + slow_k).powi(2);
        k_val = k_val + sc * (prices[i] - k_val);
        out[i] = Some(k_val);
    }
    out
}

/// Golden cross: Some(1.0) on bar where fast EMA crosses above slow EMA, else Some(0.0).
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

/// Death cross: Some(1.0) on bar where fast EMA crosses below slow EMA.
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

// ─── Trend Strength ───────────────────────────────────────────────────────────

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

/// Wilder-smoothed Average True Range.
pub fn atr(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = highs.len();
    let mut out = vec![None; n];
    if period == 0 || period >= n { return out; }
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

/// ADX with +DI and -DI. Returns (adx, plus_di, minus_di).
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

    // Wilder smoothing seed (sum of first `period` bars starting at index 1)
    let mut s_tr    = tr[1..=period].iter().sum::<f64>();
    let mut s_plus  = dm_plus[1..=period].iter().sum::<f64>();
    let mut s_minus = dm_minus[1..=period].iter().sum::<f64>();
    let alpha = 1.0 / period as f64;

    let calc_di = |sm: f64, st: f64| if st > 0.0 { 100.0 * sm / st } else { 0.0 };

    let pdi0 = calc_di(s_plus,  s_tr);
    let mdi0 = calc_di(s_minus, s_tr);
    pdi_v[period] = Some(pdi0);
    mdi_v[period] = Some(mdi0);
    let sum_di0 = pdi0 + mdi0;
    let dx0 = if sum_di0 > 0.0 { 100.0 * (pdi0 - mdi0).abs() / sum_di0 } else { 0.0 };

    let mut dx_buf: Vec<f64> = vec![dx0];

    for i in (period+1)..n {
        s_tr    = s_tr    * (1.0 - alpha) + tr[i];
        s_plus  = s_plus  * (1.0 - alpha) + dm_plus[i];
        s_minus = s_minus * (1.0 - alpha) + dm_minus[i];
        let pdi = calc_di(s_plus,  s_tr);
        let mdi = calc_di(s_minus, s_tr);
        pdi_v[i] = Some(pdi);
        mdi_v[i] = Some(mdi);
        let sum_di = pdi + mdi;
        let dx = if sum_di > 0.0 { 100.0 * (pdi - mdi).abs() / sum_di } else { 0.0 };
        dx_buf.push(dx);
        if dx_buf.len() >= period {
            let start = dx_buf.len() - period;
            let adx_val = dx_buf[start..].iter().sum::<f64>() / period as f64;
            adx_v[i] = Some(adx_val);
        }
    }
    (adx_v, pdi_v, mdi_v)
}

/// ADX only (convenience wrapper).
pub fn adx(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> Vec<Option<f64>> {
    adx_full(highs, lows, closes, period).0
}

/// Aroon Up: 100 * (position of highest high in window) / period.
pub fn aroon_up(highs: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = highs.len();
    let mut out = vec![None; n];
    if period >= n { return out; }
    for i in period..n {
        let window = &highs[(i - period)..=i];
        let max_pos = window.iter().enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(j, _)| j)
            .unwrap_or(0);
        out[i] = Some(100.0 * max_pos as f64 / period as f64);
    }
    out
}

/// Aroon Down: 100 * (position of lowest low in window) / period.
pub fn aroon_down(lows: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = lows.len();
    let mut out = vec![None; n];
    if period >= n { return out; }
    for i in period..n {
        let window = &lows[(i - period)..=i];
        let min_pos = window.iter().enumerate()
            .min_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(j, _)| j)
            .unwrap_or(0);
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

/// Supertrend signal: Some(1.0) = uptrend, Some(-1.0) = downtrend.
pub fn supertrend(highs: &[f64], lows: &[f64], closes: &[f64], period: usize, multiplier: f64)
    -> Vec<Option<f64>>
{
    let n = highs.len();
    let mut out = vec![None; n];
    let atr_v = atr(highs, lows, closes, period);
    if period == 0 || period > n { return out; }
    let mut trend = 1.0_f64;
    let mut upper = 0.0_f64;
    let mut lower = 0.0_f64;
    for i in (period-1)..n {
        if let Some(atr_val) = atr_v[i] {
            let hl2 = (highs[i] + lows[i]) / 2.0;
            let basic_upper = hl2 + multiplier * atr_val;
            let basic_lower = hl2 - multiplier * atr_val;
            if i == period - 1 {
                upper = basic_upper;
                lower = basic_lower;
            } else {
                upper = if basic_upper < upper || closes[i-1] > upper { basic_upper } else { upper };
                lower = if basic_lower > lower || closes[i-1] < lower { basic_lower } else { lower };
            }
            if closes[i] > upper {
                trend = 1.0;
            } else if closes[i] < lower {
                trend = -1.0;
            }
            out[i] = Some(trend);
        }
    }
    out
}

// ─── Momentum Oscillators ─────────────────────────────────────────────────────

/// Wilder RSI.
pub fn rsi(prices: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    if period == 0 || period >= n { return out; }
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
    if period == 0 || period > n { return out; }
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
    if period == 0 || period > n { return out; }
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
    if period == 0 || period > n { return out; }
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
    sma(&k_vals, 3).iter().map(|v| match v {
        Some(x) if x.is_finite() => Some(*x),
        _ => None,
    }).collect()
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
    if period == 0 || period >= n { return out; }
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

/// Detrended Price Oscillator: close - SMA(close, period) shifted back (period/2 + 1) bars.
pub fn dpo(prices: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    if period == 0 { return out; }
    let shift = period / 2 + 1;
    let sma_v = sma(prices, period);
    for i in (period + shift - 1)..n {
        if let Some(s) = sma_v[i.saturating_sub(shift)] {
            if s.is_finite() {
                out[i] = Some(prices[i] - s);
            }
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

// ─── Volatility ───────────────────────────────────────────────────────────────

/// Population std dev of a slice.
fn std_dev(data: &[f64]) -> f64 {
    if data.is_empty() { return 0.0; }
    let n = data.len() as f64;
    let mean = data.iter().sum::<f64>() / n;
    (data.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n).sqrt()
}

/// Bollinger Band width: (upper - lower) / middle = 2 * k * std / mean.
pub fn bb_width(prices: &[f64], period: usize, num_std: f64) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    if period == 0 || period > n { return out; }
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
    if period == 0 || period > n { return out; }
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

/// Keltner Channel width: 2 * mult * ATR / EMA.
pub fn keltner_width(highs: &[f64], lows: &[f64], closes: &[f64], period: usize, mult: f64)
    -> Vec<Option<f64>>
{
    let ema_v = ema(closes, period);
    let atr_v = atr(highs, lows, closes, period);
    let n = highs.len();
    (0..n).map(|i| match (ema_v[i], atr_v[i]) {
        (Some(e), Some(a)) if e > 1e-12 => Some(2.0 * mult * a / e),
        _ => None,
    }).collect()
}

/// Donchian Channel width: (highest_high - lowest_low) / midpoint.
pub fn donchian_width(highs: &[f64], lows: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = highs.len();
    let mut out = vec![None; n];
    if period == 0 || period > n { return out; }
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

/// Realized volatility (std dev of log returns over `period` bars).
pub fn realized_vol(prices: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    if period >= n { return out; }
    let log_ret: Vec<f64> = (1..n).map(|i|
        if prices[i-1] > 1e-12 { (prices[i] / prices[i-1]).ln() } else { 0.0 }
    ).collect();
    for i in period..n {
        let w = &log_ret[(i-period)..(i)];
        out[i] = Some(std_dev(w));
    }
    out
}

/// Parkinson volatility: sqrt(1/(4*ln2) * mean((ln(H/L))^2)).
pub fn parkinson_vol(highs: &[f64], lows: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = highs.len();
    let mut out = vec![None; n];
    if period == 0 || period > n { return out; }
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
    if period == 0 || period > n { return out; }
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

/// Squeeze signal: Some(1.0) if BB width < Keltner width (squeeze on), else Some(0.0).
pub fn squeeze_signal(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = highs.len();
    let bb = bb_width(closes, period, 2.0);
    let kc = keltner_width(highs, lows, closes, period, 1.5);
    (0..n).map(|i| match (bb[i], kc[i]) {
        (Some(b), Some(k)) => Some(if b < k { 1.0 } else { 0.0 }),
        _ => None,
    }).collect()
}
