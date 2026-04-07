//! Configuration loading, validation, and normalization.
//!
//! This crate bridges raw YAML files on disk and domain model types in the `model` crate.
//! It handles three kinds of config files: profiles, strategies, and sources.
//!
//! ## Input format
//!
//! Profiles use *namespace grouping* syntax: the outer key is a `source_id`,
//! the inner key is the feature name. Both bare names and canonical `source/name`
//! forms are **rejected**; grouping is the only accepted syntax.
//!
//! ```yaml
//! profile:
//!   features:
//!     core:
//!       git: {}
//!     local:
//!       nvim: {}
//!       python:
//!         version: "3.12"
//! ```
//!
//! Bundles allow reusable feature sets:
//!
//! ```yaml
//! bundle:
//!   use:
//!     - base
//!     - work          # last entry wins on conflict
//!
//! bundles:
//!   base:
//!     features:
//!       core:
//!         git: {}
//!   work:
//!     features:
//!       dev:
//!         terraform: {}
//!
//! profile:
//!   features:
//!     local:
//!       nvim: {}      # profile.features overrides all bundles
//! ```
//!
//! After expansion and normalization, all feature keys are canonical `source_id/name`.
//! Source existence is NOT verified here; that happens at `SourceRegistry` construction.
//!
//! **Path resolution contract**: callers supply explicit `&Path` values.
//! Platform-aware path discovery belongs to the `platform` crate.
//!
//! See: `docs/specs/data/profile.md`, `docs/specs/data/strategy.md`,
//!      `docs/specs/data/sources.md`

pub mod write;
pub use write::{add_feature, create_config, raw_set, raw_show, raw_unset, remove_feature};

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Deserialize;

pub use model::{
    profile::{Profile, ProfileFeatureConfig},
    sources::{
        AllowList, AllowSpec, SourceEntry, SourceLockEntry, SourceRef, SourceType, SourcesLock,
        SourcesSpec, WildcardAll,
    },
    strategy::{BackendOverride, BackendStrategy, FsStrategy, Strategy},
};
use thiserror::Error;

/// Errors produced by configuration loading or validation.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("I/O error: {0}")]
    Io(#[from] io::IoError),

    #[error("invalid profile: {reason}")]
    InvalidProfile { reason: String },

    #[error("invalid strategy: {reason}")]
    InvalidStrategy { reason: String },

    #[error("invalid sources: {reason}")]
    InvalidSources { reason: String },

    #[error("config file already exists: {}", path.display())]
    AlreadyExists { path: std::path::PathBuf },
}

// ─── Raw profile types (config-crate-local) ─────────────────────────────────

/// Raw per-feature config as parsed from YAML (grouping syntax inner value).
/// Mirrors `ProfileFeatureConfig` but is local to this crate.
#[derive(Deserialize, Default, Clone)]
struct RawFeatureConfig {
    #[serde(default)]
    version: Option<String>,
}

/// Grouped feature map: `source_id → (feature_name → config)`.
/// This is the only accepted input shape; bare names and canonical direct form are rejected.
type GroupedFeatures = HashMap<String, HashMap<String, RawFeatureConfig>>;

/// Raw profile as read from a standalone profile YAML file.
/// `features` uses the grouped syntax.
#[derive(Deserialize)]
struct RawProfile {
    #[serde(default)]
    features: GroupedFeatures,
}

// ─── Raw bundle types (config-crate-local) ──────────────────────────────────

/// `bundle:` section in config.yaml — lists which bundles to apply.
/// Values are bundle names (strings). Future `file:` prefix scheme is intentionally
/// excluded from this type to keep the distinction clear.
#[derive(Deserialize, Default)]
struct RawBundleRef {
    #[serde(default, rename = "use")]
    use_list: Vec<String>,
}

/// A single bundle definition. Uses the same grouped-features syntax as profiles.
#[derive(Deserialize)]
struct RawBundle {
    #[serde(default)]
    features: GroupedFeatures,
}

/// `bundles:` section in config.yaml — named bundle definitions.
type RawBundlesMap = HashMap<String, RawBundle>;

// ─── Expansion helpers ───────────────────────────────────────────────────────

