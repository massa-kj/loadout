//! Component compiler: resolves backends and produces DesiredResourceGraph.
//!
//! The compiler takes a ComponentIndex, Strategy, ResolvedComponentOrder, and
//! pre-materialized fs sources, then produces a DesiredResourceGraph with all
//! `desired_backend` fields resolved and fs sources concretized.
//!
//! Backend resolution is the only place it happens; Planner must not re-resolve backends.
//! Fs source resolution is handled by the materializer before compilation.
//!
//! See: `docs/specs/algorithms/compiler.md`

use std::collections::HashMap;

use model::component_index::FsOp as SpecFsOp;
use model::component_index::{ComponentIndex, ComponentMode, SpecFsEntryType, SpecResourceKind};
use model::desired_resource_graph::{
    ComponentDesiredResources, DesiredResource, DesiredResourceGraph, DesiredResourceKind,
    FsEntryType, FsOp, DESIRED_RESOURCE_GRAPH_SCHEMA_VERSION,
};
use model::fs::ConcreteFsSource;
use model::id::{CanonicalBackendId, ResolvedComponentOrder};
use model::params::MaterializedComponentSpec;
use model::strategy::{MatchKind, Strategy};

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

    /// No backend rule matched any rule in strategy for this resource.
    #[error(
        "no backend for {kind} resource '{resource_id}' in component '{component_id}': \
         no matching rule found in strategy; add a rule with 'kind: {kind}' to strategy.rules"
    )]
    NoBackend {
        component_id: String,
        resource_id: String,
        kind: String,
    },

    /// An fs resource is missing its materialized source entry.
    /// This indicates a programming error: the materializer must produce an entry
    /// for every fs resource before compilation.
    #[error(
        "missing materialized source for fs resource '{resource_id}' in component '{component_id}'"
    )]
    MissingMaterializedSource {
        component_id: String,
        resource_id: String,
    },
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Materialized fs source data keyed by `(component_id, resource_id)`.
pub type MaterializedSources = HashMap<(String, String), MaterializedFsResource>;

/// Materialized data for a single fs resource, produced by the materializer.
pub struct MaterializedFsResource {
    /// Fully resolved source reference.
    pub source: ConcreteFsSource,
    /// Content fingerprint if eligible and computed.
    pub source_fingerprint: Option<String>,
    /// Target path with `~` expanded to absolute form.
    pub expanded_path: String,
}

