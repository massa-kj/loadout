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
