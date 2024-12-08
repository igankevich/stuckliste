#!/bin/sh

. ./ci/preamble.sh

# TODO
#if test "$GITHUB_ACTIONS" = "true" && test "$GITHUB_REF_TYPE" != "tag"; then
#    exit 0
#fi

set -e
root="$(pwd)"
target=x86_64-unknown-linux-musl
cargo build \
    --quiet \
    --release \
    --target "$target" \
    --package stuckliste-cli
version="$GITHUB_REF_NAME"
rm -rf --one-file-system release
release_dir=release/"$version"
mkdir -p "$release_dir"/"$target"
for filename in lsbom mkbom; do
    cp -vn target/"$target"/release/"$filename" "$release_dir"/"$target"/
done
cd "$release_dir"
find . -type f -print0 >"$workdir"/files
tar -czvf "$root"/stuckliste-"$version".tar.gz --null --files-from="$workdir"/files
