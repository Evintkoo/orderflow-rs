# Contributing

Contributions, bug reports, and discussion are welcome.

## Getting Started

1. Fork the repository and clone your fork
2. Create a feature branch: `git checkout -b feat/your-feature`
3. Make your changes with tests
4. Run the full test suite: `cargo test`
5. Open a pull request against `main`

## Guidelines

- **Rust edition**: 2021; target stable Rust (no nightly features)
- **Formatting**: `cargo fmt` before committing
- **Lints**: `cargo clippy -- -D warnings` must pass
- **Tests**: new functionality should include unit tests in `tests/` or inline `#[cfg(test)]` modules
- **Commit messages**: use [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`)
- **Feature flags**: keep `ingest`/`io`/`sim` gates; core analysis must build with `--no-default-features`

## Reporting Issues

Please include:
- Rust toolchain version (`rustc --version`)
- Operating system
- Minimal reproduction steps
- Observed vs. expected behaviour

## Research Contributions

If you extend the analysis (new indicators, additional data sources, alternative signal construction), please document methodology changes in the relevant section of `docs/paper/` and update `docs/reports/`.
