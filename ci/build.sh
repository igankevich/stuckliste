#!/bin/sh

. ./ci/preamble.sh

main() {
    root="$(pwd)"
    install_rust
    cargo build \
        --quiet \
        --release \
        --target "$target" \
        --package stuckliste-cli
    archive_dir="$workdir"/archive
    mkdir -p "$archive_dir"
    for filename in lsbom mkbom; do
        cp -v target/"$target"/release/"$filename" "$archive_dir"/
    done
    cd "$archive_dir"
    create_tar_archive
}

install_rust() {
    case "$OS-$ARCH" in
    Linux-x86_64) target=x86_64-unknown-linux-musl ;;
    Darwin-arm64) target=aarch64-apple-darwin ;;
    *)
        printf "Unsupported OS/architecture combination: %s-%s\n" "$OS" "$ARCH" >&2
        exit 1
        ;;
    esac
    rustup toolchain add "$RUST_VERSION" --target "$target"
    rustup default "$RUST_VERSION"
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
        --file="$root"/stuckliste-"$OS"-"$ARCH"-"$VERSION".tar.gz \
        --null \
        --files-from="$workdir"/files
}

main
