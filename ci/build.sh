#!/bin/sh

. ./ci/preamble.sh

set -e
target=x86_64-unknown-linux-musl
cargo build \
    --quiet \
    --release \
    --target "$target" \
    --package stuckliste-cli
version="$(git describe --tags --always)"
rm -rf --one-file-system release
release_dir=release/"$version"
mkdir -p "$release_dir"/"$target"
for filename in lsbom mkbom; do
    cp -vn target/"$target"/release/"$filename" "$release_dir"/"$target"/
done
cd release
find . -type f -print0 >"$workdir"/files
tar -czvf ../stuckliste-"$version".tar.gz \
    --null --files-from="$workdir"/files