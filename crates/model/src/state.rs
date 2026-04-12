//! State data types.
//!
//! State is the single authority for what resources were created by loadout execution
//! and what backend must be used for deterministic removal.
//!
//! See: `docs/specs/data/state.md`

use crate::id::CanonicalBackendId;
use crate::tool::ToolResource;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Current version number for the state schema.
pub const STATE_VERSION: u32 = 3;

/// Authoritative state recording all resources installed by loadout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    /// Schema version. Must be [`STATE_VERSION`] (3) for this implementation.
    pub version: u32,

    /// Installed resources grouped by canonical component ID.
    #[serde(default)]
    pub components: HashMap<String, ComponentState>,
}

impl State {
    /// Construct an empty (no components installed) initial state.
    pub fn empty() -> Self {
        Self {
            version: STATE_VERSION,
            components: HashMap::new(),
        }
    }
}

/// Resources recorded for a single installed component.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentState {
    /// Resources installed for this component.
    pub resources: Vec<Resource>,
}

/// A recorded resource entry in state.
///
/// `id` must be unique within a component's resource list.
/// The `(component_id, resource.id)` pair must be unique across all components.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resource {
    /// Stable, human-readable resource identifier unique within a component.
    pub id: String,

    /// Resource kind and kind-specific data.
    #[serde(flatten)]
    pub kind: ResourceKind,
}

/// Kind-specific data for a recorded resource.
///
/// The `backend` field is present for `Package` and `Runtime` to ensure deterministic removal.
/// The backend that was used to install must be the same backend used to remove.
///
/// See `docs/specs/data/state.md` for JSON schema and invariants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResourceKind {
    /// An installed package managed by a backend.
    ///
    /// Packages are named artifacts installed by package managers (e.g., `brew`, `apt`, `scoop`).
    /// The recorded `backend` ensures the same backend is used for removal.
    Package {
        /// Canonical backend ID that was used to install this package (e.g., `core/brew`).
        /// Must be used for deterministic removal.
        backend: CanonicalBackendId,
        /// Package name and optional version.
        package: PackageDetails,
    },
    /// An installed runtime managed by a backend.
    ///
    /// Runtimes are version-managed language runtimes (e.g., `node@20`, `python@3.12`).
    /// Unlike packages, runtimes always require an explicit version.
    Runtime {
        /// Canonical backend ID that was used to install this runtime (e.g., `core/mise`).
        /// Must be used for deterministic removal.
        backend: CanonicalBackendId,
        /// Runtime name and version.
        runtime: RuntimeDetails,
    },
    /// A filesystem entry created or linked by loadout.
    ///
    /// No backend is involved; the `fs` module handles these directly.
    /// Recorded to ensure safe removal (only tracked paths may be removed).
    Fs {
        /// Path, entry type, and operation details.
        fs: FsDetails,
    },

    /// An external tool introduced via a `managed_script` component.
    ///
    /// Tools are installed and removed by component scripts, but core owns
    /// verification and state updates. Unlike packages, tools are not managed
    /// by any backend and require an identity verify contract.
    ///
    /// The `observed` field records facts captured during install verify,
    /// used for absence checks on uninstall and future drift detection.
    ///
    /// See: `docs/specs/data/state.md` (tool resource section)
    Tool {
        /// Tool name, verify contract snapshot, and observed facts.
        tool: ToolResource,
    },
}

/// Details for a recorded package resource.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageDetails {
    /// Package name as known to the backend (e.g., `"git"`, `"neovim"`).
    pub name: String,
    /// Installed version string, or `None` if unknown or unpinned.
    ///
    /// `None` means the version was not tracked at install time.
    /// It does NOT mean "latest".
    pub version: Option<String>,
}

/// Details for a recorded runtime resource.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeDetails {
    /// Runtime name (e.g., `"node"`, `"python"`, `"ruby"`).
    pub name: String,
    /// Installed version string (e.g., `"20.0.0"`, `"3.12"`).
    ///
    /// Unlike packages, runtime versions are always required and recorded.
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
        assert!(s.components.is_empty());
    }

    #[test]
    fn round_trip_package() {
        let json = r#"{
            "version": 3,
            "components": {
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
        let feat = &s.components["core/git"];
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
            "components": {
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
        let feat = &s.components["core/node"];
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
            "components": {
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
        let feat = &s.components["core/git"];
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
