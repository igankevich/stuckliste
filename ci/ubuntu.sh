#!/bin/sh

. ./ci/preamble.sh

main() {
    export DEBIAN_FRONTEND=noninteractive
    root="$PWD"
    install_system_packages
    install_bomutils
    install_rust
}

install_system_packages() {
    sudo -n apt-get update -qq
    sudo -n apt-get install -qqy --no-install-recommends clang libclang-dev g++ make jq
}

install_bomutils() {
    git clone https://github.com/hogliux/bomutils "$workdir"/bomutils
    cd "$workdir"/bomutils
    git checkout 14f5d09d6c62fef7539ebdd23ebcd42ab54f7351
    make
    sudo -n make install
    cd "$root"
}

install_rust() {
    rustup toolchain add "$RUST_VERSION" \
        --component clippy \
        --component rustfmt
    rustup default "$RUST_VERSION"-stable
}

cleanup() {
    rm -rf "$workdir"
}

main
