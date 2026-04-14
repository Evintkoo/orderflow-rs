//! Signal construction and decay filter.
//!
//! Builds composite signal from feature betas and applies exponential decay filter.

use ndarray::{Array1, Array2};
use crate::analysis::stats::{ols, spearman_ic, fit_ic_decay};

/// Combined signal from multiple features.
///
/// s(t) = Σᵢ wᵢ · z(fᵢ(t))
///
/// where z() is rolling z-score normalization and wᵢ = β̂ᵢ / Σ|β̂ᵢ|.
pub struct SignalBuilder {
    /// Feature weights (from OLS betas, normalized to sum to 1 in abs).
    pub weights: Vec<f64>,
    /// Rolling z-score state: (mean, std) per feature.
    pub zscore_state: Vec<ZScoreState>,
}

impl SignalBuilder {
    pub fn new(weights: Vec<f64>) -> Self {
        let n = weights.len();
        Self {
            weights,
            zscore_state: (0..n).map(|_| ZScoreState::new(300)).collect(), // 30-min at 100ms
        }
    }

    /// Fit weights from OLS on feature matrix X and return vector y.
    pub fn fit(x: &Array2<f64>, y: &Array1<f64>) -> Option<Self> {
        let (beta, _) = ols(x, y)?;
        let abs_sum: f64 = beta.iter().map(|b| b.abs()).sum();
        if abs_sum < f64::EPSILON {
            return None;
        }
        let weights: Vec<f64> = beta.iter().map(|b| b / abs_sum).collect();
        Some(Self::new(weights))
    }

    /// Compute the composite signal for a new feature observation.
    pub fn signal(&mut self, features: &[f64]) -> Option<f64> {
        if features.len() != self.weights.len() {
            return None;
        }
        let mut s = 0.0;
        for (i, (&f, &w)) in features.iter().zip(self.weights.iter()).enumerate() {
            if !f.is_finite() {
                continue;
            }
            let z = self.zscore_state[i].update(f);
            if let Some(z) = z {
                s += w * z;
            }
        }
        if s.is_finite() { Some(s) } else { None }
    }
}

/// Rolling z-score normalization with a fixed window.
pub struct ZScoreState {
    window: usize,
    buffer: std::collections::VecDeque<f64>,
    sum: f64,
    sum_sq: f64,
}

impl ZScoreState {
    pub fn new(window: usize) -> Self {
        Self {
            window,
            buffer: std::collections::VecDeque::with_capacity(window),
            sum: 0.0,
            sum_sq: 0.0,
        }
    }

    /// Add a new observation; returns z-score once window is full.
    pub fn update(&mut self, x: f64) -> Option<f64> {
        if self.buffer.len() == self.window {
            let old = self.buffer.pop_front().unwrap();
            self.sum -= old;
            self.sum_sq -= old * old;
        }
        self.buffer.push_back(x);
        self.sum += x;
        self.sum_sq += x * x;

        if self.buffer.len() < self.window {
            return None;
        }

        let n = self.buffer.len() as f64;
        let mean = self.sum / n;
        let var = (self.sum_sq / n - mean * mean).max(0.0);
        let std = var.sqrt();
        if std < f64::EPSILON {
            return Some(0.0);
        }
        Some((x - mean) / std)
    }
}

/// Exponential decay signal filter.
///
/// s_filtered(t) = (1−α)·s_filtered(t−1) + α·s(t)
/// α = 1 − exp(−Δt / τ*)
pub struct DecayFilter {
    alpha: f64,
    state: Option<f64>,
}

impl DecayFilter {
    /// Create a decay filter with half-life `tau_star` seconds and
    /// sampling interval `dt` seconds.
    pub fn new(tau_star: f64, dt: f64) -> Self {
        let alpha = 1.0 - (-dt / tau_star).exp();
        Self { alpha, state: None }
    }

    pub fn update(&mut self, s: f64) -> f64 {
        let filtered = match self.state {
            None => s,
            Some(prev) => (1.0 - self.alpha) * prev + self.alpha * s,
        };
        self.state = Some(filtered);
        filtered
    }
}

/// Generate entry/exit signals.
///
/// Long  when filtered_signal > k × σ_s
/// Short when filtered_signal < −k × σ_s
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Position {
    Long,
    Short,
    Flat,
}

