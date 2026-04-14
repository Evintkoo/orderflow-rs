//! Statistical utilities: Spearman IC, OLS with Newey-West HAC SEs,
//! rolling IC, IC decay curve fitting, ADF stationarity pre-check.

use ndarray::{Array1, Array2, s};

/// Compute the rank of each element in `x` (1-based, average for ties).
pub fn rank(x: &Array1<f64>) -> Array1<f64> {
    let n = x.len();
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| x[a].partial_cmp(&x[b]).unwrap_or(std::cmp::Ordering::Equal));

    let mut ranks = Array1::zeros(n);
    let mut i = 0;
    while i < n {
        let mut j = i + 1;
        while j < n && (x[idx[j]] - x[idx[i]]).abs() < f64::EPSILON {
            j += 1;
        }
        let avg_rank = (i + 1 + j) as f64 / 2.0; // 1-based average
        for k in i..j {
            ranks[idx[k]] = avg_rank;
        }
        i = j;
    }
    ranks
}

/// Spearman rank correlation between `x` and `y`.
///
/// Returns None if fewer than 2 finite pairs or if std dev is zero.
pub fn spearman_ic(x: &[f64], y: &[f64]) -> Option<f64> {
    assert_eq!(x.len(), y.len());

    // Filter to finite pairs
    let (xs, ys): (Vec<f64>, Vec<f64>) = x
        .iter()
        .zip(y.iter())
        .filter(|(a, b)| a.is_finite() && b.is_finite())
        .map(|(&a, &b)| (a, b))
        .unzip();

    if xs.len() < 2 {
        return None;
    }

    let xa = Array1::from(xs);
    let ya = Array1::from(ys);
    let rx = rank(&xa);
    let ry = rank(&ya);

    pearson_r(&rx, &ry)
}

/// Pearson correlation between two rank arrays.
fn pearson_r(rx: &Array1<f64>, ry: &Array1<f64>) -> Option<f64> {
    let _n = rx.len() as f64;
    let mx = rx.mean()?;
    let my = ry.mean()?;
    let dx = rx - mx;
    let dy = ry - my;
    let cov = (&dx * &dy).sum();
    let var_x = (&dx * &dx).sum();
    let var_y = (&dy * &dy).sum();
    let denom = (var_x * var_y).sqrt();
    if denom < f64::EPSILON {
        return None;
    }
    Some(cov / denom)
}

/// Information Coefficient = mean(IC_series) / std(IC_series).
pub fn icir(ic_series: &[f64]) -> Option<f64> {
    let finite: Vec<f64> = ic_series.iter().copied().filter(|x| x.is_finite()).collect();
    if finite.len() < 2 {
        return None;
    }
    let mean = finite.iter().sum::<f64>() / finite.len() as f64;
    let var = finite.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (finite.len() - 1) as f64;
    let std = var.sqrt();
    if std < f64::EPSILON {
        return None;
    }
    Some(mean / std)
}

/// OLS regression: y = X β + ε.
///
/// Returns (β, residuals).
/// X must have full column rank. Uses normal equations: β = (XᵀX)⁻¹ Xᵀy.
pub fn ols(x: &Array2<f64>, y: &Array1<f64>) -> Option<(Array1<f64>, Array1<f64>)> {
    let xt = x.t();
    let xtx = xt.dot(x);
    let xty = xt.dot(y);
    let beta = solve_linear(&xtx, &xty)?;
    let fitted = x.dot(&beta);
    let residuals = y - &fitted;
    Some((beta, residuals))
}

