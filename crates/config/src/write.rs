// Configuration write operations.
//
// Provides typed mutations (add/remove feature) and raw YAML path-based operations
// for config files. All writes are atomic (write to .tmp, then rename).
//
// Note: round-trip operations do NOT preserve YAML comments. This is a known
// limitation; the "raw" commands are low-level escape hatches, not primary APIs.

use std::path::Path;

use serde_yaml::Value;

use crate::ConfigError;

// ---------------------------------------------------------------------------
// Config template
// ---------------------------------------------------------------------------

/// Template written by `config init`.
const CONFIG_TEMPLATE: &str = "\
# loadout config
#
# profile: list of features to enable, grouped by source
# strategy: backend selection overrides (optional)
profile:
  features:
    # Add features using the grouped syntax:
    #   <source_id>:
    #     <feature_name>: {}
    #
    # Example:
    #   local:
    #     git: {}
    #   core:
    #     node: {}
strategy:
  # Override the default backend per resource kind (optional):
  # package:
  #   backend: local/mise
  # runtime:
  #   backend: local/mise
";

// ---------------------------------------------------------------------------
// Init
// ---------------------------------------------------------------------------

/// Create a new config file from the built-in template.
///
/// Fails with `ConfigError::AlreadyExists` if the file already exists.
/// Parent directories are created as needed.
pub fn create_config(path: &Path) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        io::make_dirs(parent)?;
    }
    if path.exists() {
        return Err(ConfigError::AlreadyExists {
            path: path.to_path_buf(),
        });
    }
    // Use `create_new` so concurrent creation does not silently overwrite.
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| {
            ConfigError::Io(io::IoError::Write {
                path: path.to_path_buf(),
                source: e,
            })
        })?;
    file.write_all(CONFIG_TEMPLATE.as_bytes()).map_err(|e| {
        ConfigError::Io(io::IoError::Write {
            path: path.to_path_buf(),
            source: e,
        })
    })?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Typed mutations
// ---------------------------------------------------------------------------

/// Add a feature to `profile.features.<source>.<name>` in a config file.
///
/// If the file does not exist it is created. If intermediate YAML nodes are
/// absent they are created as empty mappings. If the feature is already present
/// the value is overwritten with `{}` (idempotent for the grouped syntax).
pub fn add_feature(path: &Path, source: &str, name: &str) -> Result<(), ConfigError> {
    validate_non_empty("source", source)?;
    validate_non_empty("name", name)?;

    let mut doc = load_or_empty(path)?;

    insert_at_path(
        &mut doc,
        &["profile", "features", source, name],
        Value::Mapping(serde_yaml::Mapping::new()),
    )?;

    io::write_yaml_atomic(path, &doc)?;
    Ok(())
}

/// Remove a feature from `profile.features.<source>.<name>` in a config file.
///
/// Returns `true` if the feature was found and removed, `false` if it was not present.
pub fn remove_feature(path: &Path, source: &str, name: &str) -> Result<bool, ConfigError> {
    validate_non_empty("source", source)?;
    validate_non_empty("name", name)?;

    if !path.exists() {
        return Ok(false);
    }

    let mut doc = load_or_empty(path)?;
    let found = remove_at_path(&mut doc, &["profile", "features", source, name]);

    if found {
        io::write_yaml_atomic(path, &doc)?;
    }
    Ok(found)
}

// ---------------------------------------------------------------------------
// Raw YAML access (escape hatch)
// ---------------------------------------------------------------------------

/// Return the raw YAML content of a config file as a string.
pub fn raw_show(path: &Path) -> Result<String, ConfigError> {
    std::fs::read_to_string(path).map_err(|e| {
        ConfigError::Io(io::IoError::Read {
            path: path.to_path_buf(),
            source: e,
        })
    })
}

/// Set the value at a dot-separated YAML path.
///
/// `key_path` uses `.` as the separator. Feature IDs containing `/` are treated
/// as a single key segment (e.g. `profile.features.local/git` is unsupported —
/// use `profile.features.local.git` with the grouped syntax instead).
///
/// `raw_value` is parsed as YAML, so `{}` sets an empty mapping, `true` sets a
/// boolean, and a quoted string sets a string value.
///
/// Missing intermediate nodes are created as empty mappings.
pub fn raw_set(path: &Path, key_path: &str, raw_value: &str) -> Result<(), ConfigError> {
    if key_path.is_empty() {
        return Err(ConfigError::InvalidProfile {
            reason: "key path must not be empty".into(),
        });
    }

    let mut doc = load_or_empty(path)?;

    let segments: Vec<&str> = key_path.split('.').collect();
    let new_val: Value =
        serde_yaml::from_str(raw_value).map_err(|_| ConfigError::InvalidProfile {
            reason: format!("cannot parse '{raw_value}' as YAML"),
        })?;

    insert_at_path(&mut doc, &segments, new_val)?;
    io::write_yaml_atomic(path, &doc)?;
    Ok(())
}