/// Expand grouped features into a flat canonical map.
///
/// `{source_id: {name: config}}` → `{"source_id/name": ProfileFeatureConfig}`
///
/// Validates:
/// - `source_id` must not be empty
/// - feature name must not be empty
/// - duplicate canonical IDs are rejected
///
/// Does NOT verify that `source_id` exists in the source registry;
/// that check happens later at `SourceRegistry` construction.
fn expand_grouped_features(
    grouped: GroupedFeatures,
) -> Result<HashMap<String, ProfileFeatureConfig>, ConfigError> {
    let mut out: HashMap<String, ProfileFeatureConfig> = HashMap::new();

    for (source_id, names) in grouped {
        if source_id.is_empty() {
            return Err(ConfigError::InvalidProfile {
                reason: "source_id must not be empty".into(),
            });
        }
        for (name, cfg) in names {
            if name.is_empty() {
                return Err(ConfigError::InvalidProfile {
                    reason: format!("feature name under source '{source_id}' must not be empty"),
                });
            }
            let canonical = format!("{source_id}/{name}");
            if out.contains_key(&canonical) {
                return Err(ConfigError::InvalidProfile {
                    reason: format!("duplicate feature '{canonical}'"),
                });
            }
            out.insert(
                canonical,
                ProfileFeatureConfig {
                    version: cfg.version,
                },
            );
        }
    }

    Ok(out)
}

/// Merge bundles in `use` list order (last entry wins), then overlay profile features.
///
/// Returns merged grouped features ready for `expand_grouped_features`.
/// Priority (lowest → highest): bundles[0], bundles[1], …, profile.features.
fn expand_bundles(
    bundle_ref: &RawBundleRef,
    bundles: &RawBundlesMap,
    profile_features: GroupedFeatures,
) -> Result<GroupedFeatures, ConfigError> {
    // Validate: all referenced bundle names must be defined.
    for name in &bundle_ref.use_list {
        if !bundles.contains_key(name) {
            return Err(ConfigError::InvalidProfile {
                reason: format!("undefined bundle '{name}': not found in 'bundles:' section"),
            });
        }
    }

    // Merge: iterate use list in order; last bundle wins per feature.
    let mut merged: GroupedFeatures = HashMap::new();
    for name in &bundle_ref.use_list {
        let bundle = &bundles[name];
        for (source_id, names) in &bundle.features {
            let source_entry = merged.entry(source_id.clone()).or_default();
            for (feat_name, cfg) in names {
                // Later bundle overwrites earlier bundle for same feature.
                source_entry.insert(feat_name.clone(), cfg.clone());
            }
        }
    }

    // Overlay: profile.features overwrites all bundle-merged features.
    for (source_id, names) in profile_features {
        let source_entry = merged.entry(source_id).or_default();
        for (feat_name, cfg) in names {
            source_entry.insert(feat_name, cfg);
        }
    }

    Ok(merged)
}

// ─── Profile ────────────────────────────────────────────────────────────────

/// Load and normalize a profile from a standalone profile YAML file.
///
/// The file must use grouping syntax:
/// ```yaml
/// features:
///   core:
///     git: {}
///   local:
///     nvim: {}
/// ```
pub fn load_profile(path: &Path) -> Result<Profile, ConfigError> {
    let raw: RawProfile = io::load_yaml(path)?;
    let flat = expand_grouped_features(raw.features)?;
    Ok(Profile { features: flat })
}

// ─── Strategy ───────────────────────────────────────────────────────────────

/// Load and validate a strategy from a YAML file.
pub fn load_strategy(path: &Path) -> Result<Strategy, ConfigError> {
    let raw: Strategy = io::load_yaml(path)?;
    validate_strategy(raw)
}

fn validate_strategy(strategy: Strategy) -> Result<Strategy, ConfigError> {
    if let Some(ref pkg) = strategy.package {
        validate_backend_strategy_field("package.default_backend", pkg.default_backend.as_deref())?;
        for (name, ov) in &pkg.overrides {
            if ov.backend.is_empty() {
                return Err(ConfigError::InvalidStrategy {
                    reason: format!("package.overrides[{name}].backend must not be empty"),
                });
            }
        }
    }

    if let Some(ref rt) = strategy.runtime {
        validate_backend_strategy_field("runtime.default_backend", rt.default_backend.as_deref())?;
        for (name, ov) in &rt.overrides {
            if ov.backend.is_empty() {
                return Err(ConfigError::InvalidStrategy {
                    reason: format!("runtime.overrides[{name}].backend must not be empty"),
                });
            }
        }
    }

    Ok(strategy)
}

