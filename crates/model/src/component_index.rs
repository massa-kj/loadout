//! Component Index data types.
//!
//! The Component Index is the parsed, merged, and validated representation of all available
//! components. It is produced by the Component Index Builder and consumed by the Resolver
//! and ComponentCompiler.
//!
//! See: `docs/specs/data/component_index.md`

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Current schema version for the Component Index.
pub const COMPONENT_INDEX_SCHEMA_VERSION: u32 = 1;

/// Parsed and normalized collection of all available components.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentIndex {
    /// Schema version. Must be [`COMPONENT_INDEX_SCHEMA_VERSION`] (1).
    pub schema_version: u32,

    /// Component metadata keyed by canonical component ID.
    #[serde(default)]
    pub components: HashMap<String, ComponentMeta>,
}

/// Normalized metadata for a single component.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentMeta {
    /// Component spec schema version. Must be 1.
    pub spec_version: u32,

    /// Execution mode: script or declarative.
    pub mode: ComponentMode,

    /// Human-readable description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Absolute path to the component directory (resolved by source registry).
    pub source_dir: String,

    /// Dependency declarations. May be empty but must be present.
    pub dep: DepSpec,

    /// Resource declarations. Present for `declarative` mode components;
    /// may be absent for `script` mode components.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spec: Option<ComponentSpec>,
}

/// Component execution mode.
///
/// Determines how the executor handles a component: via subprocess (script mode)
/// or via direct resource application (declarative mode).
///
/// See `docs/guides/components.md` and `docs/specs/api/component-host.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentMode {
    /// Execute `install.sh` / `uninstall.sh` scripts.
    ///
    /// Used when installation logic cannot be expressed as declarative resources
    /// (e.g., system configuration, templating, conditional logic).
    Script,
    /// Declare resources in `component.yaml`; executor applies them without scripts.
    ///
    /// Preferred mode for packages, runtimes, and files. Provides better plan accuracy
    /// (noop detection, replace/strengthen classification) and atomic operations.
    Declarative,
}

/// Dependency declarations for a component.
///
/// Only these fields may be read by the Resolver. ComponentCompiler and Planner must not
/// add or modify fields here.
///
/// See `docs/specs/algorithms/resolver.md` for resolution semantics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DepSpec {
    /// Canonical component IDs this component depends on explicitly (install ordering).
    ///
    /// Use when dependency is on a specific named component (e.g., `["core/git"]`).
    #[serde(default)]
    pub depends: Vec<String>,

    /// Capability names this component requires from another component (abstract dependency).
    ///
    /// Use when any provider of a capability suffices (e.g., any package manager).
    /// If no provider is in the desired set, resolution aborts.
    #[serde(default)]
    pub requires: Vec<CapabilityRef>,

    /// Capability names this component exposes to others (abstract provision).
    ///
    /// Other components can `requires` these capabilities.
    #[serde(default)]
    pub provides: Vec<CapabilityRef>,
}

/// Reference to a named capability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityRef {
    /// Capability name.
    pub name: String,
}

/// Resource spec for declarative mode components.
///
/// Contains the resource declarations that ComponentCompiler expands into
/// `DesiredResourceGraph` entries (without backend resolution applied yet).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentSpec {
    /// Declared resources for this component.
    #[serde(default)]
    pub resources: Vec<SpecResource>,
}

/// A single resource declaration in a component spec (before backend resolution).
///
/// `desired_backend` is absent here; it is resolved by ComponentCompiler.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpecResource {
    /// Stable resource identifier unique within this component.
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
        /// Path to source file/dir, relative to component directory.
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

/// Entry types valid in component spec declarations.
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
    fn round_trip_script_component() {
        let json = r#"{
            "schema_version": 1,
            "components": {
                "core/git": {
                    "spec_version": 1,
                    "mode": "script",
                    "description": "Git version control system",
                    "source_dir": "/home/user/loadout/components/git",
                    "dep": {
                        "depends": [],
                        "requires": [{ "name": "package_manager" }],
                        "provides": []
                    }
                }
            }
        }"#;
        let fi: ComponentIndex = serde_json::from_str(json).unwrap();
        assert_eq!(fi.schema_version, 1);
        let meta = &fi.components["core/git"];
        assert_eq!(meta.mode, ComponentMode::Script);
        assert_eq!(meta.dep.requires[0].name, "package_manager");
        assert!(meta.spec.is_none());
    }

    #[test]
    fn round_trip_declarative_component() {
        let json = r#"{
            "schema_version": 1,
            "components": {
                "core/neovim": {
                    "spec_version": 1,
                    "mode": "declarative",
                    "source_dir": "/home/user/loadout/components/neovim",
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
        let fi: ComponentIndex = serde_json::from_str(json).unwrap();
        let meta = &fi.components["core/neovim"];
        assert_eq!(meta.mode, ComponentMode::Declarative);
        let spec = meta.spec.as_ref().unwrap();
        assert_eq!(spec.resources.len(), 2);
        match &spec.resources[0].kind {
            SpecResourceKind::Package { name } => assert_eq!(name, "neovim"),
            _ => panic!("expected package"),
        }
    }
}
