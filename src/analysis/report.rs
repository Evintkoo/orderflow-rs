//! Full P5 analysis engine: load feature CSVs, compute stats, write reports.

use std::path::Path;
use anyhow::Result;
use ndarray::{Array1, Array2};

use crate::analysis::stats::{
    spearman_ic, icir, ols, newey_west_se, fit_ic_decay,
    adf_test, vif, ljung_box, regime_ic,
};

/// A single row loaded from a feature CSV file.
#[derive(Debug, Clone)]
pub struct FeatureRow {
    pub ts:              i64,
    pub ofi_1:           Option<f64>,
    pub ofi_5:           Option<f64>,
    pub ofi_10:          Option<f64>,
    pub depth_imb:       Option<f64>,
    pub microprice_dev:  Option<f64>,
    pub queue_imb:       Option<f64>,
    pub spread:          Option<f64>,
    pub trade_intensity: Option<f64>,
    pub price_impact:    Option<f64>,
    pub level_drain:     Option<f64>,
    pub weighted_mid_slope: Option<f64>,
    pub r_1s:            Option<f64>,
    pub r_5s:            Option<f64>,
    pub r_30s:           Option<f64>,
    pub r_300s:          Option<f64>,
    pub exchange:        String,
    pub symbol:          String,
    pub is_imputed:      bool,
    pub gap_flag:        bool,
}

/// Maximum rows loaded per CSV. Files exceeding this are uniformly stride-sampled
/// during loading so we never allocate more than ~200K FeatureRows at once.
pub const MAX_LOAD_ROWS: usize = 50_000;

/// Load (at most `MAX_LOAD_ROWS`) rows from a feature CSV file, skipping header
/// and malformed lines.  Rows where is_imputed=1 or gap_flag=1 are always skipped.
/// For very large files a uniform stride keeps temporal order intact.
pub fn load_csv(path: &Path) -> Result<Vec<FeatureRow>> {
    use std::io::{BufRead, BufReader};

    // First pass: fast byte scan to count valid rows and compute stride.
    // Avoids allocating a String per line — just counts commas and newlines.
    let valid_count: usize = {
        use std::io::Read;
        let mut buf = vec![0u8; 1 << 20]; // 1 MiB read buffer
        let mut f = std::fs::File::open(path)?;
        let mut count = 0usize;
        let mut line_num = 0usize;
        // field_index tracks which comma-delimited column we're in
        let mut field = 0u8;
        // capture the last two chars of fields 20 and 21 (imputed, gap)
        let mut col20_val = 0u8; // last non-whitespace digit seen in col 20
        let mut col21_val = 0u8;

        loop {
            let n = f.read(&mut buf)?;
            if n == 0 { break; }
            for &b in &buf[..n] {
                if b == b'\n' {
                    if line_num > 0 {
                        // Check if neither imputed nor gap
                        if col20_val != b'1' && col21_val != b'1' && field >= 21 {
                            count += 1;
                        }
                    }
                    line_num += 1;
                    field = 0;
                    col20_val = 0;
                    col21_val = 0;
                } else if b == b',' {
                    field += 1;
                } else if b != b'\r' {
                    if field == 20 { col20_val = b; }
                    else if field == 21 { col21_val = b; }
                }
            }
        }
        count
    };

    let stride = if valid_count > MAX_LOAD_ROWS {
        valid_count / MAX_LOAD_ROWS
    } else {
        1
    };

    // Second pass: parse at stride using a reusable line buffer.
    let file = std::fs::File::open(path)?;
    let mut rows = Vec::with_capacity(valid_count.min(MAX_LOAD_ROWS + 64));
    let mut valid_idx = 0usize;
    let mut line_buf = String::with_capacity(512);

    fn parse_opt(s: &str) -> Option<f64> {
        let s = s.trim();
        if s.is_empty() { None } else { s.parse().ok() }
    }

    let mut reader = BufReader::new(file);
    let mut ln = 0usize;
    loop {
        line_buf.clear();
        let bytes_read = reader.read_line(&mut line_buf)?;
        if bytes_read == 0 { break; }
        if ln == 0 { ln += 1; continue; }
        ln += 1;
        let line = line_buf.trim();
        if line.is_empty() { continue; }

        let fields: Vec<&str> = line.splitn(23, ',').collect();
        if fields.len() < 22 { continue; }

        let is_imputed = fields[20].trim() == "1";
        let gap_flag   = fields[21].trim() == "1";
        if is_imputed || gap_flag { continue; }

        let idx = valid_idx;
        valid_idx += 1;
        if idx % stride != 0 { continue; }

        let ts: i64 = match fields[0].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };

        let row = FeatureRow {
            ts,
            ofi_1:              parse_opt(fields[1]),
            ofi_5:              parse_opt(fields[2]),
            ofi_10:             parse_opt(fields[3]),
            depth_imb:          parse_opt(fields[4]),
            microprice_dev:     parse_opt(fields[5]),
            queue_imb:          parse_opt(fields[6]),
            spread:             parse_opt(fields[7]),
            trade_intensity:    parse_opt(fields[8]),
            price_impact:       parse_opt(fields[9]),
            level_drain:        parse_opt(fields[10]),
            weighted_mid_slope: parse_opt(fields[11]),
            r_1s:               parse_opt(fields[12]),
            r_5s:               parse_opt(fields[13]),
            r_30s:              parse_opt(fields[14]),
            r_300s:             parse_opt(fields[15]),
            exchange:           fields[18].trim().to_string(),
            symbol:             fields[19].trim().to_string(),
            is_imputed,
            gap_flag,
        };
        rows.push(row);
    }

    Ok(rows)
}

