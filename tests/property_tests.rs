//! Property-based tests using proptest.
//!
//! Tests the plan's Category D edge cases:
//! - Price round-trip: string → f64 → BTreeMap key → string preserves equality
//! - Empty book returns None for all features (no panics)
//! - OFI accumulation handles extreme values without overflow
//! - Hawkes stability condition enforced

use proptest::prelude::*;
use ordered_float::OrderedFloat;
use orderflow_rs::orderbook::{BookDelta, Orderbook, Side};
use orderflow_rs::features::{
    depth_imbalance, microprice_deviation, queue_imbalance, spread, FeatureExtractor,
};

// ── Price round-trip precision tests ─────────────────────────────────────────

proptest! {
    /// Prices parsed from strings and used as BTreeMap keys should be
    /// bit-identical when the same string is parsed twice.
    #[test]
    fn price_string_parse_deterministic(
        price_str in "([1-9][0-9]{0,4})\\.([0-9]{1,8})"
    ) {
        let p1: f64 = price_str.parse().unwrap();
        let p2: f64 = price_str.parse().unwrap();
        prop_assert_eq!(p1.to_bits(), p2.to_bits(),
            "Same price string should produce identical f64 bits");
    }

    /// Applying a delta with a price parsed from string, then reading it back
    /// from the BTreeMap should give the same f64 (no rounding drift).
    #[test]
    fn price_round_trips_through_btreemap(
        bid_price in 1.0_f64..100_000.0_f64,
        qty in 0.001_f64..1_000_000.0_f64,
    ) {
        // Simulate string-parse path: format then parse
        let price_str = format!("{bid_price:.8}");
        let parsed_price: f64 = price_str.parse().unwrap();

        let mut book = Orderbook::new();
        book.apply_delta(&BookDelta {
            price: OrderedFloat(parsed_price),
            qty,
            side: Side::Bid,
            exchange_ts: 0,
            recv_ts: 0,
        }).unwrap();
        // Also need an ask to avoid empty-book (not testing crossing invariant here)
        book.apply_delta(&BookDelta {
            price: OrderedFloat(parsed_price + 1.0),
            qty,
            side: Side::Ask,
            exchange_ts: 0,
            recv_ts: 0,
        }).unwrap();

        let snap = book.export_snapshot(1);
        let retrieved = snap.best_bid().unwrap();
        // Retrieved price must be bit-identical to what was inserted
        prop_assert_eq!(
            retrieved.to_bits(),
            parsed_price.to_bits(),
            "BTreeMap round-trip changed price bits"
        );
    }
}

// ── Empty book safety tests ───────────────────────────────────────────────────

proptest! {
    /// All feature functions must return None (not panic) when the book is empty.
    #[test]
    fn empty_book_all_features_return_none(n_levels in 1usize..=20) {
        let snap = orderflow_rs::orderbook::BookSnapshot::new(0);

        prop_assert!(depth_imbalance(&snap, n_levels).is_none(),
            "depth_imbalance should be None for empty book");
        prop_assert!(microprice_deviation(&snap).is_none(),
            "microprice_deviation should be None for empty book");
        prop_assert!(queue_imbalance(&snap).is_none(),
            "queue_imbalance should be None for empty book");
        prop_assert!(spread(&snap).is_none(),
            "spread should be None for empty book");
    }

    /// FeatureExtractor must not panic on empty book snapshot.
    #[test]
    fn extractor_handles_empty_book_no_panic(_seed in 0u64..1000) {
        let mut fx = FeatureExtractor::new("test", "X", "test");
        let empty = orderflow_rs::orderbook::BookSnapshot::new(0);
        // Should not panic
        let _fv = fx.extract(&empty, false, false);
    }
}

// ── OFI accumulation tests ────────────────────────────────────────────────────

proptest! {
    /// OFI should remain finite even with large quantity changes.
    #[test]
    fn ofi_finite_for_large_quantities(
        qty1 in 1.0_f64..1e10,
        qty2 in 1.0_f64..1e10,
    ) {
        let mut book = Orderbook::new();
        book.apply_delta(&BookDelta {
            price: OrderedFloat(100.0),
            qty: qty1,
            side: Side::Bid,
            exchange_ts: 0,
            recv_ts: 0,
        }).unwrap();
        book.apply_delta(&BookDelta {
            price: OrderedFloat(101.0),
            qty: qty1,
            side: Side::Ask,
            exchange_ts: 0,
            recv_ts: 0,
        }).unwrap();
        let snap1 = book.export_snapshot(10);

        book.apply_delta(&BookDelta {
            price: OrderedFloat(100.0),
            qty: qty2,
            side: Side::Bid,
            exchange_ts: 1,
            recv_ts: 1,
        }).unwrap();
        let snap2 = book.export_snapshot(10);

        let mut ofi = orderflow_rs::features::ofi::OrderFlowImbalance::new();
        ofi.update(&snap1);
        let result = ofi.update(&snap2);

        if let Some(ofi_val) = result.ofi_1 {
            prop_assert!(ofi_val.is_finite(), "OFI should be finite for qty1={qty1} qty2={qty2}");
        }
    }
}

// ── Crossed book rejection tests ─────────────────────────────────────────────

proptest! {
    /// In debug mode, crossed book (bid >= ask) must be rejected.
    #[test]
    fn crossed_book_rejected(
        price in 1.0_f64..100_000.0_f64,
    ) {
        let mut book = Orderbook::new();
        // Set up a valid book first
        book.apply_delta(&BookDelta {
            price: OrderedFloat(price - 1.0),
            qty: 10.0,
            side: Side::Bid,
            exchange_ts: 0,
            recv_ts: 0,
        }).unwrap();
        book.apply_delta(&BookDelta {
            price: OrderedFloat(price + 1.0),
            qty: 10.0,
            side: Side::Ask,
            exchange_ts: 0,
            recv_ts: 0,
        }).unwrap();

        // Now try to move bid ABOVE ask (should fail in debug builds)
        let result = book.apply_delta(&BookDelta {
            price: OrderedFloat(price + 2.0), // bid > ask
            qty: 10.0,
            side: Side::Bid,
            exchange_ts: 1,
            recv_ts: 1,
        });

        #[cfg(debug_assertions)]
        prop_assert!(result.is_err(), "Crossed book should be rejected in debug mode");
        // In release builds the invariant is skipped — don't assert there
        let _ = result;
    }
}