/// Newey-West HAC standard errors for OLS.
///
/// lag: number of lags for HAC kernel (typically τ / Δt).
/// Returns standard errors for each coefficient.
pub fn newey_west_se(
    x: &Array2<f64>,
    residuals: &Array1<f64>,
    lag: usize,
) -> Option<Array1<f64>> {
    let n = x.nrows();
    let k = x.ncols();

    if n <= k + lag {
        return None;
    }

    // Score matrix S = X * residuals (element-wise broadcast)
    let mut scores: Array2<f64> = Array2::zeros((n, k));
    for i in 0..n {
        for j in 0..k {
            scores[[i, j]] = x[[i, j]] * residuals[i];
        }
    }

    // HAC sandwich: Ω = Σ_{l=-L}^{L} w(l) * Γ(l)
    // Bartlett kernel: w(l) = 1 - |l|/(L+1)
    let mut omega: Array2<f64> = Array2::zeros((k, k));

    // l=0 term
    let g0 = scores.t().dot(&scores) / n as f64;
    omega = omega + &g0;

    for l in 1..=lag {
        let w = 1.0 - l as f64 / (lag + 1) as f64;
        // Γ(l) = (1/n) Σᵢ sᵢ sᵢ₋ₗᵀ
        let s_lead = scores.slice(s![l.., ..]);
        let s_lag = scores.slice(s![..n - l, ..]);
        let gamma_l = s_lag.t().dot(&s_lead) / n as f64;
        omega = omega + &((&gamma_l + &gamma_l.t()) * w);
    }

    // (XᵀX)⁻¹ Ω (XᵀX)⁻¹  (sandwich)
    let xtx = x.t().dot(x);
    let xtx_inv = invert_2x2_or_diag(&xtx)?;
    let sandwich = xtx_inv.dot(&omega).dot(&xtx_inv);

    // Standard errors = sqrt(diag(sandwich) * n)  [heteroskedasticity-robust scaling]
    let se: Array1<f64> = (0..k)
        .map(|j| (sandwich[[j, j]] * n as f64).sqrt())
        .collect::<Vec<_>>()
        .into();

    Some(se)
}

/// Rolling Spearman IC with a fixed window.
///
/// Returns vector of (center_index, IC) pairs.
pub fn rolling_ic(
    features: &[f64],
    returns: &[f64],
    window: usize,
    step: usize,
) -> Vec<(usize, Option<f64>)> {
    assert_eq!(features.len(), returns.len());
    let n = features.len();
    let mut result = Vec::new();
    let mut i = 0;
    while i + window <= n {
        let f_window = &features[i..i + window];
        let r_window = &returns[i..i + window];
        let ic = spearman_ic(f_window, r_window);
        result.push((i + window / 2, ic));
        i += step;
    }
    result
}

/// Fit exponential decay: IC(τ) = A · exp(−τ/τ*) + B via non-linear least squares
/// (simplified: grid search over τ* then linear fit for A, B).
///
/// Returns (A, tau_star, B, r_squared).
pub fn fit_ic_decay(
    taus: &[f64],
    ics: &[f64],
) -> Option<(f64, f64, f64, f64)> {
    if taus.len() < 3 || taus.len() != ics.len() {
        return None;
    }

    let mut best = None;
    let mut best_r2 = f64::NEG_INFINITY;

    // Grid search over τ* in [0.5s, 600s]
    let tau_star_candidates: Vec<f64> = (1..=1200)
        .map(|i| i as f64 * 0.5)
        .collect();

    for tau_star in &tau_star_candidates {
        // Given τ*, fit A and B via OLS on: IC(τ) = A · exp(−τ/τ*) + B
        // Linearize: let u_i = exp(−τ_i/τ*), then IC_i = A·u_i + B
        let n = taus.len();
        let mut x_mat = Array2::zeros((n, 2));
        for i in 0..n {
            x_mat[[i, 0]] = (-taus[i] / tau_star).exp();
            x_mat[[i, 1]] = 1.0;
        }
        let y_vec = Array1::from(ics.to_vec());
        if let Some((beta, residuals)) = ols(&x_mat, &y_vec) {
            let ss_res = residuals.mapv(|e| e * e).sum();
            let y_mean = y_vec.mean().unwrap_or(0.0);
            let ss_tot = y_vec.mapv(|v| (v - y_mean).powi(2)).sum();
            let r2 = if ss_tot > f64::EPSILON { 1.0 - ss_res / ss_tot } else { 0.0 };
            if r2 > best_r2 {
                best_r2 = r2;
                best = Some((beta[0], *tau_star, beta[1], r2));
            }
        }
    }

    best
}

// ── Numeric helpers ───────────────────────────────────────────────────────────

