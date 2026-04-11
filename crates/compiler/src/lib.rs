//! Component compiler: resolves backends and produces DesiredResourceGraph.
//!
//! The compiler takes a ComponentIndex, Strategy, and ResolvedComponentOrder and produces
//! a DesiredResourceGraph with all `desired_backend` fields resolved. This is the
//! only place where backend resolution happens; Planner must not re-resolve backends.
//!
//! See: `docs/specs/algorithms/compiler.md`

use std::collections::HashMap;

use model::component_index::FsOp as SpecFsOp;
use model::component_index::{ComponentIndex, ComponentMode, SpecFsEntryType, SpecResourceKind};
use model::desired_resource_graph::{
    ComponentDesiredResources, DesiredResource, DesiredResourceGraph, DesiredResourceKind,
    FsEntryType, FsOp, DESIRED_RESOURCE_GRAPH_SCHEMA_VERSION,
};
use model::id::{CanonicalBackendId, ResolvedComponentOrder};
use model::strategy::{BackendStrategy, Strategy};

pub use model::desired_resource_graph::{
    DesiredResource as CompiledResource, DesiredResourceGraph as CompiledGraph,
    DesiredResourceKind as CompiledResourceKind,
};

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors produced by the compiler.
#[derive(Debug, thiserror::Error)]
pub enum CompilerError {
    /// A component referenced in `resolved_order` was not found in `component_index`.
    /// This indicates a programming error upstream (resolver output inconsistent with index).
    #[error("component not found in index: {id}")]
    ComponentNotFound { id: String },

