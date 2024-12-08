#!/bin/sh

. ./ci/preamble.sh

pre-commit run --all-files --show-diff-on-failure
cargo deny check
