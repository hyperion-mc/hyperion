#!/bin/bash
set -e

cargo clippy -q --all-targets --tests --benches -- -D warnings
cargo +nightly fmt

# Run tests but exclude benchmarks due to tango-bench framework bug (conflicting -g flags)
cargo nextest run --lib --tests

