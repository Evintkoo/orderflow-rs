use anyhow::Result;
use std::fs;
use std::path::Path;
use crate::analysis::techreport::{load_techdata, compute_ic_table, write_ic_report, write_summary_report};
use crate::analysis::techind::compute_all;
use crate::analysis::report::load_csv;

fn collect_feature_csvs(dir: &str) -> Vec<std::path::PathBuf> {
    let mut results = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else { return results; };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            results.extend(collect_feature_csvs(path.to_str().unwrap_or("")));
        } else if path.file_name().and_then(|f| f.to_str()).unwrap_or("").ends_with("_features.csv") {
            results.push(path);
        }
    }
    results
}

pub fn run(data_dir: &str, reports_dir: &str) -> Result<()> {
    let root = Path::new(data_dir).canonicalize().unwrap_or_else(|_| Path::new(data_dir).to_path_buf());
    let csv_paths = collect_feature_csvs(data_dir);
    let mut per_pair: Vec<(String, Vec<crate::analysis::techreport::IcRow>)> = Vec::new();

    for path in csv_paths {
        let fname = path.file_name().and_then(|f| f.to_str()).unwrap_or("").to_string();
        let base = fname.trim_end_matches("_features.csv");

        // Build symbol as path relative to data_dir root, minus the filename suffix.
        // e.g. data/dukascopy/EURUSD_features.csv → "dukascopy/EURUSD"
        //      data/EURUSD_features.csv            → "EURUSD"
        let symbol = {
            let abs = path.canonicalize().unwrap_or_else(|_| path.clone());
            if let Ok(rel) = abs.strip_prefix(&root) {
                rel.to_str()
                    .map(|s| s.trim_end_matches("_features.csv").to_string())
                    .unwrap_or_else(|| base.to_string())
            } else {
                base.to_string()
            }
        };

        let path_str = path.to_str().unwrap_or("").to_string();

        let mut td = match load_techdata(&path_str) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("skip {symbol}: {e}");
                continue;
            }
        };

        // Extract OFI_1 column
        let ofi: Vec<f64> = load_csv(Path::new(&path_str))
            .unwrap_or_default()
            .iter()
            .map(|r| r.ofi_1.unwrap_or(0.0))
            .collect();

        // Pad or truncate ofi to match td.prices length
        let n = td.prices.len();
        let ofi_aligned: Vec<f64> = if ofi.len() >= n {
            ofi[..n].to_vec()
        } else {
            let mut v = ofi;
            v.resize(n, 0.0);
            v
        };

        compute_all(&mut td, &ofi_aligned);
        let rows = compute_ic_table(&td);
        write_ic_report(&symbol, &rows, reports_dir)?;
        println!("{symbol}: {} indicators computed", td.indicators.len());
        per_pair.push((symbol, rows));
    }

    write_summary_report(&per_pair, reports_dir)?;
    println!("Summary written to {reports_dir}/summary_techanalysis.csv");
    Ok(())
}
