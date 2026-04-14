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

// ─── Volume ───────────────────────────────────────────────────────────────────

/// On-Balance Volume (cumulative).
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
/// Returns raw VWAP values.
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

/// VWAP deviation: (close - VWAP) / close. Normalized for IC analysis.
pub fn vwap_deviation(highs: &[f64], lows: &[f64], closes: &[f64], volumes: &[f64]) -> Vec<Option<f64>> {
    let vwap_v = vwap(highs, lows, closes, volumes);
    closes.iter().zip(vwap_v.iter()).map(|(c, vw)| {
        if *c > 1e-12 { Some((c - vw) / c) } else { None }
    }).collect()
}

/// Money Flow Index (volume-weighted RSI of typical price).
pub fn mfi(highs: &[f64], lows: &[f64], closes: &[f64], volumes: &[f64], period: usize)
    -> Vec<Option<f64>>
{
    let n = highs.len();
    let mut out = vec![None; n];
    if period == 0 || period >= n { return out; }
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

/// Volume Price Trend: cumulative (return / prev_close) * volume.
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

/// Relative Volume: current bar / SMA(volume, period).
pub fn rvol(volumes: &[f64], period: usize) -> Vec<Option<f64>> {
    let sma_v = sma(volumes, period);
    volumes.iter().zip(sma_v.iter()).map(|(v, s)| match s {
        Some(sv) if *sv > 1e-12 => Some(v / sv),
        _ => None,
    }).collect()
}

// ─── Support / Resistance ─────────────────────────────────────────────────────

/// Distance from close to rolling highest high (fraction of close).
pub fn dist_high(highs: &[f64], closes: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = highs.len();
    let mut out = vec![None; n];
    if period == 0 || period > n { return out; }
    for i in (period-1)..n {
        let hh = highs[(i+1-period)..=i].iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        if closes[i] > 1e-12 {
            out[i] = Some((hh - closes[i]) / closes[i]);
        }
    }
    out
}

/// Distance from close to rolling lowest low (fraction of close).
pub fn dist_low(lows: &[f64], closes: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = lows.len();
    let mut out = vec![None; n];
    if period == 0 || period > n { return out; }
    for i in (period-1)..n {
        let ll = lows[(i+1-period)..=i].iter().cloned().fold(f64::INFINITY, f64::min);
        if closes[i] > 1e-12 {
            out[i] = Some((closes[i] - ll) / closes[i]);
        }
    }
    out
}

/// Classic pivot distance: (close - pivot) / close, where pivot = (H_prev + L_prev + C_prev)/3.
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

/// R1 distance: (close - R1) / close, R1 = 2*pivot - L_prev.
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

/// S1 distance: (close - S1) / close, S1 = 2*pivot - H_prev.
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

/// Fibonacci 61.8% level distance: (close - fib) / close.
pub fn fib_618_dist(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = highs.len();
    let mut out = vec![None; n];
    if period == 0 || period > n { return out; }
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

/// Round number proximity: distance to nearest 0.001 multiple (fraction of price).
pub fn round_number_prox(closes: &[f64]) -> Vec<Option<f64>> {
    closes.iter().map(|c| {
        if *c > 1e-12 {
            let nearest = (c / 0.001).round() * 0.001;
            Some((c - nearest).abs() / c)
        } else { None }
    }).collect()
}

// ─── Statistical ──────────────────────────────────────────────────────────────

/// Z-score: (price - SMA) / std_dev over `period` bars.
pub fn zscore(prices: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    if period == 0 || period > n { return out; }
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
    if period == 0 || period > n { return out; }
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

/// LR R² over `period` bars.
pub fn lr_r2(prices: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    if period == 0 || period > n { return out; }
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

/// LR deviation: (close - fitted value) / mean.
pub fn lr_deviation(prices: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    if period == 0 || period > n { return out; }
    let x_mean = (period as f64 - 1.0) / 2.0;
    let ss_x: f64 = (0..period).map(|j| (j as f64 - x_mean).powi(2)).sum();
    for i in (period-1)..n {
        let w = &prices[(i+1-period)..=i];
        let y_mean = w.iter().sum::<f64>() / period as f64;
        let sp: f64 = (0..period).map(|j| (j as f64 - x_mean) * (w[j] - y_mean)).sum();
        if ss_x > 1e-12 && y_mean.abs() > 1e-12 {
            let slope = sp / ss_x;
            let intercept = y_mean - slope * x_mean;
            let predicted = slope * (period as f64 - 1.0) + intercept;
            out[i] = Some((prices[i] - predicted) / y_mean);
        }
    }
    out
}

/// Autocorrelation at `lag` over a rolling `window`.
pub fn autocorr(prices: &[f64], lag: usize, window: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    if lag + window > n || window == 0 { return out; }
    for i in (lag + window - 1)..n {
        let y = &prices[(i + 1 - window)..=i];
        let x_start = i + 1 - window - lag;
        let x = &prices[x_start..(x_start + window)];
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

/// Variance ratio (Lo-MacKinlay): Var(q-return) / (q * Var(1-return)).
pub fn variance_ratio(prices: &[f64], q: usize, window: usize) -> Vec<Option<f64>> {
    let n = prices.len();
    let mut out = vec![None; n];
    if window == 0 || q == 0 || window + q > n { return out; }
    let log_ret: Vec<f64> = (1..n)
        .map(|i| if prices[i-1] > 1e-12 { (prices[i] / prices[i-1]).ln() } else { 0.0 })
        .collect();
    for i in (window + q - 1)..n {
        let r1 = &log_ret[(i - window)..i];
        let var1 = std_dev(r1).powi(2);
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

// ─── Microstructure ───────────────────────────────────────────────────────────

/// Amihud illiquidity: mean(|log_return| / volume) over `period`.
pub fn amihud_illiquidity(closes: &[f64], volumes: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = closes.len();
    let mut out = vec![None; n];
    if period == 0 || period >= n { return out; }
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

/// Roll (1984) effective spread estimate: 2 * sqrt(max(-cov(ret_t, ret_t-1), 0)).
pub fn roll_spread_est(closes: &[f64], window: usize) -> Vec<Option<f64>> {
    let n = closes.len();
    let mut out = vec![None; n];
    if window == 0 || window + 1 > n { return out; }
    let ret: Vec<f64> = (0..n).map(|i| {
        if i == 0 || closes[i-1] < 1e-12 { 0.0 } else { closes[i] - closes[i-1] }
    }).collect();
    for i in (window + 1)..n {
        let r  = &ret[(i - window)..i];
        let rl = &ret[(i - window - 1)..(i - 1)];
        let nf = window as f64;
        let mr  = r.iter().sum::<f64>()  / nf;
        let mrl = rl.iter().sum::<f64>() / nf;
        let cov: f64 = r.iter().zip(rl.iter())
            .map(|(a, b)| (a - mr) * (b - mrl))
            .sum::<f64>() / nf;
        out[i] = Some(2.0 * (-cov).max(0.0).sqrt());
    }
    out
}

/// Kyle lambda proxy: |price_change| / sum(|ofi|) over `period`.
pub fn kyle_lambda(closes: &[f64], ofi: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = closes.len();
    let mut out = vec![None; n];
    if period == 0 || period >= n { return out; }
    for i in period..n {
        let sum_ofi: f64 = ofi[(i-period)..i].iter().map(|x| x.abs()).sum();
        if sum_ofi > 1e-12 && closes[i-period] > 1e-12 {
            let dp = (closes[i] - closes[i-period]).abs();
            out[i] = Some(dp / sum_ofi);
        }
    }
    out
}

/// Effective spread proxy: spread / close (normalized half-spread).
pub fn effective_spread(spreads: &[f64], closes: &[f64]) -> Vec<Option<f64>> {
    spreads.iter().zip(closes.iter()).map(|(s, c)| {
        if *c > 1e-12 { Some(s / c) } else { None }
    }).collect()
}

/// Book pressure: OFI[i] normalized by max |OFI| in rolling `period` window.
pub fn book_pressure_ind(ofi: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = ofi.len();
    let mut out = vec![None; n];
    if period == 0 || period > n { return out; }
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

/// Information efficiency ratio: |net displacement| / total path length.
pub fn info_efficiency_ratio(closes: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = closes.len();
    let mut out = vec![None; n];
    if period == 0 || period >= n { return out; }
    for i in period..n {
        let total_path: f64 = (i-period+1..i).map(|j| (closes[j] - closes[j-1]).abs()).sum();
        let net = (closes[i] - closes[i - period]).abs();
        if total_path > 1e-12 {
            out[i] = Some(net / total_path);
        }
    }
    out
}

// ─── compute_all ──────────────────────────────────────────────────────────────

/// Compute all indicators and populate `td.indicators`.
/// `ofi` is the OFI_1 series (same length as `td.prices`).
pub fn compute_all(td: &mut TechData, ofi: &[f64]) {
    let p: Vec<f64> = td.prices.clone();
    let h: Vec<f64> = td.highs.clone();
    let l: Vec<f64> = td.lows.clone();
    let v: Vec<f64> = td.volumes.clone();
    let sp: Vec<f64> = td.spreads.clone();
    // opens: p[0] = p[1], then p[i-1]
    let mut opens = p.clone();
    if opens.len() > 1 {
        for i in (1..opens.len()).rev() { opens[i] = opens[i-1]; }
    }

    macro_rules! push_opt {
        ($name:expr, $vals:expr) => {
            td.indicators.push(IndicatorSeries { name: $name, values: $vals });
        };
    }
    macro_rules! push_raw {
        ($name:expr, $vals:expr) => {
            td.indicators.push(IndicatorSeries {
                name: $name,
                values: $vals.into_iter().map(Some).collect(),
            });
        };
    }

    // ── Moving Averages ──────────────────────────────────────────────────────
    push_opt!("sma_10",   sma(&p, 10));
    push_opt!("sma_20",   sma(&p, 20));
    push_opt!("sma_50",   sma(&p, 50));
    push_opt!("sma_100",  sma(&p, 100));
    push_opt!("sma_200",  sma(&p, 200));
    push_opt!("ema_5",    ema(&p, 5));
    push_opt!("ema_12",   ema(&p, 12));
    push_opt!("ema_20",   ema(&p, 20));
    push_opt!("ema_26",   ema(&p, 26));
    push_opt!("ema_50",   ema(&p, 50));
    push_opt!("ema_200",  ema(&p, 200));
    push_opt!("wma_14",   wma(&p, 14));
    push_opt!("dema_20",  dema(&p, 20));
    push_opt!("tema_20",  tema(&p, 20));
    push_opt!("hma_14",   hma(&p, 14));
    push_opt!("kama_10",  kama(&p, 10));
    push_opt!("golden_cross", golden_cross(&p, 5, 20));
    push_opt!("death_cross",  death_cross(&p, 5, 20));

    // ── Trend Strength ───────────────────────────────────────────────────────
    let (adx_v, pdi_v, mdi_v) = adx_full(&h, &l, &p, 14);
    push_opt!("adx_14",   adx_v);
    push_opt!("pdi_14",   pdi_v);
    push_opt!("mdi_14",   mdi_v);
    push_opt!("aroon_up_25",   aroon_up(&h, 25));
    push_opt!("aroon_down_25", aroon_down(&l, 25));
    push_opt!("choppiness_14", choppiness(&h, &l, &p, 14));
    push_opt!("supertrend_14", supertrend(&h, &l, &p, 14, 3.0));

    // ── Momentum ─────────────────────────────────────────────────────────────
    push_opt!("rsi_7",    rsi(&p, 7));
    push_opt!("rsi_14",   rsi(&p, 14));
    push_opt!("rsi_21",   rsi(&p, 21));
    push_opt!("stoch_k_14",  stoch_k(&h, &l, &p, 14));
    push_opt!("stoch_d_14",  stoch_d(&h, &l, &p, 14));
    let (macd_l, macd_s, macd_h) = macd_components(&p, 12, 26, 9);
    push_opt!("macd_line",   macd_l);
    push_opt!("macd_signal", macd_s);
    push_opt!("macd_hist",   macd_h);
    push_opt!("cci_20",      cci(&h, &l, &p, 20));
    push_opt!("williams_r_14", williams_r(&h, &l, &p, 14));
    push_opt!("roc_10",   roc(&p, 10));
    push_opt!("roc_20",   roc(&p, 20));
    push_opt!("momentum_10", momentum(&p, 10));
    push_opt!("momentum_20", momentum(&p, 20));
    push_opt!("cmo_14",   cmo(&p, 14));
    push_opt!("dpo_20",   dpo(&p, 20));
    push_opt!("awesome_osc", awesome_osc(&h, &l));

    // ── Volatility ───────────────────────────────────────────────────────────
    push_opt!("bb_width_20",   bb_width(&p, 20, 2.0));
    push_opt!("bb_pct_b_20",   bb_pct_b(&p, 20, 2.0));
    push_opt!("keltner_width_20", keltner_width(&h, &l, &p, 20, 1.5));
    push_opt!("donchian_width_20", donchian_width(&h, &l, 20));
    push_opt!("atr_14",    atr(&h, &l, &p, 14));
    push_opt!("natr_14",   natr(&h, &l, &p, 14));
    push_opt!("realized_vol_20", realized_vol(&p, 20));
    push_opt!("parkinson_vol_14", parkinson_vol(&h, &l, 14));
    push_opt!("garman_klass_20", garman_klass_vol(&h, &l, &opens, &p, 20));
    push_opt!("squeeze_signal", squeeze_signal(&h, &l, &p, 20));

    // ── Volume ───────────────────────────────────────────────────────────────
    push_raw!("obv",     obv(&p, &v));
    push_opt!("vwap_dev", vwap_deviation(&h, &l, &p, &v));
    push_opt!("mfi_14",   mfi(&h, &l, &p, &v, 14));
    push_opt!("force_index_13", force_index(&p, &v, 13));
    push_opt!("ease_of_movement_14", ease_of_movement(&h, &l, &v, 14));
    push_raw!("vpt",  vpt(&p, &v));
    push_opt!("vroc_14", vroc(&v, 14));
    push_opt!("rvol_20", rvol(&v, 20));

    // ── Support / Resistance ─────────────────────────────────────────────────
    push_opt!("dist_high_20", dist_high(&h, &p, 20));
    push_opt!("dist_high_50", dist_high(&h, &p, 50));
    push_opt!("dist_low_20",  dist_low(&l, &p, 20));
    push_opt!("dist_low_50",  dist_low(&l, &p, 50));
    push_opt!("pivot_dist",   pivot_dist(&h, &l, &p));
    push_opt!("r1_dist",      r1_dist(&h, &l, &p));
    push_opt!("s1_dist",      s1_dist(&h, &l, &p));
    push_opt!("fib_618_dist", fib_618_dist(&h, &l, &p, 50));
    push_opt!("round_number_prox", round_number_prox(&p));

    // ── Statistical ──────────────────────────────────────────────────────────
    push_opt!("zscore_10",   zscore(&p, 10));
    push_opt!("zscore_20",   zscore(&p, 20));
    push_opt!("zscore_50",   zscore(&p, 50));
    push_opt!("lr_slope_20", lr_slope(&p, 20));
    push_opt!("lr_r2_20",    lr_r2(&p, 20));
    push_opt!("lr_deviation_20", lr_deviation(&p, 20));
    push_opt!("autocorr_lag1", autocorr(&p, 1, 50));
    push_opt!("autocorr_lag5", autocorr(&p, 5, 50));
    push_opt!("variance_ratio_16", variance_ratio(&p, 16, 100));
    push_opt!("hurst_100",  hurst(&p, 100));

    // ── Microstructure ───────────────────────────────────────────────────────
    push_opt!("amihud_20",      amihud_illiquidity(&p, &v, 20));
    push_opt!("roll_spread",    roll_spread_est(&p, 20));
    push_opt!("kyle_lambda_20", kyle_lambda(&p, ofi, 20));
    push_opt!("effective_spread", effective_spread(&sp, &p));
    push_opt!("book_pressure",  book_pressure_ind(ofi, 20));
    push_opt!("spread_osc",     spread_osc(&sp, 20));
    push_opt!("spread_change_rate", spread_change_rate(&sp));
    push_opt!("info_efficiency_20", info_efficiency_ratio(&p, 20));
}
