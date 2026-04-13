//! Type-safe state file assertions used by all scenarios.
//!
//! Each helper reads the state file, deserialises it using [`model::state::State`],
//! and checks a single invariant.  On failure it returns an `Err(String)` with a
//! human-readable description of what went wrong.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use model::state::{ResourceKind, State, STATE_VERSION};

// ──────────────────────────────────────────────────────────────────────────────
// Loading
// ──────────────────────────────────────────────────────────────────────────────

/// Read and deserialise the state file at `path`.
pub fn load_state(path: &Path) -> Result<State, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read state file at {}: {}", path.display(), e))?;
    serde_json::from_str(&raw).map_err(|e| format!("state file is invalid JSON: {}", e))
}

/// Read and return the raw JSON value (for diff / snapshot comparisons).
pub fn load_state_raw(path: &Path) -> Result<serde_json::Value, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read state file at {}: {}", path.display(), e))?;
    serde_json::from_str(&raw).map_err(|e| format!("state file is invalid JSON: {}", e))
}

// ──────────────────────────────────────────────────────────────────────────────
// Basic structural invariants
// ──────────────────────────────────────────────────────────────────────────────

/// Assert that `state.version == STATE_VERSION`.
pub fn assert_state_version(state: &State) -> Result<(), String> {
    if state.version != STATE_VERSION {
        return Err(format!(
            "expected state version {}, got {}",
            STATE_VERSION, state.version
        ));
    }
    Ok(())
}

/// Assert that the `components` map is present (non-absent; may be empty).
pub fn assert_components_present(state: &State) -> Result<(), String> {
    // `components` is always present after deserialisation (defaults to `{}`),
    // so this is a structural sanity-check rather than a content check.
    let _ = &state.components;
    Ok(())
}

/// Assert that every component's resource list contains no duplicate `id` values.
pub fn assert_no_duplicate_resource_ids(state: &State) -> Result<(), String> {
    for (component_id, component_state) in &state.components {
        let mut seen: HashSet<&str> = HashSet::new();
        for resource in &component_state.resources {
            if !seen.insert(resource.id.as_str()) {
                return Err(format!(
                    "duplicate resource id '{}' in component '{}'",
                    resource.id, component_id
                ));
            }
        }
    }
    Ok(())
}

