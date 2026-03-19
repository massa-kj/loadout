//! State data types.
//!
//! State is the single authority for what resources were created by loadout execution
//! and what backend must be used for deterministic removal.
//!
//! See: `docs/specs/data/state.md`

use crate::id::CanonicalBackendId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Current version number for the state schema.
pub const STATE_VERSION: u32 = 3;

/// Authoritative state recording all resources installed by loadout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    /// Schema version. Must be [`STATE_VERSION`] (3) for this implementation.
    pub version: u32,

    /// Installed resources grouped by canonical feature ID.
    #[serde(default)]
    pub features: HashMap<String, FeatureState>,
}

impl State {
    /// Construct an empty (no features installed) initial state.
    pub fn empty() -> Self {
        Self {
            version: STATE_VERSION,
            features: HashMap::new(),
        }
    }
}

/// Resources recorded for a single installed feature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeatureState {
    /// Resources installed for this feature.
    pub resources: Vec<Resource>,
}

/// A recorded resource entry in state.
///
/// `id` must be unique within a feature's resource list.
/// The `(feature_id, resource.id)` pair must be unique across all features.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resource {
    /// Stable, human-readable resource identifier unique within a feature.
    pub id: String,

    /// Resource kind and kind-specific data.
    #[serde(flatten)]
    pub kind: ResourceKind,
}

/// Kind-specific data for a recorded resource.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResourceKind {
    /// An installed package managed by a backend.
    Package {
        /// Backend that was used to install this package.
        backend: CanonicalBackendId,
        /// Package-specific details.
        package: PackageDetails,
    },
    /// An installed runtime managed by a backend.
    Runtime {
        /// Backend that was used to install this runtime.
        backend: CanonicalBackendId,
        /// Runtime-specific details.
        runtime: RuntimeDetails,
    },
    /// A filesystem entry created or linked by loadout.
    Fs {
        /// Filesystem-specific details.
        fs: FsDetails,
    },
}

/// Details for a recorded package resource.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageDetails {
    /// Package name as known to the backend.
    pub name: String,
    /// Installed version, or `null` if unknown/unpinned.
    pub version: Option<String>,
}

/// Details for a recorded runtime resource.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeDetails {
    /// Runtime name (e.g. `node`, `python`).
    pub name: String,
    /// Installed version string.
    pub version: String,
}

/// Details for a recorded filesystem resource.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FsDetails {
    /// Absolute path that was created or linked.
    pub path: String,
    /// Type of filesystem entry that was created.
    pub entry_type: FsEntryType,
    /// Operation that was performed.
    pub op: FsOp,
}

/// Type of filesystem entry recorded in state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FsEntryType {
    File,
    Dir,
    Symlink,
    Junction,
}

/// Filesystem operation recorded in state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FsOp {
    Copy,
    Link,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_state() {
        let s = State::empty();
        assert_eq!(s.version, STATE_VERSION);
        assert!(s.features.is_empty());
    }

    #[test]
    fn round_trip_package() {
        let json = r#"{
            "version": 3,
            "features": {
                "core/git": {
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
        }"#;
        let s: State = serde_json::from_str(json).unwrap();
        assert_eq!(s.version, 3);
        let feat = &s.features["core/git"];
        assert_eq!(feat.resources.len(), 1);
        assert_eq!(feat.resources[0].id, "pkg:git");
        match &feat.resources[0].kind {
            ResourceKind::Package { backend, package } => {
                assert_eq!(backend.as_str(), "core/brew");
                assert_eq!(package.name, "git");
                assert!(package.version.is_none());
            }
            _ => panic!("expected package"),
        }
    }

    #[test]
    fn round_trip_runtime() {
        let json = r#"{
            "version": 3,
            "features": {
                "core/node": {
                    "resources": [
                        {
                            "id": "rt:node@20",
                            "kind": "runtime",
                            "backend": "core/mise",
                            "runtime": { "name": "node", "version": "20" }
                        }
                    ]
                }
            }
        }"#;
        let s: State = serde_json::from_str(json).unwrap();
        let feat = &s.features["core/node"];
        match &feat.resources[0].kind {
            ResourceKind::Runtime { backend, runtime } => {
                assert_eq!(backend.as_str(), "core/mise");
                assert_eq!(runtime.version, "20");
            }
            _ => panic!("expected runtime"),
        }
    }

    #[test]
    fn round_trip_fs() {
        let json = r#"{
            "version": 3,
            "features": {
                "core/git": {
                    "resources": [
                        {
                            "id": "fs:gitconfig",
                            "kind": "fs",
                            "fs": {
                                "path": "/home/user/.gitconfig",
                                "entry_type": "symlink",
                                "op": "link"
                            }
                        }
                    ]
                }
            }
        }"#;
        let s: State = serde_json::from_str(json).unwrap();
        let feat = &s.features["core/git"];
        match &feat.resources[0].kind {
            ResourceKind::Fs { fs } => {
                assert_eq!(fs.path, "/home/user/.gitconfig");
                assert_eq!(fs.entry_type, FsEntryType::Symlink);
                assert_eq!(fs.op, FsOp::Link);
            }
            _ => panic!("expected fs"),
        }
    }
}
