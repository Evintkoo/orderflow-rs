pub mod depth;
pub mod microprice;
pub mod ofi;
pub mod spread;
pub mod trade_pressure;

use crate::orderbook::BookSnapshot;
use ofi::OrderFlowImbalance;
use trade_pressure::TradeBuffer;

pub use depth::{depth_imbalance, level_drain_rate};
pub use microprice::{microprice_deviation, weighted_mid_slope};
pub use ofi::OfiResult;
pub use spread::{queue_imbalance, relative_spread, spread};
pub use trade_pressure::{hawkes_intensity, price_impact_ratio, TradeEvent};

/// Full feature vector for one 100ms snapshot tick.
///
/// All feature fields are `Option<f64>` — `None` means "unavailable" and maps
/// to null in Parquet and NaN in ndarray.  Callers must never silently
/// treat None as 0.
#[derive(Debug, Clone)]
pub struct FeatureVector {
    /// Exchange timestamp of the snapshot (microseconds).
    pub ts: i64,
    // OFI family
    pub ofi_1: Option<f64>,
    pub ofi_5: Option<f64>,
    pub ofi_10: Option<f64>,
    // Depth
    pub depth_imb: Option<f64>,
    // Microprice
    pub microprice_dev: Option<f64>,
    // Queue
    pub queue_imb: Option<f64>,
    // Spread
    pub spread: Option<f64>,
    // Trade-derived
    pub trade_intensity: Option<f64>,
    pub price_impact: Option<f64>,
    pub level_drain: Option<f64>,
    pub weighted_mid_slope: Option<f64>,
    // Metadata
    pub exchange: String,
    pub symbol: String,
    pub data_source: String,
    pub is_imputed: bool,
    pub gap_flag: bool,
}

/// Stateful feature extractor — feed one snapshot per tick.
///
/// Owns:
/// - Previous snapshot (for OFI delta computation)
/// - Previous best-bid qty (for level drain rate)
/// - Trade buffer (for Hawkes intensity)
/// - Hawkes parameters (fit externally, set once)
pub struct FeatureExtractor {
    ofi: OrderFlowImbalance,
    prev_best_bid_qty: Option<f64>,
    prev_ts_us: Option<i64>,
    trade_buffer: TradeBuffer,
    hawkes_mu: f64,
    hawkes_alpha: f64,
    hawkes_beta: f64,
    exchange: String,
    symbol: String,
    data_source: String,
}

impl FeatureExtractor {
    pub fn new(exchange: &str, symbol: &str, data_source: &str) -> Self {
        Self {
            ofi: OrderFlowImbalance::new(),
            prev_best_bid_qty: None,
            prev_ts_us: None,
            trade_buffer: TradeBuffer::new(60_000_000), // 60s window
            hawkes_mu: 1.0,
            hawkes_alpha: 0.5,
            hawkes_beta: 1.0,
            exchange: exchange.to_string(),
            symbol: symbol.to_string(),
            data_source: data_source.to_string(),
        }
    }

    pub fn set_hawkes_params(&mut self, mu: f64, alpha: f64, beta: f64) {
        assert!(alpha < beta, "Hawkes stability condition violated");
        self.hawkes_mu = mu;
        self.hawkes_alpha = alpha;
        self.hawkes_beta = beta;
    }

    pub fn push_trade(&mut self, trade: TradeEvent) {
        let ts = trade.ts;
        self.trade_buffer.push(trade, ts);
    }

    /// Extract a full feature vector from a new snapshot.
    pub fn extract(&mut self, snap: &BookSnapshot, is_imputed: bool, gap_flag: bool) -> FeatureVector {
        let ofi_result = self.ofi.update(snap);

        let depth_imb = depth_imbalance(snap, 5);
        let microprice_dev = microprice_deviation(snap);
        let queue_imb = queue_imbalance(snap);
        let spread_val = spread(snap);
        let wms = weighted_mid_slope(snap, 5);

        // Level drain: requires previous timestamp
        let level_drain = if let (Some(prev_qty), Some(prev_ts)) =
            (self.prev_best_bid_qty, self.prev_ts_us)
        {
            let dt_s = (snap.ts - prev_ts) as f64 * 1e-6;
            level_drain_rate(snap, prev_qty, dt_s)
        } else {
            None
        };

        // Hawkes intensity
        let trade_intensity = hawkes_intensity(
            &self.trade_buffer,
            snap.ts,
            self.hawkes_mu,
            self.hawkes_alpha,
            self.hawkes_beta,
        );

        // Price impact of most recent trade
        let price_impact = if let Some(trade) = self.trade_buffer.last_trade() {
            snap.mid().and_then(|mid| price_impact_ratio(mid, trade))
        } else {
            None
        };

        // Update rolling state
        self.prev_best_bid_qty = snap.best_bid_qty();
        self.prev_ts_us = Some(snap.ts);

        FeatureVector {
            ts: snap.ts,
            ofi_1: ofi_result.ofi_1,
            ofi_5: ofi_result.ofi_5,
            ofi_10: ofi_result.ofi_10,
            depth_imb,
            microprice_dev,
            queue_imb,
            spread: spread_val,
            trade_intensity,
            price_impact,
            level_drain,
            weighted_mid_slope: wms,
            exchange: self.exchange.clone(),
            symbol: self.symbol.clone(),
            data_source: self.data_source.clone(),
            is_imputed,
            gap_flag,
        }
    }

    pub fn reset_on_gap(&mut self) {
        self.ofi.reset();
        self.prev_best_bid_qty = None;
        self.prev_ts_us = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orderbook::{BookDelta, Orderbook, Side};
    use ordered_float::OrderedFloat;

    fn make_snap(ts: i64, bids: &[(f64, f64)], asks: &[(f64, f64)]) -> BookSnapshot {
        let mut book = Orderbook::new();
        for &(p, q) in bids {
            book.apply_delta(&BookDelta {
                price: OrderedFloat(p),
                qty: q,
                side: Side::Bid,
                exchange_ts: ts,
                recv_ts: ts,
            })
            .unwrap();
        }
        for &(p, q) in asks {
            book.apply_delta(&BookDelta {
                price: OrderedFloat(p),
                qty: q,
                side: Side::Ask,
                exchange_ts: ts,
                recv_ts: ts,
            })
            .unwrap();
        }
        book.export_snapshot(10)
    }

    #[test]
    fn extract_produces_valid_feature_vector() {
        let mut fx = FeatureExtractor::new("binance", "BTC/USDT", "live");
        let s1 = make_snap(1_000_000, &[(100.0, 10.0)], &[(101.0, 5.0)]);
        let s2 = make_snap(1_100_000, &[(100.0, 15.0)], &[(101.0, 5.0)]);
        fx.extract(&s1, false, false);
        let fv = fx.extract(&s2, false, false);
        // OFI(1): bid went 10→15 (+5), ask unchanged → OFI = +5
        assert_eq!(fv.ofi_1, Some(5.0));
        assert!(fv.spread.is_some());
        assert!(fv.depth_imb.is_some());
    }

    #[test]
    fn gap_reset_clears_ofi() {
        let mut fx = FeatureExtractor::new("binance", "BTC/USDT", "live");
        let s1 = make_snap(1_000_000, &[(100.0, 10.0)], &[(101.0, 5.0)]);
        fx.extract(&s1, false, false);
        fx.reset_on_gap();
        let fv = fx.extract(&s1, false, true);
        assert!(fv.ofi_1.is_none()); // After reset, first call returns None
        assert!(fv.gap_flag);
    }
}
