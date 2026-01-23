FROM rust:1.92.0

RUN export DEBIAN_FRONTEND=noninteractive \
    && apt-get -qq update \
    && apt-get -qq install --no-install-recommends \
    pre-commit \
    python3-pip \
    libclang-dev \
    shellcheck \
    clang \
    curl \
    lcov \
    && rm -rf /var/lib/apt/lists/*

ENV CARGO_HOME=/usr/local/cargo \
    RUSTUP_HOME=/usr/local/rustup \
    RUST_VERSION=1.92.0 \
    PATH=/usr/local/cargo/bin:$PATH

RUN rustup toolchain add nightly \
    --target x86_64-unknown-linux-gnu \
    --component rustfmt --component miri --component llvm-tools-preview
RUN rustup toolchain add 1.92.0 \
    --target x86_64-unknown-linux-gnu \
    --component clippy --component rustfmt --component llvm-tools-preview
RUN rustup target add --toolchain 1.92.0 \
    x86_64-unknown-linux-musl
RUN rustup default 1.92.0-x86_64-unknown-linux-gnu
RUN cargo install --quiet cargo-deny

RUN curl --location --silent --fail --output /usr/local/bin/shfmt https://github.com/mvdan/sh/releases/download/v3.8.0/shfmt_v3.8.0_linux_amd64 && \
    chmod +x /usr/local/bin/shfmt

# install bomutils
RUN export DEBIAN_FRONTEND=noninteractive \
    && apt-get -qq update \
    && apt-get -qq install --no-install-recommends \
    g++ make \
    && git clone https://github.com/hogliux/bomutils /tmp/bomutils \
    && cd /tmp/bomutils \
    && git checkout 14f5d09d6c62fef7539ebdd23ebcd42ab54f7351 \
    && make \
    && make install \
    && cd / \
    && rm -rf /tmp/bomutils \
    && rm -rf /var/lib/apt/lists/*

RUN export DEBIAN_FRONTEND=noninteractive \
    && apt-get -qq update \
    && apt-get -qq install --no-install-recommends \
    jq \
    && rm -rf /var/lib/apt/lists/*

LABEL org.opencontainers.image.source=https://github.com/igankevich/stuckliste
LABEL org.opencontainers.image.description="CI image"

CMD ["/bin/bash"]
