use criterion::{black_box, criterion_group, criterion_main, Criterion};
use ordered_float::OrderedFloat;
use orderflow_rs::orderbook::{BookDelta, Orderbook, Side};

fn make_populated_book() -> Orderbook {
    let mut book = Orderbook::new();
    // Seed 20 bid levels and 20 ask levels
    for i in 0..20 {
        book.apply_delta(&BookDelta {
            price: OrderedFloat(100.0 - i as f64 * 0.1),
            qty: 10.0 + i as f64,
            side: Side::Bid,
            exchange_ts: 0,
            recv_ts: 0,
        })
        .unwrap();
        book.apply_delta(&BookDelta {
            price: OrderedFloat(101.0 + i as f64 * 0.1),
            qty: 10.0 + i as f64,
            side: Side::Ask,
            exchange_ts: 0,
            recv_ts: 0,
        })
        .unwrap();
    }
    book
}

/// Benchmark: update an existing price level (most common case in live data).
fn bench_delta_update_existing(c: &mut Criterion) {
    let mut book = make_populated_book();
    let delta = BookDelta {
        price: OrderedFloat(100.0), // Existing best bid
        qty: 15.0,
        side: Side::Bid,
        exchange_ts: 1,
        recv_ts: 1,
    };
    c.bench_function("delta_update_existing_level", |b| {
        b.iter(|| {
            book.apply_delta(black_box(&delta)).unwrap();
        })
    });
}

/// Benchmark: insert a new price level (less common, tests BTreeMap allocation).
fn bench_delta_new_level(c: &mut Criterion) {
    let mut book = make_populated_book();
    let mut price = 99.0_f64;
    c.bench_function("delta_insert_new_level", |b| {
        b.iter(|| {
            price -= 0.01;
            let delta = BookDelta {
                price: OrderedFloat(price),
                qty: 5.0,
                side: Side::Bid,
                exchange_ts: 1,
                recv_ts: 1,
            };
            book.apply_delta(black_box(&delta)).unwrap();
        })
    });
}

/// Benchmark: export 10-level snapshot (feature extraction tick).
fn bench_snapshot_export_10levels(c: &mut Criterion) {
    let book = make_populated_book();
    c.bench_function("snapshot_export_10_levels", |b| {
        b.iter(|| {
            black_box(book.export_snapshot(10));
        })
    });
}

/// Benchmark: full cycle — apply delta + export snapshot (end-to-end feature tick).
fn bench_delta_plus_snapshot(c: &mut Criterion) {
    let mut book = make_populated_book();
    let delta = BookDelta {
        price: OrderedFloat(100.0),
        qty: 12.0,
        side: Side::Bid,
        exchange_ts: 2,
        recv_ts: 2,
    };
    c.bench_function("delta_apply_plus_snapshot_10", |b| {
        b.iter(|| {
            book.apply_delta(black_box(&delta)).unwrap();
            black_box(book.export_snapshot(10));
        })
    });
}

criterion_group!(
    benches,
    bench_delta_update_existing,
    bench_delta_new_level,
    bench_snapshot_export_10levels,
    bench_delta_plus_snapshot,
);
criterion_main!(benches);
