use std::cmp::Reverse;
use std::collections::BTreeMap;
use ordered_float::OrderedFloat;

/// Immutable snapshot of the top-N levels of the order book at a point in time.
#[derive(Debug, Clone)]
pub struct BookSnapshot {
    /// Bids sorted descending by price (best bid = first entry).
    /// Key: Reverse<OrderedFloat<f64>> so BTreeMap iteration is high-to-low.
    pub bids: BTreeMap<Reverse<OrderedFloat<f64>>, f64>,
    /// Asks sorted ascending by price (best ask = first entry).
    pub asks: BTreeMap<OrderedFloat<f64>, f64>,
    /// Exchange timestamp in microseconds.
    pub ts: i64,
}

impl BookSnapshot {
    pub fn new(ts: i64) -> Self {
        Self {
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            ts,
        }
    }

    /// Best bid price, or None if the bid side is empty.
    pub fn best_bid(&self) -> Option<f64> {
        self.bids.iter().next().map(|(Reverse(p), _)| p.0)
    }

    /// Best ask price, or None if the ask side is empty.
    pub fn best_ask(&self) -> Option<f64> {
        self.asks.iter().next().map(|(p, _)| p.0)
    }

    /// Best bid quantity, or None if bid side is empty.
    pub fn best_bid_qty(&self) -> Option<f64> {
        self.bids.iter().next().map(|(_, q)| *q)
    }

    /// Best ask quantity, or None if ask side is empty.
    pub fn best_ask_qty(&self) -> Option<f64> {
        self.asks.iter().next().map(|(_, q)| *q)
    }

    /// Mid price = (best_ask + best_bid) / 2, or None if either side is empty.
    pub fn mid(&self) -> Option<f64> {
        Some((self.best_bid()? + self.best_ask()?) / 2.0)
    }

    /// Bid-ask spread = best_ask - best_bid, or None if either side is empty.
    pub fn spread(&self) -> Option<f64> {
        Some(self.best_ask()? - self.best_bid()?)
    }

    /// Returns up to `n` bid levels as (price, qty) sorted best-first.
    pub fn top_bids(&self, n: usize) -> Vec<(f64, f64)> {
        self.bids
            .iter()
            .take(n)
            .map(|(Reverse(p), q)| (p.0, *q))
            .collect()
    }

    /// Returns up to `n` ask levels as (price, qty) sorted best-first.
    pub fn top_asks(&self, n: usize) -> Vec<(f64, f64)> {
        self.asks
            .iter()
            .take(n)
            .map(|(p, q)| (p.0, *q))
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.bids.is_empty() || self.asks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ordered_float::OrderedFloat;

    fn make_snap() -> BookSnapshot {
        let mut snap = BookSnapshot::new(1_000_000);
        snap.bids.insert(Reverse(OrderedFloat(100.0)), 10.0);
        snap.bids.insert(Reverse(OrderedFloat(99.0)), 20.0);
        snap.asks.insert(OrderedFloat(101.0), 15.0);
        snap.asks.insert(OrderedFloat(102.0), 5.0);
        snap
    }

    #[test]
    fn best_bid_ask() {
        let s = make_snap();
        assert_eq!(s.best_bid(), Some(100.0));
        assert_eq!(s.best_ask(), Some(101.0));
    }

    #[test]
    fn mid_spread() {
        let s = make_snap();
        assert!((s.mid().unwrap() - 100.5).abs() < 1e-10);
        assert!((s.spread().unwrap() - 1.0).abs() < 1e-10);
    }

    #[test]
    fn top_n() {
        let s = make_snap();
        let bids = s.top_bids(2);
        assert_eq!(bids[0], (100.0, 10.0));
        assert_eq!(bids[1], (99.0, 20.0));
        let asks = s.top_asks(1);
        assert_eq!(asks[0], (101.0, 15.0));
    }

    #[test]
    fn empty_book_returns_none() {
        let s = BookSnapshot::new(0);
        assert!(s.best_bid().is_none());
        assert!(s.best_ask().is_none());
        assert!(s.mid().is_none());
        assert!(s.spread().is_none());
    }
}
