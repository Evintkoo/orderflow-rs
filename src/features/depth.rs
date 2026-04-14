//! Depth imbalance at N levels.
//!
//! depth_imbalance(N) = (Σ Qᵇᵢ − Σ Qᵃᵢ) / (Σ Qᵇᵢ + Σ Qᵃᵢ)  for i = 1..N
//!
//! Range: [−1, +1].  +1 means all visible depth is on the bid side.
//! Returns None if the book is empty or total quantity is zero.

use crate::orderbook::BookSnapshot;

/// Depth imbalance at `n` levels.
pub fn depth_imbalance(snap: &BookSnapshot, n: usize) -> Option<f64> {
    if snap.is_empty() {
        return None;
    }

    let bid_qty: f64 = snap.top_bids(n).iter().map(|(_, q)| q).sum();
    let ask_qty: f64 = snap.top_asks(n).iter().map(|(_, q)| q).sum();
    let total = bid_qty + ask_qty;

    if total < f64::EPSILON {
        return None;
    }

    Some((bid_qty - ask_qty) / total)
}

/// Level drain rate for the best bid: (Q̄ᵇ₁(t−w) − Qᵇ₁(t)) / w
///
/// Measures how fast liquidity at the best bid is being consumed.
/// `prev_best_bid_qty`: best bid qty `w` seconds ago.
/// `window_secs`: elapsed time in seconds.
pub fn level_drain_rate(
    snap: &BookSnapshot,
    prev_best_bid_qty: f64,
    window_secs: f64,
) -> Option<f64> {
    if snap.is_empty() || window_secs < f64::EPSILON {
        return None;
    }
    let curr = snap.best_bid_qty()?;
    Some((prev_best_bid_qty - curr) / window_secs)
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
    fn balanced_book_zero_imbalance() {
        let s = snap(&[(100.0, 10.0)], &[(101.0, 10.0)]);
        let di = depth_imbalance(&s, 1).unwrap();
        assert!((di - 0.0).abs() < 1e-10);
    }

    #[test]
    fn all_bid_depth_returns_one() {
        // 20 on bid, 0 on ask — but ask must exist for the book to be non-empty
        // Use tiny ask qty
        let s = snap(&[(100.0, 20.0)], &[(101.0, 0.001)]);
        let di = depth_imbalance(&s, 1).unwrap();
        // (20 - 0.001) / (20 + 0.001) ≈ 0.9999
        assert!(di > 0.99);
    }

    #[test]
    fn five_level_imbalance() {
        let s = snap(
            &[(100.0, 10.0), (99.0, 8.0), (98.0, 6.0), (97.0, 4.0), (96.0, 2.0)],
            &[(101.0, 5.0), (102.0, 5.0), (103.0, 5.0), (104.0, 5.0), (105.0, 5.0)],
        );
        let di = depth_imbalance(&s, 5).unwrap();
        // bid_sum = 30, ask_sum = 25, total = 55
        let expected = (30.0 - 25.0) / 55.0;
        assert!((di - expected).abs() < 1e-10);
    }

    #[test]
    fn empty_book_returns_none() {
        let s = BookSnapshot::new(0);
        assert!(depth_imbalance(&s, 5).is_none());
    }

    #[test]
    fn level_drain_rate_basic() {
        let s = snap(&[(100.0, 5.0)], &[(101.0, 5.0)]);
        // Was 10.0 one second ago, now 5.0 → drain = 5.0/s
        let drain = level_drain_rate(&s, 10.0, 1.0).unwrap();
        assert!((drain - 5.0).abs() < 1e-10);
    }
}