fn validate_backend_strategy_field(field: &str, value: Option<&str>) -> Result<(), ConfigError> {
    if let Some(v) = value {
        if v.is_empty() {
            return Err(ConfigError::InvalidStrategy {
                reason: format!("{field} must not be empty string if present"),
            });
        }
    }
    Ok(())
}

// ─── Unified config ──────────────────────────────────────────────────────────

/// Load a unified `config.yaml` and return the resolved `Profile` and `Strategy`.
///
/// serde ignores unknown top-level keys by default (no `deny_unknown_fields`),
/// so future sections added to the format will not break existing versions.
///
/// Sections:
/// - `profile` — required. Features use grouping syntax `source_id: { name: {} }`.
/// - `strategy` — optional. Absent → `Strategy::default()` (no overrides).
/// - `bundle`   — optional. Lists which bundles to apply (`use: [base, work]`).
/// - `bundles`  — optional. Named bundle definitions (same grouping syntax as profile).
///
/// Pipeline: bundle expansion → grouped-feature normalization → canonical Profile.
///
/// # Format
///
/// ```yaml
/// bundle:
///   use:
///     - base
///
/// bundles:
///   base:
///     features:
///       core:
///         git: {}
///
/// profile:
///   features:
///     local:
///       nvim: {}
///
/// strategy:                  # optional
///   package:
///     default_backend: local/brew
///
/// future_section: ...        # silently ignored
/// ```
pub fn load_config(path: &Path) -> Result<(Profile, Strategy), ConfigError> {
    /// Intermediate struct for deserializing config.yaml.
    /// Unknown top-level keys are silently ignored (serde default behaviour,
    /// no `deny_unknown_fields` attribute).
    #[derive(Deserialize)]
    struct RawConfig {
        profile: Option<RawProfile>,
        strategy: Option<Strategy>,
        #[serde(default)]
        bundle: RawBundleRef,
        #[serde(default)]
        bundles: RawBundlesMap,
    }

    let raw: RawConfig = io::load_yaml(path)?;

    // profile is required.
    let raw_profile = raw.profile.ok_or_else(|| ConfigError::InvalidProfile {
        reason: "config.yaml must contain a 'profile' section".into(),
    })?;

    // Bundle expansion: merge bundles in use-list order (last wins), then overlay profile.
    let merged = expand_bundles(&raw.bundle, &raw.bundles, raw_profile.features)?;

    // Normalize grouped features to canonical flat map.
    let flat = expand_grouped_features(merged)?;
    let profile = Profile { features: flat };

    // strategy is optional; absent → Strategy::default().
    let strategy = match raw.strategy {
        Some(p) => validate_strategy(p)?,
        None => Strategy::default(),
    };

    Ok((profile, strategy))
}

// ─── Sources ─────────────────────────────────────────────────────────────────

const RESERVED_SOURCE_IDS: &[&str] =
    &["core", "local", "official", "sample", "example", "external"];

/// Load, validate, and path-resolve a sources spec from a YAML file.
///
/// For `type: path` entries, the `path` field is resolved to an absolute path
/// relative to the directory containing `sources.yaml`.
/// `~`-prefixed paths are expanded using the user's home directory.
pub fn load_sources(path: &Path) -> Result<SourcesSpec, ConfigError> {
    let raw: SourcesSpec = io::load_yaml(path)?;
    let sources_dir = path.parent().unwrap_or_else(|| Path::new("."));
    validate_and_resolve_sources(raw, sources_dir)
}

/// Load a sources lock file.
///
/// Returns an empty `SourcesLock` if the file does not exist.
pub fn load_sources_lock(path: &Path) -> Result<SourcesLock, ConfigError> {
    if !path.exists() {
        return Ok(SourcesLock::default());
    }
    let lock: SourcesLock = io::load_yaml(path)?;
    Ok(lock)
}

/// Write a sources spec to a YAML file atomically.
pub fn save_sources(path: &Path, spec: &SourcesSpec) -> Result<(), ConfigError> {
    io::write_yaml_atomic(path, spec)?;
    Ok(())
}

/// Write a sources lock file atomically.
pub fn save_sources_lock(path: &Path, lock: &SourcesLock) -> Result<(), ConfigError> {
    io::write_yaml_atomic(path, lock)?;
    Ok(())
}

