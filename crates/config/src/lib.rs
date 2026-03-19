//! Configuration loading, validation, and normalization.
//!
//! This crate bridges raw YAML files on disk and domain model types in the `model` crate.
//! It handles three kinds of config files: profiles, policies, and sources.
//!
//! **Phase 3 contract**: path resolution (XDG, AppData) is NOT handled here.
//! Callers supply explicit `&Path` values. Platform-aware path discovery belongs to
//! the `platform` crate (Phase 4).
//!
//! See: `docs/specs/data/profile.md`, `docs/specs/data/policy.md`,
//!      `docs/specs/data/sources.md`

use std::collections::HashSet;
use std::path::Path;

pub use model::{
    policy::{BackendOverride, BackendPolicy, FsPolicy, Policy},
    profile::{Profile, ProfileFeatureConfig},
    sources::{AllowList, AllowSpec, SourceEntry, SourceType, SourcesSpec, WildcardAll},
};
use thiserror::Error;

/// Errors produced by configuration loading or validation.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("I/O error: {0}")]
    Io(#[from] io::IoError),

    #[error("invalid profile: {reason}")]
    InvalidProfile { reason: String },

    #[error("invalid policy: {reason}")]
    InvalidPolicy { reason: String },

    #[error("invalid sources: {reason}")]
    InvalidSources { reason: String },
}

// ─── Profile ────────────────────────────────────────────────────────────────

/// Load, normalize, and validate a profile from a YAML file.
///
/// Normalization: bare feature names (no `/`) are prefixed with `core/`.
/// Names already containing exactly one `/` are preserved as-is.
/// Names with more than one `/` are rejected.
pub fn load_profile(path: &Path) -> Result<Profile, ConfigError> {
    let raw: Profile = io::load_yaml(path)?;
    validate_and_normalize_profile(raw)
}

fn validate_and_normalize_profile(raw: Profile) -> Result<Profile, ConfigError> {
    let mut normalized = Profile {
        features: std::collections::HashMap::new(),
    };

    for (key, config) in raw.features {
        if key.is_empty() {
            return Err(ConfigError::InvalidProfile {
                reason: "feature key must not be empty".into(),
            });
        }

        let slash_count = key.chars().filter(|&c| c == '/').count();
        match slash_count {
            // Bare name → canonicalize to core/<name>
            0 => {
                let canonical = format!("core/{}", key);
                normalized.features.insert(canonical, config);
            }
            // Exactly one slash: already canonical (source/name), keep as-is
            1 => {
                // Validate: neither part must be empty
                let parts: Vec<&str> = key.splitn(2, '/').collect();
                if parts[0].is_empty() || parts[1].is_empty() {
                    return Err(ConfigError::InvalidProfile {
                        reason: format!(
                            "invalid feature key '{key}': source or name segment is empty"
                        ),
                    });
                }
                normalized.features.insert(key, config);
            }
            // More than one slash: reject
            _ => {
                return Err(ConfigError::InvalidProfile {
                    reason: format!("invalid feature key '{key}': at most one '/' is allowed (got {slash_count})"),
                });
            }
        }
    }

    Ok(normalized)
}

// ─── Policy ─────────────────────────────────────────────────────────────────

/// Load and validate a policy from a YAML file.
pub fn load_policy(path: &Path) -> Result<Policy, ConfigError> {
    let raw: Policy = io::load_yaml(path)?;
    validate_policy(raw)
}

fn validate_policy(policy: Policy) -> Result<Policy, ConfigError> {
    if let Some(ref pkg) = policy.package {
        validate_backend_policy_field("package.default_backend", pkg.default_backend.as_deref())?;
        for (name, ov) in &pkg.overrides {
            if ov.backend.is_empty() {
                return Err(ConfigError::InvalidPolicy {
                    reason: format!("package.overrides[{name}].backend must not be empty"),
                });
            }
        }
    }

    if let Some(ref rt) = policy.runtime {
        validate_backend_policy_field("runtime.default_backend", rt.default_backend.as_deref())?;
        for (name, ov) in &rt.overrides {
            if ov.backend.is_empty() {
                return Err(ConfigError::InvalidPolicy {
                    reason: format!("runtime.overrides[{name}].backend must not be empty"),
                });
            }
        }
    }

    Ok(policy)
}

fn validate_backend_policy_field(field: &str, value: Option<&str>) -> Result<(), ConfigError> {
    if let Some(v) = value {
        if v.is_empty() {
            return Err(ConfigError::InvalidPolicy {
                reason: format!("{field} must not be empty string if present"),
            });
        }
    }
    Ok(())
}

// ─── Sources ─────────────────────────────────────────────────────────────────

const RESERVED_SOURCE_IDS: &[&str] = &["core", "user", "official"];

