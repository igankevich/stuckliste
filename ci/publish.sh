#!/bin/sh

. ./ci/preamble.sh

cargo_publish() {
    for package in stuckliste stuckliste-cli; do
        cargo publish --quiet --package "$package"
    done
}

# TODO
#if test "$GITHUB_ACTIONS" = "true" && test "$GITHUB_REF_TYPE" != "tag"; then
#    exit 0
#fi
cargo_publish
