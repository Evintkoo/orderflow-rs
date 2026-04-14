#[cfg(test)]
mod report_tests {
    use orderflow_rs::analysis::techreport::{compute_ic_table, IcRow};
    use orderflow_rs::analysis::techind::{TechData, IndicatorSeries};

    #[test]
    fn ic_table_produces_rows() {
        let n = 200;
        let prices: Vec<f64> = (0..n).map(|i| 1.0 + i as f64 * 0.001).collect();
        let r_1s: Vec<Option<f64>> = (0..n).map(|i| Some(0.001 + (i as f64 * 0.1).sin() * 0.0005)).collect();
        let mut td = TechData::new(
            (0..n as i64).collect(),
            prices.clone(),
            prices.iter().map(|p| p + 0.001).collect(),
            prices.iter().map(|p| p - 0.001).collect(),
            vec![100.0; n],
            vec![0.0001; n],
            r_1s.clone(), r_1s.clone(), r_1s.clone(), r_1s.clone(),
        );
        td.indicators.push(IndicatorSeries {
            name: "test_indicator",
            values: (0..n).map(|i| Some(i as f64 * 0.001)).collect(),
        });
        let rows = compute_ic_table(&td);
        assert!(!rows.is_empty());
        assert_eq!(rows[0].name, "test_indicator");
    }

    #[test]
    fn write_reports_creates_files() {
        use orderflow_rs::analysis::techreport::{write_ic_report, write_summary_report};
        use std::fs;

        let dir = tempfile::tempdir().unwrap();
        let rows = vec![
            IcRow { name: "sma_20".into(), ic_1s: Some(0.05), ic_5s: Some(0.08), ic_30s: Some(0.02), ic_300s: Some(-0.01) },
            IcRow { name: "rsi_14".into(), ic_1s: None, ic_5s: Some(-0.03), ic_30s: None, ic_300s: Some(0.04) },
        ];
        write_ic_report("EURUSD", &rows, dir.path().to_str().unwrap()).unwrap();
        let csv = dir.path().join("EURUSD_techanalysis.csv");
        assert!(csv.exists(), "CSV should be created");
        let content = fs::read_to_string(&csv).unwrap();
        assert!(content.contains("sma_20"));
        assert!(content.contains("0.050000"));

        // Summary
        let per_pair = vec![
            ("EURUSD".into(), rows.clone()),
        ];
        write_summary_report(&per_pair, dir.path().to_str().unwrap()).unwrap();
        let summary = dir.path().join("summary_techanalysis.csv");
        assert!(summary.exists());
        let sc = fs::read_to_string(&summary).unwrap();
        assert!(sc.contains("sma_20"));
    }
}

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

#[cfg(test)]
mod trend_tests {
    use orderflow_rs::analysis::techind::{atr, adx, aroon_up, aroon_down, choppiness, supertrend};

    #[test]
    fn atr_positive() {
        let h = vec![1.01, 1.02, 1.03, 1.04, 1.05, 1.06, 1.07, 1.08, 1.09, 1.10,
                     1.11, 1.12, 1.13, 1.14, 1.15];
        let l: Vec<f64> = h.iter().map(|x| x - 0.005).collect();
        let c: Vec<f64> = h.iter().map(|x| x - 0.002).collect();
        let result = atr(&h, &l, &c, 14);
        assert_eq!(result.len(), 15);
        assert!(result[13].unwrap() > 0.0);
    }

    #[test]
    fn adx_range() {
        let n = 60;
        let h: Vec<f64> = (0..n).map(|i| 1.0 + i as f64 * 0.01).collect();
        let l: Vec<f64> = h.iter().map(|x| x - 0.005).collect();
        let c: Vec<f64> = h.iter().map(|x| x - 0.002).collect();
        let result = adx(&h, &l, &c, 14);
        for v in result.iter().filter_map(|x| *x) {
            assert!(v >= 0.0 && v <= 100.0, "ADX out of [0,100]: {}", v);
        }
    }

