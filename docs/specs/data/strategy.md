# Strategy Specification

## Scope

This document defines the normative contract for strategy files.

Covered: schema, backend resolution model, override rules, and resolution order.

Not covered: how backends work (see `specs/api/backend.md`),
or how to configure strategies (see `guides/usage.md`).

## Schema

Strategy is embedded as the optional `strategy:` section within a `config.yaml` file.
When the `strategy:` section is absent, `Strategy::default()` is used (no backend selection, backup enabled).

```yaml
# configs/linux.yaml
profile:
  components:
    ...

strategy:      # optional section
  package:
    default_backend: <backend_id>
    overrides:
      <package_name>:
        backend: <backend_id>

  runtime:
    default_backend: <backend_id>
    overrides:
      <runtime_name>:
        backend: <backend_id>

  fs:
    backup: true | false
    backup_dir: "<path>"
```

## File Location

Strategy is a section within the config file selected by `--config` / `-c`.
No standalone strategy file is used.

Config file location:

* Linux/WSL: `$XDG_CONFIG_HOME/loadout/configs/`
* Linux/WSL fallback: `~/.config/loadout/configs/`
* Windows: `%APPDATA%\loadout\configs\`

See `specs/data/profile.md` for config resolution rules.

## Backend Resolution Model

Strategy determines which backend is used for each resource installation.

Backend identifiers accept the same two forms as component identifiers:

* bare backend name, e.g. `brew`
* canonical backend ID, e.g. `core/brew`, `local/custompkg`

Bare backend names are normalized to `core/<name>` before backend loading.
`local` and external source backends must be explicit.

Resolution applies per resource kind:

* `package` resources — resolved via `package.default_backend` and `package.overrides`
* `runtime` resources — resolved via `runtime.default_backend` and `runtime.overrides`
* `fs` resources — no backend; handled by the fs module directly

## Override Rules

Override keys for `package` are exact package names as passed to the backend install command.
Override keys for `runtime` are runtime names (e.g. `node`, `python`), not component names.

If an override exists for a resource, it takes precedence over `default_backend`.
If no override exists, `default_backend` is used.
If `default_backend` is absent and no override matches, execution aborts.

## Resolution Order

```
per-resource override  >  default_backend  >  abort
```

## Validation Rules

* `package.default_backend` must be a non-empty string if present.
* `runtime.default_backend` must be a non-empty string if present.
* Override values must contain a `backend` key with a non-empty string.
* Unknown top-level keys are reserved and must not be present.
* External and `local` backends are resolved only if their source allow-list permits them.
