#!/bin/sh

. ./ci/preamble.sh

clean() {
    find target -type f -name '*.profraw' -delete || true
    find target -type f -name '*.gcda' -delete || true
}

test_all() {
    cargo test --workspace --quiet --no-run
    cargo test --workspace --no-fail-fast -- --nocapture
}

test_coverage_preamble() {
    export CARGO_INCREMENTAL=0
    export RUSTFLAGS='-Zprofile -Ccodegen-units=1 -Cllvm-args=--inline-threshold=0 -Clink-dead-code -Coverflow-checks=off -Cpanic=abort -Zpanic_abort_tests'
    export LLVM_PROFILE_FILE="stuckliste-%p-%m.profraw"
}

test_coverage_postamble() {
    grcov \
        . \
        --binary-path target/debug/ \
        -s . \
        -t lcov \
        --branch --ignore-not-existing --ignore "*index.crates.io*" \
        --excl-start '.*cfg\(.*test.*' \
        --excl-br-start '.*cfg\(.*test.*' \
        --excl-br-line '.*cfg.*test.*' \
        -o target/debug/lcov.info
    lcov --summary target/debug/lcov.info
    genhtml -o target/debug/coverage/ \
        --branch-coverage \
        --highlight \
        --ignore-errors source \
        --legend \
        target/debug/lcov.info
}

do_test_miri() {
    env MIRIFLAGS=-Zmiri-disable-isolation cargo +nightly miri test "$@"
}

clean
#test_coverage_preamble
test_all
#test_coverage_postamble