    #[test]
    fn aroon_range() {
        let n = 40;
        let h: Vec<f64> = (0..n).map(|i| 1.0 + (i as f64 * 0.7).sin() * 0.1).collect();
        let l: Vec<f64> = h.iter().map(|x| x - 0.005).collect();
        let up = aroon_up(&h, 25);
        let dn = aroon_down(&l, 25);
        for v in up.iter().chain(dn.iter()).filter_map(|x| *x) {
            assert!(v >= 0.0 && v <= 100.0, "Aroon out of [0,100]");
        }
    }

    #[test]
    fn choppiness_range() {
        let n = 30;
        let h: Vec<f64> = (0..n).map(|i| 1.0 + (i as f64 * 0.4).sin() * 0.05 + i as f64 * 0.001).collect();
        let l: Vec<f64> = h.iter().map(|x| x - 0.003).collect();
        let c: Vec<f64> = h.iter().map(|x| x - 0.001).collect();
        let result = choppiness(&h, &l, &c, 14);
        for v in result.iter().filter_map(|x| *x) {
            assert!(v >= 0.0 && v <= 200.0, "Choppiness extreme: {}", v);
        }
    }
}

#[cfg(test)]
mod momentum_tests {
    use orderflow_rs::analysis::techind::{rsi, macd_components, cci, williams_r, stoch_k, stoch_d, roc, momentum, cmo, dpo, awesome_osc};

    #[test]
    fn rsi_range() {
        let n = 30;
        let p: Vec<f64> = (0..n).map(|i| 10.0 + (i as f64 * 0.5).sin()).collect();
        let result = rsi(&p, 14);
        assert_eq!(result.len(), n);
        for v in result.iter().filter_map(|x| *x) {
            assert!(v >= 0.0 && v <= 100.0, "RSI out of [0,100]: {v}");
        }
    }

    #[test]
    fn macd_line_positive_for_rising() {
        let p: Vec<f64> = (0..100).map(|i| 1.0 + i as f64 * 0.01).collect();
        let (line, signal, hist) = macd_components(&p, 12, 26, 9);
        let last_line = line.iter().rev().find_map(|x| *x).unwrap();
        assert!(last_line > 0.0, "MACD line should be positive for rising prices");
        assert_eq!(line.len(), 100);
        assert_eq!(signal.len(), 100);
        assert_eq!(hist.len(), 100);
    }

    #[test]
    fn cci_varies() {
        let n = 30;
        let h: Vec<f64> = (0..n).map(|i| 1.0 + (i as f64 * 0.3).sin() * 0.1).collect();
        let l: Vec<f64> = h.iter().map(|x| x - 0.005).collect();
        let c: Vec<f64> = h.iter().map(|x| x - 0.002).collect();
        let result = cci(&h, &l, &c, 20);
        let vals: Vec<f64> = result.iter().filter_map(|x| *x).collect();
        assert!(!vals.is_empty());
        let range = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
                  - vals.iter().cloned().fold(f64::INFINITY, f64::min);
        assert!(range > 0.1, "CCI should vary: range={range}");
    }

    #[test]
    fn stoch_range() {
        let n = 20;
        let h: Vec<f64> = (0..n).map(|i| 1.0 + (i as f64 * 0.3).sin() * 0.1 + 0.001).collect();
        let l: Vec<f64> = h.iter().map(|x| x - 0.005).collect();
        let c: Vec<f64> = h.iter().map(|x| x - 0.002).collect();
        let k = stoch_k(&h, &l, &c, 14);
        let d = stoch_d(&h, &l, &c, 14);
        for v in k.iter().chain(d.iter()).filter_map(|x| *x) {
            assert!(v >= 0.0 && v <= 100.0, "Stoch out of [0,100]: {v}");
        }
    }
}

#[cfg(test)]
mod volatility_tests {
    use orderflow_rs::analysis::techind::{bb_width, bb_pct_b, keltner_width, donchian_width, natr, realized_vol, parkinson_vol, squeeze_signal};

    #[test]
    fn bb_width_positive() {
        let n = 30;
        let p: Vec<f64> = (0..n).map(|i| 1.0 + (i as f64 * 0.3).sin() * 0.05).collect();
        let w = bb_width(&p, 20, 2.0);
        assert_eq!(w.len(), n);
        let last = w[29].unwrap();
        assert!(last > 0.0, "BB width should be positive: {last}");
    }

