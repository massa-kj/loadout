# loadout

A declarative and deterministic environment manager system for Linux, WSL, and Windows.
Designed to safely reproduce development environments.

> ⚠️ This project is under active development.
> 
> The architecture is still evolving and breaking changes may occur until the first stable release.

## Overview

This project turns environment setup into a deterministic system.
Features are declared in a profile; the system installs, updates, and removes them safely.

Key goals:

* **Reproducible** — the same profile produces the same environment on any machine
* **Safe** — uninstall removes only what the system installed, never anything else
* **Deterministic** — given the same inputs, execution always produces the same plan
* **Plan / Apply execution model** — changes are always previewed as a plan before execution
* **Cross-platform** — Linux, WSL, and Windows share the same model

## Quick Start

Define your environment in a profile:

```yaml
# profiles/wsl.yaml
features:
  git: {}
  neovim: {}
  node:
    version: "22.17.1"
```

Define installation strategy with policy:

```yaml
# policies/default.yaml
package:
  default_backend: brew
runtime:
  default_backend: mise
```

Preview changes without applying:

```bash
./loadout plan profiles/wsl.yaml
```

Apply to your machine:

```bash
./loadout apply profiles/wsl.yaml
```

Re-running apply is safe. Features already in the correct state are skipped.

## Design Goals

**Declaration over scripting**
Profiles express intent, not procedures. The system decides how to produce the result.

**State over inference**
Installed resources are recorded in state.
The system never scans the filesystem to infer what exists.
State is the only authority for uninstall decisions.

**Safety over convenience**
The system aborts rather than guesses.
Destructive operations require explicit intent.

**Replaceability**
Backends (Homebrew, mise, winget, …) and features are interchangeable adapters.
Core does not embed tool-specific logic.

## Key Concepts

**Profile** — declares which features should be present and at what version.

**Policy** — declares which installation strategy to use for package and runtime management.

**Feature** — a self-contained module: `meta.yaml` + `install` + `uninstall` + `files/`.

**Backend** — executes package/runtime operations (brew, mise, scoop, winget, npm, uv).

**Plan** — a deterministic list of actions computed from profile vs state.

**State** — the authoritative record of what the system has installed and how to remove it.

## Documentation

Design documents are in [`docs/`](docs/README.md).

| | |
|---|---|
| [Architecture](docs/architecture/README.md) | System design principles, execution model, and architectural boundaries |
| [Guides](docs/guides/README.md) | How to use the system and how to implement features or backends |
| [Specifications](docs/specs/README.md) | Formal specifications such as state schema and execution contracts |