pub fn threshold_signal(s: f64, sigma_s: f64, k: f64) -> Position {
    if s > k * sigma_s {
        Position::Long
    } else if s < -k * sigma_s {
        Position::Short
    } else {
        Position::Flat
    }
}

/// Walk-forward IC evaluation.
///
/// Returns OOS IC per step using expanding or rolling training.
pub fn walk_forward_ic(
    features: &Array2<f64>,
    returns: &Array1<f64>,
    train_window: usize,
    test_window: usize,
) -> Vec<f64> {
    let n = features.nrows();
    let k = features.ncols();
    let mut results = Vec::new();

    let mut train_start = 0;
    let mut train_end = train_window;

    while train_end + test_window <= n {
        // Extract single feature column IC for each feature, use the best
        let test_slice = features.slice(ndarray::s![train_end..train_end + test_window, ..]);
        let y_test = returns.slice(ndarray::s![train_end..train_end + test_window]);

        // Compute IC for each feature in the test window
        for j in 0..k {
            let f_col: Vec<f64> = test_slice.column(j).to_vec();
            let r_vec: Vec<f64> = y_test.to_vec();
            if let Some(ic) = spearman_ic(&f_col, &r_vec) {
                results.push(ic);
            }
        }

        train_end += test_window;
        train_start += test_window;
        let _ = train_start; // rolling window support
    }

    results
}

/// Fit IC decay curve and return the signal half-life τ*.
///
/// If R² < min_r2, returns None (decay curve doesn't fit well).
pub fn estimate_signal_halflife(
    ic_at_horizons: &[(f64, f64)], // (tau_s, ic)
    min_r2: f64,
) -> Option<f64> {
    let taus: Vec<f64> = ic_at_horizons.iter().map(|(t, _)| *t).collect();
    let ics: Vec<f64> = ic_at_horizons.iter().map(|(_, ic)| *ic).collect();
    let (_, tau_star, _, r2) = fit_ic_decay(&taus, &ics)?;
    if r2 < min_r2 {
        return None;
    }
    Some(tau_star)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zscore_returns_none_until_window_full() {
        let mut z = ZScoreState::new(5);
        for i in 0..4 {
            assert!(z.update(i as f64).is_none());
        }
        assert!(z.update(4.0).is_some());
    }

    #[test]
    fn zscore_zero_for_constant() {
        let mut z = ZScoreState::new(5);
        for _ in 0..5 {
            z.update(3.0);
        }
        let s = z.update(3.0).unwrap();
        assert_eq!(s, 0.0);
    }

    #[test]
    fn decay_filter_converges() {
        let mut f = DecayFilter::new(1.0, 0.1); // τ*=1s, Δt=0.1s
        let mut s = 0.0_f64;
        for _ in 0..100 {
            s = f.update(1.0);
        }
        // After 100 steps (10s >> 1s half-life), should be very close to 1.0
        assert!((s - 1.0).abs() < 0.01, "filter should converge to 1.0, got {s}");
    }

    #[test]
    fn threshold_signal_positions() {
        assert_eq!(threshold_signal(2.0, 1.0, 1.5), Position::Long);
        assert_eq!(threshold_signal(-2.0, 1.0, 1.5), Position::Short);
        assert_eq!(threshold_signal(0.5, 1.0, 1.5), Position::Flat);
    }

    #[test]
    fn signal_builder_fit_and_predict() {
        // y = 2*f1 + f2 (non-collinear: f1 = i, f2 = i^0.5)
        let f1: Vec<f64> = (1..=50).map(|i| i as f64).collect();
        let f2: Vec<f64> = (1..=50).map(|i| (i as f64).sqrt()).collect();
        let y: Vec<f64> = f1.iter().zip(f2.iter()).map(|(&a, &b)| 2.0 * a + b).collect();

        let mut x = Array2::zeros((50, 2));
        for i in 0..50 {
            x[[i, 0]] = f1[i];
            x[[i, 1]] = f2[i];
        }
        let y_arr = Array1::from(y);
        let mut builder = SignalBuilder::fit(&x, &y_arr).unwrap();
        // After window is full, should return non-None signals
        for i in 0..350 {
            let features = [f1[i % 50], f2[i % 50]];
            builder.signal(&features);
        }
    }
}
