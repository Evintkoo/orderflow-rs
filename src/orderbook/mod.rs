pub mod delta;
pub mod snapshot;

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use ordered_float::OrderedFloat;

pub use delta::{BookDelta, BookError, Side};
pub use snapshot::BookSnapshot;

/// L2 order book — mutable state owned by the ingestion/delta thread.
///
/// A shared read view is available via `shared_snapshot()`.
pub struct Orderbook {
    bids: BTreeMap<Reverse<OrderedFloat<f64>>, f64>,
    asks: BTreeMap<OrderedFloat<f64>, f64>,
    /// Shared snapshot updated after every delta application.
    shared: Arc<RwLock<BookSnapshot>>,
    ts: i64,
}

impl Orderbook {
    pub fn new() -> Self {
        let snap = BookSnapshot::new(0);
        Self {
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            shared: Arc::new(RwLock::new(snap)),
            ts: 0,
        }
    }

    /// Returns true if the book has at least one bid or ask level.
    pub fn has_data(&self) -> bool {
        !self.bids.is_empty() || !self.asks.is_empty()
    }

    /// Returns a clone of the Arc so feature threads can acquire read locks.
    pub fn shared_snapshot(&self) -> Arc<RwLock<BookSnapshot>> {
        Arc::clone(&self.shared)
    }

    /// Apply a single incremental delta.  O(log n).
    ///
    /// On success, updates the shared snapshot.
    /// In debug builds, checks the crossed-book invariant after each update.
    pub fn apply_delta(&mut self, delta: &BookDelta) -> Result<(), BookError> {
        // Validate price
        if !delta.price.is_finite() {
            return Err(BookError::NonFinitePrice(delta.price.0));
        }
        if delta.qty < 0.0 {
            return Err(BookError::NegativeQty {
                price: delta.price.0,
                qty: delta.qty,
            });
        }

        match delta.side {
            Side::Bid => {
                let key = Reverse(delta.price);
                if delta.is_delete() {
                    self.bids.remove(&key);
                } else {
                    self.bids.insert(key, delta.qty);
                }
            }
            Side::Ask => {
                let key = delta.price;
                if delta.is_delete() {
                    self.asks.remove(&key);
                } else {
                    self.asks.insert(key, delta.qty);
                }
            }
        }

        self.ts = delta.exchange_ts.max(self.ts);

        #[cfg(debug_assertions)]
        self.check_invariants()?;

        self.publish_snapshot();
        Ok(())
    }

    /// Apply a snapshot reset (e.g., after a sequence gap resync).
    pub fn apply_snapshot_reset(
        &mut self,
        bids: impl IntoIterator<Item = (f64, f64)>,
        asks: impl IntoIterator<Item = (f64, f64)>,
        ts: i64,
    ) {
        self.bids.clear();
        self.asks.clear();
        for (price, qty) in bids {
            if qty >= f64::EPSILON {
                self.bids.insert(Reverse(OrderedFloat(price)), qty);
            }
        }
        for (price, qty) in asks {
            if qty >= f64::EPSILON {
                self.asks.insert(OrderedFloat(price), qty);
            }
        }
        self.ts = ts;
        self.publish_snapshot();
    }

    /// Export the current top-`n` levels as an owned `BookSnapshot`.
    pub fn export_snapshot(&self, n: usize) -> BookSnapshot {
        let mut snap = BookSnapshot::new(self.ts);
        for (k, v) in self.bids.iter().take(n) {
            snap.bids.insert(*k, *v);
        }
        for (k, v) in self.asks.iter().take(n) {
            snap.asks.insert(*k, *v);
        }
        snap
    }

    /// Consume liquidity as a market order (matching engine).
    ///
    /// A buy order walks up the ask side; a sell order walks down the bid side.
    /// Partially-filled levels have their quantity reduced; fully-consumed levels
    /// are removed. Returns the volume-weighted average fill price (0.0 if unfilled).
    pub fn consume_market_order(&mut self, mut qty: f64, is_buy: bool, ts: i64) -> f64 {
        if qty <= 0.0 { return 0.0; }

        let mut total_cost = 0.0_f64;
        let mut total_filled = 0.0_f64;

        if is_buy {
            let mut to_delete: Vec<OrderedFloat<f64>> = Vec::new();
            for (&price, ask_qty) in self.asks.iter_mut() {
                if qty <= 0.0 { break; }
                let fill = ask_qty.min(qty);
                total_cost   += price.0 * fill;
                total_filled += fill;
                qty          -= fill;
                *ask_qty     -= fill;
                if *ask_qty < f64::EPSILON {
                    to_delete.push(price);
                }
            }
            for p in to_delete { self.asks.remove(&p); }
        } else {
            let mut to_delete: Vec<std::cmp::Reverse<OrderedFloat<f64>>> = Vec::new();
            for (&rev_price, bid_qty) in self.bids.iter_mut() {
                if qty <= 0.0 { break; }
                let fill = bid_qty.min(qty);
                total_cost   += rev_price.0.0 * fill;
                total_filled += fill;
                qty          -= fill;
                *bid_qty     -= fill;
                if *bid_qty < f64::EPSILON {
                    to_delete.push(rev_price);
                }
            }
            for p in to_delete { self.bids.remove(&p); }
        }

        self.ts = ts.max(self.ts);
        self.publish_snapshot();

        if total_filled > 0.0 { total_cost / total_filled } else { 0.0 }
    }