/// Assert that no `fs` resource path appears more than once across all components.
pub fn assert_no_duplicate_fs_paths(state: &State) -> Result<(), String> {
    let mut seen: HashMap<&str, &str> = HashMap::new();
    for (component_id, component_state) in &state.components {
        for resource in &component_state.resources {
            if let ResourceKind::Fs { fs } = &resource.kind {
                if let Some(prev_component) = seen.insert(fs.path.as_str(), component_id.as_str()) {
                    return Err(format!(
                        "fs path '{}' is tracked by both '{}' and '{}'",
                        fs.path, prev_component, component_id
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Assert that every `fs` resource path is absolute.
pub fn assert_all_fs_paths_absolute(state: &State) -> Result<(), String> {
    for (component_id, component_state) in &state.components {
        for resource in &component_state.resources {
            if let ResourceKind::Fs { fs } = &resource.kind {
                if !Path::new(&fs.path).is_absolute() {
                    return Err(format!(
                        "non-absolute fs path '{}' in component '{}'",
                        fs.path, component_id
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Assert that every recorded `tool` resource whose `resolved_path` is set has
/// an absolute path.
pub fn assert_all_tool_paths_absolute(state: &State) -> Result<(), String> {
    for (component_id, component_state) in &state.components {
        for resource in &component_state.resources {
            if let ResourceKind::Tool { tool } = &resource.kind {
                if let Some(p) = &tool.observed.resolved_path {
                    if !Path::new(p).is_absolute() {
                        return Err(format!(
                            "non-absolute tool resolved_path '{}' in component '{}'",
                            p, component_id
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Run all standard structural assertions in one call.
///
/// Combines [`assert_state_version`], [`assert_components_present`],
/// [`assert_no_duplicate_resource_ids`], [`assert_no_duplicate_fs_paths`],
/// [`assert_all_fs_paths_absolute`], and [`assert_all_tool_paths_absolute`].
pub fn assert_state_valid(state: &State) -> Result<(), String> {
    assert_state_version(state)?;
    assert_components_present(state)?;
    assert_no_duplicate_resource_ids(state)?;
    assert_no_duplicate_fs_paths(state)?;
    assert_all_fs_paths_absolute(state)?;
    assert_all_tool_paths_absolute(state)?;
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// Component-level assertions
// ──────────────────────────────────────────────────────────────────────────────

/// Assert that `component_id` is present in the state.
pub fn assert_component_present(state: &State, component_id: &str) -> Result<(), String> {
    if !state.components.contains_key(component_id) {
        return Err(format!(
            "component '{}' is missing from state",
            component_id
        ));
    }
    Ok(())
}

/// Assert that `component_id` is absent from the state.
pub fn assert_component_absent(state: &State, component_id: &str) -> Result<(), String> {
    if state.components.contains_key(component_id) {
        return Err(format!(
            "component '{}' should be absent but is still in state",
            component_id
        ));
    }
    Ok(())
}

/// Assert that the components map contains exactly zero entries.
pub fn assert_components_empty(state: &State) -> Result<(), String> {
    if !state.components.is_empty() {
        let keys: Vec<&str> = state.components.keys().map(String::as_str).collect();
        return Err(format!(
            "expected components to be empty but found: {:?}",
            keys
        ));
    }
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// Resource-level assertions
// ──────────────────────────────────────────────────────────────────────────────

/// Return the recorded runtime version for `component_id`, if any.
///
/// Fails if the component is missing or if no runtime resource is recorded.
pub fn get_runtime_version<'a>(state: &'a State, component_id: &str) -> Result<&'a str, String> {
    let component = state
        .components
        .get(component_id)
        .ok_or_else(|| format!("component '{}' not found in state", component_id))?;

    component
        .resources
        .iter()
        .find_map(|r| {
            if let ResourceKind::Runtime { runtime, .. } = &r.kind {
                Some(runtime.version.as_str())
            } else {
                None
            }
        })
        .ok_or_else(|| {
            format!(
                "no runtime resource recorded for component '{}'",
                component_id
            )
        })
}

/// Assert that `component_id` has no runtime resource recorded.
pub fn assert_no_runtime(state: &State, component_id: &str) -> Result<(), String> {
    let component = state
        .components
        .get(component_id)
        .ok_or_else(|| format!("component '{}' not found in state", component_id))?;

    let has_runtime = component
        .resources
        .iter()
        .any(|r| matches!(&r.kind, ResourceKind::Runtime { .. }));

    if has_runtime {
        return Err(format!(
            "component '{}' should have no runtime recorded but does",
            component_id
        ));
    }
    Ok(())
}

/// Assert that no package resources remain in state (all components combined).
pub fn assert_no_packages_in_state(state: &State) -> Result<(), String> {
    for (component_id, component_state) in &state.components {
        for resource in &component_state.resources {
            if let ResourceKind::Package { package, .. } = &resource.kind {
                return Err(format!(
                    "package '{}' still in state under component '{}'",
                    package.name, component_id
                ));
            }
        }
    }
    Ok(())
}

/// Assert that a tool resource with `tool_name` is present in `component_id`.
pub fn assert_tool_resource_present(
    state: &State,
    component_id: &str,
    tool_name: &str,
) -> Result<(), String> {
    let component = state
        .components
        .get(component_id)
        .ok_or_else(|| format!("component '{}' not found in state", component_id))?;

    let found = component
        .resources
        .iter()
        .any(|r| matches!(&r.kind, ResourceKind::Tool { tool } if tool.name == tool_name));

    if !found {
        return Err(format!(
            "tool '{}' not found in component '{}'",
            tool_name, component_id
        ));
    }
    Ok(())
}

/// Assert that no tool resource with `tool_name` is present in `component_id`.
#[allow(dead_code)]
pub fn assert_tool_resource_absent(
    state: &State,
    component_id: &str,
    tool_name: &str,
) -> Result<(), String> {
    if let Some(component) = state.components.get(component_id) {
        let found = component
            .resources
            .iter()
            .any(|r| matches!(&r.kind, ResourceKind::Tool { tool } if tool.name == tool_name));
        if found {
            return Err(format!(
                "tool '{}' should be absent from component '{}' but is still recorded",
                tool_name, component_id
            ));
        }
    }
    Ok(())
}

/// Return the `resolved_path` of the first tool resource in `component_id`.
///
/// Returns `Ok(None)` if the tool has no `resolved_path` recorded yet.
#[allow(dead_code)]
pub fn get_tool_resolved_path<'a>(
    state: &'a State,
    component_id: &str,
) -> Result<Option<&'a str>, String> {
    let component = state
        .components
        .get(component_id)
        .ok_or_else(|| format!("component '{}' not found in state", component_id))?;

    let tool = component
        .resources
        .iter()
        .find_map(|r| {
            if let ResourceKind::Tool { tool } = &r.kind {
                Some(tool)
            } else {
                None
            }
        })
        .ok_or_else(|| format!("no tool resource recorded for component '{}'", component_id))?;

    Ok(tool.observed.resolved_path.as_deref())
}

// ──────────────────────────────────────────────────────────────────────────────
// Snapshot / diff helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Assert that two raw JSON values are identical, producing a diff-style error
/// message if they differ.
pub fn assert_state_unchanged(
    before: &serde_json::Value,
    after: &serde_json::Value,
    label: &str,
) -> Result<(), String> {
    if before == after {
        return Ok(());
    }
    let before_str = serde_json::to_string_pretty(before).unwrap_or_default();
    let after_str = serde_json::to_string_pretty(after).unwrap_or_default();
    Err(format!(
        "{}: state changed between runs.\n--- before ---\n{}\n--- after ---\n{}",
        label, before_str, after_str
    ))
}

// ──────────────────────────────────────────────────────────────────────────────
// Filesystem safety assertions (uninstall safety)
// ──────────────────────────────────────────────────────────────────────────────

/// Assert that none of the given paths exist on disk.
///
/// Used after an uninstall to confirm that tracked files were removed.
pub fn assert_paths_removed(paths: &[String]) -> Result<(), String> {
    for path in paths {
        if Path::new(path).exists() {
            return Err(format!(
                "tracked path '{}' still exists after uninstall",
                path
            ));
        }
    }
    Ok(())
}

/// Assert that a path still exists on disk.
///
/// Used to confirm that untracked files were preserved during uninstall.
pub fn assert_path_exists(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!(
            "expected path '{}' to exist but it was removed (filesystem scan violation)",
            path.display()
        ));
    }
    Ok(())
}

/// Collect all `fs` paths recorded in state.
pub fn collect_fs_paths(state: &State) -> Vec<String> {
    state
        .components
        .values()
        .flat_map(|fs| &fs.resources)
        .filter_map(|r| {
            if let ResourceKind::Fs { fs } = &r.kind {
                Some(fs.path.clone())
            } else {
                None
            }
        })
        .collect()
}
