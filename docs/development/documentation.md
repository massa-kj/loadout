# Documentation Guide

## Principle

Separate stable design intent from volatile implementation detail.

Documents describe responsibilities, boundaries, and guarantees
that remain valid across implementation changes.
Source comments describe callable APIs that change with implementation.

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

## What NOT to Document

Do not write in documents:
* Function arguments or return value formats (→ source comments)
* Implementation examples with shell commands (→ source comments or guides)
* Volatile API details (→ source comments)
* Core module internals — core boundaries and responsibilities are defined in `architecture/layers.md`;
  function-level API is source-authoritative (module header comments in `core/lib/`)

## Source Comment Rules

Source comments define callable API only.

Every module must have a header comment with:
* Module name
* Responsibility (one sentence)
* Public API list (stable)
* Internal API list (optional)

Function comments must include:
* Argument names and purpose
* Return value / exit code semantics
* Error conditions

Example:
```bash
# resolve_dependencies <desired_features_nameref> <output_array_nameref>
# Resolve capability deps and return topologically sorted feature list.
# Returns 1 if a cycle or missing provider is detected.
```

## API Stability

Stable APIs are documented in module headers. Breaking changes require:
1. Version bump or explicit migration path
2. Update to the relevant spec
3. Update to `architecture/boundaries.md` if a boundary changes

Internal APIs may change freely; they must not be referenced across modules.

## Maintenance Workflow

When adding a new feature module: update `guides/features.md` if new patterns emerge.

When adding a new backend: no doc update required unless new capability types are introduced.

When changing the state schema: update `specs/data/state.md`, bump version, provide migration path.

When changing the planner decision table: update `specs/algorithms/planner.md`.

When changing an architectural boundary: update `architecture/boundaries.md` and the relevant spec.

Documents must not be updated to document implementation transients.
If a doc needs updating every sprint, the content belongs in source comments.
