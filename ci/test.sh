#!/bin/sh

. ./ci/preamble.sh

cargo clippy --workspace --quiet --all-targets -- -D warnings
cargo test --workspace --quiet --no-fail-fast -- --nocapture
