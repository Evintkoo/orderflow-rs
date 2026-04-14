//! Stochastic LOB simulation event loop.
//!
//! Simulates a full trading day using:
//! - Ornstein-Uhlenbeck price process with jumps
//! - 3-agent population: MarketMaker, NoiseTrader, InformedTrader
//! - Hawkes process order arrival
//! - 100ms snapshot export (configurable)

#[cfg(feature = "sim")]
use rand::Rng;
#[cfg(feature = "sim")]
use rand_distr::{Normal, Distribution};

use crate::orderbook::Orderbook;
use crate::features::FeatureVector;
use crate::simulator::agents::{InformedTrader, MarketMaker, NoiseTrader};
use crate::simulator::flow_gen::{HawkesArrival, HawkesParams, PoissonArrival};

/// Parameters for the stochastic price process.
/// dS(t) = κ(μ − S(t))dt + σ dW(t) + J dN(t)
#[derive(Debug, Clone)]
pub struct PriceProcessParams {
    /// Mean reversion speed (1/s).  0 = random walk.
    pub kappa: f64,
    /// Long-run mean price.
    pub mu: f64,
    /// Diffusion coefficient (per √s).
    pub sigma: f64,
    /// Jump intensity (jumps/s).
    pub lambda_jump: f64,
    /// Jump size std dev.
    pub sigma_jump: f64,
    /// Time step in seconds.
    pub dt: f64,
    /// Minimum tick size (prices snap to multiples of tick_size).
    /// Required for OFI to be non-zero: must have repeated level updates.
    pub tick_size: f64,
}

impl Default for PriceProcessParams {
    fn default() -> Self {
        Self {
            kappa: 0.1,
            mu: 100.0,
            sigma: 0.01,  // 10× larger than before → realistic daily vol ~1.7%
            lambda_jump: 0.02,
            sigma_jump: 0.05,
            dt: 0.001, // 1ms
            tick_size: 0.01, // prices snap to nearest cent → levels get reused → OFI ≠ 0
        }
    }
}

/// Configuration for one simulation run.
#[derive(Debug, Clone)]
pub struct SimConfig {
    pub price_params: PriceProcessParams,
    /// Snapshot interval in seconds.
    pub snapshot_interval_s: f64,
    /// Total simulation duration in seconds.
    pub duration_s: f64,
    /// Hawkes params for noise trader arrivals.
    pub nt_hawkes: HawkesParams,
    /// Poisson arrival rate for informed traders.
    pub it_lambda: f64,
    /// MM refresh rate (quotes/second).
    pub mm_refresh_rate: f64,
    /// Number of market makers.
    pub n_mm: usize,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            price_params: PriceProcessParams::default(),
            snapshot_interval_s: 0.1,
            duration_s: 28800.0, // 8 hours
            nt_hawkes: HawkesParams::new(5.0, 0.3, 1.0),
            it_lambda: 0.5,
            mm_refresh_rate: 2.0,
            n_mm: 3,
        }
    }
}

/// Output of a simulation run.
pub struct SimOutput {
    pub feature_vectors: Vec<FeatureVector>,
    /// Ground-truth mid-price series at each snapshot tick (for label computation).
    pub mid_prices: Vec<(f64, f64)>, // (sim_time, mid_price)
    pub informed_arrivals: Vec<f64>, // sim times of informed trader orders
}

/// Snap a price to the nearest tick.
#[inline]
fn snap_to_tick(price: f64, tick: f64) -> f64 {
    if tick <= 0.0 { return price; }
    (price / tick).round() * tick
}