/// Solve Ax = b via Cholesky (positive-definite) or fall back to pseudo-inverse
/// for small matrices. Specialized for k ≤ 10 (typical regression case).
fn solve_linear(a: &Array2<f64>, b: &Array1<f64>) -> Option<Array1<f64>> {
    let k = a.nrows();
    assert_eq!(a.ncols(), k);
    assert_eq!(b.len(), k);

    // Gaussian elimination with partial pivoting
    let mut mat: Vec<Vec<f64>> = (0..k)
        .map(|i| {
            let mut row: Vec<f64> = (0..k).map(|j| a[[i, j]]).collect();
            row.push(b[i]);
            row
        })
        .collect();

    for col in 0..k {
        // Find pivot
        let max_row = (col..k)
            .max_by(|&r1, &r2| mat[r1][col].abs().partial_cmp(&mat[r2][col].abs()).unwrap())?;
        mat.swap(col, max_row);

        let pivot = mat[col][col];
        if pivot.abs() < 1e-12 {
            return None; // Singular
        }

        for row in (col + 1)..k {
            let factor = mat[row][col] / pivot;
            for j in col..=k {
                let val = mat[col][j] * factor;
                mat[row][j] -= val;
            }
        }
    }

    // Back substitution
    let mut x = vec![0.0_f64; k];
    for i in (0..k).rev() {
        x[i] = mat[i][k];
        for j in (i + 1)..k {
            x[i] -= mat[i][j] * x[j];
        }
        x[i] /= mat[i][i];
    }

    Some(Array1::from(x))
}

/// Matrix inverse for small matrices (up to 4×4 analytically, larger via Gauss).
fn invert_2x2_or_diag(a: &Array2<f64>) -> Option<Array2<f64>> {
    let k = a.nrows();
    let mut aug: Vec<Vec<f64>> = (0..k)
        .map(|i| {
            let mut row: Vec<f64> = (0..k).map(|j| a[[i, j]]).collect();
            let id_row: Vec<f64> = (0..k).map(|j| if j == i { 1.0 } else { 0.0 }).collect();
            row.extend(id_row);
            row
        })
        .collect();

    // Gauss-Jordan elimination
    for col in 0..k {
        let max_row = (col..k)
            .max_by(|&r1, &r2| aug[r1][col].abs().partial_cmp(&aug[r2][col].abs()).unwrap())?;
        aug.swap(col, max_row);

        let pivot = aug[col][col];
        if pivot.abs() < 1e-12 {
            return None;
        }

        for j in 0..2 * k {
            aug[col][j] /= pivot;
        }

        for row in 0..k {
            if row != col {
                let factor = aug[row][col];
                for j in 0..2 * k {
                    let val = aug[col][j] * factor;
                    aug[row][j] -= val;
                }
            }
        }
    }

    let mut inv = Array2::zeros((k, k));
    for i in 0..k {
        for j in 0..k {
            inv[[i, j]] = aug[i][k + j];
        }
    }
    Some(inv)
}

/// Variance Inflation Factor for column j of feature matrix X.
///
/// VIF_j = 1 / (1 − R²_j) where R²_j comes from regressing column j on all other columns.
/// Returns None if matrix is too small or singular.
pub fn vif(x: &Array2<f64>, j: usize) -> Option<f64> {
    let n = x.nrows();
    let k = x.ncols();
    if n < k + 2 || k < 2 || j >= k {
        return None;
    }

    // Build y = column j, X_other = all other columns + intercept
    let y: Array1<f64> = x.column(j).to_owned();
    let n_other = k - 1 + 1; // (k-1) features + intercept
    let mut x_other = Array2::zeros((n, n_other));
    let mut col_idx = 0;
    for c in 0..k {
        if c == j {
            continue;
        }
        for r in 0..n {
            x_other[[r, col_idx]] = x[[r, c]];
        }
        col_idx += 1;
    }
    // intercept column
    for r in 0..n {
        x_other[[r, k - 1]] = 1.0;
    }

    let (_, residuals) = ols(&x_other, &y)?;
    let y_mean = y.mean()?;
    let ss_res = residuals.mapv(|e| e * e).sum();
    let ss_tot = y.mapv(|v| (v - y_mean).powi(2)).sum();
    if ss_tot < f64::EPSILON {
        return None;
    }
    let r2 = 1.0 - ss_res / ss_tot;
    // Clamp R² to avoid division by zero or negative VIF
    let r2_clamped = r2.min(1.0 - f64::EPSILON);
    Some(1.0 / (1.0 - r2_clamped))
}

