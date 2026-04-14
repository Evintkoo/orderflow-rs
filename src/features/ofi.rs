//! Order Flow Imbalance (OFI) — Cont, Kukanov & Stoikov (2014).
//!
//! OFI(N) = Σᵢ₌₁ᴺ [ΔQᵇᵢ(t) − ΔQᵃᵢ(t)]
//!
//! Requires stateful tracking of the previous snapshot.

use crate::orderbook::BookSnapshot;

/// Stateful OFI calculator.  Must be updated on every 100ms snapshot tick.
pub struct OrderFlowImbalance {
    prev: Option<BookSnapshot>,
}

impl OrderFlowImbalance {
    pub fn new() -> Self {
        Self { prev: None }
    }

    /// Feed a new snapshot; returns OFI at 1, 5, and 10 levels (or None on
    /// first call or empty book).
    pub fn update(&mut self, snap: &BookSnapshot) -> OfiResult {
        let result = if let Some(prev) = &self.prev {
            OfiResult {
                ofi_1: compute_ofi(prev, snap, 1),
                ofi_5: compute_ofi(prev, snap, 5),
                ofi_10: compute_ofi(prev, snap, 10),
            }
        } else {
            OfiResult::none()
        };
        self.prev = Some(snap.clone());
        result
    }

    pub fn reset(&mut self) {
        self.prev = None;
    }
}

impl Default for OrderFlowImbalance {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct OfiResult {
    pub ofi_1: Option<f64>,
    pub ofi_5: Option<f64>,
    pub ofi_10: Option<f64>,
}

impl OfiResult {
    fn none() -> Self {
        Self { ofi_1: None, ofi_5: None, ofi_10: None }
    }
}

/// Compute OFI at `n` levels between two consecutive snapshots.
///
/// ΔQᵇᵢ = Qᵇᵢ(t) − Qᵇᵢ(t−1)  (positive = bid depth increased)
/// ΔQᵃᵢ = Qᵃᵢ(t) − Qᵃᵢ(t−1)  (positive = ask depth increased)
/// OFI(N) = Σᵢ (ΔQᵇᵢ − ΔQᵃᵢ)
///
/// Returns None if either snapshot's relevant side is empty.
fn compute_ofi(prev: &BookSnapshot, curr: &BookSnapshot, n: usize) -> Option<f64> {
    if curr.is_empty() || prev.is_empty() {
        return None;
    }

    let prev_bids = level_map(prev.top_bids(n));
    let curr_bids = level_map(curr.top_bids(n));
    let prev_asks = level_map(prev.top_asks(n));
    let curr_asks = level_map(curr.top_asks(n));

    let mut ofi = 0.0_f64;

    // Bid side deltas: union of price levels present in either snapshot
    let all_bid_prices: std::collections::HashSet<u64> = prev_bids
        .keys()
        .chain(curr_bids.keys())
        .copied()
        .collect();
    for key in &all_bid_prices {
        let prev_q = prev_bids.get(key).copied().unwrap_or(0.0);
        let curr_q = curr_bids.get(key).copied().unwrap_or(0.0);
        ofi += curr_q - prev_q;
    }

    // Ask side deltas
    let all_ask_prices: std::collections::HashSet<u64> = prev_asks
        .keys()
        .chain(curr_asks.keys())
        .copied()
        .collect();
    for key in &all_ask_prices {
        let prev_q = prev_asks.get(key).copied().unwrap_or(0.0);
        let curr_q = curr_asks.get(key).copied().unwrap_or(0.0);
        ofi -= curr_q - prev_q;
    }

    Some(ofi)
}

/// Convert (price, qty) pairs to a HashMap keyed by price bits for exact
/// float equality comparison.  Prices are exchange-parsed strings so
/// bit-identical f64s map to the same level.
fn level_map(levels: Vec<(f64, f64)>) -> std::collections::HashMap<u64, f64> {
    levels
        .into_iter()
        .map(|(p, q)| (p.to_bits(), q))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orderbook::{BookDelta, Orderbook, Side};
    use ordered_float::OrderedFloat;

    fn make_book(bids: &[(f64, f64)], asks: &[(f64, f64)]) -> BookSnapshot {
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
    fn first_call_returns_none() {
        let mut ofi = OrderFlowImbalance::new();
        let snap = make_book(&[(100.0, 10.0)], &[(101.0, 5.0)]);
        let r = ofi.update(&snap);
        assert!(r.ofi_1.is_none());
    }

    #[test]
    fn bid_depth_increase_positive_ofi() {
        let mut ofi = OrderFlowImbalance::new();
        let s1 = make_book(&[(100.0, 10.0)], &[(101.0, 5.0)]);
        ofi.update(&s1);
        // Increase bid qty from 10 → 20
        let s2 = make_book(&[(100.0, 20.0)], &[(101.0, 5.0)]);
        let r = ofi.update(&s2);
        assert_eq!(r.ofi_1, Some(10.0)); // +10 bid delta, 0 ask delta
    }

    #[test]
    fn ask_depth_increase_negative_ofi() {
        let mut ofi = OrderFlowImbalance::new();
        let s1 = make_book(&[(100.0, 10.0)], &[(101.0, 5.0)]);
        ofi.update(&s1);
        // Increase ask qty from 5 → 15
        let s2 = make_book(&[(100.0, 10.0)], &[(101.0, 15.0)]);
        let r = ofi.update(&s2);
        assert_eq!(r.ofi_1, Some(-10.0)); // 0 bid delta, +10 ask delta → −10
    }

    #[test]
    fn empty_book_returns_none() {
        let mut ofi = OrderFlowImbalance::new();
        let s1 = make_book(&[(100.0, 10.0)], &[(101.0, 5.0)]);
        ofi.update(&s1);
        // Empty snapshot (no levels)
        let empty = BookSnapshot::new(1000);
        let r = ofi.update(&empty);
        assert!(r.ofi_1.is_none());
    }

    #[test]
    fn reset_clears_state() {
        let mut ofi = OrderFlowImbalance::new();
        let snap = make_book(&[(100.0, 10.0)], &[(101.0, 5.0)]);
        ofi.update(&snap);
        ofi.reset();
        let r = ofi.update(&snap);
        assert!(r.ofi_1.is_none()); // After reset, first call is None again
    }
}
