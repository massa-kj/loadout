# Stage 1: base
# Minimal OS + repo copy for pre-release binary validation.
# Use this stage to test binaries before installation.
FROM ubuntu:24.04 AS base

# Install minimal dependencies
RUN apt-get update && apt-get install -y \
    curl \
    sudo \
    ca-certificates \
    bash \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /tmp/loadout-repo

# Copy loadout repository
COPY . .

CMD ["/bin/bash"]

# ─────────────────────────────────────────────

# Stage 2: installed
# loadout binary installed + features/backends configured.
# Use this stage for scenario tests.
FROM base AS installed

WORKDIR /tmp/loadout-repo

# Copy repository for installation and configuration
COPY . .

# Install loadout binary to ~/.local/bin/
RUN mkdir -p ~/.local/bin \
    && cp target/debug/loadout ~/.local/bin/loadout \
    && chmod +x ~/.local/bin/loadout \
    && echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc

# Set up configuration directory structure
RUN mkdir -p ~/.config/loadout/configs \
    && cp -r features ~/.config/loadout/ \
    && cp -r backends ~/.config/loadout/ \
    && cp tests/environment/linux/docker/fixtures/* ~/.config/loadout/configs/ \
    && find tests/environment/linux/docker/scenarios -type f -name "*.sh" -exec chmod +x {} \;

# Set PATH for this session
ENV PATH="/root/.local/bin:${PATH}"

WORKDIR /tmp/loadout-repo

CMD ["/bin/bash"]