/// Compile a ComponentIndex into a DesiredResourceGraph.
///
/// Processes components in `resolved_order`. Script-mode components are excluded from
/// the output (they have no resources to compile). For declarative-mode components,
/// each resource's `desired_backend` is resolved via `strategy`.
///
/// `materialized_specs` provides param-resolved specs for components that declare
/// `params_schema`. When present, the compiler uses the materialized resources
/// instead of the raw spec resources. Components without params use their original spec.
///
/// `materialized_sources` provides pre-resolved fs source references produced by
/// the materialize stage. The compiler looks up materialized data for each fs resource
/// by `(component_id, resource_id)`.
///
/// # Errors
///
/// Returns [`CompilerError::ComponentNotFound`] if a component ID in `resolved_order`
/// is missing from `component_index`. Returns [`CompilerError::NoBackend`] if no backend
/// can be resolved for a package or runtime resource.
pub fn compile(
    component_index: &ComponentIndex,
    materialized_specs: &HashMap<String, MaterializedComponentSpec>,
    strategy: &Strategy,
    resolved_order: &ResolvedComponentOrder,
    materialized_sources: &MaterializedSources,
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

        // ManagedScript components declare `tool` resources that are compiled like declarative
        // resources. Scripts handle install/uninstall; core handles verification and state.

        // Declarative mode: expand spec resources into desired resources.
        // Use materialized spec (param-resolved) if available, otherwise fall back to raw spec.
        let resources = if let Some(ms) = materialized_specs.get(id_str) {
            &ms.resources
        } else {
            match &meta.spec {
                Some(s) => &s.resources,
                None => {
                    // Should not occur after component-index validation, but handle gracefully.
                    components.insert(
                        id_str.to_string(),
                        ComponentDesiredResources { resources: vec![] },
                    );
                    continue;
                }
            }
        };

        let mut compiled: Vec<DesiredResource> = Vec::new();
        for resource in resources {
            let kind = compile_resource(resource, strategy, id_str, materialized_sources)?;
            compiled.push(DesiredResource {
                id: resource.id.clone(),
                kind,
            });
        }

        components.insert(
            id_str.to_string(),
            ComponentDesiredResources {
                resources: compiled,
            },
        );
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
    materialized_sources: &MaterializedSources,
) -> Result<DesiredResourceKind, CompilerError> {
    match &resource.kind {
        SpecResourceKind::Package { name, version } => {
            let backend = resolve_backend_from_rules(
                strategy,
                &MatchKind::Package,
                name,
                component_id,
                &resource.id,
            )?;
            Ok(DesiredResourceKind::Package {
                name: name.clone(),
                version: version.clone(),
                desired_backend: backend,
            })
        }

        SpecResourceKind::Runtime { name, version } => {
            let backend = resolve_backend_from_rules(
                strategy,
                &MatchKind::Runtime,
                name,
                component_id,
                &resource.id,
            )?;
            Ok(DesiredResourceKind::Runtime {
                name: name.clone(),
                version: version.clone(),
                desired_backend: backend,
            })
        }

        SpecResourceKind::Fs { entry_type, op, .. } => {
            let key = (component_id.to_string(), resource.id.clone());
            let materialized = materialized_sources.get(&key).ok_or_else(|| {
                CompilerError::MissingMaterializedSource {
                    component_id: component_id.to_string(),
                    resource_id: resource.id.clone(),
                }
            })?;
            Ok(DesiredResourceKind::Fs {
                source: materialized.source.clone(),
                path: materialized.expanded_path.clone(),
                entry_type: map_entry_type(entry_type.clone()),
                op: map_fs_op(op.clone()),
                source_fingerprint: materialized.source_fingerprint.clone(),
            })
        }

        // Tool resources have no backend; identity verify and observed facts are core-managed.
        SpecResourceKind::Tool { name, verify } => Ok(DesiredResourceKind::Tool {
            name: name.clone(),
            verify: verify.clone(),
        }),
    }
}

