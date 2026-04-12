//! State authority — the single module permitted to read and write loadout's authoritative state.
//!
//! # Contract
//!
//! Only this crate may write `state.json`. No other crate may call
//! [`commit`] or perform direct writes to the state file.
//!
//! # Operations
//!
//! * [`load`] — Load and validate state from disk. Returns `NeedsMigration` if file is v2.
//! * [`load_raw`] — Load state as untyped JSON (for the migrate command).
//! * [`commit`] — Atomically write validated state to disk.
//! * [`validate`] — Enforce all state invariants without I/O.
//! * [`migrate_v2_to_v3`] — Transform raw v2 JSON into a valid v3 [`State`].
//! * [`empty`] — Construct an empty initial state value.
//!
//! # Atomic Commit Protocol
//!
//! 1. Serialize `state` to pretty JSON.
//! 2. Write to `state.json.tmp`.
//! 3. Re-parse the written bytes and validate invariants.
//! 4. Rename `state.json.tmp` → `state.json`.
//! 5. Remove `state.json.tmp` on any failure after step 2.
//!
//! See: `docs/specs/data/state.md`

use std::collections::{HashMap, HashSet};
use std::path::Path;

use thiserror::Error;

pub use model::state::{
    ComponentState, FsDetails, FsEntryType, FsOp, PackageDetails, Resource, ResourceKind,
    RuntimeDetails, State, STATE_VERSION,
};
pub use model::tool::{ToolObservedFacts, ToolResource};

/// Errors produced by state operations.
#[derive(Debug, Error)]
pub enum StateError {
    /// Underlying I/O or parse failure.
    #[error("state I/O error: {0}")]
    Io(#[from] io::IoError),

    /// `state.json` is v2 (bare component keys). Run `loadout migrate` first.
    #[error("state version {version} requires migration; run `loadout migrate`")]
    NeedsMigration { version: u32 },

    /// State version is neither 2 nor 3 — unknown, cannot continue.
    #[error("unknown state version {found}; supported: 3")]
    VersionMismatch { found: u32 },

    /// Structural parse failure (not valid JSON or missing required fields).
    #[error("state file is corrupt: {reason}")]
    Corrupt { reason: String },

    /// An invariant is violated in an otherwise-parseable state.
    #[error("state invariant violation: {reason}")]
    InvalidState { reason: String },
}

// ─── Public API ──────────────────────────────────────────────────────────────

/// Return an empty (no components installed) initial state.
pub fn empty() -> State {
    State::empty()
}

/// Load and validate state from disk.
///
/// If the file does not exist, returns an empty v3 state (safe initial condition).
/// If the version is 2, returns [`StateError::NeedsMigration`].
/// If any invariant is violated, returns [`StateError::InvalidState`].
pub fn load(path: &Path) -> Result<State, StateError> {
    if !path.exists() {
        return Ok(State::empty());
    }

    // Read raw JSON to inspect version before full parse.
    let raw: serde_json::Value = load_raw(path)?;

    let version = extract_version(&raw)?;
    match version {
        2 => return Err(StateError::NeedsMigration { version: 2 }),
        STATE_VERSION => {}
        other => return Err(StateError::VersionMismatch { found: other }),
    }

    let state: State = serde_json::from_value(raw).map_err(|e| StateError::Corrupt {
        reason: format!("schema parse failed: {e}"),
    })?;

    validate(&state)?;
    Ok(state)
}

/// Load state as raw untyped JSON. Used by the migrate command.
///
/// Does not enforce version or invariants. Returns `Corrupt` if the file cannot be read or parsed.
pub fn load_raw(path: &Path) -> Result<serde_json::Value, StateError> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        StateError::Io(io::IoError::Read {
            path: path.to_path_buf(),
            source: e,
        })
    })?;

    serde_json::from_str(&content).map_err(|e| StateError::Corrupt {
        reason: format!("invalid JSON: {e}"),
    })
}

