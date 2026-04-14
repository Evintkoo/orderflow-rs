# orderflow-rs

**A high-performance Rust pipeline for limit order book (LOB) microstructure research.**

Ingests raw tick data, computes order-flow imbalance (OFI) features and 87 technical indicators, runs synthetic LOB simulations, backtests signal-based strategies, and produces ranked IC reports — all from a single binary.

> Companion research: *"Predictability of High-Frequency Limit Order Book Dynamics"*  
> Full thesis: [`docs/paper/orderflow_research.pdf`](docs/paper/orderflow_research.pdf)

---

## Research Summary

| Phase | Question | Verdict |
|-------|----------|---------|
| P4 — Simulation | Does OFI_1 predict synthetic price moves? | **PASS** (IC 0.15–0.33) |
| P5 — Real data | Does OFI_1 predict real FX returns? | **PARTIAL** (5/10 pairs; IC 0.04–0.10) |
| P6 — Backtest | Can OFI signals generate net positive PnL? | **FAIL** (negative after 0.04 % fee + half-spread) |
| P7 — Tech indicators | Which classical indicators best predict 1–300 s returns? | `pivot_dist` IC = 0.40@1 s, power-law decay τ^−0.43 |

---

## Features

- **Ingest** — streams Dukascopy LOB tick data, reconstructs bid/ask ladder, labels forward returns (1 s / 5 s / 30 s / 300 s)
- **OFI** — computes multi-level order-flow imbalance features (OFI_1 … OFI_5, cumulative)
- **87 technical indicators** — momentum, mean-reversion, trend, volatility, microstructure, and market-impact measures
- **IC engine** — Spearman rank IC for every indicator × horizon combination; cross-pair averaging
- **Simulation** — synthetic LOB with configurable spread / queue dynamics for hypothesis testing
- **Backtest** — walk-forward OOS evaluation with transaction-cost accounting
- **Feature flags** — `ingest`, `io` (Parquet), `sim` are optional; core analysis compiles with zero async deps

---

## Quick Start

### Prerequisites

- Rust 1.75+ (`rustup update stable`)
- Feature data CSVs in `data/` (see [Data](#data))

### Build

```bash
cargo build --release
```

### Run pipeline phases

```bash
# Ingest raw tick data → feature CSVs
cargo run --release -- ingest data/

# Compute OFI features
cargo run --release -- features data/

# Run synthetic LOB simulation (P4)
cargo run --release -- simulate

# Backtest OFI signals (P6)
cargo run --release -- backtest data/ reports/

# Technical indicator IC analysis (P7)
cargo run --release -- techanalysis data/ reports/
```

Reports are written to `reports/` as CSV files (gitignored — run locally to reproduce).

---

## Project Structure

```
orderflow-rs/
├── src/
│   ├── main.rs                  # CLI entry point
│   ├── commands/                # Subcommand handlers
│   │   ├── techanalysis.rs
│   │   ├── backtest.rs
│   │   └── ...
│   ├── analysis/
│   │   ├── techind.rs           # 87 indicator implementations
│   │   ├── techreport.rs        # IC computation & CSV output
│   │   ├── stats.rs             # Spearman IC, variance ratio, Hurst
│   │   ├── report.rs            # CSV loader / feature row schema
│   │   └── ...
│   ├── features/                # OFI feature extraction
│   ├── orderbook/               # LOB reconstruction
│   ├── simulator/               # Synthetic LOB
│   └── pipeline/                # Ingest orchestration
├── tests/                       # Integration & property tests
├── benches/                     # Criterion benchmarks
├── docs/
│   ├── paper/                   # LaTeX source + compiled PDF
│   └── reports/                 # Markdown phase reports
├── Cargo.toml
└── Cargo.lock
```

---

## Data

Raw tick data is **not** included in this repository (files are too large and subject to Dukascopy terms of use).

To reproduce the analysis:

1. Download Dukascopy LOB tick data for the target FX pairs (EURUSD, GBPUSD, USDJPY, etc.)
2. Place raw files under `data/<source>/<PAIR>/`
3. Run `cargo run --release -- ingest data/` to produce `*_features.csv` files

Pre-processed feature CSVs (187 files, ~2 GB) are available on request.

---

## Reproducing Results

```bash
# P5 — OFI IC analysis
cargo run --release -- analyze data/ reports/

# P6 — Walk-forward backtest
cargo run --release -- backtest data/ reports/

# P7 — Technical indicator IC ranking
cargo run --release -- techanalysis data/ reports/
```

All results are deterministic given the same input CSVs. See [`docs/paper/orderflow_research.pdf`](docs/paper/orderflow_research.pdf) Appendix B for full reproduction instructions.

---

## Testing

```bash
cargo test                        # unit + integration tests
cargo test --features sim         # includes simulation tests
cargo bench                       # Criterion benchmarks
```

---

## License

MIT — see [`LICENSE`](LICENSE).

---

## Citation

```bibtex
@thesis{tkoovonzko2025orderflow,
  author  = {Tkoovonzko, Evin},
  title   = {Predictability of High-Frequency Limit Order Book Dynamics},
  school  = {},
  year    = {2025},
  url     = {https://github.com/Evintkoo/orderflow-rs}
}
```
