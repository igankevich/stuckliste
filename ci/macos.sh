#!/bin/sh

. ./ci/preamble.sh

rustup toolchain add "$RUST_VERSION" \
    --component clippy \
    --component rustfmt
rustup default "$RUST_VERSION"-stable
