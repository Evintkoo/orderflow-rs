//! Trade-derived pressure features.
//!
//! - trade_intensity: Hawkes λ̂(t) — conditional arrival intensity (Bacry 2015)
//! - price_impact: ΔP_last_trade / Q_last_trade (Kyle 1985)

/// A single trade event.
#[derive(Debug, Clone)]
pub struct TradeEvent {
    /// Trade execution timestamp in microseconds.
    pub ts: i64,
    /// Trade price.
    pub price: f64,
    /// Trade quantity.
    pub qty: f64,
    /// True if the aggressor was a buyer (taker buy).
    pub is_buy: bool,
}

/// Ring buffer of recent trades used for Hawkes intensity estimation.
pub struct TradeBuffer {
    trades: std::collections::VecDeque<TradeEvent>,
    /// Maximum age of a trade to retain (microseconds).
    max_age_us: i64,
}

impl TradeBuffer {
    pub fn new(max_age_us: i64) -> Self {
        Self {
            trades: std::collections::VecDeque::new(),
            max_age_us,
        }
    }

    pub fn push(&mut self, trade: TradeEvent, now_us: i64) {
        self.trades.push_back(trade);
        self.evict(now_us);
    }

    fn evict(&mut self, now_us: i64) {
        while let Some(front) = self.trades.front() {
            if now_us - front.ts > self.max_age_us {
                self.trades.pop_front();
            } else {
                break;
            }
        }
    }

    pub fn trades(&self) -> &std::collections::VecDeque<TradeEvent> {
        &self.trades
    }

    pub fn last_trade(&self) -> Option<&TradeEvent> {
        self.trades.back()
    }

    pub fn len(&self) -> usize {
        self.trades.len()
    }

    pub fn is_empty(&self) -> bool {
        self.trades.is_empty()
    }
}

/// Hawkes conditional intensity at time `now_us` using exponential kernel.
///
/// λ(t) = μ₀ + Σᵢ α · exp(−β · (t − tᵢ))
///
/// Parameters must satisfy α < β (stability condition).
pub fn hawkes_intensity(
    buffer: &TradeBuffer,
    now_us: i64,
    mu: f64,
    alpha: f64,
    beta: f64,
) -> Option<f64> {
    debug_assert!(alpha < beta, "Hawkes stability violated: α >= β");
    if buffer.is_empty() {
        return Some(mu);
    }
    let mut intensity = mu;
    let now_s = now_us as f64 * 1e-6;
    for trade in buffer.trades() {
        let dt = now_s - trade.ts as f64 * 1e-6;
        if dt < 0.0 {
            continue;
        }
        intensity += alpha * (-beta * dt).exp();
    }
    if intensity.is_finite() { Some(intensity) } else { None }
}

/// Price impact of the last trade: ΔP / Q.
///
/// `prev_mid`: mid price before the trade.
/// `trade`: the trade event.
/// Returns signed impact (positive for buys moving price up).
pub fn price_impact_ratio(prev_mid: f64, trade: &TradeEvent) -> Option<f64> {
    if trade.qty < f64::EPSILON || prev_mid < f64::EPSILON {
        return None;
    }
    let dp = trade.price - prev_mid;
    Some(dp / trade.qty)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hawkes_no_trades_equals_mu() {
        let buf = TradeBuffer::new(10_000_000);
        let lambda = hawkes_intensity(&buf, 1_000_000, 0.5, 0.3, 1.0).unwrap();
        assert!((lambda - 0.5).abs() < 1e-10);
    }

    #[test]
    fn hawkes_decays_with_time() {
        let mut buf = TradeBuffer::new(100_000_000);
        // One trade at t=0
        buf.push(
            TradeEvent { ts: 0, price: 100.0, qty: 1.0, is_buy: true },
            1_000_000,
        );
        let lambda_early = hawkes_intensity(&buf, 100_000, 0.5, 0.3, 1.0).unwrap(); // t=0.1s
        let lambda_late = hawkes_intensity(&buf, 5_000_000, 0.5, 0.3, 1.0).unwrap(); // t=5s
        assert!(lambda_early > lambda_late, "intensity should decay");
    }

    #[test]
    fn price_impact_positive_for_buy() {
        let trade = TradeEvent { ts: 0, price: 101.0, qty: 10.0, is_buy: true };
        let impact = price_impact_ratio(100.0, &trade).unwrap();
        assert!(impact > 0.0);
        // (101 - 100) / 10 = 0.1
        assert!((impact - 0.1).abs() < 1e-10);
    }

    #[test]
    fn price_impact_zero_qty_returns_none() {
        let trade = TradeEvent { ts: 0, price: 101.0, qty: 0.0, is_buy: true };
        assert!(price_impact_ratio(100.0, &trade).is_none());
    }

    #[test]
    fn trade_buffer_evicts_old_trades() {
        let mut buf = TradeBuffer::new(1_000_000); // 1 second max age
        buf.push(TradeEvent { ts: 0, price: 100.0, qty: 1.0, is_buy: true }, 500_000);
        buf.push(TradeEvent { ts: 500_000, price: 100.0, qty: 1.0, is_buy: false }, 500_000);
        assert_eq!(buf.len(), 2);
        // Now at t=2s, both trades are older than 1s
        buf.push(TradeEvent { ts: 2_000_000, price: 100.0, qty: 1.0, is_buy: true }, 2_000_000);
        // Only the trade at t=2s should remain
        assert_eq!(buf.len(), 1);
    }
}
