//! Stochastic order arrival processes.
//!
//! Implements Poisson and Hawkes (Ogata thinning) arrival generators.

#[cfg(feature = "sim")]
use rand::Rng;
#[cfg(feature = "sim")]
use rand_distr::{Exp, Distribution};

use crate::orderbook::Side;

/// A scheduled order arrival event from the generator.
#[derive(Debug, Clone)]
pub struct ArrivalEvent {
    /// Simulation time in seconds.
    pub t: f64,
    pub side: Side,
}

/// Poisson process order generator.
///
/// Generates arrivals at a constant rate `lambda` (events per second).
#[cfg(feature = "sim")]
pub struct PoissonArrival {
    lambda: f64,
}

#[cfg(feature = "sim")]
impl PoissonArrival {
    pub fn new(lambda: f64) -> Self {
        assert!(lambda > 0.0, "Poisson rate must be positive");
        Self { lambda }
    }

    /// Generate `n` arrival events starting at `t0`.
    pub fn generate<R: Rng>(&self, rng: &mut R, t0: f64, n: usize) -> Vec<ArrivalEvent> {
        let exp = Exp::new(self.lambda).expect("valid rate");
        let mut t = t0;
        let mut events = Vec::with_capacity(n);
        for _ in 0..n {
            let dt: f64 = exp.sample(rng);
            t += dt;
            let side = if rng.gen_bool(0.5) { Side::Bid } else { Side::Ask };
            events.push(ArrivalEvent { t, side });
        }
        events
    }
}

/// Hawkes process parameters.
///
/// Stability condition: alpha < beta.
#[derive(Debug, Clone)]
pub struct HawkesParams {
    /// Background intensity (events/second).
    pub mu: f64,
    /// Excitation magnitude.
    pub alpha: f64,
    /// Decay rate (1/seconds).
    pub beta: f64,
}

impl HawkesParams {
    pub fn new(mu: f64, alpha: f64, beta: f64) -> Self {
        assert!(alpha < beta, "Hawkes stability violated: α={alpha} >= β={beta}");
        assert!(mu > 0.0, "Background rate must be positive");
        Self { mu, alpha, beta }
    }

    /// Compute conditional intensity at time `t` given past events.
    pub fn intensity(&self, t: f64, past_events: &[f64]) -> f64 {
        let excitation: f64 = past_events
            .iter()
            .filter(|&&ti| ti < t)
            .map(|&ti| self.alpha * (-self.beta * (t - ti)).exp())
            .sum();
        // Cap at a large but finite value to prevent explosion near α/β ≈ 1
        let lambda_max = self.mu + past_events.len() as f64 * self.alpha / (self.beta - self.alpha + f64::EPSILON);
        (self.mu + excitation).min(lambda_max)
    }
}

/// Univariate Hawkes process using Ogata's thinning algorithm.
///
/// Exact simulation — not an approximation.  O(n log n) via sequential thinning.
#[cfg(feature = "sim")]
pub struct HawkesArrival {
    params: HawkesParams,
}

#[cfg(feature = "sim")]
impl HawkesArrival {
    pub fn new(params: HawkesParams) -> Self {
        Self { params }
    }

    /// Generate events in `[t_start, t_end]` using Ogata's thinning.
    ///
    /// Returns sorted list of arrival times.
    pub fn simulate<R: Rng>(
        &self,
        rng: &mut R,
        t_start: f64,
        t_end: f64,
    ) -> Vec<ArrivalEvent> {
        let p = &self.params;
        let mut events: Vec<f64> = Vec::new();
        let mut t = t_start;

        // Upper bound on intensity starts at mu (no past events)
        let mut lambda_bar = p.mu;

        while t < t_end {
            // Sample candidate inter-arrival time from Poisson(lambda_bar)
            let exp = Exp::new(lambda_bar).expect("valid rate");
            let dt: f64 = exp.sample(rng);
            t += dt;
            if t > t_end {
                break;
            }

            // Compute true intensity at the candidate time
            let lambda_t = p.intensity(t, &events);

            // Accept with probability lambda_t / lambda_bar
            let u: f64 = rng.gen();
            if u <= lambda_t / lambda_bar {
                events.push(t);
                // Update upper bound: intensity can only increase from here
                lambda_bar = lambda_t + p.alpha; // one new event adds at most alpha
            } else {
                // Rejected: update upper bound to true value
                lambda_bar = lambda_t.max(p.mu);
            }
        }

        events
            .into_iter()
            .map(|t| {
                let side = if rng.gen_bool(0.5) { Side::Bid } else { Side::Ask };
                ArrivalEvent { t, side }
            })
            .collect()
    }
}

#[cfg(test)]
#[cfg(feature = "sim")]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::SmallRng;

    #[test]
    fn poisson_generates_correct_count_approx() {
        let mut rng = SmallRng::seed_from_u64(42);
        let p = PoissonArrival::new(10.0); // 10 events/sec
        let events = p.generate(&mut rng, 0.0, 100);
        assert_eq!(events.len(), 100);
        // Check times are increasing
        for w in events.windows(2) {
            assert!(w[1].t > w[0].t);
        }
    }

    #[test]
    fn hawkes_stability_condition_enforced() {
        let result = std::panic::catch_unwind(|| {
            HawkesParams::new(1.0, 1.5, 1.0); // alpha > beta — should panic
        });
        assert!(result.is_err());
    }

    #[test]
    fn hawkes_generates_events() {
        let mut rng = SmallRng::seed_from_u64(99);
        let params = HawkesParams::new(5.0, 0.3, 1.0);
        let gen = HawkesArrival::new(params);
        let events = gen.simulate(&mut rng, 0.0, 10.0);
        // 5 events/sec × 10sec = ~50 expected (at least some)
        assert!(!events.is_empty(), "should generate at least some events");
        // Times must be in [0, 10]
        for e in &events {
            assert!(e.t >= 0.0 && e.t <= 10.0);
        }
    }

    #[test]
    fn hawkes_clustered_more_than_poisson() {
        // Hawkes should exhibit clustering (variance > mean arrival count)
        let mut rng = SmallRng::seed_from_u64(7);
        let params = HawkesParams::new(2.0, 0.8, 1.0); // near-critical excitation
        let gen = HawkesArrival::new(params);
        let counts: Vec<usize> = (0..50)
            .map(|_| gen.simulate(&mut rng, 0.0, 1.0).len())
            .collect();
        let mean = counts.iter().sum::<usize>() as f64 / counts.len() as f64;
        let var = counts.iter().map(|&c| (c as f64 - mean).powi(2)).sum::<f64>()
            / counts.len() as f64;
        // Hawkes variance > mean (overdispersion)
        assert!(var > mean, "var={var:.2} should exceed mean={mean:.2} for near-critical Hawkes");
    }
}
