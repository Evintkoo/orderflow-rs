//! Forward-return label computation using a delay queue.
//!
//! Ensures zero label leakage: feature vectors are held for max(τ) before
//! being emitted with all forward-return labels attached.
//!
//! Labels:
//!   r(t, τ) = (mid(t+τ) − mid(t)) / mid(t)   for τ ∈ {1s, 5s, 30s, 300s}
//!   sign_label = +1 if r > θ·spread; -1 if r < -θ·spread; 0 otherwise  (θ=0.5)

use std::collections::VecDeque;
use crate::features::FeatureVector;

/// A labeled feature vector ready for output.
#[derive(Debug, Clone)]
pub struct LabeledFeatureVector {
    pub fv: FeatureVector,
    /// Forward return at 1s.
    pub r_1s: Option<f64>,
    /// Forward return at 5s.
    pub r_5s: Option<f64>,
    /// Forward return at 30s.
    pub r_30s: Option<f64>,
    /// Forward return at 300s (5 min).
    pub r_300s: Option<f64>,
    /// Sign label at 1s: +1, 0, or -1.
    pub sign_1s: Option<i8>,
    /// Sign label at 5s.
    pub sign_5s: Option<i8>,
}

/// Label horizons in microseconds.
const HORIZONS_US: [i64; 4] = [
    1_000_000,   // 1s
    5_000_000,   // 5s
    30_000_000,  // 30s
    300_000_000, // 300s
];

const MAX_HORIZON_US: i64 = 300_000_000;

/// Sign label threshold: signal is θ × spread.
const THETA: f64 = 0.5;

/// Pending feature vector waiting for its labels.
struct Pending {
    fv: FeatureVector,
    mid_at_t: f64,
    spread_at_t: f64,
    // Filled in as future snapshots arrive
    r_1s: Option<f64>,
    r_5s: Option<f64>,
    r_30s: Option<f64>,
    r_300s: Option<f64>,
}

/// Delay queue that attaches forward-return labels to feature vectors.
pub struct LabelQueue {
    /// Pending feature vectors waiting for labels.
    queue: VecDeque<Pending>,
    /// Completed labeled vectors ready to emit.
    ready: VecDeque<LabeledFeatureVector>,
}

impl LabelQueue {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            ready: VecDeque::new(),
        }
    }

    /// Feed a new (feature_vector, current_mid, current_spread) observation.
    ///
    /// Call this every 100ms snapshot tick.
    pub fn push(&mut self, fv: FeatureVector, mid: f64, spread: f64) {
        let ts = fv.ts;

        // Update pending labels for earlier vectors
        for pending in &mut self.queue {
            let dt = ts - pending.fv.ts;
            let r = (mid - pending.mid_at_t) / pending.mid_at_t;

            if pending.r_1s.is_none() && dt >= HORIZONS_US[0] {
                pending.r_1s = Some(r);
            }
            if pending.r_5s.is_none() && dt >= HORIZONS_US[1] {
                pending.r_5s = Some(r);
            }
            if pending.r_30s.is_none() && dt >= HORIZONS_US[2] {
                pending.r_30s = Some(r);
            }
            if pending.r_300s.is_none() && dt >= HORIZONS_US[3] {
                pending.r_300s = Some(r);
            }
        }

        // Emit completed vectors (those older than max horizon)
        while let Some(front) = self.queue.front() {
            if ts - front.fv.ts >= MAX_HORIZON_US {
                let p = self.queue.pop_front().unwrap();
                self.ready.push_back(make_labeled(p));
            } else {
                break;
            }
        }

        // Enqueue new observation
        if mid > 0.0 {
            self.queue.push_back(Pending {
                fv,
                mid_at_t: mid,
                spread_at_t: spread,
                r_1s: None,
                r_5s: None,
                r_30s: None,
                r_300s: None,
            });
        }
    }

    /// Flush all remaining pending vectors (end of data stream).
    pub fn flush(&mut self) {
        while let Some(p) = self.queue.pop_front() {
            self.ready.push_back(make_labeled(p));
        }
    }

    /// Drain all ready labeled vectors.
    pub fn drain_ready(&mut self) -> Vec<LabeledFeatureVector> {
        self.ready.drain(..).collect()
    }

    pub fn pending_count(&self) -> usize {
        self.queue.len()
    }
}

impl Default for LabelQueue {
    fn default() -> Self {
        Self::new()
    }
}

fn make_labeled(p: Pending) -> LabeledFeatureVector {
    let threshold = THETA * p.spread_at_t / p.mid_at_t;

    let sign_1s = p.r_1s.map(|r| sign_label(r, threshold));
    let sign_5s = p.r_5s.map(|r| sign_label(r, threshold));

    LabeledFeatureVector {
        fv: p.fv,
        r_1s: p.r_1s,
        r_5s: p.r_5s,
        r_30s: p.r_30s,
        r_300s: p.r_300s,
        sign_1s,
        sign_5s,
    }
}

fn sign_label(r: f64, threshold: f64) -> i8 {
    if r > threshold { 1 }
    else if r < -threshold { -1 }
    else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_fv(ts: i64) -> FeatureVector {
        FeatureVector {
            ts,
            ofi_1: None, ofi_5: None, ofi_10: None,
            depth_imb: None, microprice_dev: None,
            queue_imb: None, spread: None,
            trade_intensity: None, price_impact: None,
            level_drain: None, weighted_mid_slope: None,
            exchange: "test".into(), symbol: "X".into(),
            data_source: "test".into(),
            is_imputed: false, gap_flag: false,
        }
    }

    #[test]
    fn label_after_max_horizon() {
        let mut q = LabelQueue::new();
        // Push at t=0 with mid=100
        q.push(dummy_fv(0), 100.0, 0.1);
        // Not yet ready
        assert!(q.drain_ready().is_empty());

        // Push at t+300s+1 with mid=102
        q.push(dummy_fv(300_001_000), 102.0, 0.1);
        let ready = q.drain_ready();
        assert_eq!(ready.len(), 1);
        // r_300s = (102 - 100) / 100 = 0.02
        let r = ready[0].r_300s.unwrap();
        assert!((r - 0.02).abs() < 1e-10);
    }

    #[test]
    fn flush_emits_remaining() {
        let mut q = LabelQueue::new();
        q.push(dummy_fv(0), 100.0, 0.1);
        q.push(dummy_fv(100_000), 100.1, 0.1);
        q.flush();
        let ready = q.drain_ready();
        assert_eq!(ready.len(), 2);
    }

    #[test]
    fn sign_label_positive() {
        // r = 0.01, threshold = 0.5 * 0.1 / 100 = 0.0005 → sign = +1
        assert_eq!(sign_label(0.01, 0.0005), 1);
    }

    #[test]
    fn sign_label_neutral() {
        // r = 0.0001, threshold = 0.001 → sign = 0
        assert_eq!(sign_label(0.0001, 0.001), 0);
    }
}
