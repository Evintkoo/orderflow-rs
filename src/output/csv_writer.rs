//! CSV output for labeled feature vectors.

use std::io::{BufWriter, Write};
use crate::pipeline::LabeledFeatureVector;

/// Write labeled feature vectors to a CSV writer.
pub fn write_csv<W: Write>(
    writer: &mut BufWriter<W>,
    rows: &[LabeledFeatureVector],
) -> std::io::Result<()> {
    // Header
    writeln!(
        writer,
        "ts,ofi_1,ofi_5,ofi_10,depth_imb,microprice_dev,queue_imb,spread,\
         trade_intensity,price_impact,level_drain,weighted_mid_slope,\
         r_1s,r_5s,r_30s,r_300s,sign_1s,sign_5s,\
         exchange,symbol,data_source,is_imputed,gap_flag"
    )?;

    for row in rows {
        let fv = &row.fv;
        writeln!(
            writer,
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            fv.ts,
            opt_f64(fv.ofi_1),
            opt_f64(fv.ofi_5),
            opt_f64(fv.ofi_10),
            opt_f64(fv.depth_imb),
            opt_f64(fv.microprice_dev),
            opt_f64(fv.queue_imb),
            opt_f64(fv.spread),
            opt_f64(fv.trade_intensity),
            opt_f64(fv.price_impact),
            opt_f64(fv.level_drain),
            opt_f64(fv.weighted_mid_slope),
            opt_f64(row.r_1s),
            opt_f64(row.r_5s),
            opt_f64(row.r_30s),
            opt_f64(row.r_300s),
            opt_i8(row.sign_1s),
            opt_i8(row.sign_5s),
            fv.exchange,
            fv.symbol,
            fv.data_source,
            fv.is_imputed as u8,
            fv.gap_flag as u8,
        )?;
    }

    writer.flush()
}

fn opt_f64(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("{x:.8}"),
        None => String::new(),
    }
}

fn opt_i8(v: Option<i8>) -> String {
    match v {
        Some(x) => x.to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::FeatureVector;
    use crate::pipeline::LabeledFeatureVector;

    fn dummy_lfv() -> LabeledFeatureVector {
        LabeledFeatureVector {
            fv: FeatureVector {
                ts: 1_000_000,
                ofi_1: Some(5.0),
                ofi_5: None,
                ofi_10: None,
                depth_imb: Some(0.2),
                microprice_dev: Some(-0.01),
                queue_imb: Some(0.55),
                spread: Some(1.0),
                trade_intensity: Some(3.5),
                price_impact: Some(0.001),
                level_drain: None,
                weighted_mid_slope: Some(0.5),
                exchange: "binance".into(),
                symbol: "BTC/USDT".into(),
                data_source: "live".into(),
                is_imputed: false,
                gap_flag: false,
            },
            r_1s: Some(0.001),
            r_5s: Some(0.002),
            r_30s: None,
            r_300s: None,
            sign_1s: Some(1),
            sign_5s: Some(1),
        }
    }

    #[test]
    fn csv_writes_header_and_row() {
        let mut buf = BufWriter::new(Vec::new());
        write_csv(&mut buf, &[dummy_lfv()]).unwrap();
        let bytes = buf.into_inner().unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("ts,ofi_1"));
        assert!(text.contains("binance"));
        assert!(text.contains("BTC/USDT"));
    }
}
