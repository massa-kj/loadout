# Stage 1: base
# Minimal OS dependencies + repo copy.
# Use this stage to test bootstrap itself.
FROM ubuntu:24.04 AS base

# Install minimal dependencies required for bootstrap
RUN apt-get update && apt-get install -y \
    git \
    curl \
    sudo \
    ca-certificates \
    bash \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /loadout

# Copy loadout repository
COPY . .

# Make scripts executable
RUN chmod +x loadout \
    && chmod +x platforms/linux/bootstrap.sh \
    && cp target/debug/loadout target \
    && find tests/environment/linux/docker/scenarios -type f -name "*.sh" -exec chmod +x {} \;

CMD ["/bin/bash"]

# ─────────────────────────────────────────────

# Stage 2: bootstrapped
# Bootstrap already run; brew and mise are installed.
# Use this stage for scenario tests (fast startup, no bootstrap overhead).
FROM base AS bootstrapped

RUN ./platforms/linux/bootstrap.sh

CMD ["/bin/bash"]