// ── Feature extraction helpers ─────────────────────────────────────────────────

const FEATURE_NAMES: &[&str] = &[
    "ofi_1", "ofi_5", "ofi_10", "depth_imb", "microprice_dev",
    "queue_imb", "spread", "trade_intensity", "price_impact",
    "level_drain", "weighted_mid_slope",
];

const HORIZON_NAMES: &[&str] = &["r_1s", "r_5s", "r_30s", "r_300s"];
const HORIZON_SECS:  &[usize] = &[1, 5, 30, 300];

fn extract_feature(rows: &[FeatureRow], feat: usize) -> Vec<f64> {
    rows.iter().map(|r| match feat {
        0  => r.ofi_1.unwrap_or(f64::NAN),
        1  => r.ofi_5.unwrap_or(f64::NAN),
        2  => r.ofi_10.unwrap_or(f64::NAN),
        3  => r.depth_imb.unwrap_or(f64::NAN),
        4  => r.microprice_dev.unwrap_or(f64::NAN),
        5  => r.queue_imb.unwrap_or(f64::NAN),
        6  => r.spread.unwrap_or(f64::NAN),
        7  => r.trade_intensity.unwrap_or(f64::NAN),
        8  => r.price_impact.unwrap_or(f64::NAN),
        9  => r.level_drain.unwrap_or(f64::NAN),
        10 => r.weighted_mid_slope.unwrap_or(f64::NAN),
        _  => f64::NAN,
    }).collect()
}

fn extract_return(rows: &[FeatureRow], horizon: usize) -> Vec<f64> {
    rows.iter().map(|r| match horizon {
        0 => r.r_1s.unwrap_or(f64::NAN),
        1 => r.r_5s.unwrap_or(f64::NAN),
        2 => r.r_30s.unwrap_or(f64::NAN),
        3 => r.r_300s.unwrap_or(f64::NAN),
        _ => f64::NAN,
    }).collect()
}

/// Filter two parallel slices to finite pairs.
fn finite_pairs(x: &[f64], y: &[f64]) -> (Vec<f64>, Vec<f64>) {
    x.iter().zip(y.iter())
        .filter(|(a, b)| a.is_finite() && b.is_finite())
        .map(|(&a, &b)| (a, b))
        .unzip()
}

/// Build [feature, intercept] design matrix from finite pairs.
fn design_matrix_1d(feat: &[f64], ret: &[f64]) -> Option<(Array2<f64>, Array1<f64>)> {
    let (xs, ys) = finite_pairs(feat, ret);
    if xs.len() < 4 {
        return None;
    }
    let n = xs.len();
    let mut x_mat = Array2::zeros((n, 2));
    for i in 0..n {
        x_mat[[i, 0]] = xs[i];
        x_mat[[i, 1]] = 1.0;
    }
    Some((x_mat, Array1::from(ys)))
}

