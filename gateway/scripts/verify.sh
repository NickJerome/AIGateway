#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
cargo fmt --all -- --check
timeout 1200 cargo clippy --locked --all-targets -- -D warnings
timeout 1200 cargo test --locked --no-fail-fast
timeout 1200 cargo build --release --locked