/// Load and validate a sources spec from a YAML file.
pub fn load_sources(path: &Path) -> Result<SourcesSpec, ConfigError> {
    let raw: SourcesSpec = io::load_yaml(path)?;
    validate_sources(raw)
}

fn validate_sources(spec: SourcesSpec) -> Result<SourcesSpec, ConfigError> {
    let mut seen_ids: HashSet<String> = HashSet::new();

    for entry in &spec.sources {
        // Reserved ID check
        if RESERVED_SOURCE_IDS.contains(&entry.id.as_str()) {
            return Err(ConfigError::InvalidSources {
                reason: format!(
                    "source id '{}' is reserved and must not appear in sources.yaml",
                    entry.id
                ),
            });
        }

        // Uniqueness check
        if !seen_ids.insert(entry.id.clone()) {
            return Err(ConfigError::InvalidSources {
                reason: format!("duplicate source id '{}'", entry.id),
            });
        }

        // URL must not be empty
        if entry.url.is_empty() {
            return Err(ConfigError::InvalidSources {
                reason: format!("source '{}': url must not be empty", entry.id),
            });
        }

        // Validate allow-list names if Detailed
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

#[cfg(test)]
mod tests {
    use super::*;

    fn write_yaml_file(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, content).unwrap();
        p
    }

    // ── Profile tests ──────────────────────────────────────────────────────

    #[test]
    fn profile_bare_name_normalized_to_core() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml_file(dir.path(), "profile.yaml", "features:\n  git: {}\n");
        let profile = load_profile(&p).unwrap();
        assert!(
            profile.features.contains_key("core/git"),
            "bare 'git' must become 'core/git'"
        );
        assert!(!profile.features.contains_key("git"));
    }

    #[test]
    fn profile_canonical_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml_file(dir.path(), "profile.yaml", "features:\n  user/myvim: {}\n");
        let profile = load_profile(&p).unwrap();
        assert!(profile.features.contains_key("user/myvim"));
    }

    #[test]
    fn profile_core_prefix_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml_file(dir.path(), "profile.yaml", "features:\n  core/git: {}\n");
        let profile = load_profile(&p).unwrap();
        // Must not double-prefix: core/git → core/git, not core/core/git
        assert!(profile.features.contains_key("core/git"));
        assert!(!profile.features.contains_key("core/core/git"));
    }

    #[test]
    fn profile_multi_slash_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml_file(dir.path(), "profile.yaml", "features:\n  a/b/c: {}\n");
        let err = load_profile(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidProfile { .. }));
    }

    #[test]
    fn profile_empty_key_rejected() {
        let raw = Profile {
            features: [("".to_string(), Default::default())].into_iter().collect(),
        };
        let err = validate_and_normalize_profile(raw).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidProfile { .. }));
    }

    #[test]
    fn profile_empty_segment_rejected() {
        let dir = tempfile::tempdir().unwrap();
        // "/git" has empty source segment
        let p = write_yaml_file(dir.path(), "profile.yaml", "features:\n  /git: {}\n");
        let err = load_profile(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidProfile { .. }));
    }

    #[test]
    fn profile_version_config_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml_file(
            dir.path(),
            "profile.yaml",
            "features:\n  node:\n    version: \"20\"\n",
        );
        let profile = load_profile(&p).unwrap();
        let cfg = profile.features.get("core/node").unwrap();
        assert_eq!(cfg.version.as_deref(), Some("20"));
    }

    // ── Policy tests ───────────────────────────────────────────────────────

    #[test]
    fn policy_load_minimal() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml_file(
            dir.path(),
            "policy.yaml",
            "package:\n  default_backend: brew\n",
        );
        let policy = load_policy(&p).unwrap();
        assert_eq!(
            policy.package.unwrap().default_backend.as_deref(),
            Some("brew")
        );
    }

    #[test]
    fn policy_empty_default_backend_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml_file(
            dir.path(),
            "policy.yaml",
            "package:\n  default_backend: \"\"\n",
        );
        let err = load_policy(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidPolicy { .. }));
    }

    #[test]
    fn policy_empty_override_backend_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "package:\n  overrides:\n    ripgrep:\n      backend: \"\"\n";
        let p = write_yaml_file(dir.path(), "policy.yaml", yaml);
        let err = load_policy(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidPolicy { .. }));
    }

    #[test]
    fn policy_no_defaults_ok() {
        // Policy with no package/runtime fields at all is valid.
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml_file(dir.path(), "policy.yaml", "{}\n");
        let policy = load_policy(&p).unwrap();
        assert!(policy.package.is_none());
        assert!(policy.runtime.is_none());
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
        for reserved in &["core", "user", "official"] {
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
}
