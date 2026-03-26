//! Strategy data type.
//!
//! A strategy declares implementation strategy: which backend to use per resource kind,
//! filesystem backup settings, etc.
//!
//! See: `docs/specs/data/strategy.md`

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// User-declared implementation strategy.
///
/// `strategy` (the strategy ID string) is optional metadata and not used by core logic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Strategy {
    /// Optional strategy identifier label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,

    /// Backend selection for `package` resources.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<BackendStrategy>,

    /// Backend selection for `runtime` resources.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<BackendStrategy>,

    /// Filesystem operation settings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fs: Option<FsStrategy>,
}

/// Backend resolution strategy for a resource kind (package or runtime).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackendStrategy {
    /// Default backend to use when no per-resource override matches.
    /// Must be present unless every resource has an explicit override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_backend: Option<String>,

    /// Per-resource overrides. Keys are package names or runtime names.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub overrides: HashMap<String, BackendOverride>,
}

/// Per-resource backend override entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackendOverride {
    /// Backend identifier for this specific resource.
    pub backend: String,
}

/// Filesystem operation strategy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct FsStrategy {
    /// Whether to back up existing files before overwriting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<bool>,

    /// Directory where backups are stored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_dir: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_full() {
        let json = r#"{
            "strategy": "linux-default",
            "package": {
                "default_backend": "brew",
                "overrides": {
                    "ripgrep": { "backend": "cargo" }
                }
            },
            "runtime": {
                "default_backend": "mise"
            },
            "fs": {
                "backup": true,
                "backup_dir": "~/.backup/loadout"
            }
        }"#;
        let p: Strategy = serde_json::from_str(json).unwrap();
        let pkg = p.package.unwrap();
        assert_eq!(pkg.default_backend.as_deref(), Some("brew"));
        assert_eq!(pkg.overrides["ripgrep"].backend, "cargo");
        assert_eq!(p.fs.unwrap().backup, Some(true));
    }

    #[test]
    fn round_trip_empty() {
        let json = r#"{}"#;
        let p: Strategy = serde_json::from_str(json).unwrap();
        assert!(p.package.is_none());
    }
}