/// Run a single simulation path.
///
/// Key design decisions vs. the original stub:
///  - Prices snapped to `tick_size` grid → repeated level updates → OFI ≠ 0
///  - Market orders CONSUME liquidity (matching engine) instead of adding to book
///  - MMs cancel their old quotes before re-posting (no stale depth accumulation)
///  - Larger σ (0.01/√s) for visible price dynamics
#[cfg(feature = "sim")]
pub fn run_simulation<R: Rng>(rng: &mut R, config: &SimConfig) -> SimOutput {
    use crate::features::FeatureExtractor;

    let p = &config.price_params;
    let dt = p.dt;
    let tick = p.tick_size;
    let steps = (config.duration_s / dt) as usize;
    let snap_every = (config.snapshot_interval_s / dt) as usize;

    // Price process state
    let mut s = snap_to_tick(p.mu, tick);

    // LOB engine — seed with 5 levels on each side
    let mut book = Orderbook::new();
    {
        use crate::orderbook::{BookDelta, Side};
        use ordered_float::OrderedFloat;
        let half_spread = snap_to_tick(2.0 * tick, tick).max(tick); // 2-tick half-spread
        for i in 0..5_usize {
            let bid_p = snap_to_tick(s - half_spread - i as f64 * tick, tick);
            let ask_p = snap_to_tick(s + half_spread + i as f64 * tick, tick);
            book.apply_delta(&BookDelta { price: OrderedFloat(bid_p), qty: 10.0, side: Side::Bid, exchange_ts: 0, recv_ts: 0 }).ok();
            book.apply_delta(&BookDelta { price: OrderedFloat(ask_p), qty: 10.0, side: Side::Ask, exchange_ts: 0, recv_ts: 0 }).ok();
        }
    }

    // Feature extractor
    let mut fx = FeatureExtractor::new("sim", "SIM/USD", "simulation");

    // Agents — MMs track their active quote prices for cancellation
    // (bid_price, ask_price) per MM; None before first quote
    let mut mm_agents: Vec<MarketMaker> = (0..config.n_mm)
        .map(|i| MarketMaker::new(i as u64, 2.0 * tick, 0.005, 10.0, 500.0))
        .collect();
    let mut mm_active: Vec<Option<(f64, f64)>> = vec![None; config.n_mm];

    let nt = NoiseTrader::new(100, 5.0);
    let it = InformedTrader::new(200, p.sigma * 15.0, p.sigma * 3.0, 5.0);

    // Arrival generators
    let nt_gen = HawkesArrival::new(config.nt_hawkes.clone());
    let it_gen = PoissonArrival::new(config.it_lambda);

    let nt_arrivals = nt_gen.simulate(rng, 0.0, config.duration_s);
    let it_arrivals = it_gen.generate(rng, 0.0, (config.it_lambda * config.duration_s * 2.0) as usize);

    let mut nt_idx = 0;
    let mut it_idx = 0;
    let mut mm_last_refresh = -1.0_f64;

    let mut output = SimOutput {
        feature_vectors: Vec::new(),
        mid_prices: Vec::new(),
        informed_arrivals: Vec::new(),
    };

    let normal = Normal::new(0.0, 1.0_f64).unwrap();
    let jump_normal = Normal::new(0.0, p.sigma_jump).unwrap();

    for step in 0..steps {
        let t = step as f64 * dt;
        let ts_us = (t * 1_000_000.0) as i64;

        // --- Price process: Euler-Maruyama ---
        let dw: f64 = normal.sample(rng) * dt.sqrt();
        let mean_rev = p.kappa * (p.mu - s) * dt;
        let diffusion = p.sigma * dw;
        let jump = if rng.gen_bool(p.lambda_jump * dt) { jump_normal.sample(rng) } else { 0.0 };
        s = snap_to_tick((s + mean_rev + diffusion + jump).max(0.001 * p.mu), tick);

        // --- MM quoting ---
        let mm_refresh_dt = 1.0 / config.mm_refresh_rate;
        if t - mm_last_refresh >= mm_refresh_dt {
            mm_last_refresh = t;
            let snap = book.export_snapshot(10);
            use crate::orderbook::{BookDelta, Side};
            use ordered_float::OrderedFloat;

            for (idx, mm) in mm_agents.iter_mut().enumerate() {
                // Cancel previous quotes
                if let Some((old_bid, old_ask)) = mm_active[idx].take() {
                    book.apply_delta(&BookDelta { price: OrderedFloat(old_bid), qty: 0.0, side: Side::Bid, exchange_ts: ts_us, recv_ts: ts_us }).ok();
                    book.apply_delta(&BookDelta { price: OrderedFloat(old_ask), qty: 0.0, side: Side::Ask, exchange_ts: ts_us, recv_ts: ts_us }).ok();
                }

                // Post new quotes (prices snapped to tick grid)
                if let Some((mut bid_d, mut ask_d)) = mm.quote(&snap) {
                    bid_d.price = OrderedFloat(snap_to_tick(bid_d.price.0, tick));
                    ask_d.price = OrderedFloat(snap_to_tick(ask_d.price.0, tick));
                    // Ensure no crossed book after snapping
                    if bid_d.price < ask_d.price && bid_d.qty > 0.0 && ask_d.qty > 0.0 {
                        book.apply_delta(&bid_d).ok();
                        book.apply_delta(&ask_d).ok();
                        mm_active[idx] = Some((bid_d.price.0, ask_d.price.0));
                    }
                }
            }
        }

        // --- NT arrivals (consume liquidity, not add) ---
        while nt_idx < nt_arrivals.len() && nt_arrivals[nt_idx].t <= t {
            let snap = book.export_snapshot(3);
            if snap.mid().is_some() {
                let is_buy = rng.gen_bool(0.5);
                let qty = {
                    use rand_distr::{LogNormal, Distribution};
                    let mu_log = nt.mean_qty.ln() - 0.5 * 0.25;
                    LogNormal::new(mu_log, 0.5).ok()
                        .map(|d| d.sample(rng))
                        .unwrap_or(nt.mean_qty)
                };
                book.consume_market_order(qty, is_buy, ts_us);
            }
            nt_idx += 1;
        }

        // --- IT arrivals (consume in signal direction) ---
        while it_idx < it_arrivals.len() && it_arrivals[it_idx].t <= t {
            let snap = book.export_snapshot(3);
            if let Some(order) = it.trade(rng, &snap) {
                let is_buy = order.delta.side == crate::orderbook::Side::Ask;
                let qty = order.delta.qty;
                book.consume_market_order(qty, is_buy, ts_us);
                output.informed_arrivals.push(t);
            }
            it_idx += 1;
        }

        // --- Snapshot export ---
        if step % snap_every == 0 {
            let snap = book.export_snapshot(10);
            let mut snap_with_ts = snap.clone();
            snap_with_ts.ts = ts_us;

            let fv = fx.extract(&snap_with_ts, false, false);
            // Always push mid (use fundamental price `s` when book is empty)
            let mid = snap_with_ts.mid().unwrap_or(s);
            output.mid_prices.push((t, mid));
            output.feature_vectors.push(fv);
        }
    }

    output
}

#[cfg(test)]
#[cfg(feature = "sim")]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::SmallRng;

    #[test]
    fn simulation_produces_feature_vectors() {
        let mut rng = SmallRng::seed_from_u64(42);
        let mut config = SimConfig::default();
        config.duration_s = 60.0; // 1 minute for fast test
        config.price_params.dt = 0.01; // 10ms steps

        let output = run_simulation(&mut rng, &config);
        // 1 minute / 0.1s snapshot interval = ~600 snapshots
        assert!(!output.feature_vectors.is_empty(), "should produce feature vectors");
        assert!(!output.mid_prices.is_empty(), "should track mid prices");
    }

    #[test]
    fn price_process_stays_positive() {
        let mut rng = SmallRng::seed_from_u64(7);
        let mut config = SimConfig::default();
        config.duration_s = 100.0;
        config.price_params.dt = 0.01;
        config.price_params.sigma_jump = 0.1; // Large jumps

        let output = run_simulation(&mut rng, &config);
        for (_, mid) in &output.mid_prices {
            assert!(*mid > 0.0, "mid price should stay positive");
        }
    }
}
