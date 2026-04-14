use ordered_float::OrderedFloat;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Bid,
    Ask,
}

/// A single incremental update to the order book.
#[derive(Debug, Clone)]
pub struct BookDelta {
    /// Price level (parsed from string, never computed arithmetically).
    pub price: OrderedFloat<f64>,
    /// New total quantity at this price level. qty < f64::EPSILON means delete.
    pub qty: f64,
    pub side: Side,
    /// Exchange-provided timestamp in microseconds (0 if unavailable).
    pub exchange_ts: i64,
    /// Local receipt timestamp in microseconds.
    pub recv_ts: i64,
}

impl BookDelta {
    pub fn is_delete(&self) -> bool {
        self.qty < f64::EPSILON
    }
}

#[derive(Debug, Error)]
pub enum BookError {
    #[error("crossed book: best_bid {best_bid} >= best_ask {best_ask}")]
    CrossedBook { best_bid: f64, best_ask: f64 },
    #[error("negative quantity {qty} at price {price}")]
    NegativeQty { price: f64, qty: f64 },
    #[error("non-finite price {0}")]
    NonFinitePrice(f64),
}