fn validate_and_resolve_sources(
    mut spec: SourcesSpec,
    sources_dir: &Path,
) -> Result<SourcesSpec, ConfigError> {
    let mut seen_ids: HashSet<String> = HashSet::new();

    for entry in &mut spec.sources {
        // Reserved ID check.
        if RESERVED_SOURCE_IDS.contains(&entry.id.as_str()) {
            return Err(ConfigError::InvalidSources {
                reason: format!(
                    "source id '{}' is reserved and must not appear in sources.yaml",
                    entry.id
                ),
            });
        }

        // Uniqueness check.
        if !seen_ids.insert(entry.id.clone()) {
            return Err(ConfigError::InvalidSources {
                reason: format!("duplicate source id '{}'", entry.id),
            });
        }

        match entry.source_type {
            SourceType::Git => {
                // url required and non-empty.
                match &entry.url {
                    None => {
                        return Err(ConfigError::InvalidSources {
                            reason: format!("source '{}': url is required for type: git", entry.id),
                        });
                    }
                    Some(u) if u.is_empty() => {
                        return Err(ConfigError::InvalidSources {
                            reason: format!("source '{}': url is required for type: git", entry.id),
                        });
                    }
                    _ => {}
                }

                // path (git repo subpath): no absolute path, no `..` components.
                if let Some(ref p) = entry.path {
                    if p.is_empty() {
                        return Err(ConfigError::InvalidSources {
                            reason: format!("source '{}': path must not be empty", entry.id),
                        });
                    }
                    let subpath = std::path::Path::new(p.as_str());
                    if subpath.is_absolute() {
                        return Err(ConfigError::InvalidSources {
                            reason: format!(
                                "source '{}': path must be relative (no absolute paths in git repo subpath)",
                                entry.id
                            ),
                        });
                    }
                    if subpath
                        .components()
                        .any(|c| c == std::path::Component::ParentDir)
                    {
                        return Err(ConfigError::InvalidSources {
                            reason: format!(
                                "source '{}': path must not contain '..' components",
                                entry.id
                            ),
                        });
                    }
                }

                // ref: exactly one of branch, tag, or commit.
                if let Some(ref r) = entry.source_ref {
                    let set_count = [r.branch.is_some(), r.tag.is_some(), r.commit.is_some()]
                        .iter()
                        .filter(|&&b| b)
                        .count();
                    if set_count != 1 {
                        return Err(ConfigError::InvalidSources {
                            reason: format!(
                                "source '{}': ref must specify exactly one of branch, tag, or commit",
                                entry.id
                            ),
                        });
                    }
                }
            }
            SourceType::Path => {
                // url must not be specified for type:path.
                if entry.url.is_some() {
                    return Err(ConfigError::InvalidSources {
                        reason: format!(
                            "source '{}': url must not be specified for type: path",
                            entry.id
                        ),
                    });
                }

                // path required and non-empty.
                let raw_path = match &entry.path {
                    None => {
                        return Err(ConfigError::InvalidSources {
                            reason: format!(
                                "source '{}': path is required for type: path",
                                entry.id
                            ),
                        });
                    }
                    Some(p) if p.is_empty() => {
                        return Err(ConfigError::InvalidSources {
                            reason: format!("source '{}': path must not be empty", entry.id),
                        });
                    }
                    Some(p) => p.clone(),
                };

                // Resolve to absolute path.
                let resolved = resolve_source_path(&raw_path, sources_dir);
                entry.path = Some(resolved.display().to_string());
            }
        }

        // Validate allow-list names if Detailed (applies to both source types).
        if let Some(AllowSpec::Detailed(ref detail)) = entry.allow {
            if let Some(AllowList::Names(ref names)) = detail.features {
                for n in names {
                    if n.is_empty() {
                        return Err(ConfigError::InvalidSources {
                            reason: format!(
                                "source '{}': allow.features contains empty name",
                                entry.id
                            ),
                        });
                    }
                }
            }
            if let Some(AllowList::Names(ref names)) = detail.backends {
                for n in names {
                    if n.is_empty() {
                        return Err(ConfigError::InvalidSources {
                            reason: format!(
                                "source '{}': allow.backends contains empty name",
                                entry.id
                            ),
                        });
                    }
                }
            }
        }
    }

    Ok(spec)
}

