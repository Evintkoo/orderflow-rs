#[cfg(test)]
mod loader_tests {
    use orderflow_rs::analysis::techreport::load_techdata;
    use std::io::Write as IoWrite;

    #[test]
    fn load_techdata_reconstructs_prices() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_features.csv");
        let mut f = std::fs::File::create(&path).unwrap();
        // CSV columns: ts,ofi_1,...,r_300s,sign_1s,sign_5s,exchange,symbol,is_imputed,gap_flag (22 total)
        writeln!(f, "ts,ofi_1,ofi_5,ofi_10,depth_imb,microprice_dev,queue_imb,spread,trade_intensity,price_impact,level_drain,weighted_mid_slope,r_1s,r_5s,r_30s,r_300s,sign_1s,sign_5s,exchange,symbol,is_imputed,gap_flag").unwrap();
        writeln!(f, "1000,0.1,0.05,0.02,0.01,0.001,0.02,0.0001,5.0,0.001,0.5,0.0002,0.001,0.002,0.003,0.004,1,1,DUKA,EURUSD,0,0").unwrap();
        writeln!(f, "2000,0.2,0.1,0.04,0.02,0.002,0.03,0.0001,6.0,0.001,0.6,0.0003,0.002,0.004,0.006,0.008,1,1,DUKA,EURUSD,0,0").unwrap();
        writeln!(f, "3000,0.0,0.0,0.0,0.01,0.001,0.02,0.0001,4.0,0.001,0.4,0.0001,,,,,-1,-1,DUKA,EURUSD,0,0").unwrap();

        let td = load_techdata(path.to_str().unwrap()).unwrap();
        assert_eq!(td.prices.len(), 3);
        assert!(td.prices[0] > 0.0);
        assert!(td.prices[1] > td.prices[0], "price should rise when r_1s > 0: {} vs {}", td.prices[1], td.prices[0]);
        assert_eq!(td.spreads.len(), 3);
        assert!(td.highs[0] > td.prices[0]);
        assert!(td.lows[0] < td.prices[0]);
    }
}

#[cfg(test)]
mod tests {
    use orderflow_rs::analysis::techind::{TechData, IndicatorSeries};

    #[test]
    fn techdata_builds_from_vecs() {
        let td = TechData {
            ts: vec![0, 1, 2],
            prices: vec![1.0, 1.01, 1.02],
            highs: vec![1.005, 1.015, 1.025],
            lows: vec![0.995, 1.005, 1.015],
            volumes: vec![10.0, 10.0, 10.0],
            spreads: vec![0.001, 0.001, 0.001],
            r_1s: vec![Some(0.001), Some(0.002), None],
            r_5s: vec![Some(0.002), None, None],
            r_30s: vec![None; 3],
            r_300s: vec![None; 3],
            indicators: vec![],
        };
        assert_eq!(td.prices.len(), 3);
    }

    #[test]
    fn indicator_series_has_name() {
        let s = IndicatorSeries { name: "sma_10", values: vec![Some(1.0), None] };
        assert_eq!(s.name, "sma_10");
    }
}

#[cfg(test)]
mod moving_average_tests {
    use orderflow_rs::analysis::techind::{sma, ema, wma, dema, tema, hma, kama, golden_cross, death_cross};

    #[test]
    fn sma_basic() {
        let p = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let result = sma(&p, 3);
        assert_eq!(result.len(), 5);
        assert_eq!(result[0], None);
        assert_eq!(result[1], None);
        assert!((result[2].unwrap() - 2.0).abs() < 1e-9);
        assert!((result[4].unwrap() - 4.0).abs() < 1e-9);
    }

    #[test]
    fn ema_converges() {
        let p: Vec<f64> = (0..50).map(|i| 10.0 + i as f64 * 0.1).collect();
        let result = ema(&p, 12);
        let last = result[49].unwrap();
        let first_valid = result[11].unwrap();
        assert!(last > first_valid, "EMA should track rising prices");
    }

    #[test]
    fn golden_cross_signal() {
        let mut p: Vec<f64> = vec![10.0; 30];
        for i in 20..30 { p[i] = 10.0 + (i - 20) as f64 * 0.5; }
        let gc = golden_cross(&p, 5, 20);
        assert_eq!(gc.len(), 30);
        // At least some non-None values after warmup
        assert!(gc.iter().skip(20).any(|v| v.is_some()));
    }

    #[test]
    fn wma_weighted_correctly() {
        // WMA(3) on [1,2,3] = (1*1 + 2*2 + 3*3)/(1+2+3) = 14/6 ≈ 2.333
        let p = vec![1.0, 2.0, 3.0];
        let result = wma(&p, 3);
        assert!((result[2].unwrap() - 14.0/6.0).abs() < 1e-9);
    }

    #[test]
    fn hma_length_matches() {
        let p: Vec<f64> = (0..50).map(|i| i as f64).collect();
        let result = hma(&p, 14);
        assert_eq!(result.len(), 50);
    }
}
