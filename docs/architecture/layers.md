# Layer Model

## Layer Overview

```
platforms  →  profiles  →  loadout  →  cmd  →  core  →  features
                                          ↓
                                        state
```

Each layer has strict responsibility constraints.
Layer violations are architectural errors.

## Layer Responsibilities

### platforms

Bootstrap the minimum runtime environment required to execute loadout.

Responsibilities: detect OS, set environment variables, install minimal dependencies (git, jq, yq).

Must NOT: interpret profiles, install features, modify state, resolve dependencies.

### profiles

Declare desired environment composition.

Responsibilities: list enabled features, provide optional configuration values (e.g. version).

Must NOT: contain logic, OS branching, commands, or install details.

### cmd

Coordinate execution flow.

`plan`: load → resolve → compile → plan → display. Must NOT modify state.
`apply`: load → resolve → compile → plan → execute → commit state.

Must NOT: perform package management directly, re-classify after planner has decided.

### core

Provide infrastructure primitives.

Includes: resolver, planner, executor, state, backend_registry, source_registry, env, fs, logger, runner.

Must NOT: contain feature-specific logic, contain backend-specific logic,
let plugins read policy or write state directly.

### features

Implement environment units. One feature = one responsibility.

Responsibilities: install tool, place configuration files, register resources via state API.

Must NOT: resolve dependencies, modify other features, access state directly.

### backends

Execute package and runtime operations as adapters.

Responsibilities: install/uninstall packages and runtimes, report installed versions.

Must NOT: read policy, write state, contain orchestration logic.

### state

Record the effects of successful execution.

State is the single authority for what is installed and what is safe to remove.
No layer other than the state module may write to state.

See `specs/data/state.md` for the full contract.

## Data Flow

```
Profile + Policy + State + Sources
    ↓
  Source Registry       (pure lookup — canonical ID → source paths / allow-list)
    ↓
  Feature Index Builder (discovery, parse, spec_version validation → Feature Index)
    ↓
  Resolver              (pure — dep fields only → ResolvedFeatureOrder)
    ↓
  FeatureCompiler       (pure — Feature Index + Policy → DesiredResourceGraph)
    ↓
  Planner               (pure — diff + classify + decide → Plan)
    ↓
  Executor              (impure — executes actions, commits state)
    ↓
  State
```

Planner is pure: same inputs always produce the same Plan.
Executor is impure: calls feature scripts and backend plugins, commits state atomically.
State is both input (current reality) and output (recorded effects).
Source registry data influences lookup and admission only; it must not introduce hidden fallback or side effects.
Resolver reads only `dep` fields from the Feature Index; it must not read `resources` fields.
FeatureCompiler applies policy to resolve `desired_backend` for each resource; the result is embedded in DesiredResourceGraph.

## Repository Structure

The logical layer model maps to this directory structure:

```
loadout/
├── loadout / loadout.ps1   # CLI entry point (dispatcher)
├── cmd/                       # Orchestration layer (plan, apply)
├── core/lib/                  # Infrastructure primitives
│   ├── env, logger, fs, runner
│   ├── resolver, planner, executor
│   └── state, backend_registry, source_registry, orchestrator
├── features/                  # Self-contained feature modules
├── backends/                  # Backend plugin scripts
├── platforms/                 # Bootstrap scripts per platform
├── profiles/                  # Repository examples / fixtures
├── policies/                  # Repository examples / fixtures
├── tools/                     # Tools for development
└── tests/                     # Unit and integration tests
```

Directory boundaries enforce layer separation.
Feature independence is enforced through self-contained module directories.
State authority is enforced through a single authoritative file under the platform state directory.
Authoritative runtime paths for profiles, policies, state, and sources live under XDG/AppData locations, not under the repository root.

## Layer Violation Examples

* A feature script reading `state.json` directly → state authority violation
* A backend reading policy to select its own strategy → plugin isolation violation
* The planner calling a backend to check if a package is installed → purity violation
* A profile containing `if linux` branching → declaration layer violation