    #[test]
    fn bb_pct_b_finite() {
        let n = 30;
        let p: Vec<f64> = (0..n).map(|i| 1.0 + i as f64 * 0.001).collect();
        let pct = bb_pct_b(&p, 20, 2.0);
        for v in pct.iter().filter_map(|x| *x) {
            assert!(v.is_finite(), "%b should be finite: {v}");
        }
    }

    #[test]
    fn realized_vol_positive() {
        let n = 30;
        let p: Vec<f64> = (0..n).map(|i| 1.0 + i as f64 * 0.001 + (i as f64 * 0.7).sin() * 0.002).collect();
        let rv = realized_vol(&p, 20);
        let last = rv[29].unwrap();
        assert!(last > 0.0, "realized vol should be positive");
    }

    #[test]
    fn parkinson_vol_positive() {
        let n = 20;
        let h: Vec<f64> = (0..n).map(|i| 1.0 + i as f64 * 0.001 + 0.005).collect();
        let l: Vec<f64> = (0..n).map(|i| 1.0 + i as f64 * 0.001).collect();
        let pv = parkinson_vol(&h, &l, 14);
        for v in pv.iter().filter_map(|x| *x) {
            assert!(v >= 0.0, "Parkinson vol should be non-negative");
        }
    }
}

#[cfg(test)]
mod volume_tests {
    use orderflow_rs::analysis::techind::{obv, mfi, vwap_deviation, rvol};

    #[test]
    fn obv_rises_on_up_days() {
        let closes = vec![1.0, 1.01, 1.02, 1.01, 1.03];
        let vols   = vec![100.0; 5];
        let result = obv(&closes, &vols);
        assert_eq!(result.len(), 5);
        // Day 1→2: up, so OBV should rise
        assert!(result[1] > result[0]);
        // Day 3→4: down, so OBV should fall
        assert!(result[3] < result[2]);
    }

    #[test]
    fn mfi_range() {
        let n = 20;
        let h: Vec<f64> = (0..n).map(|i| 1.0 + (i as f64 * 0.5).sin() * 0.1).collect();
        let l: Vec<f64> = h.iter().map(|x| x - 0.005).collect();
        let c: Vec<f64> = h.iter().map(|x| x - 0.002).collect();
        let vol: Vec<f64> = vec![100.0; n];
        let result = mfi(&h, &l, &c, &vol, 14);
        for v in result.iter().filter_map(|x| *x) {
            assert!(v >= 0.0 && v <= 100.0, "MFI out of [0,100]: {v}");
        }
    }

    #[test]
    fn rvol_positive() {
        let n = 30;
        let vol: Vec<f64> = (0..n).map(|i| 100.0 + i as f64 * 10.0).collect();
        let result = rvol(&vol, 20);
        for v in result.iter().skip(20).filter_map(|x| *x) {
            assert!(v > 0.0, "RVOL should be positive");
        }
    }
}

#[cfg(test)]
mod sr_stats_tests {
    use orderflow_rs::analysis::techind::{dist_high, dist_low, pivot_dist, zscore, lr_slope, autocorr, hurst, variance_ratio};

    #[test]
    fn dist_high_non_negative() {
        let h = vec![1.01, 1.02, 1.03, 1.04, 1.05, 1.03];
        let c = vec![1.00, 1.01, 1.02, 1.03, 1.04, 1.02];
        let result = dist_high(&h, &c, 5);
        for v in result.iter().filter_map(|x| *x) {
            assert!(v >= 0.0, "dist_high should be non-negative: {v}");
        }
    }

    #[test]
    fn zscore_constant_is_zero() {
        let p = vec![1.0_f64; 30];
        let result = zscore(&p, 20);
        for v in result.iter().skip(19).filter_map(|x| *x) {
            assert!(v.abs() < 1e-6, "z-score of constant = 0: {v}");
        }
    }

    #[test]
    fn lr_slope_positive_for_rising() {
        let p: Vec<f64> = (0..30).map(|i| 1.0 + i as f64 * 0.01).collect();
        let result = lr_slope(&p, 20);
        let last = result[29].unwrap();
        assert!(last > 0.0, "LR slope should be positive for rising prices: {last}");
    }

