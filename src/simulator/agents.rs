//! Agent models for LOB simulation.
//!
//! Three agent types:
//! - MarketMaker (MM): Avellaneda-Stoikov inventory-aware quoting
//! - NoiseTrader (NT): Random market orders at Poisson rate
//! - InformedTrader (IT): Directional orders based on private signal

use crate::orderbook::{BookDelta, BookSnapshot, Side};
use ordered_float::OrderedFloat;

/// An order emitted by an agent.
#[derive(Debug, Clone)]
pub struct AgentOrder {
    pub delta: BookDelta,
    /// True if this is a market order (crosses the spread immediately).
    pub is_market_order: bool,
}

/// Market maker — posts symmetric quotes around mid with inventory adjustment.
///
/// δ_bid = δ + γ·I   (widen bid quote when long inventory)
/// δ_ask = δ − γ·I
pub struct MarketMaker {
    /// Base half-spread.
    pub base_half_spread: f64,
    /// Inventory penalty coefficient.
    pub gamma: f64,
    /// Current inventory (positive = long).
    pub inventory: f64,
    pub inventory_limit: f64,
    /// Quote size.
    pub quote_qty: f64,
    /// Agent ID.
    pub id: u64,
}

impl MarketMaker {
    pub fn new(id: u64, base_half_spread: f64, gamma: f64, quote_qty: f64, inventory_limit: f64) -> Self {
        Self {
            base_half_spread,
            gamma,
            inventory: 0.0,
            inventory_limit,
            quote_qty,
            id,
        }
    }

    /// Generate bid and ask limit orders given the current book state and time.
    ///
    /// Returns (bid_delta, ask_delta) or None if MM has withdrawn from that side.
    pub fn quote(&self, snap: &BookSnapshot) -> Option<(BookDelta, BookDelta)> {
        let mid = snap.mid()?;
        let ts = snap.ts;

        let delta_bid = self.base_half_spread + self.gamma * self.inventory;
        let delta_ask = self.base_half_spread - self.gamma * self.inventory;

        // Clamp to avoid negative or zero spread adjustments
        let bid_price = mid - delta_bid.max(0.001 * mid);
        let ask_price = mid + delta_ask.max(0.001 * mid);

        // Inventory-exhausted side: don't quote
        let bid_qty = if self.inventory <= -self.inventory_limit {
            0.0 // Cannot go more short
        } else {
            self.quote_qty
        };
        let ask_qty = if self.inventory >= self.inventory_limit {
            0.0 // Cannot go more long
        } else {
            self.quote_qty
        };

        let bid_delta = BookDelta {
            price: OrderedFloat(bid_price),
            qty: bid_qty,
            side: Side::Bid,
            exchange_ts: ts,
            recv_ts: ts,
        };
        let ask_delta = BookDelta {
            price: OrderedFloat(ask_price),
            qty: ask_qty,
            side: Side::Ask,
            exchange_ts: ts,
            recv_ts: ts,
        };

        Some((bid_delta, ask_delta))
    }

    /// Update inventory after a fill.
    pub fn fill(&mut self, qty: f64, side: Side) {
        match side {
            Side::Bid => self.inventory += qty,  // Bought
            Side::Ask => self.inventory -= qty,  // Sold
        }
    }
}

/// Noise trader — random market orders with no information.
pub struct NoiseTrader {
    pub id: u64,
    pub mean_qty: f64,
}

impl NoiseTrader {
    pub fn new(id: u64, mean_qty: f64) -> Self {
        Self { id, mean_qty }
    }

    /// Generate a random market order.
    #[cfg(feature = "sim")]
    pub fn market_order<R: rand::Rng>(
        &self,
        rng: &mut R,
        snap: &BookSnapshot,
    ) -> Option<AgentOrder> {
        use rand_distr::{LogNormal, Distribution};
        let mid = snap.mid()?;
        let ts = snap.ts;

        let sigma = 0.5_f64;
        let mu = self.mean_qty.ln() - 0.5 * sigma * sigma;
        let log_normal = LogNormal::new(mu, sigma).ok()?;
        let qty = log_normal.sample(rng);

        let is_buy = rng.gen_bool(0.5);
        let (price, side) = if is_buy {
            (mid * 1.01, Side::Ask) // Aggressive buy = hit the ask (price above mid)
        } else {
            (mid * 0.99, Side::Bid) // Aggressive sell = hit the bid
        };

        Some(AgentOrder {
            delta: BookDelta {
                price: OrderedFloat(price),
                qty,
                side,
                exchange_ts: ts,
                recv_ts: ts,
            },
            is_market_order: true,
        })
    }
}