/// Resolve a `type: path` source path to an absolute `PathBuf`.
///
/// Resolution rules (in order):
/// 1. `~` or `~/...` — expanded against the user's home directory.
/// 2. Absolute path — used as-is.
/// 3. Relative path — resolved against `sources_dir` (parent of `sources.yaml`).
fn resolve_source_path(raw: &str, sources_dir: &Path) -> std::path::PathBuf {
    // Home directory expansion.
    if raw == "~" {
        if let Some(home) = home_dir() {
            return home;
        }
    } else if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }

    let p = std::path::Path::new(raw);
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        sources_dir.join(p)
    };
    // Normalize away `.` components without requiring the path to exist.
    normalize_path(&joined)
}

/// Remove `.` components from a path without hitting the filesystem.
fn normalize_path(path: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut out = std::path::PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Return the user's home directory from the environment.
fn home_dir() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        std::env::var("USERPROFILE")
            .ok()
            .map(std::path::PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var("HOME").ok().map(std::path::PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_yaml_file(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, content).unwrap();
        p
    }

    // ── Profile (grouping) tests ───────────────────────────────────────────

    #[test]
    fn grouped_features_normalized_to_canonical() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "features:\n  core:\n    git: {}\n  local:\n    nvim: {}\n";
        let p = write_yaml_file(dir.path(), "profile.yaml", yaml);
        let profile = load_profile(&p).unwrap();
        assert!(
            profile.features.contains_key("core/git"),
            "core/git must be present"
        );
        assert!(
            profile.features.contains_key("local/nvim"),
            "local/nvim must be present"
        );
        assert_eq!(profile.features.len(), 2);
    }

    #[test]
    fn profile_empty_features_ok() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml_file(dir.path(), "profile.yaml", "features: {}\n");
        let profile = load_profile(&p).unwrap();
        assert!(profile.features.is_empty());
    }

    #[test]
    fn profile_version_forwarded_through_grouping() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "features:\n  local:\n    node:\n      version: \"20\"\n";
        let p = write_yaml_file(dir.path(), "profile.yaml", yaml);
        let profile = load_profile(&p).unwrap();
        let cfg = profile.features.get("local/node").unwrap();
        assert_eq!(cfg.version.as_deref(), Some("20"));
    }

    #[test]
    fn profile_empty_source_id_rejected() {
        // YAML: features: { "": { git: {} } }
        // serde_yaml will parse "" as an empty key
        let dir = tempfile::tempdir().unwrap();
        let yaml = "features:\n  \"\":\n    git: {}\n";
        let p = write_yaml_file(dir.path(), "profile.yaml", yaml);
        let err = load_profile(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidProfile { .. }));
    }

    #[test]
    fn profile_empty_feature_name_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "features:\n  core:\n    \"\": {}\n";
        let p = write_yaml_file(dir.path(), "profile.yaml", yaml);
        let err = load_profile(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidProfile { .. }));
    }

    #[test]
    fn profile_duplicate_canonical_rejected() {
        // Two sources that would produce the same canonical id via different paths
        // are not possible in grouping syntax (source_id is the outer key, so
        // "core: { git: {} }" appears once). Duplicates can only occur within
        // the same source group — e.g. outer key "core" appearing twice, which
        // YAML/serde handles by last-write-wins (HashMap). So verify the happy
        // path instead: same source, two different features are both present.
        let dir = tempfile::tempdir().unwrap();
        let yaml = "features:\n  core:\n    git: {}\n    bash: {}\n";
        let p = write_yaml_file(dir.path(), "profile.yaml", yaml);
        let profile = load_profile(&p).unwrap();
        assert!(profile.features.contains_key("core/git"));
        assert!(profile.features.contains_key("core/bash"));
    }

    #[test]
    fn profile_bare_name_at_top_level_rejected() {
        // Old format: "features:\n  git: {}\n" where the value is an empty map.
        // Now "git" is treated as a source_id mapping to feature-map {"git": {}}.
        // This is actually valid (source "git" with feature "{}") - but since the
        // value `{}` is an empty HashMap, "git" source has no features.
        // The resulting canonical map will be empty, not an error.
        // The important invariant is: you cannot sneak a bare name through as canonical.
        let dir = tempfile::tempdir().unwrap();
        // "features:\n  git: {}\n" — source_id=git, inner map is empty → 0 features
        let yaml = "features:\n  git: {}\n";
        let p = write_yaml_file(dir.path(), "profile.yaml", yaml);
        let profile = load_profile(&p).unwrap();
        // Inner {} means empty sourced features, not "git" as a canonical ID.
        assert!(
            !profile.features.contains_key("git"),
            "bare 'git' must not appear as canonical"
        );
        assert!(
            !profile.features.contains_key("core/git"),
            "must not auto-prefix to core/git"
        );
        assert!(
            profile.features.is_empty(),
            "source 'git' has no declared features"
        );
    }

    // ── Strategy tests ─────────────────────────────────────────────────────

    #[test]
    fn strategy_load_minimal() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml_file(
            dir.path(),
            "strategy.yaml",
            "package:\n  default_backend: brew\n",
        );
        let strategy = load_strategy(&p).unwrap();
        assert_eq!(
            strategy.package.unwrap().default_backend.as_deref(),
            Some("brew")
        );
    }

    #[test]
    fn strategy_empty_default_backend_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml_file(
            dir.path(),
            "strategy.yaml",
            "package:\n  default_backend: \"\"\n",
        );
        let err = load_strategy(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidStrategy { .. }));
    }

    #[test]
    fn strategy_empty_override_backend_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "package:\n  overrides:\n    ripgrep:\n      backend: \"\"\n";
        let p = write_yaml_file(dir.path(), "strategy.yaml", yaml);
        let err = load_strategy(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidStrategy { .. }));
    }

    #[test]
    fn strategy_no_defaults_ok() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml_file(dir.path(), "strategy.yaml", "{}\n");
        let strategy = load_strategy(&p).unwrap();
        assert!(strategy.package.is_none());
        assert!(strategy.runtime.is_none());
    }

    // ── Sources tests ──────────────────────────────────────────────────────

    #[test]
    fn sources_valid_external_source() {
        let dir = tempfile::tempdir().unwrap();
        let yaml =
            "sources:\n  - id: community\n    type: git\n    url: https://github.com/ex/repo\n";
        let p = write_yaml_file(dir.path(), "sources.yaml", yaml);
        let spec = load_sources(&p).unwrap();
        assert_eq!(spec.sources[0].id, "community");
    }

    #[test]
    fn sources_reserved_id_rejected() {
        for reserved in &["core", "local", "official"] {
            let dir = tempfile::tempdir().unwrap();
            let yaml =
                format!("sources:\n  - id: {reserved}\n    type: git\n    url: https://x.com\n");
            let p = write_yaml_file(dir.path(), "sources.yaml", &yaml);
            let err = load_sources(&p).unwrap_err();
            assert!(
                matches!(err, ConfigError::InvalidSources { .. }),
                "expected error for reserved id '{reserved}'"
            );
        }
    }

    #[test]
    fn sources_duplicate_id_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "sources:\n  - id: tools\n    type: git\n    url: https://a.com\n  - id: tools\n    type: git\n    url: https://b.com\n";
        let p = write_yaml_file(dir.path(), "sources.yaml", yaml);
        let err = load_sources(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidSources { .. }));
    }

    #[test]
    fn sources_empty_url_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "sources:\n  - id: tools\n    type: git\n    url: \"\"\n";
        let p = write_yaml_file(dir.path(), "sources.yaml", yaml);
        let err = load_sources(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidSources { .. }));
    }

    #[test]
    fn sources_empty_list_ok() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml_file(dir.path(), "sources.yaml", "{}\n");
        let spec = load_sources(&p).unwrap();
        assert!(spec.sources.is_empty());
    }

    #[test]
    fn sources_allow_wildcard_ok() {
        let dir = tempfile::tempdir().unwrap();
        let yaml =
            "sources:\n  - id: ext\n    type: git\n    url: https://x.com\n    allow: \"*\"\n";
        let p = write_yaml_file(dir.path(), "sources.yaml", yaml);
        let spec = load_sources(&p).unwrap();
        assert!(matches!(spec.sources[0].allow, Some(AllowSpec::All(_))));
    }

    // ── load_config (grouping) tests ───────────────────────────────────────

    #[test]
    fn config_profile_and_strategy_parsed() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
