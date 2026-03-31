# ─── Stage 1: os-base ────────────────────────────────────────────────────────
# System dependencies only. No source code, no Rust toolchain.
# All other stages derive from this to share the apt layer.
FROM ubuntu:24.04 AS os-base

RUN apt-get update && apt-get install -y \
    curl \
    git \
    sudo \
    ca-certificates \
    bash \
    jq \
    && rm -rf /var/lib/apt/lists/*

ENV PATH="/root/.local/bin:${PATH}"
WORKDIR /tmp/loadout-repo

# ─── Stage 2: os ─────────────────────────────────────────────────────────────
# os-base + source code.
# No loadout installed — use this as the starting point for manual install testing.
#
# Use case: walk through install.sh or copy a binary by hand.
FROM os-base AS os

COPY . .

CMD ["/bin/bash"]

# ─── Stage 3: rust-toolchain ─────────────────────────────────────────────────
# os-base + Rust toolchain. No source code yet.
# Placing rustup installation BEFORE COPY . . means this layer is cached
# across source changes — only invalidated when the Dockerfile itself changes.
FROM os-base AS rust-toolchain

RUN apt-get update && apt-get install -y build-essential && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
    sh -s -- -y --default-toolchain stable --profile minimal --no-modify-path

ENV PATH="/root/.cargo/bin:${PATH}"

# ─── Stage 4: builder ────────────────────────────────────────────────────────
# rust-toolchain + source code + compiled binaries.
# Not used directly — dev is built on top of this.
#
# COPY . . is here (not in rust-toolchain) so that only cargo build is
# re-executed on source changes, not the rustup installation.
FROM rust-toolchain AS builder

COPY . .

# BuildKit cache mounts persist the Cargo registry and target/ across rebuilds.
# Even when COPY . . invalidates the Docker layer cache, Cargo's incremental
# compilation kicks in — only changed crates are recompiled.
# Binaries are copied to /usr/local/bin-loadout/ (outside the cache mount scope)
# so they are available to subsequent stages.
RUN --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/root/.cargo/git \
    --mount=type=cache,target=/tmp/loadout-repo/target \
    cargo build -p loadout -p loadout-e2e --release \
    && mkdir -p /usr/local/bin-loadout \
    && cp target/release/loadout target/release/loadout-e2e /usr/local/bin-loadout/

# ─── Stage 5: dev ─────────────────────────────────────────────────────────────
# loadout and loadout-e2e built from source inside Docker.
# No host pre-build required — self-contained.
#
# Use case: quickly try the latest source in an isolated environment.
FROM builder AS dev

RUN mkdir -p ~/.local/bin \
    && cp /usr/local/bin-loadout/loadout /usr/local/bin-loadout/loadout-e2e ~/.local/bin/ \
    && chmod +x ~/.local/bin/loadout ~/.local/bin/loadout-e2e

# Install configs, features, and backends.
# Fixture backends/features (tests/fixtures/) are merged on top — they register
# as local/<name> and override or supplement the repo-bundled ones.
RUN mkdir -p ~/.config/loadout/configs \
    && cp -r features ~/.config/loadout/ \
    && cp -r backends ~/.config/loadout/ \
    && cp -r tests/fixtures/backends/. ~/.config/loadout/backends/ \
    && cp -r tests/fixtures/features/. ~/.config/loadout/features/ \
    && cp tests/fixtures/configs/* ~/.config/loadout/configs/

CMD ["/bin/bash"]

# ─── Stage 6: test ─────────────────────────────────────────────────────────────
# Binaries built inside Docker (from builder), installed into the os image.
# Smaller image than dev (no Rust toolchain).
# No host pre-build required — binaries come from the builder stage.
#
# Use case: run E2E scenarios against a hermetic, reproducible build.
FROM os AS test

# Pull only the compiled binaries from builder — no Rust toolchain in this image.
COPY --from=builder /usr/local/bin-loadout/loadout /usr/local/bin-loadout/loadout-e2e /root/.local/bin/

# Install configs, features, and backends.
# Fixture backends/features (tests/fixtures/) are merged on top — they register
# as local/<name> and override or supplement the repo-bundled ones.
RUN mkdir -p ~/.config/loadout/configs \
    && cp -r features ~/.config/loadout/ \
    && cp -r backends ~/.config/loadout/ \
    && cp -r tests/fixtures/backends/. ~/.config/loadout/backends/ \
    && cp -r tests/fixtures/features/. ~/.config/loadout/features/ \
    && cp tests/fixtures/configs/* ~/.config/loadout/configs/

CMD ["/bin/bash"]