/// Informed trader — uses a private price signal to trade directionally.
///
/// Arrives at Poisson rate lambda_it.
/// Has private signal ΔS ~ N(0, sigma_signal).
/// Trades in direction of signal if |ΔS| > theta.
pub struct InformedTrader {
    pub id: u64,
    pub sigma_signal: f64,
    pub theta: f64,
    pub base_qty: f64,
}

impl InformedTrader {
    pub fn new(id: u64, sigma_signal: f64, theta: f64, base_qty: f64) -> Self {
        Self { id, sigma_signal, theta, base_qty }
    }

    /// Generate an order based on private signal.
    ///
    /// Returns None if |signal| < theta (no edge, don't trade).
    #[cfg(feature = "sim")]
    pub fn trade<R: rand::Rng>(
        &self,
        rng: &mut R,
        snap: &BookSnapshot,
    ) -> Option<AgentOrder> {
        use rand_distr::{Normal, Distribution};
        let mid = snap.mid()?;
        let ts = snap.ts;

        let normal = Normal::new(0.0, self.sigma_signal).ok()?;
        let signal: f64 = normal.sample(rng);

        if signal.abs() < self.theta {
            return None;
        }

        // Size proportional to signal strength
        let qty = self.base_qty * (signal.abs() / self.sigma_signal);
        let is_buy = signal > 0.0;

        let (price, side) = if is_buy {
            (mid * 1.005, Side::Ask)
        } else {
            (mid * 0.995, Side::Bid)
        };

        Some(AgentOrder {
            delta: BookDelta {
                price: OrderedFloat(price),
                qty,
                side,
                exchange_ts: ts,
                recv_ts: ts,
            },
            is_market_order: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orderbook::{Orderbook, BookDelta as BD, Side as S};
    use ordered_float::OrderedFloat as OF;

    fn snap_with_mid(mid: f64) -> BookSnapshot {
        let mut book = Orderbook::new();
        book.apply_delta(&BD { price: OF(mid - 0.5), qty: 10.0, side: S::Bid, exchange_ts: 0, recv_ts: 0 }).unwrap();
        book.apply_delta(&BD { price: OF(mid + 0.5), qty: 10.0, side: S::Ask, exchange_ts: 0, recv_ts: 0 }).unwrap();
        book.export_snapshot(5)
    }

    #[test]
    fn mm_quotes_around_mid() {
        let mm = MarketMaker::new(1, 0.1, 0.01, 5.0, 100.0);
        let snap = snap_with_mid(100.0);
        let (bid, ask) = mm.quote(&snap).unwrap();
        assert!(bid.price.0 < 100.0, "bid should be below mid");
        assert!(ask.price.0 > 100.0, "ask should be above mid");
        assert!(ask.price > bid.price, "ask must be above bid");
    }

    #[test]
    fn mm_widens_bid_when_long() {
        let mut mm = MarketMaker::new(1, 0.1, 0.05, 5.0, 100.0);
        mm.inventory = 10.0; // Long
        let snap = snap_with_mid(100.0);
        let (bid1, _) = mm.quote(&snap).unwrap();

        let mut mm2 = MarketMaker::new(2, 0.1, 0.05, 5.0, 100.0);
        mm2.inventory = 0.0;
        let (bid2, _) = mm2.quote(&snap).unwrap();

        // Long inventory → bid price lower (wider bid)
        assert!(bid1.price < bid2.price, "long MM should quote lower bid");
    }

    #[test]
    fn empty_book_returns_none() {
        let mm = MarketMaker::new(1, 0.1, 0.01, 5.0, 100.0);
        let snap = BookSnapshot::new(0);
        assert!(mm.quote(&snap).is_none());
    }
}