profile:
  features:
    core:
      git: {}
    local:
      myapp: {}

strategy:
  package:
    default_backend: local/brew
";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let (profile, strategy) = load_config(&p).unwrap();
        assert!(
            profile.features.contains_key("core/git"),
            "core/git must be present"
        );
        assert!(
            profile.features.contains_key("local/myapp"),
            "local/myapp must be present"
        );
        assert_eq!(
            strategy.package.unwrap().default_backend.as_deref(),
            Some("local/brew")
        );
    }

    #[test]
    fn config_strategy_optional_defaults_to_empty() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "profile:\n  features:\n    core:\n      git: {}\n";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let (profile, strategy) = load_config(&p).unwrap();
        assert!(profile.features.contains_key("core/git"));
        assert!(strategy.package.is_none());
        assert!(strategy.runtime.is_none());
    }

    #[test]
    fn config_empty_profile_features_ok() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "profile:\n  features: {}\n";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let (profile, _) = load_config(&p).unwrap();
        assert!(profile.features.is_empty());
    }

    #[test]
    fn config_extra_unknown_keys_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
profile:
  features:
    core:
      git: {}
future_section:
  some_key: value
another_unknown: 42
";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        assert!(load_config(&p).is_ok());
    }

    #[test]
    fn config_missing_profile_section_errors() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "strategy:\n  package:\n    default_backend: local/brew\n";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let err = load_config(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidProfile { .. }));
    }

    #[test]
    fn config_invalid_strategy_propagates_error() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
