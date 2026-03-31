# ─── Stage 1: os ─────────────────────────────────────────────────────────────
# Base Ubuntu with all runtime dependencies.
# No loadout installed — use this as the starting point for manual install testing.
#
# Use case: walk through install.sh or copy a binary by hand.
FROM ubuntu:24.04 AS os

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
COPY . .

CMD ["/bin/bash"]

# ─── Stage 2: builder ────────────────────────────────────────────────────────
# Intermediate stage: Rust toolchain + cargo build inside Docker.
# Not used directly — dev is built on top of this.
FROM os AS builder

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
    sh -s -- -y --default-toolchain stable --profile minimal --no-modify-path
ENV PATH="/root/.cargo/bin:${PATH}"

RUN cargo build -p loadout -p loadout-e2e --release

# ─── Stage 3: dev ─────────────────────────────────────────────────────────────
# loadout and loadout-e2e built from source inside Docker.
# No host pre-build required — self-contained.
#
# Use case: quickly try the latest source in an isolated environment.
FROM builder AS dev

RUN mkdir -p ~/.local/bin \
    && cp target/release/loadout target/release/loadout-e2e ~/.local/bin/ \
    && chmod +x ~/.local/bin/loadout ~/.local/bin/loadout-e2e

RUN mkdir -p ~/.config/loadout/configs \
    && cp -r features ~/.config/loadout/ \
    && cp -r backends ~/.config/loadout/ \
    && cp tests/fixtures/configs/* ~/.config/loadout/configs/

CMD ["/bin/bash"]

# ─── Stage 4: test ────────────────────────────────────────────────────────────
# Pre-built binaries from the host are installed.
# Smaller image than dev (no Rust toolchain).
#
# Prerequisite: cargo build -p loadout -p loadout-e2e --release on the host.
#
# Use case: run E2E scenarios against a specific binary build (CI, release testing).
FROM os AS test

RUN mkdir -p ~/.local/bin \
    && cp target/release/loadout target/release/loadout-e2e ~/.local/bin/ \
    && chmod +x ~/.local/bin/loadout ~/.local/bin/loadout-e2e

RUN mkdir -p ~/.config/loadout/configs \
    && cp -r features ~/.config/loadout/ \
    && cp -r backends ~/.config/loadout/ \
    && cp tests/fixtures/configs/* ~/.config/loadout/configs/

CMD ["/bin/bash"]
