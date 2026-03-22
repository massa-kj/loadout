# Layer Model

## Layer Overview

```
platforms  →  configs  →  loadout  →  app  →  core  →  features
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

### configs

Declare desired environment composition and implementation strategy.

Responsibilities: list enabled features, provide optional configuration values (e.g. version),
optionally specify backend selection and backup strategy.

Must NOT: contain logic, OS branching, commands, or install details.

### app

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

The system executes a deterministic pipeline from user declarations to state commit:

```
Orchestrator (app)
    ↓
Load Configuration
  ├─ Config          (config.yaml — profile section + policy section)
  ├─   Profile        (desired features, versions — decoded from config.profile)
  ├─   Policy         (backend selection, backup strategy — decoded from config.policy
  │                    or Policy::default() when section is absent)
  ├─ Sources         (sources.yaml — plugin locations, admission control)
  └─ State           (state.json — installed resources, backends, versions)
    ↓
SourceRegistry
  (source clone, path resolution, allow rules)
    ↓
Feature Discovery
  (scan features/ directories in each source)
    ↓
FeatureIndexBuilder
  (parse feature.yaml, validate schema)
    ↓ FeatureIndex (normalized feature metadata)
Resolver
  (dependency resolution, topological sort using dep fields only)
    ↓ ResolvedFeatureOrder (ordered list of feature IDs)
FeatureCompiler
  ├─ resource expansion (from feature specs)
  └─ backend resolution (using Policy + platform defaults)
    ↓ DesiredResourceGraph (resources with desired_backend embedded)
Planner
  (diff with State, classify, decide: create/destroy/replace/noop/blocked)
    ↓ Plan (authoritative instruction set)
Executor
  inputs:
    ├─ Plan                   (what to do)
    ├─ DesiredResourceGraph   (execution payload for declarative resources)
    ├─ FeatureIndex           (mode/script metadata lookup)
    └─ BackendRegistry        (backend dispatch)
  operations:
    ├─ Declarative Execution  (install/remove via backends)
    ├─ Imperative Execution   (install.sh/uninstall.sh scripts)
    └─ Fs Operations          (copy/symlink files/)
    ↓
State Commit
  (atomic write of installed resources to state.json)
```

### Key Invariants

- **Planner is pure**: Same inputs always produce the same Plan.
- **Executor is impure**: Calls feature scripts and backend plugins, commits state atomically.
- **State is both input and output**: Input = current reality, Output = recorded effects.
- **Source registry**: Influences lookup and admission only; must not introduce hidden fallback or side effects.
- **Resolver**: Reads only `dep` fields from Feature Index; must not read `resources` fields.
- **FeatureCompiler**: Applies policy to resolve `desired_backend` for each resource; the result is embedded in DesiredResourceGraph.

### Phase Characteristics

| Phase | Purity | Inputs | Outputs |
|---|---|---|---|
| Load | Impure (I/O) | Filesystem | Profile, Policy, Sources, State |
| SourceRegistry | Pure (lookup) | Sources | Source paths, allow-lists |
| FeatureIndexBuilder | Impure (I/O) | Source paths, feature.yaml | FeatureIndex |
| Resolver | Pure | Profile.features, FeatureIndex.dep | ResolvedFeatureOrder |
| FeatureCompiler | Pure | ResolvedFeatureOrder, FeatureIndex, Policy | DesiredResourceGraph |
| Planner | Pure | DesiredResourceGraph, State, ResolvedFeatureOrder | Plan |
| Executor | Impure (side effects) | Plan, DesiredResourceGraph, FeatureIndex, BackendRegistry | Effects |
| State Commit | Impure (I/O) | Execution results | state.json |

### Data Structure Roles

- **Profile**: User's desired environment (feature list, versions, enable/disable) — the `profile:` section of a config file
- **Policy**: User's implementation strategy (backend selection, backup policy) — the optional `policy:` section of a config file; absent → `Policy::default()`
- **Sources**: Plugin locations and security allow-lists
- **State**: Single authority for installed resources (backend, version, fs paths)
- **FeatureIndex**: Normalized feature metadata (depends, capabilities, resources)
- **ResolvedFeatureOrder**: Topologically sorted feature IDs (dependency-resolved)
- **DesiredResourceGraph**: Expanded resources with resolved backends
- **Plan**: Authoritative instruction set (create/destroy/replace/noop/blocked)
- **BackendRegistry**: Dispatcher mapping `CanonicalBackendId` → `Backend` trait implementation

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
├── configs/                   # Repository example config files
│   └── <platform>.yaml        # profile + policy sections
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