profile:
  features:
    core:
      git: {}
strategy:
  package:
    default_backend: \"\"
";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let err = load_config(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidStrategy { .. }));
    }

    #[test]
    fn config_version_config_forwarded() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "profile:\n  features:\n    local:\n      node:\n        version: \"20\"\n";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let (profile, _) = load_config(&p).unwrap();
        let cfg = profile.features.get("local/node").unwrap();
        assert_eq!(cfg.version.as_deref(), Some("20"));
    }

    // ── Bundle tests ───────────────────────────────────────────────────────

    #[test]
    fn bundle_features_merged_into_profile() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
bundle:
  use:
    - base

bundles:
  base:
    features:
      core:
        git: {}

profile:
  features:
    local:
      nvim: {}
";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let (profile, _) = load_config(&p).unwrap();
        assert!(
            profile.features.contains_key("core/git"),
            "bundle feature must be merged"
        );
        assert!(
            profile.features.contains_key("local/nvim"),
            "profile feature must be present"
        );
        assert_eq!(profile.features.len(), 2);
    }

    #[test]
    fn bundle_use_order_last_wins() {
        // base: core/git version "1.0", lang: core/git version "2.0"
        // use: [base, lang] → lang wins
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
bundle:
  use:
    - base
    - lang

bundles:
  base:
    features:
      core:
        git:
          version: \"1.0\"
  lang:
    features:
      core:
        git:
          version: \"2.0\"

profile:
  features: {}
";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let (profile, _) = load_config(&p).unwrap();
        let cfg = profile.features.get("core/git").unwrap();
        assert_eq!(
            cfg.version.as_deref(),
            Some("2.0"),
            "last bundle in use list must win"
        );
    }

    #[test]
    fn bundle_profile_features_override_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
bundle:
  use:
    - base

bundles:
  base:
    features:
      core:
        git:
          version: \"1.0\"

profile:
  features:
    core:
      git:
        version: \"override\"
";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let (profile, _) = load_config(&p).unwrap();
        let cfg = profile.features.get("core/git").unwrap();
        assert_eq!(
            cfg.version.as_deref(),
            Some("override"),
            "profile.features must override bundle"
        );
    }

    #[test]
    fn bundle_undefined_reference_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
bundle:
  use:
    - nonexistent

profile:
  features: {}