/// Resolve a backend ID from strategy rules using fixed-priority specificity comparison.
///
/// All rules are evaluated against the resource. The most specific matching rule wins.
/// Equal specificity is broken by rule index — last matching rule (highest index) wins.
///
/// Specificity is determined by `MatchSelector::specificity()`, which returns a
/// `(has_component, has_kind, has_name, has_group)` tuple compared lexicographically.
/// This guarantees `component` always outranks `name`, etc., regardless of other axes.
///
/// Returns `CompilerError::NoBackend` if no rule matches.
fn resolve_backend_from_rules(
    strategy: &Strategy,
    resource_kind: &MatchKind,
    resource_name: &str,
    component_id: &str,
    resource_id: &str,
) -> Result<CanonicalBackendId, CompilerError> {
    let kind_str = resource_kind.as_str();

    // Evaluate all rules; collect matching (index, specificity) pairs.
    let winner = strategy
        .rules
        .iter()
        .enumerate()
        .filter_map(|(i, rule)| {
            // Compute group membership only when the selector references a group.
            let group_member = match &rule.selector.group {
                None => false, // matches() treats None group field as always-pass
                Some(group_name) => strategy
                    .groups
                    .get(group_name)
                    .and_then(|g| g.names_for_kind(kind_str))
                    .map(|names| names.contains(&resource_name.to_string()))
                    .unwrap_or(false),
            };

            if rule
                .selector
                .matches(resource_kind, resource_name, component_id, group_member)
            {
                Some((i, rule.selector.specificity()))
            } else {
                None
            }
        })
        // Max by specificity first; tie-break by index (last wins = higher index).
        .max_by(|(i_a, spec_a), (i_b, spec_b)| spec_a.cmp(spec_b).then(i_a.cmp(i_b)));

    let no_backend = || CompilerError::NoBackend {
        component_id: component_id.to_string(),
        resource_id: resource_id.to_string(),
        kind: kind_str.to_string(),
    };

    match winner {
        Some((i, _)) => {
            let backend_str = &strategy.rules[i].use_backend;
            CanonicalBackendId::new(backend_str).map_err(|_| no_backend())
        }
        None => Err(no_backend()),
    }
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
    use model::fs::ConcreteFsSource;
    use model::id::CanonicalComponentId;
    use model::params::MaterializedComponentSpec;
    use model::strategy::{MatchKind, MatchSelector, Strategy, StrategyGroup, StrategyRule};
    use std::collections::HashMap;
    use std::path::PathBuf;

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

    fn empty_ms() -> MaterializedSources {
        HashMap::new()
    }

    fn empty_mcs() -> HashMap<String, MaterializedComponentSpec> {
        HashMap::new()
    }

    fn make_ms(entries: Vec<(&str, &str, ConcreteFsSource, &str)>) -> MaterializedSources {
        entries
            .into_iter()
            .map(|(comp, res, source, expanded_path)| {
                (
                    (comp.to_string(), res.to_string()),
                    MaterializedFsResource {
                        source,
                        source_fingerprint: None,
                        expanded_path: expanded_path.to_string(),
                    },
                )
            })
            .collect()
    }

    fn script_meta() -> ComponentMeta {
        ComponentMeta {
            spec_version: 1,
            mode: ComponentMode::Script,
            description: None,
            source_dir: "/tmp/feat".to_string(),
            dep: DepSpec::default(),
            spec: None,
            scripts: None,
            params_schema: None,
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
            scripts: None,
            params_schema: None,
        }
    }

    fn package_resource(id: &str, name: &str) -> SpecResource {
        SpecResource {
            id: id.to_string(),
            kind: SpecResourceKind::Package {
                name: name.to_string(),
                version: None,
            },
            for_each: None,
        }
    }

    fn runtime_resource(id: &str, name: &str, version: &str) -> SpecResource {
        SpecResource {
            id: id.to_string(),
            kind: SpecResourceKind::Runtime {
                name: name.to_string(),
                version: version.to_string(),
            },
            for_each: None,
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
            for_each: None,
        }
    }

    /// Build a rule that matches all resources of the given kind.
    fn rule_kind(kind: MatchKind, backend: &str) -> StrategyRule {
        StrategyRule {
            selector: MatchSelector {
                kind: Some(kind),
                ..Default::default()
            },
            use_backend: backend.to_string(),
        }
    }

    /// Build a rule that matches kind + exact name.
    fn rule_kind_name(kind: MatchKind, name: &str, backend: &str) -> StrategyRule {
        StrategyRule {
            selector: MatchSelector {
                kind: Some(kind),
                name: Some(name.to_string()),
                ..Default::default()
            },
            use_backend: backend.to_string(),
        }
    }

    /// Build a rule that matches kind + group membership.
    fn rule_kind_group(kind: MatchKind, group: &str, backend: &str) -> StrategyRule {
        StrategyRule {
            selector: MatchSelector {
                kind: Some(kind),
                group: Some(group.to_string()),
                ..Default::default()
            },
            use_backend: backend.to_string(),
        }
    }

    /// Build a rule that matches component + kind.
    fn rule_component_kind(component: &str, kind: MatchKind, backend: &str) -> StrategyRule {
        StrategyRule {
            selector: MatchSelector {
                component: Some(component.to_string()),
                kind: Some(kind),
                ..Default::default()
            },
            use_backend: backend.to_string(),
        }
    }

    fn strategy_with_rules(rules: Vec<StrategyRule>) -> Strategy {
        Strategy {
            rules,
            ..Default::default()
        }
    }

    // --- Tests --------------------------------------------------------------

    /// Script-mode components are included in the output graph with empty resources.
    #[test]
    fn script_component_is_included_with_empty_resources() {
        let index = make_index(vec![("core/bash", script_meta())]);
        let strategy = Strategy::default();
        let order = vec![make_component_id("core/bash")];

        let graph = compile(&index, &empty_mcs(), &strategy, &order, &empty_ms()).unwrap();
        assert_eq!(graph.components.len(), 1);
        assert!(graph.components["core/bash"].resources.is_empty());
    }

    /// Declarative package resource resolves backend from a kind rule.
    #[test]
    fn package_resolved_from_kind_rule() {
        let index = make_index(vec![(
            "core/git",
            declarative_meta(vec![package_resource("package:git", "git")]),
        )]);
        let strategy = strategy_with_rules(vec![rule_kind(MatchKind::Package, "core/brew")]);
        let order = vec![make_component_id("core/git")];

        let graph = compile(&index, &empty_mcs(), &strategy, &order, &empty_ms()).unwrap();
        let resources = &graph.components["core/git"].resources;
        assert_eq!(resources.len(), 1);
        match &resources[0].kind {
            DesiredResourceKind::Package {
                name,
                desired_backend,
                ..
            } => {
                assert_eq!(name, "git");
                assert_eq!(desired_backend.as_str(), "core/brew");
            }
            _ => panic!("expected Package"),
        }
    }

    /// More specific kind+name rule wins over kind-only rule.
    #[test]
    fn name_rule_wins_over_kind_rule() {
        let index = make_index(vec![(
            "core/ripgrep",
            declarative_meta(vec![package_resource("package:ripgrep", "ripgrep")]),
        )]);
        let strategy = strategy_with_rules(vec![
            rule_kind(MatchKind::Package, "core/brew"),
            rule_kind_name(MatchKind::Package, "ripgrep", "core/cargo"),
        ]);
        let order = vec![make_component_id("core/ripgrep")];

        let graph = compile(&index, &empty_mcs(), &strategy, &order, &empty_ms()).unwrap();
        match &graph.components["core/ripgrep"].resources[0].kind {
            DesiredResourceKind::Package {
                desired_backend, ..
            } => {
                assert_eq!(desired_backend.as_str(), "core/cargo");
            }
            _ => panic!("expected Package"),
        }
    }

    /// Runtime resource resolves backend from a kind rule.
    #[test]
    fn runtime_resolved_from_kind_rule() {
        let index = make_index(vec![(
            "core/node",
            declarative_meta(vec![runtime_resource("runtime:node", "node", "20")]),
        )]);
        let strategy = strategy_with_rules(vec![rule_kind(MatchKind::Runtime, "core/mise")]);
        let order = vec![make_component_id("core/node")];

        let graph = compile(&index, &empty_mcs(), &strategy, &order, &empty_ms()).unwrap();
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

    /// More specific kind+name runtime rule wins over kind-only rule.
    #[test]
    fn runtime_name_rule_wins_over_kind_rule() {
        let index = make_index(vec![(
            "core/python",
            declarative_meta(vec![runtime_resource("runtime:python", "python", "3.12")]),
        )]);
        let strategy = strategy_with_rules(vec![
            rule_kind(MatchKind::Runtime, "core/mise"),
            rule_kind_name(MatchKind::Runtime, "python", "core/uv"),
        ]);
        let order = vec![make_component_id("core/python")];

        let graph = compile(&index, &empty_mcs(), &strategy, &order, &empty_ms()).unwrap();
        match &graph.components["core/python"].resources[0].kind {
            DesiredResourceKind::Runtime {
                desired_backend, ..
            } => {
                assert_eq!(desired_backend.as_str(), "core/uv");
            }
            _ => panic!("expected Runtime"),
        }
    }

    /// component+kind rule wins over kind+name rule (component dominates name).
    #[test]
    fn component_kind_rule_wins_over_kind_name_rule() {
        let index = make_index(vec![(
            "core/cli-tools",
            declarative_meta(vec![package_resource("package:eslint", "eslint")]),
        )]);
        // kind+name specificity = (0,1,1,0); component+kind specificity = (1,1,0,0)
        // component dominates name → component+kind wins.
        let strategy = strategy_with_rules(vec![
            rule_kind_name(MatchKind::Package, "eslint", "core/npm-global"),
            rule_component_kind("core/cli-tools", MatchKind::Package, "core/npm"),
        ]);
        let order = vec![make_component_id("core/cli-tools")];

        let graph = compile(&index, &empty_mcs(), &strategy, &order, &empty_ms()).unwrap();
        match &graph.components["core/cli-tools"].resources[0].kind {
            DesiredResourceKind::Package {
                desired_backend, ..
            } => {
                assert_eq!(desired_backend.as_str(), "core/npm");
            }
            _ => panic!("expected Package"),
        }
    }

    /// Among equal-specificity rules, the last one (higher index) wins.
    #[test]
    fn tie_break_last_rule_wins() {
        let index = make_index(vec![(
            "core/git",
            declarative_meta(vec![package_resource("package:git", "git")]),
        )]);
        // Two kind-only rules — same specificity (0,1,0,0); last wins.
        let strategy = strategy_with_rules(vec![
            rule_kind(MatchKind::Package, "core/brew"),
            rule_kind(MatchKind::Package, "core/apt"),
        ]);
        let order = vec![make_component_id("core/git")];

        let graph = compile(&index, &empty_mcs(), &strategy, &order, &empty_ms()).unwrap();
        match &graph.components["core/git"].resources[0].kind {
            DesiredResourceKind::Package {
                desired_backend, ..
            } => {
                assert_eq!(desired_backend.as_str(), "core/apt");
            }
            _ => panic!("expected Package"),
        }
    }

    /// Group-based rule selects resources listed in the group.
    #[test]
    fn group_rule_selects_group_members() {
        let index = make_index(vec![(
            "core/node",
            declarative_meta(vec![package_resource("package:eslint", "eslint")]),
        )]);
        let mut group_inner = HashMap::new();
        group_inner.insert(
            "package".to_string(),
            vec!["eslint".to_string(), "prettier".to_string()],
        );
        let mut groups = HashMap::new();
        groups.insert("npm_global".to_string(), StrategyGroup(group_inner));
        // kind rule has lower specificity; group rule (kind+group) has higher for group members.
        let strategy = Strategy {
            groups,
            rules: vec![
                rule_kind(MatchKind::Package, "core/brew"),
                rule_kind_group(MatchKind::Package, "npm_global", "core/npm"),
            ],
            ..Default::default()
        };
        let order = vec![make_component_id("core/node")];

        let graph = compile(&index, &empty_mcs(), &strategy, &order, &empty_ms()).unwrap();
        match &graph.components["core/node"].resources[0].kind {
            DesiredResourceKind::Package {
                desired_backend, ..
            } => {
                assert_eq!(desired_backend.as_str(), "core/npm");
            }
            _ => panic!("expected Package"),
        }
    }

    /// Group rule does not match resources not listed in the group.
    #[test]
    fn group_rule_does_not_match_non_members() {
        let index = make_index(vec![(
            "core/cli",
            declarative_meta(vec![package_resource("package:git", "git")]),
        )]);
        let mut group_inner = HashMap::new();
        group_inner.insert("package".to_string(), vec!["eslint".to_string()]);
        let mut groups = HashMap::new();
        groups.insert("npm_global".to_string(), StrategyGroup(group_inner));
        let strategy = Strategy {
            groups,
            rules: vec![
                rule_kind(MatchKind::Package, "core/brew"),
                rule_kind_group(MatchKind::Package, "npm_global", "core/npm"),
            ],
            ..Default::default()
        };
        let order = vec![make_component_id("core/cli")];

        // "git" is not in npm_global → falls back to kind rule → core/brew.
        let graph = compile(&index, &empty_mcs(), &strategy, &order, &empty_ms()).unwrap();
        match &graph.components["core/cli"].resources[0].kind {
            DesiredResourceKind::Package {
                desired_backend, ..
            } => {
                assert_eq!(desired_backend.as_str(), "core/brew");
            }
            _ => panic!("expected Package"),
        }
    }

    /// Fs resource uses materialized source (File + Link).
    #[test]
    fn fs_resource_file_link_uses_materialized_source() {
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
        let ms = make_ms(vec![(
            "core/git",
            "fs:gitconfig",
            ConcreteFsSource::component_relative(PathBuf::from("/tmp/feat/files/.gitconfig")),
            "/root/.gitconfig",
        )]);

        let graph = compile(&index, &empty_mcs(), &strategy, &order, &ms).unwrap();
        match &graph.components["core/git"].resources[0].kind {
            DesiredResourceKind::Fs {
                source,
                path,
                entry_type,
                op,
                ..
            } => {
                assert_eq!(source.resolved, PathBuf::from("/tmp/feat/files/.gitconfig"));
                assert_eq!(path, "/root/.gitconfig");
                assert_eq!(*entry_type, FsEntryType::File);
                assert_eq!(*op, FsOp::Link);
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
        let ms = make_ms(vec![(
            "core/nvim",
            "fs:nvim-config",
            ConcreteFsSource::component_relative(PathBuf::from("/tmp/feat/files/nvim")),
            "/root/.config/nvim",
        )]);

        let graph = compile(&index, &empty_mcs(), &strategy, &order, &ms).unwrap();
        match &graph.components["core/nvim"].resources[0].kind {
            DesiredResourceKind::Fs { entry_type, op, .. } => {
                assert_eq!(*entry_type, FsEntryType::Dir);
                assert_eq!(*op, FsOp::Copy);
            }
            _ => panic!("expected Fs"),
        }
    }

    /// No rules in strategy → NoBackend error.
    #[test]
    fn no_rules_returns_no_backend_error() {
        let index = make_index(vec![(
            "core/git",
            declarative_meta(vec![package_resource("package:git", "git")]),
        )]);
        let strategy = Strategy::default();
        let order = vec![make_component_id("core/git")];

        let err = compile(&index, &empty_mcs(), &strategy, &order, &empty_ms()).unwrap_err();
        assert!(matches!(err, CompilerError::NoBackend { .. }));
    }

    /// Only a package rule exists; runtime resource → NoBackend error.
    #[test]
    fn no_matching_kind_rule_returns_no_backend() {
        let index = make_index(vec![(
            "core/node",
            declarative_meta(vec![runtime_resource("runtime:node", "node", "20")]),
        )]);
        let strategy = strategy_with_rules(vec![rule_kind(MatchKind::Package, "core/brew")]);
        let order = vec![make_component_id("core/node")];

        let err = compile(&index, &empty_mcs(), &strategy, &order, &empty_ms()).unwrap_err();
        assert!(matches!(err, CompilerError::NoBackend { kind, .. } if kind == "runtime"));
    }

    /// Component referenced in resolved_order but absent from index → ComponentNotFound.
    #[test]
    fn component_not_in_index_returns_error() {
        let index = make_index(vec![]);
        let strategy = Strategy::default();
        let order = vec![make_component_id("core/missing")];

        let err = compile(&index, &empty_mcs(), &strategy, &order, &empty_ms()).unwrap_err();
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
        let strategy = strategy_with_rules(vec![
            rule_kind(MatchKind::Package, "core/brew"),
            rule_kind(MatchKind::Runtime, "core/mise"),
        ]);
        let order = vec![
            make_component_id("core/git"),
            make_component_id("core/bash"), // script: included with empty resources
            make_component_id("core/node"),
        ];

        let graph = compile(&index, &empty_mcs(), &strategy, &order, &empty_ms()).unwrap();
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

        let graph = compile(&index, &empty_mcs(), &strategy, &order, &empty_ms()).unwrap();
        assert_eq!(graph.schema_version, DESIRED_RESOURCE_GRAPH_SCHEMA_VERSION);
    }

    /// When a materialized spec is provided, the compiler uses its resources
    /// instead of the raw spec from the component index.
    #[test]
    fn materialized_spec_overrides_raw_spec() {
        // Raw spec has runtime version "20" — materialized spec overrides to "22".
        let index = make_index(vec![(
            "core/node",
            declarative_meta(vec![runtime_resource(
                "runtime:node",
                "node",
                "${params.version}",
            )]),
        )]);
        let strategy = strategy_with_rules(vec![rule_kind(MatchKind::Runtime, "core/mise")]);
        let order = vec![make_component_id("core/node")];

        let mut mcs = HashMap::new();
        mcs.insert(
            "core/node".to_string(),
            MaterializedComponentSpec {
                resources: vec![runtime_resource("runtime:node", "node", "22")],
            },
        );

        let graph = compile(&index, &mcs, &strategy, &order, &empty_ms()).unwrap();
        match &graph.components["core/node"].resources[0].kind {
            DesiredResourceKind::Runtime { version, .. } => {
                assert_eq!(version, "22");
            }
            _ => panic!("expected Runtime"),
        }
    }

    /// Components without materialized spec fall back to raw spec as before.
    #[test]
    fn no_materialized_spec_falls_back_to_raw() {
        let index = make_index(vec![(
            "core/git",
            declarative_meta(vec![package_resource("package:git", "git")]),
        )]);
        let strategy = strategy_with_rules(vec![rule_kind(MatchKind::Package, "core/brew")]);
        let order = vec![make_component_id("core/git")];

        // Empty materialized specs — should use raw spec.
        let graph = compile(&index, &empty_mcs(), &strategy, &order, &empty_ms()).unwrap();
        assert_eq!(graph.components["core/git"].resources.len(), 1);
    }
}
