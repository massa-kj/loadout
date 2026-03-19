//! Feature Index data types.
//!
//! The Feature Index is the parsed, merged, and validated representation of all available
//! features. It is produced by the Feature Index Builder and consumed by the Resolver
//! and FeatureCompiler.
//!
//! See: `docs/specs/data/feature_index.md`

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Current schema version for the Feature Index.
pub const FEATURE_INDEX_SCHEMA_VERSION: u32 = 1;

/// Parsed and normalized collection of all available features.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeatureIndex {
    /// Schema version. Must be [`FEATURE_INDEX_SCHEMA_VERSION`] (1).
    pub schema_version: u32,

    /// Feature metadata keyed by canonical feature ID.
    #[serde(default)]
    pub features: HashMap<String, FeatureMeta>,
}

/// Normalized metadata for a single feature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeatureMeta {
    /// Feature spec schema version. Must be 1.
    pub spec_version: u32,

    /// Execution mode: script or declarative.
    pub mode: FeatureMode,

    /// Human-readable description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Absolute path to the feature directory (resolved by source registry).
    pub source_dir: String,

    /// Dependency declarations. May be empty but must be present.
    pub dep: DepSpec,

    /// Resource declarations. Present for `declarative` mode features;
    /// may be absent for `script` mode features.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spec: Option<FeatureSpec>,
}

/// Feature execution mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureMode {
    Script,
    Declarative,
}

/// Dependency declarations for a feature.
///
/// Only these fields may be read by the Resolver. FeatureCompiler and Planner must not
/// add or modify fields here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DepSpec {
    /// Canonical feature IDs this feature depends on explicitly.
    #[serde(default)]
    pub depends: Vec<String>,

    /// Capability names this feature requires from another feature.
    #[serde(default)]
    pub requires: Vec<CapabilityRef>,

    /// Capability names this feature exposes to others.
    #[serde(default)]
    pub provides: Vec<CapabilityRef>,
}

/// Reference to a named capability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityRef {
    /// Capability name.
    pub name: String,
}

/// Resource spec for declarative mode features.
///
/// Contains the resource declarations that FeatureCompiler expands into
/// `DesiredResourceGraph` entries (without backend resolution applied yet).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeatureSpec {
    /// Declared resources for this feature.
    #[serde(default)]
    pub resources: Vec<SpecResource>,
}

/// A single resource declaration in a feature spec (before backend resolution).
///
/// `desired_backend` is absent here; it is resolved by FeatureCompiler.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpecResource {
    /// Stable resource identifier unique within this feature.
    pub id: String,

    /// Resource kind and kind-specific data (no backend yet).
    #[serde(flatten)]
    pub kind: SpecResourceKind,
}

/// Kind-specific data for a spec resource (pre-compiler).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SpecResourceKind {
    /// Package to be installed.
    Package {
        /// Package name as known to the backend.
        name: String,
    },
    /// Runtime to be installed.
    Runtime {
        /// Runtime name (e.g. `node`, `python`).
        name: String,
        /// Required version string.
        version: String,
    },
    /// Filesystem entry to be created or linked.
    Fs {
        /// Path to source file/dir, relative to feature directory.
        /// Defaults to `files/<basename(path)>` if omitted.
        #[serde(skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        /// Target path (absolute or `~`-relative).
        path: String,
        /// Type of entry to create.
        entry_type: SpecFsEntryType,
        /// Operation to perform.
        op: FsOp,
    },
}

/// Entry types valid in feature spec declarations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecFsEntryType {
    File,
    Dir,
}

/// Filesystem operation.
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
    fn round_trip_script_feature() {
        let json = r#"{
            "schema_version": 1,
            "features": {
                "core/git": {
                    "spec_version": 1,
                    "mode": "script",
                    "description": "Git version control system",
                    "source_dir": "/home/user/loadout/features/git",
                    "dep": {
                        "depends": [],
                        "requires": [{ "name": "package_manager" }],
                        "provides": []
                    }
                }
            }
        }"#;
        let fi: FeatureIndex = serde_json::from_str(json).unwrap();
        assert_eq!(fi.schema_version, 1);
        let meta = &fi.features["core/git"];
        assert_eq!(meta.mode, FeatureMode::Script);
        assert_eq!(meta.dep.requires[0].name, "package_manager");
        assert!(meta.spec.is_none());
    }

    #[test]
    fn round_trip_declarative_feature() {
        let json = r#"{
            "schema_version": 1,
            "features": {
                "core/neovim": {
                    "spec_version": 1,
                    "mode": "declarative",
                    "source_dir": "/home/user/loadout/features/neovim",
                    "dep": { "depends": ["core/git"] },
                    "spec": {
                        "resources": [
                            { "id": "package:neovim", "kind": "package", "name": "neovim" },
                            {
                                "id": "fs:nvim-config",
                                "kind": "fs",
                                "path": "~/.config/nvim",
                                "entry_type": "dir",
                                "op": "link"
                            }
                        ]
                    }
                }
            }
        }"#;
        let fi: FeatureIndex = serde_json::from_str(json).unwrap();
        let meta = &fi.features["core/neovim"];
        assert_eq!(meta.mode, FeatureMode::Declarative);
        let spec = meta.spec.as_ref().unwrap();
        assert_eq!(spec.resources.len(), 2);
        match &spec.resources[0].kind {
            SpecResourceKind::Package { name } => assert_eq!(name, "neovim"),
            _ => panic!("expected package"),
        }
    }
}