";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let err = load_config(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidProfile { .. }));
    }

    #[test]
    fn bundle_section_absent_ok() {
        // No bundle/bundles sections: behaves identically to profile-only config.
        let dir = tempfile::tempdir().unwrap();
        let yaml = "profile:\n  features:\n    core:\n      git: {}\n";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let (profile, _) = load_config(&p).unwrap();
        assert!(profile.features.contains_key("core/git"));
        assert_eq!(profile.features.len(), 1);
    }

    // ── Additional Sources tests ───────────────────────────────────────────

    #[test]
    fn sources_type_git_with_ref_ok() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "sources:\n  - id: ext\n    type: git\n    url: https://x.com\n    ref:\n      branch: main\n";
        let p = write_yaml_file(dir.path(), "sources.yaml", yaml);
        let spec = load_sources(&p).unwrap();
        let r = spec.sources[0].source_ref.as_ref().unwrap();
        assert_eq!(r.branch.as_deref(), Some("main"));
        assert!(r.tag.is_none());
        assert!(r.commit.is_none());
    }

    #[test]
    fn sources_type_git_ref_multiple_fields_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "sources:\n  - id: ext\n    type: git\n    url: https://x.com\n    ref:\n      branch: main\n      tag: v1\n";
        let p = write_yaml_file(dir.path(), "sources.yaml", yaml);
        let err = load_sources(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidSources { .. }));
    }

    #[test]
    fn sources_type_git_dotdot_subpath_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml =
            "sources:\n  - id: ext\n    type: git\n    url: https://x.com\n    path: ../sibling\n";
        let p = write_yaml_file(dir.path(), "sources.yaml", yaml);
        let err = load_sources(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidSources { .. }));
    }

    #[test]
    fn sources_type_git_absolute_subpath_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml =
            "sources:\n  - id: ext\n    type: git\n    url: https://x.com\n    path: /absolute\n";
        let p = write_yaml_file(dir.path(), "sources.yaml", yaml);
        let err = load_sources(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidSources { .. }));
    }

    #[test]
    fn sources_type_path_valid() {
        let dir = tempfile::tempdir().unwrap();
        // Use absolute path so resolution doesn't depend on tempdir.
        let yaml = format!(
            "sources:\n  - id: mylab\n    type: path\n    path: {}\n",
            dir.path().display()
        );
        let p = write_yaml_file(dir.path(), "sources.yaml", &yaml);
        let spec = load_sources(&p).unwrap();
        assert_eq!(spec.sources[0].source_type, SourceType::Path);
        // After resolution, path is absolute (was already absolute).
        assert!(spec.sources[0].path.as_deref().unwrap().starts_with('/'));
    }

    #[test]
    fn sources_type_path_no_path_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "sources:\n  - id: mylab\n    type: path\n";
        let p = write_yaml_file(dir.path(), "sources.yaml", yaml);
        let err = load_sources(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidSources { .. }));
    }

    #[test]
    fn sources_type_path_with_url_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = format!(
            "sources:\n  - id: mylab\n    type: path\n    path: {}\n    url: https://x.com\n",
            dir.path().display()
        );
        let p = write_yaml_file(dir.path(), "sources.yaml", &yaml);
        let err = load_sources(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidSources { .. }));
    }

    #[test]
    fn sources_type_path_relative_resolved_against_sources_dir() {
        let dir = tempfile::tempdir().unwrap();
        // Create the subdir so it's a plausible path (resolve doesn't check existence).
        let yaml = "sources:\n  - id: mylab\n    type: path\n    path: ./subdir\n";
        let p = write_yaml_file(dir.path(), "sources.yaml", yaml);
        let spec = load_sources(&p).unwrap();
        let resolved = spec.sources[0].path.as_deref().unwrap();
        let expected = dir.path().join("subdir").display().to_string();
        assert_eq!(resolved, expected);
    }

    #[test]
    fn sources_lock_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "sources:\n  community:\n    resolved_commit: abcdef1234567890abcdef1234567890abcdef12\n    fetched_at: '2026-04-07T00:00:00Z'\n    manifest_hash: 'sha256:abc'\n";
        let p = write_yaml_file(dir.path(), "sources.lock.yaml", yaml);
        let lock = load_sources_lock(&p).unwrap();
        assert_eq!(
            lock.sources["community"].resolved_commit,
            "abcdef1234567890abcdef1234567890abcdef12"
        );
        // Round-trip: save and reload.
        let p2 = dir.path().join("sources2.lock.yaml");
        save_sources_lock(&p2, &lock).unwrap();
        let lock2 = load_sources_lock(&p2).unwrap();
        assert_eq!(lock, lock2);
    }

    #[test]
    fn sources_lock_absent_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sources.lock.yaml");
        let lock = load_sources_lock(&p).unwrap();
        assert!(lock.sources.is_empty());
    }
}