    fn publish_snapshot(&self) {
        // Export full book into shared snapshot (no allocation in common case —
        // BTreeMap clone reuses allocator, acceptable outside hot-path).
        let snap = BookSnapshot {
            bids: self.bids.clone(),
            asks: self.asks.clone(),
            ts: self.ts,
        };
        if let Ok(mut guard) = self.shared.write() {
            *guard = snap;
        }
    }

    #[cfg(debug_assertions)]
    fn check_invariants(&self) -> Result<(), BookError> {
        let best_bid = self.bids.iter().next().map(|(Reverse(p), _)| p.0);
        let best_ask = self.asks.iter().next().map(|(p, _)| p.0);
        if let (Some(bb), Some(ba)) = (best_bid, best_ask) {
            if bb >= ba {
                return Err(BookError::CrossedBook {
                    best_bid: bb,
                    best_ask: ba,
                });
            }
        }
        Ok(())
    }
}

impl Default for Orderbook {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delta(price: f64, qty: f64, side: Side) -> BookDelta {
        BookDelta {
            price: OrderedFloat(price),
            qty,
            side,
            exchange_ts: 1_000_000,
            recv_ts: 1_000_001,
        }
    }

    #[test]
    fn apply_and_read() {
        let mut book = Orderbook::new();
        book.apply_delta(&delta(100.0, 10.0, Side::Bid)).unwrap();
        book.apply_delta(&delta(101.0, 5.0, Side::Ask)).unwrap();

        let snap = book.export_snapshot(10);
        assert_eq!(snap.best_bid(), Some(100.0));
        assert_eq!(snap.best_ask(), Some(101.0));
    }

    #[test]
    fn delete_level() {
        let mut book = Orderbook::new();
        book.apply_delta(&delta(100.0, 10.0, Side::Bid)).unwrap();
        book.apply_delta(&delta(101.0, 5.0, Side::Ask)).unwrap();
        // Delete bid level
        book.apply_delta(&delta(100.0, 0.0, Side::Bid)).unwrap();
        let snap = book.export_snapshot(10);
        assert!(snap.bids.is_empty());
        assert!(snap.best_bid().is_none());
    }

    #[test]
    fn multiple_levels_sorted() {
        let mut book = Orderbook::new();
        book.apply_delta(&delta(98.0, 5.0, Side::Bid)).unwrap();
        book.apply_delta(&delta(100.0, 10.0, Side::Bid)).unwrap();
        book.apply_delta(&delta(99.0, 8.0, Side::Bid)).unwrap();
        book.apply_delta(&delta(101.0, 5.0, Side::Ask)).unwrap();

        let snap = book.export_snapshot(3);
        let bids = snap.top_bids(3);
        // Must be sorted descending
        assert_eq!(bids[0].0, 100.0);
        assert_eq!(bids[1].0, 99.0);
        assert_eq!(bids[2].0, 98.0);
    }

    #[test]
    fn shared_snapshot_updated() {
        let mut book = Orderbook::new();
        let shared = book.shared_snapshot();
        book.apply_delta(&delta(100.0, 10.0, Side::Bid)).unwrap();
        book.apply_delta(&delta(101.0, 5.0, Side::Ask)).unwrap();

        let snap = shared.read().unwrap();
        assert_eq!(snap.best_bid(), Some(100.0));
    }

    #[test]
    fn snapshot_reset_clears_book() {
        let mut book = Orderbook::new();
        book.apply_delta(&delta(100.0, 10.0, Side::Bid)).unwrap();
        book.apply_delta(&delta(101.0, 5.0, Side::Ask)).unwrap();
        book.apply_snapshot_reset(
            vec![(99.0, 20.0)],
            vec![(102.0, 10.0)],
            2_000_000,
        );
        let snap = book.export_snapshot(5);
        assert_eq!(snap.best_bid(), Some(99.0));
        assert_eq!(snap.best_ask(), Some(102.0));
        assert_eq!(snap.bids.len(), 1);
    }

    #[test]
    fn negative_qty_rejected() {
        let mut book = Orderbook::new();
        let err = book.apply_delta(&BookDelta {
            price: OrderedFloat(100.0),
            qty: -1.0,
            side: Side::Bid,
            exchange_ts: 0,
            recv_ts: 0,
        });
        assert!(err.is_err());
    }

    #[test]
    fn non_finite_price_rejected() {
        let mut book = Orderbook::new();
        let err = book.apply_delta(&BookDelta {
            price: OrderedFloat(f64::NAN),
            qty: 1.0,
            side: Side::Bid,
            exchange_ts: 0,
            recv_ts: 0,
        });
        assert!(err.is_err());
    }
}