/// Augmented Dickey-Fuller test statistic for stationarity.
///
/// Regresses Δy(t) on y(t−1) + lags Δy terms (num_lags).
/// Returns the t-statistic for the y(t−1) coefficient.
/// H0: unit root (non-stationary). Reject if t_stat < −2.86 (5% level).
pub fn adf_test(y: &[f64], num_lags: usize) -> Option<f64> {
    let n = y.len();
    // Need at least: 1 (y[t-1]) + num_lags (diff lags) + 1 (intercept) + enough obs
    let min_obs = num_lags + 3;
    if n < min_obs + 1 {
        return None;
    }

    // Compute first differences Δy(t) = y(t) - y(t-1)
    let dy: Vec<f64> = (1..n).map(|i| y[i] - y[i - 1]).collect(); // length n-1

    // Start index: we need y(t-1) and num_lags lagged diffs
    // t ranges from num_lags+1 to n-1 (using 0-based original indices)
    // dy index for t: dy[t-1] = Δy(t), dy[t-2] = Δy(t-1), ...
    // y[t-1] is straightforward.
    let start = num_lags; // dy index start (dy has indices 0..n-2)
    let n_reg = dy.len() - start; // number of regression observations
    if n_reg < num_lags + 3 {
        return None;
    }

    let n_cols = 1 + num_lags + 1; // y(t-1), Δy(t-1..t-num_lags), intercept
    let mut x_mat = Array2::zeros((n_reg, n_cols));
    let mut y_vec = Array1::zeros(n_reg);

    for (i, t) in (start..dy.len()).enumerate() {
        // Response: Δy(t+1) = dy[t]
        y_vec[i] = dy[t];
        // y(t): original y at index t (dy[t-1] = y[t]-y[t-1] means t is dy index+1 in original)
        // dy[t] corresponds to y[t+1] - y[t], so y at index t (0-based original) = y[t]
        x_mat[[i, 0]] = y[t]; // y(t) as the lagged level
        // Lagged differences: Δy(t), Δy(t-1), ..., Δy(t-num_lags+1)
        for lag in 0..num_lags {
            x_mat[[i, 1 + lag]] = dy[t - 1 - lag];
        }
        // Intercept
        x_mat[[i, n_cols - 1]] = 1.0;
    }

    let (beta, residuals) = ols(&x_mat, &y_vec)?;
    let se = newey_west_se(&x_mat, &residuals, num_lags.max(1))?;
    if se[0] < f64::EPSILON {
        return None;
    }
    Some(beta[0] / se[0])
}

/// Ljung-Box Q statistic for residual autocorrelation up to `max_lag` lags.
///
/// Q = n(n+2) Σ_{k=1}^{h} ρ̂_k² / (n−k)
/// Returns (Q_stat, degrees_of_freedom). Reject H0 (white noise) if Q > χ²(h, 0.05).
pub fn ljung_box(residuals: &[f64], max_lag: usize) -> Option<(f64, usize)> {
    let n = residuals.len();
    if n < max_lag + 2 || max_lag == 0 {
        return None;
    }

    let mean = residuals.iter().sum::<f64>() / n as f64;
    let demeaned: Vec<f64> = residuals.iter().map(|&r| r - mean).collect();

    let variance: f64 = demeaned.iter().map(|&e| e * e).sum();
    if variance < f64::EPSILON {
        return None;
    }

    let mut q = 0.0;
    for k in 1..=max_lag {
        // ρ̂_k = Σ_{t=k}^{n-1} e_t * e_{t-k} / Σ e_t²
        let autocov: f64 = (k..n).map(|t| demeaned[t] * demeaned[t - k]).sum();
        let rho_k = autocov / variance;
        q += rho_k * rho_k / (n - k) as f64;
    }

    q *= n as f64 * (n as f64 + 2.0);
    Some((q, max_lag))
}