    #[test]
    fn hurst_positive() {
        let n = 200;
        let p: Vec<f64> = (0..n).map(|i| 1.0 + (i as f64 * 0.05).sin() * 0.01 + i as f64 * 0.0001).collect();
        let result = hurst(&p, 100);
        for v in result.iter().filter_map(|x| *x) {
            assert!(v > 0.0 && v < 2.0, "Hurst should be in (0,2): {v}");
        }
    }
}

#[cfg(test)]
mod microstructure_tests {
    use orderflow_rs::analysis::techind::{amihud_illiquidity, roll_spread_est, book_pressure_ind, spread_osc, info_efficiency_ratio};

    #[test]
    fn amihud_non_negative() {
        let n = 30;
        let p: Vec<f64> = (0..n).map(|i| 1.0 + (i as f64 * 0.3).sin() * 0.01).collect();
        let vol: Vec<f64> = vec![1000.0; n];
        let result = amihud_illiquidity(&p, &vol, 20);
        for v in result.iter().skip(20).filter_map(|x| *x) {
            assert!(v >= 0.0, "Amihud should be non-negative: {v}");
        }
    }

    #[test]
    fn book_pressure_range() {
        let ofi: Vec<f64> = (0..25).map(|i| (i as f64 * 0.5).sin()).collect();
        let result = book_pressure_ind(&ofi, 20);
        for v in result.iter().filter_map(|x| *x) {
            assert!(v >= -1.0 && v <= 1.0, "Book pressure out of [-1,1]: {v}");
        }
    }

    #[test]
    fn spread_osc_finite() {
        let n = 30;
        let spreads: Vec<f64> = (0..n).map(|i| 0.0001 + (i as f64 * 0.1).sin() * 0.00001).collect();
        let result = spread_osc(&spreads, 20);
        for v in result.iter().filter_map(|x| *x) {
            assert!(v.is_finite(), "spread_osc should be finite: {v}");
        }
    }
}

#[cfg(test)]
mod compute_all_tests {
    use orderflow_rs::analysis::techind::{TechData, compute_all};

    #[test]
    fn compute_all_produces_indicators() {
        let n = 250;
        let prices: Vec<f64> = (0..n).map(|i| 1.0 + i as f64 * 0.001 + (i as f64 * 0.1).sin() * 0.01).collect();
        let highs: Vec<f64> = prices.iter().map(|p| p + 0.001).collect();
        let lows: Vec<f64> = prices.iter().map(|p| p - 0.001).collect();
        let volumes: Vec<f64> = vec![1000.0; n];
        let spreads: Vec<f64> = vec![0.0001; n];
        let ofi: Vec<f64> = vec![0.1; n];
        let r_1s: Vec<Option<f64>> = vec![Some(0.001); n];
        let mut td = TechData::new(
            (0..n as i64).collect(), prices.clone(), highs, lows,
            volumes, spreads, r_1s.clone(), r_1s.clone(), r_1s.clone(), r_1s.clone(),
        );
        compute_all(&mut td, &ofi);
        assert!(!td.indicators.is_empty(), "should have indicators");
        // Check for key indicator names
        let names: Vec<&str> = td.indicators.iter().map(|s| s.name).collect();
        assert!(names.contains(&"sma_20"), "missing sma_20");
        assert!(names.contains(&"rsi_14"), "missing rsi_14");
        assert!(names.contains(&"bb_width_20"), "missing bb_width_20");
        assert!(names.contains(&"obv"), "missing obv");
        assert!(names.contains(&"amihud_20"), "missing amihud_20");
        // All indicator vecs have same length as prices
        for ind in &td.indicators {
            assert_eq!(ind.values.len(), n, "length mismatch for {}", ind.name);
        }
    }

    #[test]
    fn compute_all_no_panic_short_data() {
        let n = 10; // shorter than most periods
        let prices = vec![1.0; n];
        let highs = vec![1.001; n];
        let lows = vec![0.999; n];
        let volumes = vec![100.0; n];
        let spreads = vec![0.0001; n];
        let ofi = vec![0.0; n];
        let r = vec![Some(0.0); n];
        let mut td = TechData::new(
            (0..n as i64).collect(), prices, highs, lows, volumes, spreads,
            r.clone(), r.clone(), r.clone(), r.clone(),
        );
        compute_all(&mut td, &ofi); // should not panic
        assert!(!td.indicators.is_empty());
    }
}
