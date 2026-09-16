#!/bin/sh

. ./ci/preamble.sh

main() {
    root="$(pwd)"
    target=x86_64-unknown-linux-musl
    rustup target add "$RUST_VERSION"-"$target"
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
    create_tar_archive
}

create_tar_archive() {
    find . -type f -print0 | sort --unique --zero-terminated >"$workdir"/files
    tar --create \
        --mtime=@0 \
        --numeric-owner \
        --owner=0 \
        --group=0 \
        --gzip \
        --verbose \
        --file="$root"/stuckliste-"$version".tar.gz \
        --null \
        --files-from="$workdir"/files
}

main