/// Regime-conditional IC: stratify data by volatility quartile.
///
/// `regime_var` is used as the regime proxy (e.g., spread).
/// Returns Vec of (quartile_label, ic) where quartile_label is 1..=4.
pub fn regime_ic(
    features: &[f64],
    returns: &[f64],
    regime_var: &[f64],
) -> Vec<(usize, Option<f64>)> {
    let n = features.len();
    assert_eq!(returns.len(), n);
    assert_eq!(regime_var.len(), n);

    // Filter to finite triples
    let mut triples: Vec<(f64, f64, f64)> = features.iter()
        .zip(returns.iter())
        .zip(regime_var.iter())
        .filter(|((f, r), v)| f.is_finite() && r.is_finite() && v.is_finite())
        .map(|((&f, &r), &v)| (f, r, v))
        .collect();

    if triples.len() < 4 {
        return vec![
            (1, None), (2, None), (3, None), (4, None),
        ];
    }

    // Sort by regime_var
    triples.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

    let m = triples.len();
    let q_size = m / 4;

    let mut result = Vec::with_capacity(4);
    for q in 0..4 {
        let start = q * q_size;
        let end = if q == 3 { m } else { (q + 1) * q_size };
        let slice = &triples[start..end];
        let f_vals: Vec<f64> = slice.iter().map(|t| t.0).collect();
        let r_vals: Vec<f64> = slice.iter().map(|t| t.1).collect();
        let ic = spearman_ic(&f_vals, &r_vals);
        result.push((q + 1, ic));
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;

    #[test]
    fn rank_basic() {
        let x = array![3.0, 1.0, 2.0];
        let r = rank(&x);
        assert_eq!(r[0], 3.0); // 3.0 is rank 3
        assert_eq!(r[1], 1.0); // 1.0 is rank 1
        assert_eq!(r[2], 2.0); // 2.0 is rank 2
    }

    #[test]
    fn rank_ties_average() {
        let x = array![1.0, 1.0, 3.0];
        let r = rank(&x);
        // Tied 1.0s get average rank (1+2)/2=1.5
        assert!((r[0] - 1.5).abs() < 1e-10);
        assert!((r[1] - 1.5).abs() < 1e-10);
        assert_eq!(r[2], 3.0);
    }

    #[test]
    fn spearman_perfect_positive() {
        let x: Vec<f64> = (0..10).map(|i| i as f64).collect();
        let y = x.clone();
        let ic = spearman_ic(&x, &y).unwrap();
        assert!((ic - 1.0).abs() < 1e-10);
    }

    #[test]
    fn spearman_perfect_negative() {
        let x: Vec<f64> = (0..10).map(|i| i as f64).collect();
        let y: Vec<f64> = (0..10).map(|i| (9 - i) as f64).collect();
        let ic = spearman_ic(&x, &y).unwrap();
        assert!((ic + 1.0).abs() < 1e-10);
    }

    #[test]
    fn spearman_ignores_nan() {
        let x = vec![1.0, 2.0, f64::NAN, 4.0];
        let y = vec![1.0, 2.0, 3.0, 4.0];
        // Should not panic, drops NaN pair
        let ic = spearman_ic(&x, &y);
        assert!(ic.is_some());
    }

    #[test]
    fn ols_simple_regression() {
        // y = 2x + 1
        let x_data: Vec<f64> = (0..10).map(|i| i as f64).collect();
        let y_data: Vec<f64> = x_data.iter().map(|&x| 2.0 * x + 1.0).collect();

        let mut x_mat = Array2::zeros((10, 2));
        for (i, &xi) in x_data.iter().enumerate() {
            x_mat[[i, 0]] = xi;
            x_mat[[i, 1]] = 1.0;
        }
        let y_vec = Array1::from(y_data);
        let (beta, residuals) = ols(&x_mat, &y_vec).unwrap();

        assert!((beta[0] - 2.0).abs() < 1e-8, "slope should be 2.0, got {}", beta[0]);
        assert!((beta[1] - 1.0).abs() < 1e-8, "intercept should be 1.0, got {}", beta[1]);
        assert!(residuals.mapv(|e| e.abs()).sum() < 1e-8);
    }

    #[test]
    fn icir_basic() {
        let ic = vec![0.1, 0.2, 0.1, 0.15, 0.12];
        let ir = icir(&ic).unwrap();
        assert!(ir > 0.0); // Positive IC series → positive ICIR
    }

    #[test]
    fn rolling_ic_produces_results() {
        let f: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let r: Vec<f64> = (0..100).map(|i| i as f64 + 0.1).collect();
        let ics = rolling_ic(&f, &r, 30, 10);
        assert!(!ics.is_empty());
        // Perfect rank correlation → IC = 1.0 for all windows
        for (_, ic) in &ics {
            assert!((ic.unwrap() - 1.0).abs() < 1e-8);
        }
    }
}
