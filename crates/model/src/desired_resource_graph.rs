//! DesiredResourceGraph data types.
//!
//! The DesiredResourceGraph is the structured representation of all resources that should
//! exist after a successful apply, grouped by component. It is produced by ComponentCompiler
//! and consumed exclusively by Planner.
//!
//! See: `docs/specs/data/desired_resource_graph.md`

use crate::id::CanonicalBackendId;
use crate::tool::ToolVerifyContract;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Current schema version for DesiredResourceGraph.
pub const DESIRED_RESOURCE_GRAPH_SCHEMA_VERSION: u32 = 1;

/// Compiled desired resources grouped by component, with backends already resolved.
///
/// Immutable once produced by ComponentCompiler. Neither Planner nor Executor may modify it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesiredResourceGraph {
    /// Schema version. Must be [`DESIRED_RESOURCE_GRAPH_SCHEMA_VERSION`] (1).
    pub schema_version: u32,

    /// Desired resources keyed by canonical component ID.
    #[serde(default)]
    pub components: HashMap<String, ComponentDesiredResources>,
}

/// Desired resources for a single component.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentDesiredResources {
    /// Resources that should exist for this component after apply.
    pub resources: Vec<DesiredResource>,
}

/// A single desired resource with its resolved backend.
///
/// `id` is stable and human-readable (format: `<kind>:<name>`, e.g. `package:git`).
/// Changing a resource `id` is a breaking change requiring state migration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesiredResource {
    /// Stable resource identifier unique within a component's resource list.
    pub id: String,

    /// Resource kind and kind-specific data (backend already resolved).
    #[serde(flatten)]
    pub kind: DesiredResourceKind,
}

/// Kind-specific data for a desired resource (post-compiler, backend resolved).
///
/// `desired_backend` is present for `Package` and `Runtime` because ComponentCompiler
/// has already resolved strategy. The Planner uses this field for backend-mismatch detection.
///
/// `Fs` resources have no backend; they are handled directly by the `fs` module.
///
/// See `docs/specs/data/desired_resource_graph.md` for JSON schema and compatibility rules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DesiredResourceKind {
    /// A package to be installed via a resolved backend.
    ///
    /// Packages are named artifacts installed by package managers.
    /// Strategy has already determined which backend should be used.
    Package {
        /// Package name as known to the backend (e.g., `"git"`, `"neovim"`).
        name: String,
        /// Canonical backend ID resolved by ComponentCompiler (e.g., `"core/brew"`).
        ///
        /// The Planner uses this for backend-mismatch detection. The Executor dispatches to this backend.
        desired_backend: CanonicalBackendId,
    },
    /// A runtime to be installed via a resolved backend.
    ///
    /// Runtimes are version-managed language runtimes. Unlike packages, runtimes always require a version.
    Runtime {
        /// Runtime name (e.g., `"node"`, `"python"`, `"ruby"`).
        name: String,
        /// Version string (exact or constraint, e.g., `"20.0.0"`, `"3.12"`).
        version: String,
        /// Canonical backend ID resolved by ComponentCompiler (e.g., `"core/mise"`).
        desired_backend: CanonicalBackendId,
    },
    /// A filesystem entry to be created or linked (no backend).
    ///
    /// Handled directly by the `fs` module without backend involvement.
    Fs {
        /// Path to source file/dir relative to the component directory.
        ///
        /// Defaults to `files/<basename(path)>` if omitted in the component spec.
        #[serde(skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        /// Target path where the file/dir should exist (absolute or `~`-relative).
        path: String,
        /// Type of filesystem entry to create (`file` or `dir`).
        entry_type: FsEntryType,
        /// Operation to perform (`link` for symlink, `copy` for copy).
        op: FsOp,
    },

    /// An external tool to be introduced via a `managed_script` component.
    ///
    /// Unlike packages and runtimes, tools have no backend. The component's install/uninstall
    /// scripts are solely responsible for deployment. Core owns verify and state.
    ///
    /// The Planner uses `verify.identity` (and `verify.version.constraint` if present)
    /// for compatibility checks. Actual verification is performed by the Executor.
    ///
    /// See: `docs/specs/data/desired_resource_graph.md` (tool resource section)
    Tool {
        /// Tool name as declared in the component (e.g., `"brew"`, `"deno"`).
        name: String,
        /// Verification contract. `identity` is required; `version` is optional.
        ///
        /// The Planner's compatibility check is based on this contract:
        /// - `identity` change → `replace`
        /// - `version.constraint` change → `replace`
        /// - Other changes (script improvements etc.) do not cause `replace`
        verify: ToolVerifyContract,
    },
}

/// Entry types valid in DesiredResourceGraph declarations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FsEntryType {
    /// A regular file.
    File,
    /// A directory.
    Dir,
}

/// Filesystem operation declared in DesiredResourceGraph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FsOp {
    /// Create a symlink from `source` to `path`.
    Link,
    /// Copy the file/dir from `source` to `path`.
    Copy,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let json = r#"{
            "schema_version": 1,
            "components": {
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

        let git = &g.components["core/git"];
        assert_eq!(git.resources.len(), 2);
        match &git.resources[0].kind {
            DesiredResourceKind::Package {
                name,
                desired_backend,
            } => {
                assert_eq!(name, "git");
                assert_eq!(desired_backend.as_str(), "core/brew");
            }
            _ => panic!("expected package"),
        }
        match &git.resources[1].kind {
            DesiredResourceKind::Fs {
                path,
                entry_type,
                op,
                ..
            } => {
                assert_eq!(path, "~/.gitconfig");
                assert_eq!(*entry_type, FsEntryType::File);
                assert_eq!(*op, FsOp::Link);
            }
            _ => panic!("expected fs"),
        }

        let node = &g.components["core/node"];
        match &node.resources[0].kind {
            DesiredResourceKind::Runtime {
                name,
                version,
                desired_backend,
            } => {
                assert_eq!(name, "node");
                assert_eq!(version, "20");
                assert_eq!(desired_backend.as_str(), "core/mise");
            }
            _ => panic!("expected runtime"),
        }
    }
}