fn fmt_opt(v: Option<f64>, prec: usize) -> String {
    match v {
        Some(x) => format!("{x:.prec$}"),
        None    => "   N/A  ".to_string(),
    }
}

/// Run the full P5 analysis on a Vec<FeatureRow> and write a report.
pub fn run_analysis(rows: &[FeatureRow]) -> String {
    let mut out = String::new();

    macro_rules! w {
        ($($arg:tt)*) => { out.push_str(&format!($($arg)*)); out.push('\n'); }
    }

    let n = rows.len();
    if n == 0 {
        w!("No data rows to analyze.");
        return out;
    }

    let exchange = rows.first().map(|r| r.exchange.as_str()).unwrap_or("?");
    let symbol   = rows.first().map(|r| r.symbol.as_str()).unwrap_or("?");
    let ts_min   = rows.iter().map(|r| r.ts).min().unwrap_or(0);
    let ts_max   = rows.iter().map(|r| r.ts).max().unwrap_or(0);

    // ──────────────────────────────────────────────────────────────────
    // Section 1: Dataset summary
    // ──────────────────────────────────────────────────────────────────
    w!("═══════════════════════════════════════════════════════════════");
    w!("  ORDERFLOW-RS  P5  ANALYSIS  REPORT");
    w!("═══════════════════════════════════════════════════════════════");
    w!("");
    w!("SECTION 1: Dataset Summary");
    w!("─────────────────────────────────────────────────────────────");
    w!("  Exchange   : {exchange}");
    w!("  Symbol     : {symbol}");
    w!("  Rows       : {n}");
    w!("  ts range   : {ts_min} .. {ts_max}");
    w!("");

    // ──────────────────────────────────────────────────────────────────
    // Section 2: Feature IC table
    // ──────────────────────────────────────────────────────────────────
    w!("SECTION 2: Feature IC Table  (IC | ICIR | OLS_beta | NW_tstat)");
    w!("─────────────────────────────────────────────────────────────");

    // header
    let hdr: String = HORIZON_NAMES.iter()
        .map(|h| format!("{:^36}", h))
        .collect::<Vec<_>>()
        .join(" | ");
    w!("{:<20} | {}", "Feature", hdr);
    w!("{:-<20}-+-{:-<147}", "", "");

    // Precompute IC table data
    // ic_table[feat][horizon] = (ic, icir, beta, nw_tstat)
    let mut ic_table: Vec<Vec<(Option<f64>, Option<f64>, Option<f64>, Option<f64>)>> =
        vec![vec![(None, None, None, None); 4]; 11];

    // We'll accumulate rolling IC windows per feature for ICIR
    // Use entire dataset as single "window" for now (ICIR from rolling 252-obs windows)
    let rolling_window = 252;

    for fi in 0..11 {
        let feat_vals = extract_feature(rows, fi);
        for hi in 0..4 {
            let ret_vals = extract_return(rows, hi);
            let ic = spearman_ic(&feat_vals, &ret_vals);

            // Rolling IC for ICIR
            let n_data = feat_vals.len();
            let mut rolling_ics = Vec::new();
            if n_data >= rolling_window {
                let step = rolling_window / 4;
                let mut i = 0;
                while i + rolling_window <= n_data {
                    let f_w = &feat_vals[i..i + rolling_window];
                    let r_w = &ret_vals[i..i + rolling_window];
                    if let Some(ic_w) = spearman_ic(f_w, r_w) {
                        rolling_ics.push(ic_w);
                    }
                    i += step;
                }
            }
            let icir_val = if rolling_ics.len() >= 2 { icir(&rolling_ics) } else { None };

            // OLS beta + NW t-stat
            let lag = HORIZON_SECS[hi] * 10;
            let (beta_val, tstat_val) = if let Some((x_mat, y_vec)) =
                design_matrix_1d(&feat_vals, &ret_vals)
            {
                if let Some((beta, resid)) = ols(&x_mat, &y_vec) {
                    let tstat = newey_west_se(&x_mat, &resid, lag.max(1))
                        .and_then(|se| {
                            if se[0] > f64::EPSILON { Some(beta[0] / se[0]) } else { None }
                        });
                    (Some(beta[0]), tstat)
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            };

            ic_table[fi][hi] = (ic, icir_val, beta_val, tstat_val);
        }
    }

    for fi in 0..11 {
        let row_str: String = (0..4).map(|hi| {
            let (ic, ir, beta, tstat) = ic_table[fi][hi];
            format!("{:>8} {:>7} {:>10} {:>7}",
                fmt_opt(ic,   4),
                fmt_opt(ir,   3),
                fmt_opt(beta, 5),
                fmt_opt(tstat, 2))
        }).collect::<Vec<_>>().join(" | ");
        w!("{:<20} | {}", FEATURE_NAMES[fi], row_str);
    }
    w!("");

    // ──────────────────────────────────────────────────────────────────
    // Section 3: IC decay curve
    // ──────────────────────────────────────────────────────────────────
    w!("SECTION 3: IC Decay Curve  (fitted A·exp(−τ/τ*) + B)");
    w!("─────────────────────────────────────────────────────────────");
    w!("{:<20}  {:>8}  {:>8}  {:>8}  {:>8}", "Feature", "A", "tau*", "B", "R2");
    w!("{:-<20}  {:->8}  {:->8}  {:->8}  {:->8}", "", "", "", "", "");

    let taus = [1.0_f64, 5.0, 30.0, 300.0];
    for fi in 0..11 {
        let ics: Vec<f64> = (0..4).map(|hi| {
            ic_table[fi][hi].0.unwrap_or(f64::NAN)
        }).collect();
        match fit_ic_decay(&taus, &ics) {
            Some((a, tau_star, b, r2)) => {
                w!("{:<20}  {:>8.4}  {:>8.2}  {:>8.4}  {:>8.4}",
                    FEATURE_NAMES[fi], a, tau_star, b, r2);
            }
            None => {
                w!("{:<20}  {:>8}  {:>8}  {:>8}  {:>8}",
                    FEATURE_NAMES[fi], "N/A", "N/A", "N/A", "N/A");
            }
        }
    }
    w!("");

    // ──────────────────────────────────────────────────────────────────
    // Section 4: ADF stationarity
    // ──────────────────────────────────────────────────────────────────
    w!("SECTION 4: ADF Stationarity  (num_lags=5, reject H0 if t < −2.86)");
    w!("─────────────────────────────────────────────────────────────");
    w!("{:<20}  {:>10}  {:>10}", "Feature", "t-stat", "Stationary?");
    w!("{:-<20}  {:->10}  {:->10}", "", "", "");

    for fi in 0..11 {
        let feat_vals = extract_feature(rows, fi);
        let finite: Vec<f64> = feat_vals.iter().copied().filter(|x| x.is_finite()).collect();
        match adf_test(&finite, 5) {
            Some(t) => {
                let verdict = if t < -2.86 { "YES" } else { "NO " };
                w!("{:<20}  {:>10.4}  {:>10}", FEATURE_NAMES[fi], t, verdict);
            }
            None => {
                w!("{:<20}  {:>10}  {:>10}", FEATURE_NAMES[fi], "N/A", "N/A");
            }
        }
    }
    w!("");

    // ──────────────────────────────────────────────────────────────────
    // Section 5: VIF
    // ──────────────────────────────────────────────────────────────────
    w!("SECTION 5: VIF (Variance Inflation Factors, available-feature model)");
    w!("─────────────────────────────────────────────────────────────");
    w!("{:<20}  {:>10}", "Feature", "VIF");
    w!("{:-<20}  {:->10}", "", "");

    let ret_vals_300 = extract_return(rows, 3);
    let all_feats: Vec<Vec<f64>> = (0..11).map(|fi| extract_feature(rows, fi)).collect();

    // Identify features with ≥ 10% finite values AND non-trivial variance.
    // Constant features (e.g. trade_intensity=1.0 always) cause singular VIF matrix.
    let min_valid = (n / 10).max(14);
    let candidate_idx: Vec<usize> = (0..11).filter(|&fi| {
        let finite: Vec<f64> = all_feats[fi].iter().copied().filter(|x| x.is_finite()).collect();
        if finite.len() < min_valid { return false; }
        let mean = finite.iter().sum::<f64>() / finite.len() as f64;
        let var = finite.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / finite.len() as f64;
        var > 1e-20
    }).collect();

    // Deduplicate near-perfectly correlated features (|r| > 0.9999) — in L1 tick data
    // ofi_1/ofi_5/ofi_10 are identical, causing singular matrices.  Keep first occurrence.
    let valid_feat_idx: Vec<usize> = {
        let mut kept: Vec<usize> = Vec::new();
        for &fi in &candidate_idx {
            let is_dup = kept.iter().any(|&kj| {
                let pairs: Vec<(f64,f64)> = all_feats[fi].iter().zip(all_feats[kj].iter())
                    .filter(|(a,b)| a.is_finite() && b.is_finite())
                    .map(|(&a,&b)| (a,b)).collect();
                if pairs.len() < 10 { return false; }
                let n_p = pairs.len() as f64;
                let (mx, my): (f64,f64) = pairs.iter().fold((0.,0.), |acc,&(a,b)| (acc.0+a, acc.1+b));
                let (mx, my) = (mx/n_p, my/n_p);
                let (sxx,syy,sxy) = pairs.iter().fold((0.,0.,0.), |acc,&(a,b)| {
                    (acc.0+(a-mx).powi(2), acc.1+(b-my).powi(2), acc.2+(a-mx)*(b-my))
                });
                if sxx < 1e-20 || syy < 1e-20 { return true; } // degenerate
                let r = sxy / (sxx * syy).sqrt();
                r.abs() > 0.9999
            });
            if !is_dup { kept.push(fi); }
        }
        kept
    };

    // Find rows where all valid features and r_300s are finite
    let complete_mask: Vec<bool> = (0..n).map(|i| {
        ret_vals_300[i].is_finite() && valid_feat_idx.iter().all(|&fi| all_feats[fi][i].is_finite())
    }).collect();
    let n_complete: usize = complete_mask.iter().filter(|&&b| b).count();
    let n_valid_feats = valid_feat_idx.len();

    let mut vif_map: Vec<Option<f64>> = vec![None; 11];
    if n_complete >= n_valid_feats + 3 && n_valid_feats >= 2 {
        let mut x_mat = Array2::zeros((n_complete, n_valid_feats));
        let mut row_idx = 0;
        for i in 0..n {
            if complete_mask[i] {
                for (col, &fi) in valid_feat_idx.iter().enumerate() {
                    x_mat[[row_idx, col]] = all_feats[fi][i];
                }
                row_idx += 1;
            }
        }
        for (col, &fi) in valid_feat_idx.iter().enumerate() {
            vif_map[fi] = vif(&x_mat, col);
        }
    }

    for fi in 0..11 {
        w!("{:<20}  {:>10}", FEATURE_NAMES[fi], fmt_opt(vif_map[fi], 3));
    }
    w!("");

    // ──────────────────────────────────────────────────────────────────
    // Section 6: Ljung-Box on r_300s residuals
    // ──────────────────────────────────────────────────────────────────
    w!("SECTION 6: Ljung-Box  (r_300s residuals from multivariate OLS, max_lag=20)");
    w!("─────────────────────────────────────────────────────────────");

    if n_complete >= 25 && n_valid_feats >= 1 {
        let mut y_vec_data = Vec::with_capacity(n_complete);
        let n_cols = n_valid_feats + 1; // features + intercept
        let mut x_mat_data = Array2::zeros((n_complete, n_cols));
        let mut row_idx = 0;
        for i in 0..n {
            if complete_mask[i] {
                for (col, &fi) in valid_feat_idx.iter().enumerate() {
                    x_mat_data[[row_idx, col]] = all_feats[fi][i];
                }
                x_mat_data[[row_idx, n_valid_feats]] = 1.0; // intercept
                y_vec_data.push(ret_vals_300[i]);
                row_idx += 1;
            }
        }
        let y_arr = Array1::from(y_vec_data);
        w!("  Model: {} features + intercept, n={n_complete}", n_valid_feats);
        match ols(&x_mat_data, &y_arr) {
            Some((_, resid)) => {
                let resid_vec: Vec<f64> = resid.to_vec();
                match ljung_box(&resid_vec, 20) {
                    Some((q, df)) => {
                        // χ²(20, 0.05) ≈ 31.41
                        let reject = if q > 31.41 { "YES (autocorrelation present)" }
                                     else { "NO  (residuals ~white noise)" };
                        w!("  Q-stat (h=20): {q:.4}   df: {df}");
                        w!("  Reject H0?  : {reject}");
                    }
                    None => { w!("  Ljung-Box: insufficient data"); }
                }
            }
            None => { w!("  Multivariate OLS failed (singular matrix)"); }
        }
    } else {
        w!("  Insufficient complete rows (n_complete={n_complete}, valid_feats={n_valid_feats})");
    }
    w!("");

    // ──────────────────────────────────────────────────────────────────
    // Section 7: Regime-conditional IC
    // ──────────────────────────────────────────────────────────────────
    w!("SECTION 7: Regime-Conditional IC  (OFI_1 vs r_300s, spread quartile)");
    w!("─────────────────────────────────────────────────────────────");
    w!("{:<12}  {:>10}", "Quartile", "IC");
    w!("{:-<12}  {:->10}", "", "");

    let ofi1_vals  = extract_feature(rows, 0);
    let spread_vals = extract_feature(rows, 6);
    let regime_results = regime_ic(&ofi1_vals, &ret_vals_300, &spread_vals);
    for (q, ic) in &regime_results {
        let label = match q {
            1 => "Q1 (low vol)",
            2 => "Q2",
            3 => "Q3",
            4 => "Q4 (hi vol)",
            _ => "Q?",
        };
        w!("{:<12}  {:>10}", label, fmt_opt(*ic, 4));
    }
    w!("");

    // ──────────────────────────────────────────────────────────────────
    // Section 8: Hypothesis verdicts
    // ──────────────────────────────────────────────────────────────────
    w!("SECTION 8: Hypothesis Verdicts");
    w!("─────────────────────────────────────────────────────────────");

    // H1: OFI(1) IC > 0.05 at ≥1 horizon
    let h1_pass = (0..4).any(|hi| {
        ic_table[0][hi].0.map_or(false, |ic| ic > 0.05)
    });
    w!("  H1 (OFI_1 IC > 0.05 at ≥1 horizon)        : {}", if h1_pass { "PASS" } else { "FAIL" });
    let h1_detail: String = (0..4).map(|hi| {
        format!("{}={}", HORIZON_NAMES[hi], fmt_opt(ic_table[0][hi].0, 4))
    }).collect::<Vec<_>>().join(", ");
    w!("     Detail: {h1_detail}");

    // H2: τ* < 60s and R² > 0.90 for OFI(1)
    let ofi1_ics: Vec<f64> = (0..4).map(|hi| ic_table[0][hi].0.unwrap_or(f64::NAN)).collect();
    let h2_result = fit_ic_decay(&taus, &ofi1_ics);
    let h2_pass = h2_result.map_or(false, |(_, tau_star, _, r2)| tau_star < 60.0 && r2 > 0.90);
    let h2_detail = h2_result.map_or("N/A".to_string(), |(_, tau_star, _, r2)| {
        format!("tau*={tau_star:.1}s, R2={r2:.4}")
    });
    w!("  H2 (IC decay τ* < 60s and R² > 0.90)       : {}", if h2_pass { "PASS" } else { "FAIL" });
    w!("     Detail: {h2_detail}");

    // H3: partial R² > 0.01 for depth_imb added to OFI(1) model
    // Compare R² of [ofi_1 + intercept] vs [ofi_1 + depth_imb + intercept]
    let h3_pass = compute_partial_r2(rows, &ret_vals_300, &all_feats)
        .map_or(false, |pr2| pr2 > 0.01);
    let h3_val  = compute_partial_r2(rows, &ret_vals_300, &all_feats);
    w!("  H3 (depth_imb partial R² > 0.01 over OFI_1) : {}", if h3_pass { "PASS" } else { "FAIL" });
    w!("     Detail: partial_R2={}", fmt_opt(h3_val, 5));

    // H4: IC drops in high-vol regime (Q4 < Q1)
    let (q1_ic, q4_ic) = if regime_results.len() == 4 {
        (regime_results[0].1, regime_results[3].1)
    } else {
        (None, None)
    };
    let h4_pass = match (q1_ic, q4_ic) {
        (Some(q1), Some(q4)) => q4 < q1,
        _ => false,
    };
    w!("  H4 (OFI_1 IC drops in high-vol regime Q4)   : {}", if h4_pass { "PASS" } else { "FAIL" });
    w!("     Detail: Q1_IC={}, Q4_IC={}", fmt_opt(q1_ic, 4), fmt_opt(q4_ic, 4));

    // H5: sign accuracy of microprice_dev > 55%
    let h5_acc = compute_sign_accuracy(&extract_feature(rows, 4), &ret_vals_300);
    let h5_pass = h5_acc.map_or(false, |acc| acc > 0.55);
    w!("  H5 (microprice_dev sign accuracy > 55%)      : {}", if h5_pass { "PASS" } else { "FAIL" });
    w!("     Detail: sign_acc={}", fmt_opt(h5_acc, 4));

    w!("");
    w!("═══════════════════════════════════════════════════════════════");
    w!("  END OF REPORT");
    w!("═══════════════════════════════════════════════════════════════");

    out
}

/// Compute partial R² for adding depth_imb to an OFI(1) model.
fn compute_partial_r2(
    rows: &[FeatureRow],
    ret_300: &[f64],
    all_feats: &[Vec<f64>],
) -> Option<f64> {
    let n = rows.len();
    // Indices: 0=ofi_1, 3=depth_imb
    let mask: Vec<bool> = (0..n).map(|i| {
        ret_300[i].is_finite() && all_feats[0][i].is_finite() && all_feats[3][i].is_finite()
    }).collect();
    let n_ok: usize = mask.iter().filter(|&&b| b).count();
    if n_ok < 6 {
        return None;
    }

    // Restricted: [ofi_1, intercept]
    let mut x_restr = Array2::zeros((n_ok, 2));
    let mut y_restr = Array1::zeros(n_ok);
    let mut row = 0;
    for i in 0..n {
        if mask[i] {
            x_restr[[row, 0]] = all_feats[0][i];
            x_restr[[row, 1]] = 1.0;
            y_restr[row] = ret_300[i];
            row += 1;
        }
    }

    // Unrestricted: [ofi_1, depth_imb, intercept]
    let mut x_unr = Array2::zeros((n_ok, 3));
    let mut y_unr = Array1::zeros(n_ok);
    row = 0;
    for i in 0..n {
        if mask[i] {
            x_unr[[row, 0]] = all_feats[0][i];
            x_unr[[row, 1]] = all_feats[3][i];
            x_unr[[row, 2]] = 1.0;
            y_unr[row] = ret_300[i];
            row += 1;
        }
    }

    let (_, resid_r) = ols(&x_restr, &y_restr)?;
    let (_, resid_u) = ols(&x_unr,  &y_unr)?;
    let ss_r = resid_r.mapv(|e| e * e).sum();
    let ss_u = resid_u.mapv(|e| e * e).sum();
    if ss_r < f64::EPSILON {
        return None;
    }
    Some((ss_r - ss_u) / ss_r)
}

/// Compute sign accuracy: fraction of times sign(feature) == sign(return).
fn compute_sign_accuracy(feat: &[f64], ret: &[f64]) -> Option<f64> {
    let (xs, ys): (Vec<f64>, Vec<f64>) = feat.iter().zip(ret.iter())
        .filter(|(f, r)| f.is_finite() && r.is_finite() && **f != 0.0 && **r != 0.0)
        .map(|(&f, &r)| (f, r))
        .unzip();
    if xs.len() < 2 {
        return None;
    }
    let n = xs.len() as f64;
    let correct = xs.iter().zip(ys.iter())
        .filter(|(&f, &r)| f.signum() == r.signum())
        .count() as f64;
    Some(correct / n)
}

/// Scan a directory for *_features.csv files, load them all, run analysis per file, write reports.
pub fn analyze_directory(data_dir: &Path, output_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(output_dir)?;

    let entries = std::fs::read_dir(data_dir)?;
    let mut found = 0usize;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let fname = path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        if !fname.ends_with("_features.csv") {
            continue;
        }

        eprintln!("[analyze] loading {}", path.display());
        match load_csv(&path) {
            Ok(mut rows) => {
                eprintln!("[analyze]   {} rows loaded", rows.len());
                if rows.is_empty() {
                    eprintln!("[analyze]   skipping (empty)");
                    continue;
                }
                let report = run_analysis(&rows);
                let stem = fname.trim_end_matches("_features.csv");
                let out_name = format!("{stem}_report.txt");
                let out_path = output_dir.join(&out_name);
                std::fs::write(&out_path, &report)?;
                eprintln!("[analyze]   report written to {}", out_path.display());
                found += 1;
            }
            Err(e) => {
                eprintln!("[analyze]   error loading {}: {e}", path.display());
            }
        }
    }

    if found == 0 {
        eprintln!("[analyze] no *_features.csv files found in {}", data_dir.display());
    } else {
        eprintln!("[analyze] wrote {found} report(s)");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_rows(n: usize) -> Vec<FeatureRow> {
        (0..n).map(|i| {
            let x = i as f64 / n as f64;
            FeatureRow {
                ts: i as i64 * 100_000,
                ofi_1:   Some(x - 0.5),
                ofi_5:   Some((x - 0.5) * 0.8),
                ofi_10:  Some((x - 0.5) * 0.6),
                depth_imb: Some(x * 2.0 - 1.0),
                microprice_dev: Some((x - 0.5) * 0.3),
                queue_imb: Some(x - 0.5),
                spread:  Some(0.01 + x * 0.01),
                trade_intensity: Some(x),
                price_impact:   Some(x * 0.1),
                level_drain:    Some(x * 0.05),
                weighted_mid_slope: Some((x - 0.5) * 0.2),
                r_1s:   Some((x - 0.5) * 0.01),
                r_5s:   Some((x - 0.5) * 0.02),
                r_30s:  Some((x - 0.5) * 0.03),
                r_300s: Some((x - 0.5) * 0.05),
                exchange: "test".to_string(),
                symbol:   "TEST/USD".to_string(),
                is_imputed: false,
                gap_flag:   false,
            }
        }).collect()
    }

    #[test]
    fn run_analysis_does_not_panic() {
        let rows = make_test_rows(500);
        let report = run_analysis(&rows);
        assert!(report.contains("SECTION 1"));
        assert!(report.contains("SECTION 2"));
        assert!(report.contains("SECTION 8"));
    }

    #[test]
    fn run_analysis_empty_rows() {
        let report = run_analysis(&[]);
        assert!(report.contains("No data rows"));
    }

    #[test]
    fn load_csv_roundtrip_skips_imputed() {
        // Write a tiny CSV and re-load it
        use std::io::Write;
        let tmp = std::env::temp_dir().join("test_features.csv");
        let mut f = std::fs::File::create(&tmp).unwrap();
        writeln!(f, "ts,ofi_1,ofi_5,ofi_10,depth_imb,microprice_dev,queue_imb,spread,trade_intensity,price_impact,level_drain,weighted_mid_slope,r_1s,r_5s,r_30s,r_300s,sign_1s,sign_5s,exchange,symbol,is_imputed,gap_flag").unwrap();
        writeln!(f, "1000,0.1,0.2,0.3,0.4,0.5,0.6,0.007,1.0,0.01,0.02,0.03,0.001,0.002,0.003,0.005,1,1,binance,BTCUSDT,0,0").unwrap();
        writeln!(f, "2000,0.1,0.2,0.3,0.4,0.5,0.6,0.007,1.0,0.01,0.02,0.03,0.001,0.002,0.003,0.005,1,1,binance,BTCUSDT,1,0").unwrap();
        drop(f);
        let rows = load_csv(&tmp).unwrap();
        assert_eq!(rows.len(), 1, "imputed row should be filtered");
        assert_eq!(rows[0].ts, 1000);
        std::fs::remove_file(&tmp).ok();
    }
}
