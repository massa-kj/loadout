# Documentation Guide

## Principle

Separate stable design intent from volatile implementation detail.

Documents describe responsibilities, boundaries, and guarantees
that remain valid across implementation changes.

**Rust doc** describes callable APIs, struct/enum types, and implementation algorithms.

**docs/** describes external contracts, architectural boundaries, and safety rules
that external users (plugin authors, profile writers) must know without reading source code.

## Document Types

| Type | Location | Contains |
|---|---|---|
| Architecture | `docs/architecture/` | Design philosophy, layer model, boundaries |
| Spec | `docs/specs/` | Normative contracts: schemas, invariants, algorithms |
| Guide | `docs/guides/` | Usage and implementation instructions |
| Development | `docs/development/` | Contributor process and tooling |

## What to Document

Write in documents:
* Layer responsibilities and what each layer must NOT do
* Invariants that must hold regardless of implementation
* Decision rules the system must follow (e.g. decision table, resolution order)
* Architectural constraints and the reasoning behind them
* External contracts (YAML/JSON schemas, plugin protocols)

## What NOT to Document

Do not write in documents:
* Struct/enum field types and internal representations (→ Rust doc in `crates/*/src/lib.rs`)
* Function signatures and error variants (→ Rust doc)
* Implementation-specific algorithms when the spec already defines the contract (→ Rust doc)
* Volatile API details that change with refactoring

## Documentation Boundary

### Scope

This section defines the responsibility boundary between `docs/` and Rust documentation.

Documentation must be split clearly to avoid duplication and maintain clarity.

### What Goes in `docs/`

**External contracts (language-agnostic):**
- YAML/JSON schema **meanings** (e.g., "what does `mode: declarative` mean?")
- Invariants (e.g., "state must not contain duplicate `fs.path` across features")
- Safety rules (e.g., "features must not write to `state.json` directly")
- Forbidden operations (e.g., "planner must not execute backends")
- Resolution order (e.g., "profile override > default_backend > abort")
- Decision tables (e.g., planner classification rules)
- Plugin interface protocols (e.g., JSON stdin/stdout for backends, env vars for features)

**These must remain in `docs/` because:**
- External users (plugin authors, profile writers) need them without reading Rust code
- They define correctness independent of implementation
- Breaking them is always a safety or architectural violation

### What Goes in Rust Doc

**Implementation contracts (Rust-specific):**
- Struct/enum definitions (`pub struct State { ... }`)
- Field types and constraints (`resources: HashMap<CanonicalFeatureId, Vec<Resource>>`)
- Function signatures (`pub fn classify(...) -> Classification`)
- Error type variants (`BackendError::ScriptFailed { exit_code, stderr }`)
- Implementation examples (doctests)
- Internal module organization

**These belong in Rust doc because:**
- They change with implementation refactoring (field renames, type changes)
- They are compiler-checked (out-of-date docs cause compile errors)
- Reading the code is the authoritative source for implementers

### Schema: Meaning vs. Structure

| Aspect | `docs/` | Rust Doc |
|---|---|---|
| **Meaning** | "A `package` resource represents a backend-managed package installation" | — |
| **Structure** | "Must have `kind`, `id`, `name`, `backend` fields" | `pub struct PackageResource { pub name: String, pub backend: CanonicalBackendId }` |
| **Constraint** | "`backend` must be a canonical ID of the form `<source>/<name>`" | Type: `CanonicalBackendId` (enforce via newtype) |
| **Validation** | "Core must validate all invariants before execution" | `fn validate(&self) -> Result<(), ValidationError>` |

**Rule:** Schema **semantics** in docs, schema **structure** in Rust.

### Cross-References

`docs/` should reference Rust crates for **implementation location**, not duplicate content:

```markdown
See implementation: `crates/planner/src/lib.rs` (`classify` function).
```

Rust doc should reference `docs/` for **external contract**:

```rust
/// Classify a feature based on desired vs. current state.
///
/// Classification rules are defined in `docs/specs/algorithms/planner.md`.
pub fn classify(...) -> Classification { ... }
```

### Duplication Policy

**Prohibited:**
- Duplicating decision tables in both docs and Rust doc
- Duplicating struct field lists in both docs and Rust doc
- Duplicating function signatures in docs (unless it's an external plugin interface)

**Allowed:**
- Summarizing architectural context in Rust doc (brief, with docs/ reference)
- Repeating safety rules in Rust doc comments (reinforcement, with docs/ reference)

**Required:**
- External contracts (YAML/JSON, plugin protocols) must be in docs/
- Rust doc must reference docs/ for normative rules

### Example: State Schema

**`docs/specs/data/state.md` must define:**
- "State is the single authority for what is installed"
- "`version` must be `3`"
- "`features` must be an object"
- "Within a feature, `resource.id` must be unique"
- "Across features, `fs.path` must be unique"
- "Core must validate all invariants before execution"
- JSON schema structure (field names, types)

**`crates/model/src/state.rs` must define:**
```rust
/// State is the authoritative record of installed resources.
///
/// Schema and invariants: `docs/specs/data/state.md`.
pub struct State {
    pub version: u32,
    pub features: HashMap<CanonicalFeatureId, FeatureState>,
}
```

**Why this split works:**
- docs/ defines **what is correct** (contract)
- Rust defines **how it is represented** (types)
- Changing field types (e.g., `HashMap` → `BTreeMap`) doesn't break external contract
- External users read docs/ for JSON format; implementers read Rust doc for types

---

## API Stability

### External APIs (docs/)

External APIs are documented in `docs/specs/`. Breaking changes require:

1. Version bump in the affected schema (e.g., `state.version: 3` → `4`)
2. Explicit migration path documented in the spec
3. Update to `architecture/boundaries.md` if an architectural boundary changes
4. **Never break without migration path** — external contracts are sacred

Examples of external API breakage:
- Changing YAML/JSON field names visible to users
- Changing invariants in state schema
- Changing plugin interface protocol (backend JSON schema, feature env vars)
- Changing resolution order or decision table logic

### Internal APIs (Rust)

Rust public APIs within the workspace are documented in Rust doc. Breaking changes:

1. Must not break external contracts (no docs/ change needed if semantics unchanged)
2. Require `cargo test` and `cargo clippy` to pass
3. May change freely if no external impact

Examples of internal API changes (no docs/ update needed):
- Renaming `State` struct fields (as long as JSON serde names stay the same via `#[serde(rename)]`)
- Changing function signatures within a crate's private API
- Refactoring module boundaries
- Changing error variant details

