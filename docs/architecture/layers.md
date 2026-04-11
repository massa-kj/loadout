# Layer Model

## Layer Overview

```
platforms  →  configs  →  loadout  →  app  →  core  →  components
                                         ↓
                                       state
```

Each layer has strict responsibility constraints.
Layer violations are architectural errors.

## Layer Responsibilities

### platforms

Bootstrap the minimum runtime environment required to execute loadout.

Responsibilities: detect OS, set environment variables, install minimal dependencies (git, jq, yq).

Must NOT: interpret profiles, install components, modify state, resolve dependencies.

### configs

Declare desired environment composition and implementation strategy.

Responsibilities: list enabled components, provide optional configuration values (e.g. version),
optionally specify backend selection and backup strategy.

Must NOT: contain logic, OS branching, commands, or install details.

### app

Coordinate execution flow and provide use-case entry points for the CLI.

**Mutation pipeline:**

`plan`: load → resolve → compile → plan → display. Must NOT modify state.
`apply`: load → resolve → compile → plan → execute → commit state.
`prepare_execution`: load → resolve → compile → plan + registry → return ExecutionPlan. Enables confirmation prompts by CLI.
`execute`: consume ExecutionPlan → execute → commit state. On success, writes env plan cache (`{cache_home}/env_plan.json`) for `activate`.
`activate`: read env plan cache → generate shell activation script → print to stdout.

The `prepare_execution`/`execute` separation allows CLI to insert confirmation prompts between planning and execution.
`apply` remains as a convenience wrapper around `prepare_execution` + `execute`.
The env plan cache is an **ephemeral artifact** — not part of authoritative state.

**Read-only queries:**

The app layer also exposes read-only use cases that do not involve the planner or executor.
These build a lightweight pipeline (load config → discover sources/components/backends → query)
and return typed results to the caller without modifying state.

Query operations: component index queries, backend discovery queries, source registry queries,
config file queries, and state inspection.

Must NOT: perform package management directly, re-classify after planner has decided.

The `load` step includes all of the following before handing off to core:
- Bundle expansion (`bundle.use` → merge `bundles:` definitions)
- Namespace grouping normalization (`source_id: { name: {} }` → `source_id/name`)
- Canonicalization (flat `HashMap<String, ProfileFeatureConfig>` keyed by canonical ID)
- Validation (empty keys, undefined bundle names, duplicate canonicals)

### core

Provide infrastructure primitives.

Includes: resolver, planner, executor, state, backend_registry, source_registry, env, fs, logger, runner.

Must NOT: contain component-specific logic, contain backend-specific logic,
let plugins read strategy or write state directly.

### components

Implement environment units. One component = one responsibility.

Responsibilities: install tool, place configuration files, register resources via state API.

Must NOT: resolve dependencies, modify other components, access state directly.

### backends

Execute package and runtime operations as adapters.

Responsibilities: install/uninstall packages and runtimes, report installed versions.

Must NOT: read strategy, write state, contain orchestration logic.

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
  ├─ Config          (config.yaml — profile section + strategy section)
  ├─   Profile        (desired components, versions — decoded from config.profile)
  ├─   Strategy       (backend selection, backup strategy — decoded from config.strategy
  │                    or Strategy::default() when section is absent)
  ├─ Sources         (sources.yaml — plugin locations, admission control)
  └─ State           (state.json — installed resources, backends, versions)
    ↓
SourceRegistry
  (source clone, path resolution, allow rules)
    ↓
Component Discovery
  (scan components/ directories in each source)
    ↓
ComponentIndexBuilder
  (parse component.yaml, validate schema)
    ↓ ComponentIndex (normalized component metadata)
Resolver
  (dependency resolution, topological sort using dep fields only)
    ↓ ResolvedFeatureOrder (ordered list of component IDs)
ComponentCompiler
  ├─ resource expansion (from component specs)
  └─ backend resolution (using Strategy + platform defaults)
    ↓ DesiredResourceGraph (resources with desired_backend embedded)
Planner
  (diff with State, classify, decide: create/destroy/replace/noop/blocked)
    ↓ Plan (authoritative instruction set)
Executor
  inputs:
    ├─ Plan                   (what to do)
    ├─ DesiredResourceGraph   (execution payload for declarative resources)
    ├─ ComponentIndex           (mode/script metadata lookup)
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
- **Executor is impure**: Calls component scripts and backend plugins, commits state atomically.
- **State is both input and output**: Input = current reality, Output = recorded effects.
- **Source registry**: Influences lookup and admission only; must not introduce hidden fallback or side effects.
- **Resolver**: Reads only `dep` fields from Component Index; must not read `resources` fields.
- **ComponentCompiler**: Applies strategy to resolve `desired_backend` for each resource; the result is embedded in DesiredResourceGraph.

### Phase Characteristics