/// Remove the value at a dot-separated YAML path.
///
/// Returns `true` if the key was found and removed, `false` if not present.
pub fn raw_unset(path: &Path, key_path: &str) -> Result<bool, ConfigError> {
    if key_path.is_empty() {
        return Err(ConfigError::InvalidProfile {
            reason: "key path must not be empty".into(),
        });
    }
    if !path.exists() {
        return Ok(false);
    }

    let mut doc = load_or_empty(path)?;
    let segments: Vec<&str> = key_path.split('.').collect();
    let found = remove_at_path(&mut doc, &segments);

    if found {
        io::write_yaml_atomic(path, &doc)?;
    }
    Ok(found)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn validate_non_empty(field: &str, value: &str) -> Result<(), ConfigError> {
    if value.is_empty() {
        return Err(ConfigError::InvalidProfile {
            reason: format!("{field} must not be empty"),
        });
    }
    Ok(())
}

/// Load file as `serde_yaml::Value`, or return an empty mapping if absent / empty.
///
/// Null-valued keys at any mapping level are stripped before returning.
/// This prevents keys that exist only as YAML comments in the template
/// (e.g. `strategy:` with no real children) from round-tripping as `null`.
fn load_or_empty(path: &Path) -> Result<Value, ConfigError> {
    if !path.exists() {
        return Ok(Value::Mapping(serde_yaml::Mapping::new()));
    }
    let content = std::fs::read_to_string(path).map_err(|e| {
        ConfigError::Io(io::IoError::Read {
            path: path.to_path_buf(),
            source: e,
        })
    })?;
    if content.trim().is_empty() {
        return Ok(Value::Mapping(serde_yaml::Mapping::new()));
    }
    let mut doc: Value = serde_yaml::from_str(&content).map_err(|e| {
        ConfigError::Io(io::IoError::ParseYaml {
            path: path.to_path_buf(),
            source: e,
        })
    })?;
    strip_null_values(&mut doc);
    Ok(doc)
}

/// Recursively remove keys whose values are `null` from all mappings.
fn strip_null_values(value: &mut Value) {
    if let Value::Mapping(mapping) = value {
        mapping.retain(|_, v| !v.is_null());
        for v in mapping.values_mut() {
            strip_null_values(v);
        }
    }
}

/// Recursively set `segments` path inside `node`, creating intermediate mappings.
fn insert_at_path(node: &mut Value, segments: &[&str], value: Value) -> Result<(), ConfigError> {
    let mapping = node
        .as_mapping_mut()
        .ok_or_else(|| ConfigError::InvalidProfile {
            reason: "expected a YAML mapping at this path".into(),
        })?;

    let key = Value::String(segments[0].to_string());

    if segments.len() == 1 {
        mapping.insert(key, value);
        return Ok(());
    }

    // Ensure intermediate node is a mapping.
    // Treat null as an empty mapping — this occurs when a key exists but has
    // no value (e.g., only YAML comments appear below it).
    if !mapping.contains_key(&key) || mapping[&key].is_null() {
        mapping.insert(key.clone(), Value::Mapping(serde_yaml::Mapping::new()));
    } else if !mapping[&key].is_mapping() {
        return Err(ConfigError::InvalidProfile {
            reason: format!("'{}' is not a mapping", segments[0]),
        });
    }

    let child = mapping.get_mut(&key).unwrap();
    insert_at_path(child, &segments[1..], value)
}

/// Recursively remove `segments` path from `node`.
/// Returns `true` if the key was found and removed.
fn remove_at_path(node: &mut Value, segments: &[&str]) -> bool {
    let mapping = match node.as_mapping_mut() {
        Some(m) => m,
        None => return false,
    };
    let key = Value::String(segments[0].to_string());

    if segments.len() == 1 {
        return mapping.remove(&key).is_some();
    }

    match mapping.get_mut(&key) {
        Some(child) => remove_at_path(child, &segments[1..]),
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Source ID rewriting — `feature import` / `backend import`
// ---------------------------------------------------------------------------

/// Rewrite all occurrences of `old_source/<name>` to `new_source/<name>` in the
/// `profile.features` and `bundles.*.features` sections of a config file.
///
/// Returns `true` if any changes were made.
/// Returns `false` (without error) if the file does not exist or the key is absent.
pub fn rewrite_feature_source(
    path: &Path,
    old_source: &str,
    name: &str,
    new_source: &str,
) -> Result<bool, ConfigError> {
    if !path.exists() {
        return Ok(false);
    }
    let mut doc = load_or_empty(path)?;

    // Collect bundle names first to avoid borrow conflicts later.
    let bundle_names: Vec<String> = collect_bundle_names(&doc);

    let mut changed = false;

    // Rewrite profile.features.
    {
        if let Some(features) = navigate_to_mapping_mut(&mut doc, &["profile", "features"]) {
            if move_feature_in_mapping(features, old_source, name, new_source) {
                changed = true;
            }
        }
    }

    // Rewrite bundles.*.features.
    for bundle_name in &bundle_names {
        if let Some(features) =
            navigate_to_mapping_mut(&mut doc, &["bundles", bundle_name.as_str(), "features"])
        {
            if move_feature_in_mapping(features, old_source, name, new_source) {
                changed = true;
            }
        }
    }

    if changed {
        io::write_yaml_atomic(path, &doc)?;
    }
    Ok(changed)
}

/// Rewrite all occurrences of `old_source/<name>` to `new_source/<name>` in the
/// `strategy` section of a config file (`default_backend` and `overrides.*.backend`).
///
/// Returns `true` if any changes were made.
/// Returns `false` (without error) if the file does not exist or the strategy section is absent.
pub fn rewrite_backend_source(
    path: &Path,
    old_source: &str,
    name: &str,
    new_source: &str,
) -> Result<bool, ConfigError> {
    if !path.exists() {
        return Ok(false);
    }
    let mut doc = load_or_empty(path)?;
    let old_id = format!("{old_source}/{name}");
    let new_id = format!("{new_source}/{name}");
    let changed = rewrite_strategy_ids(&mut doc, &old_id, &new_id);
    if changed {
        io::write_yaml_atomic(path, &doc)?;
    }
    Ok(changed)
}

// -- helpers for source-rewrite functions ------------------------------------

/// Collect all keys in the top-level `bundles:` mapping without holding a borrow.
fn collect_bundle_names(doc: &Value) -> Vec<String> {
    let bundles_key = Value::String("bundles".into());
    doc.as_mapping()
        .and_then(|m| m.get(&bundles_key))
        .and_then(|v| v.as_mapping())
        .map(|m| {
            m.keys()
                .filter_map(|k| k.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Navigate through a chain of mapping keys, returning the final node as `&mut Mapping`.
fn navigate_to_mapping_mut<'a>(
    root: &'a mut Value,
    path: &[&str],
) -> Option<&'a mut serde_yaml::Mapping> {
    let mut current = root;
    for key in path {
        let k = Value::String((*key).to_string());
        current = current.as_mapping_mut()?.get_mut(&k)?;
    }
    current.as_mapping_mut()
}

/// Move `features[old_source][name]` to `features[new_source][name]`.
///
/// Removes the `old_source` key if it becomes empty after the move.
/// Returns `true` if the move was performed.
fn move_feature_in_mapping(
    features: &mut serde_yaml::Mapping,
    old_source: &str,
    name: &str,
    new_source: &str,
) -> bool {
    let old_src_key = Value::String(old_source.to_string());
    let name_key = Value::String(name.to_string());

    // Extract the feature value from the old source entry.
    let feature_val = {
        let old_src = match features
            .get_mut(&old_src_key)
            .and_then(|v| v.as_mapping_mut())
        {
            Some(m) => m,
            None => return false,
        };
        match old_src.remove(&name_key) {
            Some(v) => v,
            None => return false,
        }
    };

    // Remove the old_source key if it is now empty.
    if features
        .get(&old_src_key)
        .and_then(|v| v.as_mapping())
        .map(|m| m.is_empty())
        .unwrap_or(false)
    {
        features.remove(&old_src_key);
    }

    // Insert into new_source, creating the entry if absent.
    let new_src_key = Value::String(new_source.to_string());
    if !features.contains_key(&new_src_key) {
        features.insert(
            new_src_key.clone(),
            Value::Mapping(serde_yaml::Mapping::new()),
        );
    }
    if let Some(new_src) = features
        .get_mut(&new_src_key)
        .and_then(|v| v.as_mapping_mut())
    {
        new_src.insert(name_key, feature_val);
    }
    true
}

/// Rewrite `default_backend` / `overrides.*.backend` values matching `old_id`
/// to `new_id` in the `strategy:` section of a YAML document.
fn rewrite_strategy_ids(doc: &mut Value, old_id: &str, new_id: &str) -> bool {
    let strategy_key = Value::String("strategy".into());
    let mut changed = false;

    let doc_map = match doc.as_mapping_mut() {
        Some(m) => m,
        None => return false,
    };
    let strategy_val = match doc_map.get_mut(&strategy_key) {
        Some(v) => v,
        None => return false,
    };
    let strategy = match strategy_val.as_mapping_mut() {
        Some(m) => m,
        None => return false,
    };

    for (_, kind_val) in strategy.iter_mut() {
        let Some(kind_map) = kind_val.as_mapping_mut() else {
            continue;
        };

        // Rewrite default_backend.
        let db_key = Value::String("default_backend".into());
        if let Some(db) = kind_map.get_mut(&db_key) {
            if db.as_str() == Some(old_id) {
                *db = Value::String(new_id.to_string());
                changed = true;
            }
        }

        // Rewrite overrides.*.backend.
        let overrides_key = Value::String("overrides".into());
        if let Some(overrides_val) = kind_map.get_mut(&overrides_key) {
            if let Some(overrides) = overrides_val.as_mapping_mut() {
                for (_, override_val) in overrides.iter_mut() {
                    if let Some(override_map) = override_val.as_mapping_mut() {
                        let backend_key = Value::String("backend".into());
                        if let Some(backend) = override_map.get_mut(&backend_key) {
                            if backend.as_str() == Some(old_id) {
                                *backend = Value::String(new_id.to_string());
                                changed = true;
                            }
                        }
                    }
                }
            }
        }
    }
    changed
}
