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
├── loadout / loadout.ps1      # Platform bootstrap (dispatch to Rust binary)
├── Cargo.toml                 # Rust workspace root
├── crates/                    # Rust implementation (Phases 1-6)
│   ├── cli/                   # CLI argument parsing
│   ├── app/                   # Pipeline orchestration
│   ├── model/                 # Core data types (State, Profile, Plan, etc.)
│   ├── planner/               # Planner (pure function: desired+state → plan)
│   ├── resolver/              # Dependency resolver (pure function)
│   ├── compiler/              # FeatureCompiler (policy → desired_resource_graph)
│   ├── executor/              # Executor (impure: plan → effects → state commit)
│   ├── state/                 # State persistence and atomic commit
│   ├── config/                # Profile/policy loading and normalization
│   ├── backend-host/          # Backend trait and ScriptBackend
│   ├── backends-builtin/      # Builtin backends (brew, apt, mise, npm, etc.)
│   ├── feature-host/          # Feature script execution
│   ├── feature-index/         # Feature Index Builder
│   ├── source-registry/       # Source discovery and registration
│   ├── platform/              # Platform detection
│   └── io/                    # Filesystem utilities
├── platforms/                 # Platform bootstrap scripts (sh/ps1)
│   ├── linux/
│   ├── windows/
│   └── wsl/
├── features/                  # Self-contained feature modules
│   ├── <feature>/
│   │   ├── feature.yaml       # Metadata (dep, mode, spec)
│   │   ├── install.sh         # Script mode (optional)
│   │   ├── uninstall.sh       # Script mode (optional)
│   │   └── files/             # Declarative mode (optional)
├── backends/                  # Script backend plugins (community extensions)
│   ├── <backend>/
│   │   ├── backend.yaml       # Metadata (api_version)
│   │   ├── apply.sh           # Install/upgrade operation
│   │   ├── remove.sh          # Uninstall operation
│   │   └── status.sh          # Query installation state
├── profiles/                  # Repository examples
├── policies/                  # Repository examples
├── tools/                     # Development tools
└── tests/                     # End-to-end tests (Rust unit tests in crates/)
```

**Directory boundaries enforce layer separation:**
- `crates/planner/` must not depend on `crates/executor/` (purity boundary)
- `crates/backend-host/` must not depend on `crates/state/` (plugin isolation)
- `crates/model/` is dependency-free data structures (no I/O, no side effects)

**Feature independence:**
- Each `features/<feature>/` directory is self-contained (no cross-feature imports)
- Features declare dependencies via `dep.depends` / `dep.requires` in `feature.yaml`

**State authority:**
- Only `crates/state/` may read/write authoritative `state.json`
- State file location: platform-defined (XDG/AppData), not overridable by `LOADOUT_STATE_FILE`

**Plugin isolation:**
- Backend plugins (script directories in `backends/`) communicate via JSON stdin/stdout only
- Builtin backends (`crates/backends-builtin/`) implement `Backend` trait directly
- Feature scripts (`install.sh`/`uninstall.sh`) receive environment variables only

**Authoritative runtime paths for profiles, policies, state, and sources live under XDG/AppData locations, not under the repository root.**

## Layer Violation Examples

* A feature script reading `state.json` directly → state authority violation
* A backend reading policy to select its own strategy → plugin isolation violation
* The planner calling a backend to check if a package is installed → purity violation
* A profile containing `if linux` branching → declaration layer violation
