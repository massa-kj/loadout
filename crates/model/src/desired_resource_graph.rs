//! DesiredResourceGraph data types.
//!
//! The DesiredResourceGraph is the structured representation of all resources that should
//! exist after a successful apply, grouped by feature. It is produced by FeatureCompiler
//! and consumed exclusively by Planner.
//!
//! See: `docs/specs/data/desired_resource_graph.md`

use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::id::CanonicalBackendId;

/// Current schema version for DesiredResourceGraph.
pub const DESIRED_RESOURCE_GRAPH_SCHEMA_VERSION: u32 = 1;

/// Compiled desired resources grouped by feature, with backends already resolved.
///
/// Immutable once produced by FeatureCompiler. Neither Planner nor Executor may modify it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesiredResourceGraph {
    /// Schema version. Must be [`DESIRED_RESOURCE_GRAPH_SCHEMA_VERSION`] (1).
    pub schema_version: u32,

    /// Desired resources keyed by canonical feature ID.
    #[serde(default)]
    pub features: HashMap<String, FeatureDesiredResources>,
}

/// Desired resources for a single feature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeatureDesiredResources {
    /// Resources that should exist for this feature after apply.
    pub resources: Vec<DesiredResource>,
}

/// A single desired resource with its resolved backend.
///
/// `id` is stable and human-readable (format: `<kind>:<name>`, e.g. `package:git`).
/// Changing a resource `id` is a breaking change requiring state migration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesiredResource {
    /// Stable resource identifier unique within a feature's resource list.
    pub id: String,

    /// Resource kind and kind-specific data (backend already resolved).
    #[serde(flatten)]
    pub kind: DesiredResourceKind,
}

/// Kind-specific data for a desired resource (post-compiler, backend resolved).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DesiredResourceKind {
    /// A package to be installed via a resolved backend.
    Package {
        /// Package name as known to the backend.
        name: String,
        /// Resolved backend identifier.
        desired_backend: CanonicalBackendId,
    },
    /// A runtime to be installed via a resolved backend.
    Runtime {
        /// Runtime name (e.g. `node`, `python`).
        name: String,
        /// Version string (exact or constraint).
        version: String,
        /// Resolved backend identifier.
        desired_backend: CanonicalBackendId,
    },
    /// A filesystem entry to be created or linked (no backend).
    Fs {
        /// Path to source file/dir relative to the feature directory.
        /// Defaults to `files/<basename(path)>` if omitted.
        #[serde(skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        /// Target path (absolute or `~`-relative).
        path: String,
        /// Type of entry to create (`file` or `dir`).
        entry_type: FsEntryType,
        /// Operation to perform.
        op: FsOp,
    },
}

/// Entry types valid in DesiredResourceGraph declarations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FsEntryType {
    File,
    Dir,
}

/// Filesystem operation declared in DesiredResourceGraph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FsOp {
    Link,
    Copy,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let json = r#"{
            "schema_version": 1,
            "features": {
                "core/git": {
                    "resources": [
                        {
                            "id": "package:git",
                            "kind": "package",
                            "name": "git",
                            "desired_backend": "core/brew"
                        },
                        {
                            "id": "fs:gitconfig",
                            "kind": "fs",
                            "path": "~/.gitconfig",
                            "entry_type": "file",
                            "op": "link"
                        }
                    ]
                },
                "core/node": {
                    "resources": [
                        {
                            "id": "runtime:node",
                            "kind": "runtime",
                            "name": "node",
                            "version": "20",
                            "desired_backend": "core/mise"
                        }
                    ]
                }
            }
        }"#;
        let g: DesiredResourceGraph = serde_json::from_str(json).unwrap();
        assert_eq!(g.schema_version, 1);

        let git = &g.features["core/git"];
        assert_eq!(git.resources.len(), 2);
        match &git.resources[0].kind {
            DesiredResourceKind::Package { name, desired_backend } => {
                assert_eq!(name, "git");
                assert_eq!(desired_backend.as_str(), "core/brew");
            }
            _ => panic!("expected package"),
        }
        match &git.resources[1].kind {
            DesiredResourceKind::Fs { path, entry_type, op, .. } => {
                assert_eq!(path, "~/.gitconfig");
                assert_eq!(*entry_type, FsEntryType::File);
                assert_eq!(*op, FsOp::Link);
            }
            _ => panic!("expected fs"),
        }

        let node = &g.features["core/node"];
        match &node.resources[0].kind {
            DesiredResourceKind::Runtime { name, version, desired_backend } => {
                assert_eq!(name, "node");
                assert_eq!(version, "20");
                assert_eq!(desired_backend.as_str(), "core/mise");
            }
            _ => panic!("expected runtime"),
        }
    }
}
