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

/// Assert that the `features` map is present (non-absent; may be empty).
pub fn assert_features_present(state: &State) -> Result<(), String> {
    // `features` is always present after deserialisation (defaults to `{}`),
    // so this is a structural sanity-check rather than a content check.
    let _ = &state.features;
    Ok(())
}

/// Assert that every feature's resource list contains no duplicate `id` values.
pub fn assert_no_duplicate_resource_ids(state: &State) -> Result<(), String> {
    for (feature_id, feature_state) in &state.features {
        let mut seen: HashSet<&str> = HashSet::new();
        for resource in &feature_state.resources {
            if !seen.insert(resource.id.as_str()) {
                return Err(format!(
                    "duplicate resource id '{}' in feature '{}'",
                    resource.id, feature_id
                ));
            }
        }
    }
    Ok(())
}

/// Assert that no `fs` resource path appears more than once across all features.
pub fn assert_no_duplicate_fs_paths(state: &State) -> Result<(), String> {
    let mut seen: HashMap<&str, &str> = HashMap::new();
    for (feature_id, feature_state) in &state.features {
        for resource in &feature_state.resources {
            if let ResourceKind::Fs { fs } = &resource.kind {
                if let Some(prev_feature) = seen.insert(fs.path.as_str(), feature_id.as_str()) {
                    return Err(format!(
                        "fs path '{}' is tracked by both '{}' and '{}'",
                        fs.path, prev_feature, feature_id
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Assert that every `fs` resource path is absolute.
pub fn assert_all_fs_paths_absolute(state: &State) -> Result<(), String> {
    for (feature_id, feature_state) in &state.features {
        for resource in &feature_state.resources {
            if let ResourceKind::Fs { fs } = &resource.kind {
                if !Path::new(&fs.path).is_absolute() {
                    return Err(format!(
                        "non-absolute fs path '{}' in feature '{}'",
                        fs.path, feature_id
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Run all standard structural assertions in one call.
///
/// Combines [`assert_state_version`], [`assert_features_present`],
/// [`assert_no_duplicate_resource_ids`], [`assert_no_duplicate_fs_paths`], and
/// [`assert_all_fs_paths_absolute`].
pub fn assert_state_valid(state: &State) -> Result<(), String> {
    assert_state_version(state)?;
    assert_features_present(state)?;
    assert_no_duplicate_resource_ids(state)?;
    assert_no_duplicate_fs_paths(state)?;
    assert_all_fs_paths_absolute(state)?;
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// Feature-level assertions
// ──────────────────────────────────────────────────────────────────────────────

/// Assert that `feature_id` is present in the state.
pub fn assert_feature_present(state: &State, feature_id: &str) -> Result<(), String> {
    if !state.features.contains_key(feature_id) {
        return Err(format!("feature '{}' is missing from state", feature_id));
    }
    Ok(())
}

/// Assert that `feature_id` is absent from the state.
pub fn assert_feature_absent(state: &State, feature_id: &str) -> Result<(), String> {
    if state.features.contains_key(feature_id) {
        return Err(format!(
            "feature '{}' should be absent but is still in state",
            feature_id
        ));
    }
    Ok(())
}

/// Assert that the features map contains exactly zero entries.
pub fn assert_features_empty(state: &State) -> Result<(), String> {
    if !state.features.is_empty() {
        let keys: Vec<&str> = state.features.keys().map(String::as_str).collect();
        return Err(format!(
            "expected features to be empty but found: {:?}",
            keys
        ));
    }
    Ok(())
}


// ──────────────────────────────────────────────────────────────────────────────
// Resource-level assertions
// ──────────────────────────────────────────────────────────────────────────────

/// Return the recorded runtime version for `feature_id`, if any.
///
/// Fails if the feature is missing or if no runtime resource is recorded.
pub fn get_runtime_version<'a>(state: &'a State, feature_id: &str) -> Result<&'a str, String> {
    let feature = state
        .features
        .get(feature_id)
        .ok_or_else(|| format!("feature '{}' not found in state", feature_id))?;

    feature
        .resources
        .iter()
        .find_map(|r| {
            if let ResourceKind::Runtime { runtime, .. } = &r.kind {
                Some(runtime.version.as_str())
            } else {
                None
            }
        })
        .ok_or_else(|| format!("no runtime resource recorded for feature '{}'", feature_id))
}

/// Assert that `feature_id` has no runtime resource recorded.
pub fn assert_no_runtime(state: &State, feature_id: &str) -> Result<(), String> {
    let feature = state
        .features
        .get(feature_id)
        .ok_or_else(|| format!("feature '{}' not found in state", feature_id))?;

    let has_runtime = feature
        .resources
        .iter()
        .any(|r| matches!(&r.kind, ResourceKind::Runtime { .. }));

    if has_runtime {
        return Err(format!(
            "feature '{}' should have no runtime recorded but does",
            feature_id
        ));
    }
    Ok(())
}

/// Assert that no package resources remain in state (all features combined).
pub fn assert_no_packages_in_state(state: &State) -> Result<(), String> {
    for (feature_id, feature_state) in &state.features {
        for resource in &feature_state.resources {
            if let ResourceKind::Package { package, .. } = &resource.kind {
                return Err(format!(
                    "package '{}' still in state under feature '{}'",
                    package.name, feature_id
                ));
            }
        }
    }
    Ok(())
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
        .features
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