    /// No backend could be resolved for a resource.
    /// Either `strategy.<kind>.default_backend` is absent and there is no matching override,
    /// or the strategy section itself is absent.
    #[error(
        "no backend for {kind} resource '{resource_id}' in component '{component_id}': \
         add a default_backend or an override in strategy"
    )]
    NoBackend {
        component_id: String,
        resource_id: String,
        kind: String,
    },
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Compile a ComponentIndex into a DesiredResourceGraph.
///
/// Processes components in `resolved_order`. Script-mode components are excluded from
/// the output (they have no resources to compile). For declarative-mode components,
/// each resource's `desired_backend` is resolved via `strategy`.
///
/// # Errors
///
/// Returns [`CompilerError::ComponentNotFound`] if a component ID in `resolved_order`
/// is missing from `component_index`. Returns [`CompilerError::NoBackend`] if no backend
/// can be resolved for a package or runtime resource.
pub fn compile(
    component_index: &ComponentIndex,
    strategy: &Strategy,
    resolved_order: &ResolvedComponentOrder,
) -> Result<DesiredResourceGraph, CompilerError> {
    let mut components: HashMap<String, ComponentDesiredResources> = HashMap::new();

    for component_id in resolved_order {
        let id_str = component_id.as_str();

        let meta = component_index.components.get(id_str).ok_or_else(|| {
            CompilerError::ComponentNotFound {
                id: id_str.to_string(),
            }
        })?;

        // Script-mode components have no declarative resources.
        // They are still recorded in the graph with an empty list so the planner
        // can classify them as create / destroy / noop.
        if meta.mode == ComponentMode::Script {
            components.insert(
                id_str.to_string(),
                ComponentDesiredResources { resources: vec![] },
            );
            continue;
        }

        // Declarative mode: expand spec resources into desired resources.
        let spec = match &meta.spec {
            Some(s) => s,
            None => {
                // Should not occur after component-index validation, but handle gracefully.
                components.insert(
                    id_str.to_string(),
                    ComponentDesiredResources { resources: vec![] },
                );
                continue;
            }
        };

        let mut resources: Vec<DesiredResource> = Vec::new();
        for resource in &spec.resources {
            let kind = compile_resource(resource, strategy, id_str)?;
            resources.push(DesiredResource {
                id: resource.id.clone(),
                kind,
            });
        }

        components.insert(id_str.to_string(), ComponentDesiredResources { resources });
    }

    Ok(DesiredResourceGraph {
        schema_version: DESIRED_RESOURCE_GRAPH_SCHEMA_VERSION,
        components,
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Compile a single SpecResource into a DesiredResourceKind.
fn compile_resource(
    resource: &model::component_index::SpecResource,
    strategy: &Strategy,
    component_id: &str,
) -> Result<DesiredResourceKind, CompilerError> {
    match &resource.kind {
        SpecResourceKind::Package { name } => {
            let backend = resolve_backend(
                strategy.package.as_ref(),
                name,
                component_id,
                &resource.id,
                "package",
            )?;
            Ok(DesiredResourceKind::Package {
                name: name.clone(),
                desired_backend: backend,
            })
        }

        SpecResourceKind::Runtime { name, version } => {
            let backend = resolve_backend(
                strategy.runtime.as_ref(),
                name,
                component_id,
                &resource.id,
                "runtime",
            )?;
            Ok(DesiredResourceKind::Runtime {
                name: name.clone(),
                version: version.clone(),
                desired_backend: backend,
            })
        }

        SpecResourceKind::Fs {
            source,
            path,
            entry_type,
            op,
        } => Ok(DesiredResourceKind::Fs {
            source: source.clone(),
            path: path.clone(),
            entry_type: map_entry_type(entry_type.clone()),
            op: map_fs_op(op.clone()),
        }),
    }
}

/// Resolve a backend ID from a strategy section by checking overrides first, then default.
fn resolve_backend(
    strategy_section: Option<&BackendStrategy>,
    resource_name: &str,
    component_id: &str,
    resource_id: &str,
    kind_name: &str,
) -> Result<CanonicalBackendId, CompilerError> {
    let no_backend = || CompilerError::NoBackend {
        component_id: component_id.to_string(),
        resource_id: resource_id.to_string(),
        kind: kind_name.to_string(),
    };

    let bp = strategy_section.ok_or_else(no_backend)?;

    // Per-resource override takes priority over default.
    if let Some(entry) = bp.overrides.get(resource_name) {
        return CanonicalBackendId::new(&entry.backend).map_err(|_| no_backend());
    }

    // Fall back to default_backend.
    let default = bp.default_backend.as_deref().ok_or_else(no_backend)?;
    CanonicalBackendId::new(default).map_err(|_| no_backend())
}

/// Convert SpecFsEntryType (component spec) to FsEntryType (desired resource graph).
fn map_entry_type(t: SpecFsEntryType) -> FsEntryType {
    match t {
        SpecFsEntryType::File => FsEntryType::File,
        SpecFsEntryType::Dir => FsEntryType::Dir,
    }
}

/// Convert FsOp from the component spec namespace to the desired resource graph namespace.
fn map_fs_op(op: SpecFsOp) -> FsOp {
    match op {
        SpecFsOp::Link => FsOp::Link,
        SpecFsOp::Copy => FsOp::Copy,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use model::component_index::FsOp as SpecFsOp;
    use model::component_index::{
        ComponentMeta, ComponentMode, ComponentSpec, DepSpec, SpecFsEntryType, SpecResource,
        SpecResourceKind, COMPONENT_INDEX_SCHEMA_VERSION,
    };
    use model::id::CanonicalComponentId;
    use model::strategy::{BackendOverride, BackendStrategy, Strategy};
    use std::collections::HashMap;

    // --- Builder helpers ----------------------------------------------------

    fn make_component_id(s: &str) -> CanonicalComponentId {
        CanonicalComponentId::new(s).unwrap()
    }

    fn make_index(components: Vec<(&str, ComponentMeta)>) -> ComponentIndex {
        ComponentIndex {
            schema_version: COMPONENT_INDEX_SCHEMA_VERSION,
            components: components
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        }
    }

    fn script_meta() -> ComponentMeta {
        ComponentMeta {
            spec_version: 1,
            mode: ComponentMode::Script,
            description: None,
            source_dir: "/tmp/feat".to_string(),
            dep: DepSpec::default(),
            spec: None,
        }
    }

    fn declarative_meta(resources: Vec<SpecResource>) -> ComponentMeta {
        ComponentMeta {
            spec_version: 1,
            mode: ComponentMode::Declarative,
            description: None,
            source_dir: "/tmp/feat".to_string(),
            dep: DepSpec::default(),
            spec: Some(ComponentSpec { resources }),
        }
    }

    fn package_resource(id: &str, name: &str) -> SpecResource {
        SpecResource {
            id: id.to_string(),
            kind: SpecResourceKind::Package {
                name: name.to_string(),
            },
        }
    }

    fn runtime_resource(id: &str, name: &str, version: &str) -> SpecResource {
        SpecResource {
            id: id.to_string(),
            kind: SpecResourceKind::Runtime {
                name: name.to_string(),
                version: version.to_string(),
            },
        }
    }

    fn fs_resource(
        id: &str,
        path: &str,
        entry_type: SpecFsEntryType,
        op: SpecFsOp,
    ) -> SpecResource {
        SpecResource {
            id: id.to_string(),
            kind: SpecResourceKind::Fs {
                source: None,
                path: path.to_string(),
                entry_type,
                op,
            },
        }
    }

    fn backend_strategy_default(default: &str) -> BackendStrategy {
        BackendStrategy {
            default_backend: Some(default.to_string()),
            overrides: HashMap::new(),
        }
    }

    fn backend_strategy_with_override(default: &str, name: &str, backend: &str) -> BackendStrategy {
        let mut overrides = HashMap::new();
        overrides.insert(
            name.to_string(),
            BackendOverride {
                backend: backend.to_string(),
            },
        );
        BackendStrategy {
            default_backend: Some(default.to_string()),
            overrides,
        }
    }

    // --- Tests --------------------------------------------------------------

    /// Script-mode components are included in the output graph with empty resources.
    #[test]
    fn script_component_is_included_with_empty_resources() {
        let index = make_index(vec![("core/bash", script_meta())]);
        let strategy = Strategy::default();
        let order = vec![make_component_id("core/bash")];

        let graph = compile(&index, &strategy, &order).unwrap();
        assert_eq!(graph.components.len(), 1);
        assert!(graph.components["core/bash"].resources.is_empty());
    }

    /// Declarative package resource resolves backend from default_backend.
    #[test]
    fn package_resolved_from_default_backend() {
        let index = make_index(vec![(
            "core/git",
            declarative_meta(vec![package_resource("package:git", "git")]),
        )]);
        let strategy = Strategy {
            package: Some(backend_strategy_default("core/brew")),
            ..Default::default()
        };
        let order = vec![make_component_id("core/git")];

        let graph = compile(&index, &strategy, &order).unwrap();
        let resources = &graph.components["core/git"].resources;
        assert_eq!(resources.len(), 1);
        match &resources[0].kind {
            DesiredResourceKind::Package {
                name,
                desired_backend,
            } => {
                assert_eq!(name, "git");
                assert_eq!(desired_backend.as_str(), "core/brew");
            }
            _ => panic!("expected Package"),
        }
    }

    /// Per-resource override takes precedence over default_backend.
    #[test]
    fn package_override_wins_over_default() {
        let index = make_index(vec![(
            "core/ripgrep",
            declarative_meta(vec![package_resource("package:ripgrep", "ripgrep")]),
        )]);
        let strategy = Strategy {
            package: Some(backend_strategy_with_override(
                "core/brew",
                "ripgrep",
                "core/cargo",
            )),
            ..Default::default()
        };
        let order = vec![make_component_id("core/ripgrep")];

        let graph = compile(&index, &strategy, &order).unwrap();
        match &graph.components["core/ripgrep"].resources[0].kind {
            DesiredResourceKind::Package {
                desired_backend, ..
            } => {
                assert_eq!(desired_backend.as_str(), "core/cargo");
            }
            _ => panic!("expected Package"),
        }
    }

    /// Runtime resource resolves backend from default_backend.
    #[test]
    fn runtime_resolved_from_default_backend() {
        let index = make_index(vec![(
            "core/node",
            declarative_meta(vec![runtime_resource("runtime:node", "node", "20")]),
        )]);
        let strategy = Strategy {
            runtime: Some(backend_strategy_default("core/mise")),
            ..Default::default()
        };
        let order = vec![make_component_id("core/node")];

        let graph = compile(&index, &strategy, &order).unwrap();
        match &graph.components["core/node"].resources[0].kind {
            DesiredResourceKind::Runtime {
                name,
                version,
                desired_backend,
            } => {
                assert_eq!(name, "node");
                assert_eq!(version, "20");
                assert_eq!(desired_backend.as_str(), "core/mise");
            }
            _ => panic!("expected Runtime"),
        }
    }

    /// Runtime resource resolves backend from a per-runtime override.
    #[test]
    fn runtime_override_wins_over_default() {
        let index = make_index(vec![(
            "core/python",
            declarative_meta(vec![runtime_resource("runtime:python", "python", "3.12")]),
        )]);
        let strategy = Strategy {
            runtime: Some(backend_strategy_with_override(
                "core/mise",
                "python",
                "core/uv",
            )),
            ..Default::default()
        };
        let order = vec![make_component_id("core/python")];

        let graph = compile(&index, &strategy, &order).unwrap();
        match &graph.components["core/python"].resources[0].kind {
            DesiredResourceKind::Runtime {
                desired_backend, ..
            } => {
                assert_eq!(desired_backend.as_str(), "core/uv");
            }
            _ => panic!("expected Runtime"),
        }
    }

    /// Fs resource is passed through without backend resolution (File + Link).
    #[test]
    fn fs_resource_file_link_no_backend() {
        let index = make_index(vec![(
            "core/git",
            declarative_meta(vec![fs_resource(
                "fs:gitconfig",
                "~/.gitconfig",
                SpecFsEntryType::File,
                SpecFsOp::Link,
            )]),
        )]);
        let strategy = Strategy::default();
        let order = vec![make_component_id("core/git")];

        let graph = compile(&index, &strategy, &order).unwrap();
        match &graph.components["core/git"].resources[0].kind {
            DesiredResourceKind::Fs {
                path,
                entry_type,
                op,
                source,
            } => {
                assert_eq!(path, "~/.gitconfig");
                assert_eq!(*entry_type, FsEntryType::File);
                assert_eq!(*op, FsOp::Link);
                assert!(source.is_none());
            }
            _ => panic!("expected Fs"),
        }
    }

    /// Fs resource Dir + Copy variant is mapped correctly.
    #[test]
    fn fs_resource_dir_copy() {
        let index = make_index(vec![(
            "core/nvim",
            declarative_meta(vec![fs_resource(
                "fs:nvim-config",
                "~/.config/nvim",
                SpecFsEntryType::Dir,
                SpecFsOp::Copy,
            )]),
        )]);
        let strategy = Strategy::default();
        let order = vec![make_component_id("core/nvim")];

        let graph = compile(&index, &strategy, &order).unwrap();
        match &graph.components["core/nvim"].resources[0].kind {
            DesiredResourceKind::Fs { entry_type, op, .. } => {
                assert_eq!(*entry_type, FsEntryType::Dir);
                assert_eq!(*op, FsOp::Copy);
            }
            _ => panic!("expected Fs"),
        }
    }

    /// No default_backend and no matching override → NoBackend error.
    #[test]
    fn no_backend_returns_error() {
        let index = make_index(vec![(
            "core/git",
            declarative_meta(vec![package_resource("package:git", "git")]),
        )]);
        let strategy = Strategy {
            package: Some(BackendStrategy {
                default_backend: None,
                overrides: HashMap::new(),
            }),
            ..Default::default()
        };
        let order = vec![make_component_id("core/git")];

        let err = compile(&index, &strategy, &order).unwrap_err();
        assert!(matches!(err, CompilerError::NoBackend { .. }));
    }

    /// Absent strategy section for the resource kind → NoBackend error.
    #[test]
    fn absent_strategy_section_returns_no_backend() {
        let index = make_index(vec![(
            "core/node",
            declarative_meta(vec![runtime_resource("runtime:node", "node", "20")]),
        )]);
        // strategy.runtime is None
        let strategy = Strategy {
            package: Some(backend_strategy_default("core/brew")),
            ..Default::default()
        };
        let order = vec![make_component_id("core/node")];

        let err = compile(&index, &strategy, &order).unwrap_err();
        assert!(matches!(err, CompilerError::NoBackend { .. }));
    }

    /// Component referenced in resolved_order but absent from index → ComponentNotFound.
    #[test]
    fn component_not_in_index_returns_error() {
        let index = make_index(vec![]);
        let strategy = Strategy::default();
        let order = vec![make_component_id("core/missing")];

        let err = compile(&index, &strategy, &order).unwrap_err();
        assert!(matches!(err, CompilerError::ComponentNotFound { id } if id == "core/missing"));
    }

    /// Multiple components in resolved_order are all compiled into the graph.
    #[test]
    fn multiple_components_all_compiled() {
        let index = make_index(vec![
            (
                "core/git",
                declarative_meta(vec![package_resource("package:git", "git")]),
            ),
            (
                "core/node",
                declarative_meta(vec![runtime_resource("runtime:node", "node", "20")]),
            ),
            ("core/bash", script_meta()),
        ]);
        let strategy = Strategy {
            package: Some(backend_strategy_default("core/brew")),
            runtime: Some(backend_strategy_default("core/mise")),
            ..Default::default()
        };
        let order = vec![
            make_component_id("core/git"),
            make_component_id("core/bash"), // script: skipped
            make_component_id("core/node"),
        ];

        let graph = compile(&index, &strategy, &order).unwrap();
        // bash is now included with empty resources; git and node have resources
        assert_eq!(graph.components.len(), 3);
        assert!(graph.components.contains_key("core/git"));
        assert!(graph.components.contains_key("core/node"));
        assert!(graph.components["core/bash"].resources.is_empty());
    }

    /// Schema version in output is always the canonical constant.
    #[test]
    fn output_schema_version_is_canonical() {
        let index = make_index(vec![("core/bash", script_meta())]);
        let strategy = Strategy::default();
        let order = vec![make_component_id("core/bash")];

        let graph = compile(&index, &strategy, &order).unwrap();
        assert_eq!(graph.schema_version, DESIRED_RESOURCE_GRAPH_SCHEMA_VERSION);
    }
}