| Phase | Purity | Inputs | Outputs |
|---|---|---|---|
| Load | Impure (I/O) | Filesystem | Profile, Strategy, Sources, State |
| SourceRegistry | Pure (lookup) | Sources | Source paths, allow-lists |
| ComponentIndexBuilder | Impure (I/O) | Source paths, component.yaml | ComponentIndex |
| Resolver | Pure | Profile.components, ComponentIndex.dep | ResolvedFeatureOrder |
| ComponentCompiler | Pure | ResolvedFeatureOrder, ComponentIndex, Strategy | DesiredResourceGraph |
| Planner | Pure | DesiredResourceGraph, State, ResolvedFeatureOrder | Plan |
| Executor | Impure (side effects) | Plan, DesiredResourceGraph, ComponentIndex, BackendRegistry | Effects |
| State Commit | Impure (I/O) | Execution results | state.json, env_plan.json (cache) |
| Activate | Impure (I/O) | env_plan.json (cache), shell kind | Shell activation script (stdout) |

### Data Structure Roles

- **Profile**: User's desired environment (component list, versions, enable/disable) — the `profile:` section of a config file
- **Strategy**: User's implementation strategy (backend selection, backup strategy) — the optional `strategy:` section of a config file; absent → `Strategy::default()`
- **Sources**: Plugin locations and security allow-lists
- **State**: Single authority for installed resources (backend, version, fs paths)
- **ComponentIndex**: Normalized component metadata (depends, capabilities, resources)
- **ResolvedFeatureOrder**: Topologically sorted component IDs (dependency-resolved)
- **DesiredResourceGraph**: Expanded resources with resolved backends
- **Plan**: Authoritative instruction set (create/destroy/replace/noop/blocked)
- **BackendRegistry**: Dispatcher mapping `CanonicalBackendId` → `Backend` trait implementation
- **ExecutionEnvPlan**: Ephemeral snapshot of merged environment variables after apply; cached to `{cache_home}/env_plan.json` and consumed by `activate`

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
│   ├── compiler/              # ComponentCompiler (strategy → desired_resource_graph)
│   ├── executor/              # Executor (impure: plan → effects → state commit)
│   ├── state/                 # State persistence and atomic commit
│   ├── config/                # Profile/strategy loading and normalization
│   ├── backend-host/          # Backend trait and ScriptBackend
│   ├── backends-builtin/      # Builtin backends (brew, apt, mise, npm, etc.)
│   ├── component-host/          # Component script execution
│   ├── component-index/         # Component Index Builder
│   ├── source-registry/       # Source discovery and registration
│   ├── platform/              # Platform detection
│   └── io/                    # Filesystem utilities
├── platforms/                 # Platform bootstrap scripts (sh/ps1)
│   ├── linux/
│   ├── windows/
│   └── wsl/
├── components/                  # Self-contained component modules
│   ├── <component>/
│   │   ├── component.yaml       # Metadata (dep, mode, spec)
│   │   ├── install.sh         # Script mode (optional)
│   │   ├── uninstall.sh       # Script mode (optional)
│   │   └── files/             # Declarative mode (optional)
├── backends/                  # Script backend plugins
│   ├── <backend>/
│   │   ├── backend.yaml       # Metadata (api_version)
│   │   ├── apply.sh           # Install/upgrade operation
│   │   ├── remove.sh          # Uninstall operation
│   │   ├── status.sh          # Query installation state
│   │   ├── env_pre.sh         # Pre-action env delta (optional)
│   │   └── env_post.sh        # Post-action env delta (optional)
├── configs/                   # Repository example config files
│   └── <platform>.yaml        # profile + strategy sections
├── tools/                     # Development tools
└── tests/                     # End-to-end tests (Rust unit tests in crates/)
```

**Directory boundaries enforce layer separation:**
- `crates/planner/` must not depend on `crates/executor/` (purity boundary)
- `crates/backend-host/` must not depend on `crates/state/` (plugin isolation)
- `crates/model/` is dependency-free data structures (no I/O, no side effects)

**Component independence:**
- Each `components/<component>/` directory is self-contained (no cross-component imports)
- Components declare dependencies via `dep.depends` / `dep.requires` in `component.yaml`

**State authority:**
- Only `crates/state/` may read/write authoritative `state.json`
- State file location: platform-defined (XDG/AppData), not overridable by `LOADOUT_STATE_FILE`

**Plugin isolation:**
- Backend plugins (script directories in `backends/`) receive resource data via environment variables (primary) and optionally JSON stdin. `env_pre.sh`/`env_post.sh` return env deltas as JSON stdout.
- Builtin Rust backends (`crates/backends-builtin/`) implement `Backend` trait directly; currently an empty extension point.
- Component scripts (`install.sh`/`uninstall.sh`) receive environment variables only

**Authoritative runtime paths for profiles, policies, state, and sources live under XDG/AppData locations, not under the repository root.**

## Layer Violation Examples

* A component script reading `state.json` directly → state authority violation
* A backend reading strategy to select its own strategy → plugin isolation violation
* The planner calling a backend to check if a package is installed → purity violation
* A profile containing `if linux` branching → declaration layer violation