/// Atomically commit a validated state to disk.
///
/// Protocol: write → `.tmp` → re-parse → validate → rename → done.
/// The `.tmp` file is removed if any step after creation fails.
pub fn commit(path: &Path, state: &State) -> Result<(), StateError> {
    // Validate before touching disk.
    validate(state)?;

    // Ensure parent directory exists.
    if let Some(parent) = path.parent() {
        io::make_dirs(parent)?;
    }

    let serialized = serde_json::to_string_pretty(state).expect("State is always serializable");

    let tmp = path.with_extension("json.tmp");

    // Step 1-2: write to .tmp.
    std::fs::write(&tmp, &serialized).map_err(|e| {
        StateError::Io(io::IoError::Write {
            path: tmp.clone(),
            source: e,
        })
    })?;

    // Step 3: re-parse and validate (guards against serialization regressions).
    let result: Result<State, _> = serde_json::from_str(&serialized);
    let verified = match result {
        Ok(s) => s,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(StateError::Corrupt {
                reason: format!("re-parse failed after write: {e}"),
            });
        }
    };
    if let Err(e) = validate(&verified) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }

    // Step 4: atomic rename.
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        StateError::Io(io::IoError::Rename {
            from: tmp.clone(),
            to: path.to_path_buf(),
            source: e,
        })
    })?;

    Ok(())
}

/// Validate all state invariants without performing any I/O.
///
/// Returns `Ok(())` if all invariants hold, or [`StateError::InvalidState`] on first violation.
pub fn validate(state: &State) -> Result<(), StateError> {
    if state.version != STATE_VERSION {
        return Err(StateError::InvalidState {
            reason: format!("version must be {STATE_VERSION}, found {}", state.version),
        });
    }

    // Track fs.path uniqueness across all components.
    let mut seen_fs_paths: HashSet<&str> = HashSet::new();

    for (component_id, component_state) in &state.components {
        // Check for duplicate resource.id within component.
        let mut seen_ids: HashSet<&str> = HashSet::new();
        for resource in &component_state.resources {
            if resource.id.is_empty() {
                return Err(StateError::InvalidState {
                    reason: format!("component '{component_id}': resource.id must not be empty"),
                });
            }
            if !seen_ids.insert(resource.id.as_str()) {
                return Err(StateError::InvalidState {
                    reason: format!(
                        "component '{component_id}': duplicate resource id '{}'",
                        resource.id
                    ),
                });
            }

            // Check kind-specific invariants.
            match &resource.kind {
                ResourceKind::Fs { fs } => {
                    validate_fs_resource(component_id, &resource.id, fs, &mut seen_fs_paths)?;
                }
                ResourceKind::Tool { tool } => {
                    validate_tool_resource(component_id, &resource.id, tool)?;
                }
                ResourceKind::Package { .. } | ResourceKind::Runtime { .. } => {}
            }
        }
    }

    Ok(())
}

fn validate_fs_resource<'a>(
    component_id: &str,
    resource_id: &str,
    fs: &'a FsDetails,
    seen_fs_paths: &mut HashSet<&'a str>,
) -> Result<(), StateError> {
    // fs.path must be absolute.
    let p = std::path::Path::new(&fs.path);
    if !p.is_absolute() {
        return Err(StateError::InvalidState {
            reason: format!(
                "component '{component_id}', resource '{resource_id}': fs.path '{}' must be absolute",
                fs.path
            ),
        });
    }

    // fs.path must not be recorded by multiple components.
    if !seen_fs_paths.insert(fs.path.as_str()) {
        return Err(StateError::InvalidState {
            reason: format!(
                "component '{component_id}', resource '{resource_id}': fs.path '{}' is already recorded by another component",
                fs.path
            ),
        });
    }

    Ok(())
}

/// Validate invariants for a recorded `tool` resource.
///
/// Invariants:
/// - `tool.name` must not be empty.
/// - `tool.observed.resolved_path`, if present, must be an absolute path.
fn validate_tool_resource(
    component_id: &str,
    resource_id: &str,
    tool: &ToolResource,
) -> Result<(), StateError> {
    if tool.name.is_empty() {
        return Err(StateError::InvalidState {
            reason: format!(
                "component '{component_id}', resource '{resource_id}': tool.name must not be empty"
            ),
        });
    }

    if let Some(ref path) = tool.observed.resolved_path {
        let p = std::path::Path::new(path.as_str());
        if !p.is_absolute() {
            return Err(StateError::InvalidState {
                reason: format!(
                    "component '{component_id}', resource '{resource_id}': \
                     tool.observed.resolved_path '{path}' must be absolute"
                ),
            });
        }
    }

    Ok(())
}

