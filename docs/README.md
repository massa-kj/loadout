# Documentation

## How to Read These Documents

Each document type has a distinct scope.
Do not mix concerns across types when writing or updating.

## Document Types

**[Architecture](./architecture/README.md)** — Design philosophy and layer boundaries.
Does not contain specifications or implementation details.

**[Spec](./specs/README.md)** — Normative contracts for data formats, APIs, and algorithms.
Does not contain opinions or implementation guidance.

**[Guide](./guides/README.md)** — How to use or implement something.
References specs for contracts; does not redefine them.

**[Development](./development/README.md)** — Process and tooling for contributors.
Not relevant to end users.

## Recommended Reading Order

To understand the system:
1. **[architecture/principles.md](./architecture/principles.md)** — why it is designed this way
2. **[architecture/layers.md](./architecture/layers.md)** — how the layers relate
3. **[architecture/boundaries.md](./architecture/boundaries.md)** — what is forbidden

To implement a feature:
1. **[guides/features.md](./guides/features.md)**
2. **[specs/data/state.md](./specs/data/state.md)** — state interaction rules
3. **[specs/algorithms/planner.md](./specs/algorithms/planner.md)** — how your feature appears in the plan

To implement a backend:
1. **[guides/backends.md](./guides/backends.md)**
2. **[specs/api/backend.md](./specs/api/backend.md)** — the contract you must satisfy

## Directory Structure

```
docs/
├── README.md
├── specs/
│   ├── data/        profile, policy, state contracts
│   ├── api/         backend plugin interface
│   └── algorithms/  planner, resolver
├── architecture/
│   ├── principles.md
│   ├── layers.md
│   └── boundaries.md
├── guides/
│   ├── usage.md
│   ├── features.md
│   └── backends.md
└── development/
    ├── testing.md
    ├── documentation.md
    └── direction.md
```
