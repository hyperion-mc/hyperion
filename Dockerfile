# Define base arguments for versioning and optimization
ARG RUST_NIGHTLY_VERSION=nightly-2024-12-18
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

FROM builder-base AS antithesis

# todo: assert target is amd64
# https://antithesis.com/docs/using_antithesis/sdk/rust/instrumentation/

COPY ./libvoidstar.so /usr/lib/libvoidstar.so

# Assumes libvoidstar.so is in /usr/lib
ENV LIBVOIDSTAR_PATH=/usr/lib
ENV LD_LIBRARY_PATH=/usr/lib

ENV RUSTFLAGS="-Ccodegen-units=1 \
    -Cpasses=sancov-module \
    -Cllvm-args=-sanitizer-coverage-level=3 \
    -Cllvm-args=-sanitizer-coverage-trace-pc-guard \
    -Clink-args=-Wl,--build-id \
    -Clink-args=-Wl,-z,nostart-stop-gc \
    -L/usr/lib \
    -lvoidstar"

ENV LIBVOIDSTAR_PATH=/usr/lib
ENV LD_LIBRARY_PATH=/usr/lib

RUN --mount=type=cache,target=${CARGO_HOME}/registry \
    --mount=type=cache,target=${CARGO_HOME}/git \
    --mount=type=cache,target=/antithesis-target \
    cargo build --frozen --target-dir /antithesis-target && \
    cp /antithesis-target/debug/hyperion-proxy /app/hyperion-proxy && \
    cp /antithesis-target/debug/tag /app/tag && \
    cp /antithesis-target/debug/antithesis-bot /app/antithesis-bot

# Verify instrumentation was successful
RUN --mount=type=cache,target=/antithesis-target \
    nm /antithesis-target/debug/hyperion-proxy | grep "sanitizer_cov_trace_pc_guard" && \
    ldd /antithesis-target/debug/hyperion-proxy | grep "libvoidstar" && \
    nm /antithesis-target/debug/tag | grep "sanitizer_cov_trace_pc_guard" && \
    ldd /antithesis-target/debug/tag | grep "libvoidstar"

# Release builder
FROM builder-base AS build-release

RUN --mount=type=cache,target=${CARGO_HOME}/registry \
    --mount=type=cache,target=${CARGO_HOME}/git \
    --mount=type=cache,target=/app/target \
    cargo build --profile release-full --frozen --workspace --exclude antithesis-bot && \
    mkdir -p /app/build && \
    cp target/release-full/hyperion-proxy /app/build/ && \
    cp target/release-full/tag /app/build/

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
EXPOSE 8080
ENTRYPOINT ["/hyperion-proxy"]
CMD ["0.0.0.0:8080"]
# NYC Release
FROM runtime-base AS tag
COPY --from=build-release /app/build/tag /
LABEL org.opencontainers.image.source="https://github.com/andrewgazelka/hyperion" \
      org.opencontainers.image.description="Hyperion Tag Event" \
      org.opencontainers.image.version="0.1.0"
ENTRYPOINT ["/tag"]
CMD ["--ip", "0.0.0.0", "--port", "35565"]

FROM runtime-base AS antithesis-runtime-base

COPY --from=antithesis /usr/lib/libvoidstar.so /usr/lib/libvoidstar.so
ENV LD_LIBRARY_PATH=/usr/lib

FROM antithesis-runtime-base AS antithesis-hyperion-proxy

COPY --from=antithesis /app/hyperion-proxy /
LABEL org.opencontainers.image.source="https://github.com/andrewgazelka/hyperion" \
      org.opencontainers.image.description="Hyperion Proxy Server" \
      org.opencontainers.image.version="0.1.0"
EXPOSE 8080
ENTRYPOINT ["/hyperion-proxy"]
CMD ["0.0.0.0:8080"]

FROM antithesis-runtime-base AS antithesis-tag

COPY --from=antithesis /app/tag /
LABEL org.opencontainers.image.source="https://github.com/andrewgazelka/hyperion" \
      org.opencontainers.image.description="Hyperion Tag Event" \
      org.opencontainers.image.version="0.1.0"

ENTRYPOINT ["/tag"]
CMD ["--ip", "0.0.0.0", "--port", "35565"]

FROM antithesis-runtime-base AS antithesis-bot

COPY --from=antithesis /app/antithesis-bot /
LABEL org.opencontainers.image.source="https://github.com/andrewgazelka/hyperion" \
      org.opencontainers.image.description="Hyperion Antithesis Bot" \
      org.opencontainers.image.version="0.1.0"

ENTRYPOINT ["/antithesis-bot"]