// ─── Migration ───────────────────────────────────────────────────────────────

/// Transform a raw v2 state JSON value into a validated v3 [`State`].
///
/// Migration rules:
/// * Bare component keys (no `/`) are prefixed with `core/`.
/// * Keys that already contain `/` are preserved unchanged.
/// * Version is set to 3.
/// * Resource entries are preserved unchanged.
///
/// Returns `Ok(State)` if the migrated result passes invariant checks.
pub fn migrate_v2_to_v3(raw: &serde_json::Value) -> Result<State, StateError> {
    let version = extract_version(raw)?;
    match version {
        2 | 3 => {} // Version 3 is a no-op for migration (idempotent).
        other => {
            return Err(StateError::VersionMismatch { found: other });
        }
    }

    let components_obj = raw
        .get("components")
        .and_then(|v| v.as_object())
        .ok_or_else(|| StateError::Corrupt {
            reason: "missing or invalid 'components' object".into(),
        })?;

    let mut migrated_components: HashMap<String, ComponentState> = HashMap::new();

    for (key, component_val) in components_obj {
        let canonical_key = if key.contains('/') {
            key.clone()
        } else {
            format!("core/{key}")
        };

        let component_state: ComponentState = serde_json::from_value(component_val.clone())
            .map_err(|e| StateError::Corrupt {
                reason: format!("failed to parse component '{key}': {e}"),
            })?;

        migrated_components.insert(canonical_key, component_state);
    }

    let migrated = State {
        version: STATE_VERSION,
        components: migrated_components,
    };

    validate(&migrated)?;
    Ok(migrated)
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn extract_version(raw: &serde_json::Value) -> Result<u32, StateError> {
    raw.get("version")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .ok_or_else(|| StateError::Corrupt {
            reason: "missing or invalid 'version' field".into(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::id::CanonicalBackendId;

    fn backend(s: &str) -> CanonicalBackendId {
        CanonicalBackendId::new(s).unwrap()
    }

    fn state_with_package(component: &str, res_id: &str, pkg: &str) -> State {
        let mut s = State::empty();
        s.components.insert(
            component.into(),
            ComponentState {
                resources: vec![Resource {
                    id: res_id.into(),
                    kind: ResourceKind::Package {
                        backend: backend("core/brew"),
                        package: PackageDetails {
                            name: pkg.into(),
                            version: None,
                        },
                    },
                }],
            },
        );
        s
    }

    fn state_with_fs(component: &str, res_id: &str, path: &str) -> State {
        let mut s = State::empty();
        s.components.insert(
            component.into(),
            ComponentState {
                resources: vec![Resource {
                    id: res_id.into(),
                    kind: ResourceKind::Fs {
                        fs: FsDetails {
                            path: path.into(),
                            entry_type: FsEntryType::Symlink,
                            op: FsOp::Link,
                        },
                    },
                }],
            },
        );
        s
    }

    // ── validate ─────────────────────────────────────────────────────────────

    #[test]
    fn validate_empty_state_ok() {
        validate(&State::empty()).unwrap();
    }

    #[test]
    fn validate_valid_package_ok() {
        validate(&state_with_package("core/git", "pkg:git", "git")).unwrap();
    }

    #[test]
    fn validate_wrong_version_rejected() {
        let mut s = State::empty();
        s.version = 2;
        let err = validate(&s).unwrap_err();
        assert!(matches!(err, StateError::InvalidState { .. }));
    }

    #[test]
    fn validate_duplicate_resource_id_rejected() {
        let mut s = State::empty();
        s.components.insert(
            "core/git".into(),
            ComponentState {
                resources: vec![
                    Resource {
                        id: "pkg:git".into(),
                        kind: ResourceKind::Package {
                            backend: backend("core/brew"),
                            package: PackageDetails {
                                name: "git".into(),
                                version: None,
                            },
                        },
                    },
                    Resource {
                        id: "pkg:git".into(), // duplicate
                        kind: ResourceKind::Package {
                            backend: backend("core/brew"),
                            package: PackageDetails {
                                name: "git".into(),
                                version: None,
                            },
                        },
                    },
                ],
            },
        );
        let err = validate(&s).unwrap_err();
        assert!(matches!(err, StateError::InvalidState { .. }));
    }

    #[test]
    fn validate_duplicate_fs_path_across_components_rejected() {
        let path = "/home/user/.gitconfig";
        let mut s = state_with_fs("core/git", "fs:gitconfig", path);
        // Add another component with the same fs.path.
        s.components.insert(
            "core/other".into(),
            ComponentState {
                resources: vec![Resource {
                    id: "fs:conflict".into(),
                    kind: ResourceKind::Fs {
                        fs: FsDetails {
                            path: path.into(),
                            entry_type: FsEntryType::Symlink,
                            op: FsOp::Link,
                        },
                    },
                }],
            },
        );
        let err = validate(&s).unwrap_err();
        assert!(matches!(err, StateError::InvalidState { .. }));
    }

    #[test]
    fn validate_relative_fs_path_rejected() {
        let s = state_with_fs("core/git", "fs:cfg", "relative/path");
        let err = validate(&s).unwrap_err();
        assert!(matches!(err, StateError::InvalidState { .. }));
    }

    // ── load ─────────────────────────────────────────────────────────────────

    #[test]
    fn load_missing_file_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let s = load(&path).unwrap();
        assert_eq!(s.version, STATE_VERSION);
        assert!(s.components.is_empty());
    }

    #[test]
    fn load_valid_v3_ok() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let json = r#"{"version":3,"components":{}}"#;
        std::fs::write(&path, json).unwrap();
        let s = load(&path).unwrap();
        assert_eq!(s.version, 3);
    }

    #[test]
    fn load_v2_returns_needs_migration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let json = r#"{"version":2,"components":{"git":{"resources":[]}}}"#;
        std::fs::write(&path, json).unwrap();
        let err = load(&path).unwrap_err();
        assert!(matches!(err, StateError::NeedsMigration { version: 2 }));
    }

    #[test]
    fn load_unknown_version_returns_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let json = r#"{"version":99,"components":{}}"#;
        std::fs::write(&path, json).unwrap();
        let err = load(&path).unwrap_err();
        assert!(matches!(err, StateError::VersionMismatch { found: 99 }));
    }

    #[test]
    fn load_corrupt_json_returns_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, "{not json}").unwrap();
        let err = load(&path).unwrap_err();
        assert!(matches!(err, StateError::Corrupt { .. }));
    }

    // ── commit ───────────────────────────────────────────────────────────────

    #[test]
    fn commit_creates_file_and_no_tmp_left() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let s = state_with_package("core/git", "pkg:git", "git");
        commit(&path, &s).unwrap();

        assert!(path.exists(), "state.json must exist after commit");
        let tmp = path.with_extension("json.tmp");
        assert!(!tmp.exists(), "state.json.tmp must be cleaned up");
    }

    #[test]
    fn commit_round_trips_correctly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let original = state_with_package("core/git", "pkg:git", "git");
        commit(&path, &original).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(original, loaded);
    }

    #[test]
    fn commit_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("dir").join("state.json");
        commit(&path, &State::empty()).unwrap();
        assert!(path.exists());
    }

    // ── migrate_v2_to_v3 ─────────────────────────────────────────────────────

    #[test]
    fn migrate_bare_key_prefixed_with_core() {
        let raw = serde_json::json!({
            "version": 2,
            "components": {
                "git": { "resources": [] },
                "core/ruby": { "resources": [] }
            }
        });
        let migrated = migrate_v2_to_v3(&raw).unwrap();
        assert_eq!(migrated.version, STATE_VERSION);
        assert!(
            migrated.components.contains_key("core/git"),
            "bare 'git' must become 'core/git'"
        );
        assert!(
            migrated.components.contains_key("core/ruby"),
            "'core/ruby' must be preserved"
        );
        assert!(
            !migrated.components.contains_key("git"),
            "bare key must be removed"
        );
    }

    #[test]
    fn migrate_v3_is_idempotent() {
        let raw = serde_json::json!({
            "version": 3,
            "components": {
                "core/git": { "resources": [] }
            }
        });
        let migrated = migrate_v2_to_v3(&raw).unwrap();
        assert_eq!(migrated.version, STATE_VERSION);
        assert!(migrated.components.contains_key("core/git"));
    }

    #[test]
    fn migrate_preserves_resources() {
        let raw = serde_json::json!({
            "version": 2,
            "components": {
                "git": {
                    "resources": [
                        {
                            "id": "pkg:git",
                            "kind": "package",
                            "backend": "core/brew",
                            "package": { "name": "git", "version": null }
                        }
                    ]
                }
            }
        });
        let migrated = migrate_v2_to_v3(&raw).unwrap();
        let feat = migrated.components.get("core/git").unwrap();
        assert_eq!(feat.resources.len(), 1);
        assert_eq!(feat.resources[0].id, "pkg:git");
    }

    #[test]
    fn migrate_unknown_version_rejected() {
        let raw = serde_json::json!({ "version": 1, "components": {} });
        let err = migrate_v2_to_v3(&raw).unwrap_err();
        assert!(matches!(err, StateError::VersionMismatch { .. }));
    }

    // ── tool resource invariants ──────────────────────────────────────────────

    fn state_with_tool(
        component: &str,
        res_id: &str,
        name: &str,
        resolved_path: Option<&str>,
    ) -> State {
        use model::tool::{
            OneOf, ToolIdentityVerify, ToolObservedFacts, ToolResource, ToolVerifyContract,
        };
        let mut s = State::empty();
        s.components.insert(
            component.into(),
            ComponentState {
                resources: vec![Resource {
                    id: res_id.into(),
                    kind: ResourceKind::Tool {
                        tool: ToolResource {
                            name: name.into(),
                            verify: ToolVerifyContract {
                                identity: ToolIdentityVerify::ResolvedCommand {
                                    command: name.into(),
                                    expected_path: OneOf {
                                        one_of: vec![
                                            "/home/linuxbrew/.linuxbrew/bin/brew".into(),
                                        ],
                                    },
                                },
                                version: None,
                            },
                            observed: ToolObservedFacts {
                                resolved_path: resolved_path.map(|s| s.into()),
                                version: None,
                            },
                        },
                    },
                }],
            },
        );
        s
    }

    #[test]
    fn validate_tool_with_absolute_observed_path_ok() {
        let s = state_with_tool(
            "core/brew",
            "tool:brew",
            "brew",
            Some("/home/linuxbrew/.linuxbrew/bin/brew"),
        );
        validate(&s).unwrap();
    }

    #[test]
    fn validate_tool_with_no_observed_path_ok() {
        let s = state_with_tool("core/brew", "tool:brew", "brew", None);
        validate(&s).unwrap();
    }

    #[test]
    fn validate_tool_relative_observed_path_rejected() {
        let s = state_with_tool("core/brew", "tool:brew", "brew", Some("relative/path/brew"));
        let err = validate(&s).unwrap_err();
        assert!(matches!(err, StateError::InvalidState { .. }));
        let msg = err.to_string();
        assert!(msg.contains("resolved_path"), "error must mention resolved_path");
        assert!(msg.contains("absolute"), "error must mention absolute");
    }

    #[test]
    fn validate_tool_empty_name_rejected() {
        let s = state_with_tool("core/brew", "tool:brew", "", None);
        let err = validate(&s).unwrap_err();
        assert!(matches!(err, StateError::InvalidState { .. }));
        let msg = err.to_string();
        assert!(msg.contains("tool.name"), "error must mention tool.name");
    }

    #[test]
    fn validate_tool_round_trip_via_json() {
        // Confirm tool resources survive state commit/load round-trip.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let s = state_with_tool(
            "core/brew",
            "tool:brew",
            "brew",
            Some("/home/linuxbrew/.linuxbrew/bin/brew"),
        );
        commit(&path, &s).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(s, loaded);
    }
}
