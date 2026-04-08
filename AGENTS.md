# AGENTS.md

This file provides guidance to WARP (warp.dev) when working with code in this repository.

## Project snapshot
- This is a single-crate Rust binary project (`third-eye-client`) defined in `Cargo.toml`.
- The only source file is `src/main.rs`, which currently contains a minimal `main()` entrypoint that prints `Hello, world!`.
- There are no additional modules, libraries, tests, or workspace members yet.

## Development commands
Run these from the repository root.

- Build:
  - `cargo build`
- Run:
  - `cargo run`
- Run tests:
  - `cargo test`
- Run a single test (by test name substring):
  - `cargo test <test_name>`
- Lint (Clippy):
  - `cargo clippy --all-targets --all-features -- -D warnings`
- Format check:
  - `cargo fmt --check`
- Apply formatting:
  - `cargo fmt`

## Architecture notes
- Current architecture is intentionally simple: one executable target with all behavior in `src/main.rs`.
- As functionality grows, expect architecture to evolve by introducing internal modules under `src/` (and potentially `src/lib.rs` if shared logic is needed between binary and tests).