### Cross-Boundary Changes

If a Rust refactor requires changing docs/:
- The change is **not purely internal** — treat as external API change
- Follow external API stability rules
- Consider if the change is worth the external breakage

Internal APIs may change freely; they must not be referenced across public boundaries.

## Maintenance Workflow

### When Adding a New Feature Module

Update `guides/features.md` if new patterns emerge.

No change to core docs required unless:
- New resource kind is introduced → update `docs/specs/data/state.md` and `docs/specs/data/desired_resource_graph.md`
- New dependency mechanism is introduced → update `docs/specs/algorithms/resolver.md`

### When Adding a New Backend

No doc update required unless:
- New capability type is introduced → update `docs/specs/api/backend.md`
- New resource kind is supported → update `docs/specs/data/state.md`

### When Changing State Schema

1. Update `docs/specs/data/state.md` (external contract)
2. Bump `version` field in schema
3. Provide migration path (document in state.md, implement in `crates/state/src/lib.rs`)
4. Update Rust types in `crates/model/src/state.rs`

### When Changing Planner Decision Table

1. Update `docs/specs/algorithms/planner.md` (normative table)
2. Update implementation in `crates/planner/src/lib.rs`
3. Ensure Rust doc references the spec: `/// Classification rules: docs/specs/algorithms/planner.md`

### When Changing Architectural Boundary

1. Update `docs/architecture/boundaries.md`
2. Update relevant spec (e.g., `docs/specs/algorithms/planner.md` if planner purity changes)
3. Update affected Rust crates
4. This is a **major change** — requires discussion and approval

### When Refactoring Rust Code

**No docs/ update required if:**
- Renaming structs/fields/functions without changing semantics
- Splitting or merging modules
- Changing internal implementation algorithms
- Changing error message text

**docs/ update required if:**
- External YAML/JSON field names change
- Invariants change
- Safety rules change
- Decision logic changes

### Documents vs. Implementation Transients

Documents must not be updated to document implementation transients.
If a doc needs updating every sprint, the content belongs in Rust doc comments.

**Rule:** If the change is visible to external users (plugin authors, profile writers) without reading Rust source, update docs/. Otherwise, update only Rust doc.
