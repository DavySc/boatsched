# boatsched — Rust server, containerized.
#
# Multi-stage build. The OR-Tools C++ SDK is large (~268 MB unpacked) and
# gitignored, so it is fetched fresh in the builder stage and *baked into the
# runtime image* — making the binary's native deps (libortools + bundled
# abseil/protobuf/CBC/SCIP/HiGHS) self-contained and independent of the host OS.
#
# Builder and runtime share the SAME base (ubuntu:24.04) so the glibc/libstdc++
# the binary links against at build time is exactly what runs it. The SDK build
# we download MUST match that base — hence the ubuntu-24.04 OR-Tools artifact.
#
# Build context is the REPO ROOT (the server crate embeds the SPA built from web/,
# which lives outside rust/). Build from the repo root:
#   docker build -t boatsched:latest -f rust/Dockerfile .
#
# Pin is intentional: OR-Tools 9.15.6755 matches crates/solver (protobuf ABI).

# ---------------------------------------------------------------------------
# Stage 0 — web: the React SPA, bundled into one self-contained index.html
# ---------------------------------------------------------------------------
FROM node:22-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

# ---------------------------------------------------------------------------
# Stage 1 — builder
# ---------------------------------------------------------------------------
FROM ubuntu:24.04 AS builder

ENV DEBIAN_FRONTEND=noninteractive
# rustup/cargo live here; keep them off $HOME so the layer is cache-friendly.
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

# Toolchain for the cc crate (C++ shim) + curl/CA for fetching the SDK + rustup.
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential \
        g++ \
        pkg-config \
        curl \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# OR-Tools C++ SDK — version-pinned, base-matched (ubuntu-24.04). build.rs reads
# ORTOOLS_ROOT and bakes an rpath to $ORTOOLS_ROOT/lib, so the runtime image
# must keep the libs at this exact path (/opt/or-tools/lib). Do not relocate.
ARG ORTOOLS_VERSION=9.15.6755
ARG ORTOOLS_ASSET=or-tools_amd64_ubuntu-24.04_cpp_v9.15.6755.tar.gz
RUN curl -fsSL -o /tmp/ortools.tar.gz \
        "https://github.com/google/or-tools/releases/download/v9.15/${ORTOOLS_ASSET}" \
    && mkdir -p /opt/or-tools \
    && tar xzf /tmp/ortools.tar.gz -C /opt/or-tools --strip-components=1 \
    && rm /tmp/ortools.tar.gz \
    && test -f /opt/or-tools/include/ortools/sat/cp_model.h
ENV ORTOOLS_ROOT=/opt/or-tools

# Rust toolchain — channel/version come from rust-toolchain.toml (copied first so
# rustup installs the pinned toolchain before the heavier dependency build).
COPY rust/rust-toolchain.toml ./rust-toolchain.toml
RUN curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs \
        | sh -s -- -y --no-modify-path --default-toolchain none \
    && rustup show

# Build context is the REPO ROOT (not rust/): crates/server/build.rs embeds
# ../../../web/dist/index.html, which lives OUTSIDE rust/. We mirror the repo
# layout in the image — workspace at /build/rust, bundle at /build/web/dist.
# (.dockerignore keeps target/ and vendor/ out so the SDK we just fetched — not
# the gitignored Arch one — is the only OR-Tools SDK.)
WORKDIR /build
COPY rust/ ./rust/
COPY --from=web /web/dist/index.html ./web/dist/index.html

# Compile + run the test suite as the build-time gate, then build the release
# binary. A failing test fails the image build (and therefore the deploy).
WORKDIR /build/rust
RUN cargo test --release --workspace --locked \
    && cargo build --release -p boatsched-server --locked \
    && strip target/release/boatsched-server

# ---------------------------------------------------------------------------
# Stage 2 — runtime
# ---------------------------------------------------------------------------
FROM ubuntu:24.04 AS runtime

ENV DEBIAN_FRONTEND=noninteractive
# libstdc++/libgcc for the C++ shim + OR-Tools; ca-certificates for TLS; tini as
# PID 1 so signals (SIGTERM from `docker compose down`) reach the server cleanly.
RUN apt-get update && apt-get install -y --no-install-recommends \
        libstdc++6 \
        libgcc-s1 \
        ca-certificates \
        curl \
        tini \
    && rm -rf /var/lib/apt/lists/*

# OR-Tools shared libs at the SAME path the build-time rpath points to.
COPY --from=builder /opt/or-tools/lib /opt/or-tools/lib
COPY --from=builder /build/rust/target/release/boatsched-server /usr/local/bin/boatsched-server

# Non-root runtime user owns the data dir (bind-mounted at run time).
RUN useradd --system --uid 10001 --home /app boatsched \
    && mkdir -p /data /app \
    && chown -R boatsched:boatsched /data /app

# Entrypoint wrapper: forwards SOLVER_WORKERS (from the env file) into the
# solver's --workers flag, defaulting to 2 to match the box's vCPU budget. The
# hardening flags (require_auth, secure cookies) auto-enable because --db is set.
COPY rust/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

USER boatsched
WORKDIR /app
EXPOSE 8000
ENV BOATSCHED_DB=/data/boatsched.db

# tini reaps zombies and forwards signals; the wrapper execs the server.
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/docker-entrypoint.sh"]
