//! IC computation and report formatting for technical indicator analysis.

use anyhow::{Context, Result};
use crate::analysis::techind::TechData;

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
