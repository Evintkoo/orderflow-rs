//! Technical indicator functions and TechData container.
//!
//! All indicator functions are pure: `fn(prices: &[f64], ...) -> Vec<Option<f64>>`.
//! No I/O, no state. `compute_all()` bundles every indicator into a `TechData`.

/// One named indicator series aligned with `TechData::prices`.
pub struct IndicatorSeries {
    pub name: &'static str,
    pub values: Vec<Option<f64>>,
}

/// Price/return data reconstructed from a `*_features.csv` file.
pub struct TechData {
    pub ts: Vec<i64>,
    pub prices: Vec<f64>,
    pub highs: Vec<f64>,
    pub lows: Vec<f64>,
    pub volumes: Vec<f64>,
    pub spreads: Vec<f64>,
    pub r_1s: Vec<Option<f64>>,
    pub r_5s: Vec<Option<f64>>,
    pub r_30s: Vec<Option<f64>>,
    pub r_300s: Vec<Option<f64>>,
    pub indicators: Vec<IndicatorSeries>,
}

impl TechData {
    pub fn new(
        ts: Vec<i64>,
        prices: Vec<f64>,
        highs: Vec<f64>,
        lows: Vec<f64>,
        volumes: Vec<f64>,
        spreads: Vec<f64>,
        r_1s: Vec<Option<f64>>,
        r_5s: Vec<Option<f64>>,
        r_30s: Vec<Option<f64>>,
        r_300s: Vec<Option<f64>>,
    ) -> Self {
        Self { ts, prices, highs, lows, volumes, spreads, r_1s, r_5s, r_30s, r_300s, indicators: vec![] }
    }
}
