# Backend Plugin Specification

## Scope

This document defines the normative interface contract for backend plugins.

Covered: required operations, observation API, capability declaration,
plugin isolation rules, determinism requirements, and compatibility.

Not covered: how to implement a backend (see `guides/backends.md`).

## Backend API Contract

Every backend plugin must implement the required operations listed below.
Core dispatches to backends via `backend_call <op> <args...>` through `backend_registry`.

Plugins are shell scripts loaded by sourcing. They must not have side effects on load.

## Required Operations

**`backend_api_version`**
Print the API version string (currently `"1"`). No arguments.

**`backend_capabilities`**
Print a space-separated list of supported resource kinds (e.g. `"package"`, `"runtime"`).

**`backend_package_install <name> [version]`**
Install the named package. Version is optional.
Must be idempotent: no error if already installed.

**`backend_package_uninstall <name>`**
Uninstall the named package.
Must only remove what this backend installed. Must not remove untracked resources.

## Optional Operations

**`backend_runtime_install <name> <version>`** (required if capabilities includes `runtime`)
Install the named runtime at the specified version.

**`backend_runtime_uninstall <name> <version>`** (required if capabilities includes `runtime`)
Uninstall the named runtime version.

## Capability Declaration

`backend_capabilities` defines which resource kinds this plugin handles.

| Value | Meaning |
|---|---|
| `package` | Can install/uninstall packages |
| `runtime` | Can install/uninstall versioned runtimes |

A backend declaring `runtime` must implement both `backend_runtime_install`
and `backend_runtime_uninstall`.

## Observation API

Used by the `plan` command layer (read-only, no side effects).
Must NOT be called by the planner itself.

**`backend_manager_exists`** — Return 0 if the underlying tool is available.

**`backend_package_exists <name> [version]`** — Return 0 if the package is installed.

**`backend_runtime_exists <name> <version>`** — Return 0 if the runtime version is installed.

## Plugin Isolation Rules

Backend plugins must NOT:

* read policy files directly
* read or write `state.json`
* communicate with other backend plugins
* produce side effects outside install/uninstall scope
* contain orchestration logic or dependency resolution

## Determinism Requirements

Given the same inputs (name, version), a backend must attempt the same operation.
Backends must not branch on undeclared environment state.

## Compatibility Rules

The backend API version is `1`. Breaking changes to required operations require a version bump.
New optional operations may be added without a version bump.
Core must gracefully handle backends that do not implement optional operations.
