pub mod backtest;
pub mod ml;
pub mod report;
pub mod stats;
pub mod techind;
pub mod techreport;

pub use stats::{spearman_ic, icir, ols, newey_west_se, rolling_ic, fit_ic_decay, vif, adf_test, ljung_box, regime_ic};
pub use ml::{SignalBuilder, DecayFilter, ZScoreState, threshold_signal, Position, estimate_signal_halflife};
pub use report::{FeatureRow, load_csv, run_analysis, analyze_directory};
pub use backtest::{BacktestConfig, BacktestSummary, run_backtest, backtest_directory};
