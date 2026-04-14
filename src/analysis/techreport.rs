//! IC computation and report formatting for technical indicator analysis.

use std::collections::HashMap;
use std::io::Write as IoWrite;
use anyhow::{Context, Result};
use crate::analysis::stats::spearman_ic;
use crate::analysis::techind::TechData;

/// IC result for one indicator against all four return horizons.
#[derive(Clone)]
pub struct IcRow {
    pub name:    String,
    pub ic_1s:   Option<f64>,
    pub ic_5s:   Option<f64>,
    pub ic_30s:  Option<f64>,
    pub ic_300s: Option<f64>,
}

/// Compute Spearman IC for every indicator in `td` vs each return horizon.
/// Only uses rows where both indicator and return are non-None and finite.
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

/// Write IC table for one symbol to `<reports_dir>/<symbol>_techanalysis.csv`.
pub fn write_ic_report(symbol: &str, rows: &[IcRow], reports_dir: &str) -> Result<()> {
    let path = format!("{}/{}_techanalysis.csv", reports_dir, symbol);
    // create all parent dirs (handles "subdir/SYMBOL" patterns)
    if let Some(parent) = std::path::Path::new(&path).parent() {
        std::fs::create_dir_all(parent)?;
    }
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

/// Write cross-pair average IC to `<reports_dir>/summary_techanalysis.csv`.
/// Rows sorted by abs(avg_ic_5s) descending.
pub fn write_summary_report(per_pair: &[(String, Vec<IcRow>)], reports_dir: &str) -> Result<()> {
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

/// Load a `*_features.csv` file and reconstruct price/OHLCV series.
///
/// Mid-price reconstruction:
///   p[0] = 1.0
///   p[i+1] = p[i] * (1 + r_1s[i].unwrap_or(0) * 0.1)
/// This gives a normalized price index aligned with the return series.
pub fn load_techdata(csv_path: &str) -> Result<TechData> {
    use std::path::Path;
    let rows = crate::analysis::report::load_csv(Path::new(csv_path))
        .with_context(|| format!("load_techdata: failed to read {csv_path}"))?;

    let n = rows.len();
    let mut ts     = Vec::with_capacity(n);
    let mut prices = Vec::with_capacity(n);
    let mut highs  = Vec::with_capacity(n);
    let mut lows   = Vec::with_capacity(n);
    let mut volumes = Vec::with_capacity(n);
    let mut spreads = Vec::with_capacity(n);
    let mut r_1s   = Vec::with_capacity(n);
    let mut r_5s   = Vec::with_capacity(n);
    let mut r_30s  = Vec::with_capacity(n);
    let mut r_300s = Vec::with_capacity(n);

    let mut p = 1.0_f64;
    for (i, row) in rows.iter().enumerate() {
        ts.push(row.ts);
        prices.push(p);
        let sp = row.spread.unwrap_or(0.0).abs();
        highs.push(p + sp * 0.5);
        lows.push((p - sp * 0.5).max(1e-10));
        volumes.push(row.trade_intensity.unwrap_or(0.0).abs());
        spreads.push(sp);
        r_1s.push(row.r_1s);
        r_5s.push(row.r_5s);
        r_30s.push(row.r_30s);
        r_300s.push(row.r_300s);
        // Advance price for next bar
        if i + 1 < n {
            p = (p * (1.0 + row.r_1s.unwrap_or(0.0) * 0.1)).max(1e-10);
        }
    }

    Ok(TechData::new(ts, prices, highs, lows, volumes, spreads,
                     r_1s, r_5s, r_30s, r_300s))
}
