# Define base arguments for versioning and optimization
ARG RUST_NIGHTLY_VERSION=nightly-2025-02-22
ARG RUSTFLAGS="-Z share-generics=y -Z threads=8"
ARG CARGO_HOME=/usr/local/cargo
# Install essential build packages
FROM ubuntu:24.04 AS packages
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && \
    apt-get install -y \
    binutils \
    build-essential \
    cmake \
    curl \
    gcc \
    libclang-dev \
    libclang1 \
    libssl-dev \
    linux-headers-generic \
    llvm-dev \
    perl \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Base builder stage with Rust installation
FROM packages AS builder-base
ARG RUST_NIGHTLY_VERSION
ARG RUSTFLAGS
ARG CARGO_HOME
ENV RUSTFLAGS=${RUSTFLAGS}
ENV CARGO_HOME=${CARGO_HOME}
RUN curl https://sh.rustup.rs -sSf | sh -s -- -y --default-toolchain ${RUST_NIGHTLY_VERSION} && \
    $CARGO_HOME/bin/rustup component add rust-src && \
    $CARGO_HOME/bin/rustc --version
ENV PATH="${CARGO_HOME}/bin:${PATH}"
WORKDIR /app

RUN curl -L --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash && \
    cargo binstall -y cargo-machete cargo-nextest && \
    rm -rf /root/.cargo/registry /root/.cargo/git

COPY . .

RUN --mount=type=cache,target=${CARGO_HOME}/registry \
    --mount=type=cache,target=${CARGO_HOME}/git \
    --mount=type=cache,target=/app/target \
    cargo fetch

# CI stage for checks

FROM builder-base AS machete

RUN cargo machete && touch machete-done

FROM builder-base AS builder-ci

RUN --mount=type=cache,target=${CARGO_HOME}/registry \
    --mount=type=cache,target=${CARGO_HOME}/git \
    --mount=type=cache,target=/app/target \
    cargo clippy --workspace --benches --tests --examples --all-features --frozen -- -D warnings && \
    #    cargo doc --all-features --workspace --frozen --no-deps && \
    cargo nextest run --all-features --frozen && \
    touch ci-done


FROM builder-base AS fmt

RUN cargo fmt --all -- --check && touch fmt-done

FROM builder-base AS ci

COPY --from=machete /app/machete-done /app/machete-done
COPY --from=fmt /app/fmt-done /app/fmt-done
COPY --from=builder-ci /app/ci-done /app/ci-done

# Release builder
FROM builder-base AS build-release

RUN --mount=type=cache,target=${CARGO_HOME}/registry \
    --mount=type=cache,target=${CARGO_HOME}/git \
    --mount=type=cache,target=/app/target \
    cargo build --profile release-full --frozen --workspace && \
    mkdir -p /app/build && \
    cp target/release-full/hyperion-proxy /app/build/ && \
    cp target/release-full/bedwars /app/build/ && \
    cp target/release-full/rust-mc-bot /app/build/

# Runtime base image
FROM ubuntu:24.04 AS runtime-base
RUN apt-get update && \
    apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*
ENV RUST_BACKTRACE=1 \
    RUST_LOG=info

# Hyperion Proxy Release
FROM runtime-base AS hyperion-proxy
COPY --from=build-release /app/build/hyperion-proxy /
LABEL org.opencontainers.image.source="https://github.com/andrewgazelka/hyperion" \
    org.opencontainers.image.description="Hyperion Proxy Server" \
    org.opencontainers.image.version="0.1.0"
EXPOSE 25565
ENV HYPERION_PROXY_PROXY_ADDR="0.0.0.0:25565" \
    HYPERION_PROXY_SERVER="127.0.0.1:35565"
ENTRYPOINT ["/hyperion-proxy"]

FROM runtime-base AS bedwars
COPY --from=build-release /app/build/bedwars /
LABEL org.opencontainers.image.source="https://github.com/andrewgazelka/hyperion" \
    org.opencontainers.image.description="Hyperion Bedwars Event" \
    org.opencontainers.image.version="0.1.0"
ENV BEDWARS_IP="0.0.0.0" \
    BEDWARS_PORT="35565"
EXPOSE 35565
ENTRYPOINT ["/bedwars"]

FROM runtime-base AS rust-mc-bot
COPY --from=build-release /app/build/rust-mc-bot /
LABEL org.opencontainers.image.source="https://github.com/andrewgazelka/rust-mc-bot" \
    org.opencontainers.image.description="Rust Minecraft Bot" \
    org.opencontainers.image.version="0.1.0"
ENV BOT_SERVER="hyperion-proxy:25565" \
    BOT_BOT_COUNT="500" \
    BOT_THREADS="4"
EXPOSE 25565
ENTRYPOINT ["/rust-mc-bot"]

