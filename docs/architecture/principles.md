# Architectural Principles

## Purpose

This document defines the design philosophy behind the system.
It explains *why* the architecture is structured as it is,
not *what* is built or *how* to build it.

## Core Principles

**Declarative**
Profiles express intent — not procedures.
Users declare what should exist; the system decides how to produce it.

**Determinism**
Given the same profile, policy, and state, execution must produce the same plan.
No hidden randomness, no implicit fallback, no environment-dependent branching.
Re-running apply must produce no duplicate resources and no inconsistent state.
Idempotency is guaranteed by state diff, deterministic execution order, and shallow dependencies.

**State Authority**
State is the single authority for installed resources.
The system must never infer ownership from filesystem inspection.
If it is not in state, it does not exist from the system's perspective.

**Replaceability**
Backends are adapters. Features are implementation units.
Both must be replaceable without modifying core.
Core must remain tool-agnostic and platform-insulated.

**Safety**
No resource may be removed without state confirmation.
Destructive operations require explicit intent, never implicit inference.
Safety constraints are non-negotiable and cannot be bypassed by configuration.
External sources are admitted only through explicit allow-lists; absent allow rules mean deny-by-default.

## Design Tradeoffs

**Safety vs convenience**
The system will abort rather than guess.
An explicit error is always safer than a silent assumption.

**Determinism vs flexibility**
The decision table is static and total.
Dynamic or conditional decisions belong in feature scripts, not in core.

## Non-Goals

This system does not:

* manage the OS itself (only user-space tooling and configuration)
* provide transaction rollback guarantees beyond atomic state writes
* guarantee consistency of external package managers
* support conditional or runtime-computed dependencies
