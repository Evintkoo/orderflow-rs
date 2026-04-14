//! Microprice (weighted mid-price) — Stoikov (2018).
//!
//! microprice = (Qᵃ · Pᵇ + Qᵇ · Pᵃ) / (Qᵃ + Qᵇ)
//!
//! microprice_deviation = microprice − mid
//!
//! Positive deviation → more weight on the ask side → price likely to move up.

use crate::orderbook::BookSnapshot;

/// Microprice using best bid/ask only.
pub fn microprice(snap: &BookSnapshot) -> Option<f64> {
    let pb = snap.best_bid()?;
    let pa = snap.best_ask()?;
    let qb = snap.best_bid_qty()?;
    let qa = snap.best_ask_qty()?;
    let total = qa + qb;
    if total < f64::EPSILON {
        return None;
    }
    Some((qa * pb + qb * pa) / total)
}

/// Deviation of microprice from mid-price.  Positive → upward price pressure.
pub fn microprice_deviation(snap: &BookSnapshot) -> Option<f64> {
    let mp = microprice(snap)?;
    let mid = snap.mid()?;
    Some(mp - mid)
}

/// Weighted mid-slope: captures price depth curvature across N levels.
///
/// slope = Σᵢ wᵢ · (Pᵃᵢ − Pᵇᵢ) / 2 · i⁻¹   where wᵢ ∝ 1/Qᵢ
///
/// Qᵢ = average of bid and ask qty at level i.
/// Higher slope = thinner book at deeper levels = price can move more per unit flow.
pub fn weighted_mid_slope(snap: &BookSnapshot, n: usize) -> Option<f64> {
    if snap.is_empty() {
        return None;
    }
    let bids = snap.top_bids(n);
    let asks = snap.top_asks(n);
    let levels = bids.len().min(asks.len());
    if levels == 0 {
        return None;
    }

    let mut numerator = 0.0_f64;
    let mut denominator = 0.0_f64;

    for i in 0..levels {
        let (pb, qb) = bids[i];
        let (pa, qa) = asks[i];
        let spread_i = pa - pb;
        let avg_qty = (qb + qa) / 2.0;
        if avg_qty < f64::EPSILON {
            continue;
        }
        let level_weight = 1.0 / ((i + 1) as f64 * avg_qty);
        numerator += level_weight * spread_i / 2.0;
        denominator += level_weight;
    }

    if denominator < f64::EPSILON {
        return None;
    }
    Some(numerator / denominator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orderbook::{BookDelta, Orderbook, Side};
    use ordered_float::OrderedFloat;

    fn snap(bids: &[(f64, f64)], asks: &[(f64, f64)]) -> BookSnapshot {
        let mut book = Orderbook::new();
        for &(p, q) in bids {
            book.apply_delta(&BookDelta {
                price: OrderedFloat(p),
                qty: q,
                side: Side::Bid,
                exchange_ts: 0,
                recv_ts: 0,
            })
            .unwrap();
        }
        for &(p, q) in asks {
            book.apply_delta(&BookDelta {
                price: OrderedFloat(p),
                qty: q,
                side: Side::Ask,
                exchange_ts: 0,
                recv_ts: 0,
            })
            .unwrap();
        }
        book.export_snapshot(10)
    }

    #[test]
    fn symmetric_book_zero_deviation() {
        // Equal qty on both sides → microprice = mid
        let s = snap(&[(100.0, 10.0)], &[(102.0, 10.0)]);
        let dev = microprice_deviation(&s).unwrap();
        assert!(dev.abs() < 1e-10);
    }

    #[test]
    fn more_bid_qty_pushes_micro_toward_ask() {
        // More bid qty → microprice closer to ask price → positive deviation
        let s = snap(&[(100.0, 20.0)], &[(102.0, 5.0)]);
        let mp = microprice(&s).unwrap();
        let mid = s.mid().unwrap(); // 101.0
        assert!(mp > mid, "microprice {mp} should be > mid {mid}");
        assert!(microprice_deviation(&s).unwrap() > 0.0);
    }

    #[test]
    fn more_ask_qty_pushes_micro_toward_bid() {
        let s = snap(&[(100.0, 5.0)], &[(102.0, 20.0)]);
        let dev = microprice_deviation(&s).unwrap();
        assert!(dev < 0.0, "deviation {dev} should be negative");
    }

    #[test]
    fn hand_calculated_microprice() {
        // Pᵇ=100, Qᵇ=30, Pᵃ=102, Qᵃ=10
        // micro = (10*100 + 30*102) / (10+30) = (1000+3060)/40 = 4060/40 = 101.5
        let s = snap(&[(100.0, 30.0)], &[(102.0, 10.0)]);
        let mp = microprice(&s).unwrap();
        assert!((mp - 101.5).abs() < 1e-10, "expected 101.5, got {mp}");
    }

    #[test]
    fn empty_book_returns_none() {
        let s = BookSnapshot::new(0);
        assert!(microprice(&s).is_none());
        assert!(microprice_deviation(&s).is_none());
        assert!(weighted_mid_slope(&s, 5).is_none());
    }
}
