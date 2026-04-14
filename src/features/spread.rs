//! Spread and queue imbalance metrics.
//!
//! spread = best_ask − best_bid                          (Roll 1984)
//! queue_imbalance = Qᵇ₁ / (Qᵇ₁ + Qᵃ₁)               (Cartea et al. 2015)

use crate::orderbook::BookSnapshot;

/// Absolute bid-ask spread.  Returns None if either side is empty.
pub fn spread(snap: &BookSnapshot) -> Option<f64> {
    snap.spread()
}

/// Relative spread = (best_ask − best_bid) / mid.
pub fn relative_spread(snap: &BookSnapshot) -> Option<f64> {
    let s = snap.spread()?;
    let mid = snap.mid()?;
    if mid < f64::EPSILON {
        return None;
    }
    Some(s / mid)
}

/// Queue imbalance at best level: Qᵇ₁ / (Qᵇ₁ + Qᵃ₁).
///
/// Range: [0, 1].  0.5 = balanced.  >0.5 = more depth on bid side.
pub fn queue_imbalance(snap: &BookSnapshot) -> Option<f64> {
    let qb = snap.best_bid_qty()?;
    let qa = snap.best_ask_qty()?;
    let total = qb + qa;
    if total < f64::EPSILON {
        return None;
    }
    Some(qb / total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orderbook::{BookDelta, Orderbook, Side};
    use ordered_float::OrderedFloat;

    fn snap(bid_p: f64, bid_q: f64, ask_p: f64, ask_q: f64) -> BookSnapshot {
        let mut book = Orderbook::new();
        book.apply_delta(&BookDelta {
            price: OrderedFloat(bid_p),
            qty: bid_q,
            side: Side::Bid,
            exchange_ts: 0,
            recv_ts: 0,
        })
        .unwrap();
        book.apply_delta(&BookDelta {
            price: OrderedFloat(ask_p),
            qty: ask_q,
            side: Side::Ask,
            exchange_ts: 0,
            recv_ts: 0,
        })
        .unwrap();
        book.export_snapshot(5)
    }

    #[test]
    fn spread_basic() {
        let s = snap(100.0, 10.0, 101.5, 5.0);
        assert!((spread(&s).unwrap() - 1.5).abs() < 1e-10);
    }

    #[test]
    fn relative_spread_basic() {
        let s = snap(100.0, 10.0, 102.0, 5.0);
        // spread=2, mid=101, rel=2/101
        let expected = 2.0 / 101.0;
        assert!((relative_spread(&s).unwrap() - expected).abs() < 1e-10);
    }

    #[test]
    fn queue_imbalance_balanced() {
        let s = snap(100.0, 10.0, 101.0, 10.0);
        assert!((queue_imbalance(&s).unwrap() - 0.5).abs() < 1e-10);
    }

    #[test]
    fn queue_imbalance_all_bid() {
        let s = snap(100.0, 100.0, 101.0, 0.001);
        // Approx 1
        assert!(queue_imbalance(&s).unwrap() > 0.99);
    }

    #[test]
    fn empty_book_returns_none() {
        let s = BookSnapshot::new(0);
        assert!(spread(&s).is_none());
        assert!(queue_imbalance(&s).is_none());
    }
}
