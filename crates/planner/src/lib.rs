//! Plan generator for loadout.
//!
//! The planner is **pure**: it reads inputs, computes classification, and produces a plan.
//! It does not execute install/uninstall, modify state, call backends, or inspect the filesystem.
//!
//! Pipeline position:
//!   DesiredResourceGraph + State + ResolvedComponentOrder → Planner → Plan
//!
//! See: `docs/specs/algorithms/planner.md`

use model::{
    desired_resource_graph::DesiredResourceKind,
    plan::{
        ActionDetails, BlockedEntry, NoopEntry, Operation, Plan, PlanAction, PlanSummary,
        ReplaceDetails, ResourceRef, StrengthenDetails,
    },
    CanonicalComponentId, DesiredResourceGraph, ResolvedComponentOrder, State,
};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

/// Errors produced by the planner.
#[derive(Debug, Error, PartialEq)]
pub enum PlannerError {
    /// A component in the DesiredResourceGraph is absent from `resolved_component_order`.
    ///
    /// This is always a programming error caused by a mismatch between resolver and compiler output.
    /// Should be unreachable in normal operation if the pipeline is correctly sequenced.
    ///
    /// Note: `resolved_order` may contain state-only component IDs (for correct destroy ordering)
    /// in addition to desired component IDs; this is expected and not an error.
    #[error("component '{id}' is in the DesiredResourceGraph but not in the resolved order")]
    ComponentOrderMismatch { id: String },
}

/// Component-level classification produced by the diff phase.
#[derive(Debug, Clone, PartialEq)]
enum Classification {
    Create,
    Replace {
        from_version: Option<String>,
        to_version: Option<String>,
    },
    ReplaceBackend,
    Strengthen {
        add_resource_ids: Vec<(String, String)>,
    }, // (kind, id)
    Noop,
    Blocked {
        reason: String,
    },
}

/// Generate a [`Plan`] from the given inputs.
///
/// This is a **pure function**: it does not perform I/O, does not mutate global state,
/// and is deterministic (same inputs always produce the same output).
///
/// # Parameters
/// - `desired`: compiled desired resources (ComponentCompiler output, includes resolved backends)
/// - `state`: current authoritative state (components installed, resources recorded)
/// - `resolved_order`: topologically sorted component IDs from the resolver. This order
///   includes **both** desired components (install order) and state-only components that
///   were resolvable by `resolver::resolve_extended` (for correct reverse destroy ordering).
///
/// # Returns
///
/// A [`Plan`] containing actions, noops, and blocked entries. Actions are classified
/// by operation type (Create/Destroy/Replace/ReplaceBackend/Strengthen) using the decision
/// table in `docs/specs/algorithms/planner.md`. The Executor must not re-classify.
///
/// # Errors
///
/// Returns [`PlannerError`] only on programming-level invariant violations
/// (e.g. order/graph mismatch). Blocked components are recorded in the plan, not returned as errors.
pub fn plan(
    desired: &DesiredResourceGraph,
    state: &State,
    resolved_order: &ResolvedComponentOrder,
) -> Result<Plan, PlannerError> {
    // Collect all component IDs referenced across both inputs.
    let desired_ids: HashSet<&str> = desired.components.keys().map(String::as_str).collect();
    let state_ids: HashSet<&str> = state.components.keys().map(String::as_str).collect();

    // Build a position map from resolved_order for use in both install and destroy ordering.
    // resolved_order may include state-only component IDs (for destroy ordering); this is expected.
    let order_positions: HashMap<&str, usize> = resolved_order
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();

    // Validate: every desired component must appear in resolved_order.
    // (resolved_order may also contain state-only IDs for destroy ordering; that is not an error.)
    for id in desired_ids.iter() {
        if !order_positions.contains_key(id) {
            return Err(PlannerError::ComponentOrderMismatch { id: (*id).into() });
        }
    }

    // Build ordered list of desired components, preserving the install order from resolved_order.
    // Components in desired but missing from resolved_order (shouldn't happen in normal use)
    // are appended at the end in sorted order to remain deterministic.
    let ordered_desired: Vec<&str> = {
        let mut v: Vec<&str> = resolved_order
            .iter()
            .map(|id| id.as_str())
            .filter(|id| desired_ids.contains(id))
            .collect();
        let in_order: HashSet<&str> = v.iter().copied().collect();
        let mut extras: Vec<&str> = desired_ids.difference(&in_order).copied().collect();
        extras.sort_unstable();
        v.extend(extras);
        v
    };

    // Components in state but not in desired → destroy (reverse dependency order).
    let destroy_ids: Vec<&str> = {
        let mut v: Vec<&str> = state_ids.difference(&desired_ids).copied().collect();
        // Sort by position in resolved_order (which now includes state-only components thanks to
        // resolve_extended), then emit in reverse so that dependents are destroyed before their
        // dependencies. Components absent from resolved_order (yaml deleted) fall back to
        // alphabetical descending as a best-effort tie-breaker.
        v.sort_unstable_by(|a, b| {
            let pa = order_positions.get(a).copied().unwrap_or(usize::MAX);
            let pb = order_positions.get(b).copied().unwrap_or(usize::MAX);
            pb.cmp(&pa).then(b.cmp(a))
        });
        v
    };

    // -- Diff & Classify --

    let mut actions: Vec<PlanAction> = Vec::new();
    let mut noops: Vec<NoopEntry> = Vec::new();
    let mut blocked: Vec<BlockedEntry> = Vec::new();
    let mut summary = PlanSummary::default();

    // Destroy: components present in state but not in desired (reverse order).
    for &id in &destroy_ids {
        let component = CanonicalComponentId::new(id).unwrap_or_else(|_| {
            // State may contain legacy bare keys after a migration bug; treat as non-canonical.
            // CanonicalComponentId::new only fails for non-canonical strings; we still need to
            // represent the destroy. Use a best-effort reconstruction.
            panic!("state contains non-canonical component id: {id}")
        });
        actions.push(PlanAction {
            component,
            operation: Operation::Destroy,
            details: None,
        });
        summary.destroy += 1;
    }

    // Create/noop/replace/strengthen: components in desired (in resolved install order).
    for &id in &ordered_desired {
        let desired_comp = desired.components.get(id).unwrap(); // validated above or present

        let classification = if !state_ids.contains(id) {
            // Component not in state → create.
            Classification::Create
        } else {
            // Component in both → diff resources.
            classify_existing(id, desired_comp, state)
        };

        let component =
            CanonicalComponentId::new(id).expect("desired_resource_graph keys are canonical ids");

        match classification {
            Classification::Create => {
                actions.push(PlanAction {
                    component,
                    operation: Operation::Create,
                    details: None,
                });
                summary.create += 1;
            }
            Classification::Noop => {
                noops.push(NoopEntry { component });
            }
            Classification::Replace {
                from_version,
                to_version,
            } => {
                let details = Some(ActionDetails::Replace(ReplaceDetails {
                    from_version,
                    to_version,
                }));
                actions.push(PlanAction {
                    component,
                    operation: Operation::Replace,
                    details,
                });
                summary.replace += 1;
            }
            Classification::ReplaceBackend => {
                actions.push(PlanAction {
                    component,
                    operation: Operation::ReplaceBackend,
                    details: None,
                });
                summary.replace += 1; // counted as replace in summary per spec
            }
            Classification::Strengthen { add_resource_ids } => {
                let add_resources = add_resource_ids
                    .into_iter()
                    .map(|(kind, id)| ResourceRef { kind, id })
                    .collect();
                let details = Some(ActionDetails::Strengthen(StrengthenDetails {
                    add_resources,
                }));
                actions.push(PlanAction {
                    component,
                    operation: Operation::Strengthen,
                    details,
                });
                summary.strengthen += 1;
            }
            Classification::Blocked { reason } => {
                blocked.push(BlockedEntry { component, reason });
                summary.blocked += 1;
            }
        }
    }

    Ok(Plan {
        actions,
        noops,
        blocked,
        summary,
    })
}

/// Classify a component that exists in both desired and state.
fn classify_existing(
    component_id: &str,
    desired_comp: &model::desired_resource_graph::ComponentDesiredResources,
    state: &State,
) -> Classification {
    let state_comp = state.components.get(component_id).unwrap();

    // Build lookup maps by resource id.
    let desired_map: HashMap<&str, &model::desired_resource_graph::DesiredResource> = desired_comp
        .resources
        .iter()
        .map(|r| (r.id.as_str(), r))
        .collect();
    let state_map: HashMap<&str, &model::state::Resource> = state_comp
        .resources
        .iter()
        .map(|r| (r.id.as_str(), r))
        .collect();

    // Check for unknown resource kinds in desired → blocked.
    for res in &desired_comp.resources {
        if is_unknown_kind(&res.kind) {
            return Classification::Blocked {
                reason: format!("unknown resource kind in desired: {}", kind_str(&res.kind)),
            };
        }
    }

    // Check for backend mismatch on any shared resource (resource present in both, same id).
    let shared_ids: HashSet<&str> = desired_map
        .keys()
        .filter(|&&id| state_map.contains_key(id))
        .copied()
        .collect();

    // Check for incompatible resources in shared set → replace.
    for &id in &shared_ids {
        let d = desired_map[id];
        let s = state_map[id];
        match check_compatibility(d, s) {
            Compatibility::Incompatible {
                from_version,
                to_version,
            } => {
                return Classification::Replace {
                    from_version,
                    to_version,
                };
            }
            Compatibility::BackendMismatch => {
                return Classification::ReplaceBackend;
            }
            Compatibility::Compatible => {}
        }
    }

    // Check resources in state but not in desired → replace (resources removed).
    let state_only_ids: HashSet<&str> = state_map
        .keys()
        .filter(|&&id| !desired_map.contains_key(id))
        .copied()
        .collect();
    if !state_only_ids.is_empty() {
        return Classification::Replace {
            from_version: None,
            to_version: None,
        };
    }

    // Check resources in desired but not in state → strengthen candidate.
    let desired_only: Vec<_> = desired_map
        .keys()
        .filter(|&&id| !state_map.contains_key(id))
        .copied()
        .collect();

    if desired_only.is_empty() {
        // All shared resources are compatible, no extras → noop.
        Classification::Noop
    } else {
        // New resources to add → strengthen, unless any desired resource is `tool`.
        // `managed_script` components use scripts for the entire install/uninstall cycle;
        // resource-level addition cannot be isolated, so strengthen must be replace.
        // We detect this by checking whether any desired resource (including shared ones)
        // is a Tool kind: if so, the component is managed_script and strengthen is invalid.
        let has_tool_resource = desired_comp
            .resources
            .iter()
            .any(|r| matches!(r.kind, DesiredResourceKind::Tool { .. }));
        if has_tool_resource {
            return Classification::Replace {
                from_version: None,
                to_version: None,
            };
        }
        let add_resource_ids = desired_only
            .into_iter()
            .map(|id| {
                let res = desired_map[id];
                (kind_str(&res.kind).to_string(), id.to_string())
            })
            .collect();
        Classification::Strengthen { add_resource_ids }
    }
}

/// Result of a per-resource compatibility check.
enum Compatibility {
    Compatible,
    BackendMismatch,
    Incompatible {
        from_version: Option<String>,
        to_version: Option<String>,
    },
}

/// Compare a desired resource against its state counterpart.
///
/// Compatibility rules per spec (planner.md):
/// - `package`: name and backend must match; version difference → replace
/// - `runtime`: name, version, and backend must all match; any difference → replace
/// - `fs`: path, entry_type, and op must all match; any difference → replace
fn check_compatibility(
    desired: &model::desired_resource_graph::DesiredResource,
    recorded: &model::state::Resource,
) -> Compatibility {
    use model::desired_resource_graph::DesiredResourceKind as D;
    use model::desired_resource_graph::{FsEntryType as DFsEntryType, FsOp as DFsOp};
    use model::state::{FsEntryType, FsOp, ResourceKind as S};

    match (&desired.kind, &recorded.kind) {
        (
            D::Package {
                name: dn,
                desired_backend: db,
            },
            S::Package {
                backend: sb,
                package: sp,
            },
        ) => {
            if db.as_str() != sb.as_str() {
                return Compatibility::BackendMismatch;
            }
            if dn != &sp.name {
                return Compatibility::Incompatible {
                    from_version: None,
                    to_version: None,
                };
            }
            Compatibility::Compatible
        }
        (
            D::Runtime {
                name: dn,
                version: dv,
                desired_backend: db,
            },
            S::Runtime {
                backend: sb,
                runtime: sr,
            },
        ) => {
            if db.as_str() != sb.as_str() {
                return Compatibility::BackendMismatch;
            }
            if dn != &sr.name || dv != &sr.version {
                return Compatibility::Incompatible {
                    from_version: Some(sr.version.clone()),
                    to_version: Some(dv.clone()),
                };
            }
            Compatibility::Compatible
        }
        (
            D::Fs {
                source: ds,
                path: dp,
                entry_type: det,
                op: dop,
                source_fingerprint: dfp,
            },
            S::Fs { fs: sf },
        ) => {
            // Compatibility table: desired (entry_type + op) → allowed state entry_type.
            //
            // | desired entry_type | desired op | state entry_type (compatible)         |
            // |--------------------|------------|---------------------------------------|
            // | file               | link       | symlink                               |
            // | dir                | link       | symlink (Unix) / junction or symlink   |
            // | file               | copy       | file                                  |
            // | dir                | copy       | dir                                   |
            let et_compatible = match (det, dop) {
                (DFsEntryType::File, DFsOp::Link) => sf.entry_type == FsEntryType::Symlink,
                (DFsEntryType::Dir, DFsOp::Link) => {
                    matches!(sf.entry_type, FsEntryType::Symlink | FsEntryType::Junction)
                }
                (DFsEntryType::File, DFsOp::Copy) => sf.entry_type == FsEntryType::File,
                (DFsEntryType::Dir, DFsOp::Copy) => sf.entry_type == FsEntryType::Dir,
            };

            let op_match = matches!(
                (dop, &sf.op),
                (DFsOp::Link, FsOp::Link) | (DFsOp::Copy, FsOp::Copy)
            );

            // Source comparison: if state has recorded source, compare resolved paths.
            let source_match = match &sf.source {
                Some(state_src) => ds.resolved == state_src.resolved,
                None => true, // Legacy state without source: skip source comparison.
            };

            if dp != &sf.path || !et_compatible || !op_match || !source_match {
                return Compatibility::Incompatible {
                    from_version: None,
                    to_version: None,
                };
            }

            // Fingerprint comparison for copy resources.
            // Both must be Some for comparison; if either is None, skip (compatible).
            if let (Some(dfp_val), Some(sfp_val)) = (dfp, &sf.source_fingerprint) {
                if dfp_val != sfp_val {
                    return Compatibility::Incompatible {
                        from_version: None,
                        to_version: None,
                    };
                }
            }

            Compatibility::Compatible
        }
        // Tool resources: delegate to check_tool_compatibility.
        (D::Tool { verify: dv, .. }, S::Tool { tool: st }) => check_tool_compatibility(dv, st),
        // Kind mismatch (e.g. package in desired, runtime in state) → replace.
        _ => Compatibility::Incompatible {
            from_version: None,
            to_version: None,
        },
    }
}

/// Compare a desired `tool` resource against its recorded state counterpart.
///
/// Compatibility rules (design doc `managed_script` section):
/// - `verify.identity` change → incompatible (replace)
/// - `verify.version.constraint` change (add/remove/modify) → incompatible (replace)
/// - Other changes (script changes, name cosmetics, etc.) → compatible
fn check_tool_compatibility(
    desired_verify: &model::tool::ToolVerifyContract,
    recorded: &model::tool::ToolResource,
) -> Compatibility {
    // Identity contract must match exactly.
    if desired_verify.identity != recorded.verify.identity {
        return Compatibility::Incompatible {
            from_version: None,
            to_version: None,
        };
    }

    // Version constraint change (including add/remove) → replace.
    let desired_constraint = desired_verify
        .version
        .as_ref()
        .and_then(|v| v.constraint.as_deref());
    let recorded_constraint = recorded
        .verify
        .version
        .as_ref()
        .and_then(|v| v.constraint.as_deref());
    if desired_constraint != recorded_constraint {
        return Compatibility::Incompatible {
            from_version: recorded_constraint.map(str::to_owned),
            to_version: desired_constraint.map(str::to_owned),
        };
    }

    Compatibility::Compatible
}

fn is_unknown_kind(_kind: &DesiredResourceKind) -> bool {
    // All current kinds (package/runtime/fs) are known. This function is a hook for future
    // extension; the serde unknown-kind handling happens at deserialization before planner runs.
    false
}

fn kind_str(kind: &DesiredResourceKind) -> &'static str {
    match kind {
        DesiredResourceKind::Package { .. } => "package",
        DesiredResourceKind::Runtime { .. } => "runtime",
        DesiredResourceKind::Fs { .. } => "fs",
        DesiredResourceKind::Tool { .. } => "tool",
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use model::{
        desired_resource_graph::{
            ComponentDesiredResources, DesiredResource, DesiredResourceGraph, DesiredResourceKind,
            FsEntryType, FsOp,
        },
        fs::ConcreteFsSource,
        plan::Operation,
        state::{
            ComponentState, FsDetails, FsEntryType as SFsEntryType, FsOp as SFsOp, PackageDetails,
            Resource, ResourceKind, RuntimeDetails, State,
        },
        CanonicalBackendId,
    };

    fn backend(s: &str) -> CanonicalBackendId {
        CanonicalBackendId::new(s).unwrap()
    }

    fn cid(s: &str) -> CanonicalComponentId {
        CanonicalComponentId::new(s).unwrap()
    }

    /// Create a dummy ConcreteFsSource for tests.
    fn dummy_source(path: &str) -> ConcreteFsSource {
        ConcreteFsSource::component_relative(PathBuf::from(path))
    }

    fn empty_desired(ids: &[&str]) -> DesiredResourceGraph {
        let components = ids
            .iter()
            .map(|&id| {
                (
                    id.to_string(),
                    ComponentDesiredResources { resources: vec![] },
                )
            })
            .collect();
        DesiredResourceGraph {
            schema_version: 1,
            components,
        }
    }

    fn with_package(
        mut g: DesiredResourceGraph,
        feat: &str,
        pkg: &str,
        be: &str,
    ) -> DesiredResourceGraph {
        g.components
            .entry(feat.to_string())
            .or_insert(ComponentDesiredResources { resources: vec![] })
            .resources
            .push(DesiredResource {
                id: format!("package:{pkg}"),
                kind: DesiredResourceKind::Package {
                    name: pkg.to_string(),
                    desired_backend: backend(be),
                },
            });
        g
    }

    fn with_runtime(
        mut g: DesiredResourceGraph,
        feat: &str,
        rt: &str,
        ver: &str,
        be: &str,
    ) -> DesiredResourceGraph {
        g.components
            .entry(feat.to_string())
            .or_insert(ComponentDesiredResources { resources: vec![] })
            .resources
            .push(DesiredResource {
                id: format!("runtime:{rt}"),
                kind: DesiredResourceKind::Runtime {
                    name: rt.to_string(),
                    version: ver.to_string(),
                    desired_backend: backend(be),
                },
            });
        g
    }

    fn state_with_package(feat: &str, pkg: &str, be: &str) -> State {
        let mut s = State::empty();
        s.components.insert(
            feat.to_string(),
            ComponentState {
                resources: vec![Resource {
                    id: format!("package:{pkg}"),
                    kind: ResourceKind::Package {
                        backend: backend(be),
                        package: PackageDetails {
                            name: pkg.to_string(),
                            version: None,
                        },
                    },
                }],
            },
        );
        s
    }

    fn state_with_runtime(feat: &str, rt: &str, ver: &str, be: &str) -> State {
        let mut s = State::empty();
        s.components.insert(
            feat.to_string(),
            ComponentState {
                resources: vec![Resource {
                    id: format!("runtime:{rt}"),
                    kind: ResourceKind::Runtime {
                        backend: backend(be),
                        runtime: RuntimeDetails {
                            name: rt.to_string(),
                            version: ver.to_string(),
                        },
                    },
                }],
            },
        );
        s
    }

    // --- create ---

    #[test]
    fn create_when_not_in_state() {
        let desired = with_package(empty_desired(&["core/git"]), "core/git", "git", "core/brew");
        let state = State::empty();
        let order = vec![cid("core/git")];
        let p = plan(&desired, &state, &order).unwrap();
        assert_eq!(p.actions.len(), 1);
        assert_eq!(p.actions[0].operation, Operation::Create);
        assert_eq!(p.summary.create, 1);
    }

    // --- noop ---

    #[test]
    fn noop_when_identical() {
        let desired = with_package(empty_desired(&["core/git"]), "core/git", "git", "core/brew");
        let state = state_with_package("core/git", "git", "core/brew");
        let order = vec![cid("core/git")];
        let p = plan(&desired, &state, &order).unwrap();
        assert!(p.actions.is_empty());
        assert_eq!(p.noops.len(), 1);
        assert_eq!(p.noops[0].component.as_str(), "core/git");
    }

    // --- destroy ---

    #[test]
    fn destroy_when_not_in_desired() {
        let desired = empty_desired(&[]); // nothing desired
        let state = state_with_package("core/old", "old-tool", "core/brew");
        let order: ResolvedComponentOrder = vec![];
        let p = plan(&desired, &state, &order).unwrap();
        assert_eq!(p.actions.len(), 1);
        assert_eq!(p.actions[0].operation, Operation::Destroy);
        assert_eq!(p.summary.destroy, 1);
    }

    // --- replace: version mismatch ---

    #[test]
    fn replace_on_runtime_version_mismatch() {
        let desired = with_runtime(
            empty_desired(&["core/node"]),
            "core/node",
            "node",
            "20",
            "core/mise",
        );
        let state = state_with_runtime("core/node", "node", "18", "core/mise");
        let order = vec![cid("core/node")];
        let p = plan(&desired, &state, &order).unwrap();
        assert_eq!(p.actions.len(), 1);
        assert_eq!(p.actions[0].operation, Operation::Replace);
        match &p.actions[0].details {
            Some(ActionDetails::Replace(d)) => {
                assert_eq!(d.from_version.as_deref(), Some("18"));
                assert_eq!(d.to_version.as_deref(), Some("20"));
            }
            _ => panic!("expected replace details"),
        }
        assert_eq!(p.summary.replace, 1);
    }

    // --- replace_backend ---

    #[test]
    fn replace_backend_mismatch() {
        let desired = with_package(empty_desired(&["core/git"]), "core/git", "git", "core/apt");
        let state = state_with_package("core/git", "git", "core/brew");
        let order = vec![cid("core/git")];
        let p = plan(&desired, &state, &order).unwrap();
        assert_eq!(p.actions[0].operation, Operation::ReplaceBackend);
    }

    // --- strengthen ---

    #[test]
    fn strengthen_when_new_resource_added() {
        // State has only package:git; desired adds fs:gitconfig.
        let mut desired =
            with_package(empty_desired(&["core/git"]), "core/git", "git", "core/brew");
        desired
            .components
            .get_mut("core/git")
            .unwrap()
            .resources
            .push(DesiredResource {
                id: "fs:gitconfig".to_string(),
                kind: DesiredResourceKind::Fs {
                    source: dummy_source("/tmp/test/files/.gitconfig"),
                    source_fingerprint: None,
                    path: "~/.gitconfig".to_string(),
                    entry_type: FsEntryType::File,
                    op: FsOp::Link,
                },
            });
        let state = state_with_package("core/git", "git", "core/brew");
        let order = vec![cid("core/git")];
        let p = plan(&desired, &state, &order).unwrap();
        assert_eq!(p.actions[0].operation, Operation::Strengthen);
        match &p.actions[0].details {
            Some(ActionDetails::Strengthen(d)) => {
                assert_eq!(d.add_resources.len(), 1);
                assert_eq!(d.add_resources[0].id, "fs:gitconfig");
            }
            _ => panic!("expected strengthen details"),
        }
        assert_eq!(p.summary.strengthen, 1);
    }

    // --- fs compatibility ---

    #[test]
    fn replace_on_fs_path_change() {
        let mut desired = empty_desired(&["core/git"]);
        desired
            .components
            .get_mut("core/git")
            .unwrap()
            .resources
            .push(DesiredResource {
                id: "fs:gitconfig".to_string(),
                kind: DesiredResourceKind::Fs {
                    source: dummy_source("/tmp/test/files/.gitconfig"),
                    source_fingerprint: None,
                    path: "~/.gitconfig_new".to_string(),
                    entry_type: FsEntryType::File,
                    op: FsOp::Link,
                },
            });
        let mut state = State::empty();
        state.components.insert(
            "core/git".to_string(),
            ComponentState {
                resources: vec![Resource {
                    id: "fs:gitconfig".to_string(),
                    kind: ResourceKind::Fs {
                        fs: FsDetails {
                            path: "~/.gitconfig".to_string(),
                            entry_type: SFsEntryType::Symlink,
                            op: SFsOp::Link,
                            source: None,
                            source_fingerprint: None,
                        },
                    },
                }],
            },
        );
        let order = vec![cid("core/git")];
        let p = plan(&desired, &state, &order).unwrap();
        assert_eq!(p.actions[0].operation, Operation::Replace);
    }

    // --- ordering: destroys before creates ---

    #[test]
    fn ordering_destroy_then_create() {
        let desired = with_package(
            empty_desired(&["core/new"]),
            "core/new",
            "new-tool",
            "core/brew",
        );
        let state = state_with_package("core/old", "old-tool", "core/brew");
        let order = vec![cid("core/new")];
        let p = plan(&desired, &state, &order).unwrap();
        // Destroy comes first in actions list.
        assert_eq!(p.actions[0].operation, Operation::Destroy);
        assert_eq!(p.actions[1].operation, Operation::Create);
    }

    // --- summary ---

    #[test]
    fn summary_counts() {
        // create one, noop one, destroy one
        let mut desired = with_package(
            empty_desired(&["core/new", "core/keep"]),
            "core/new",
            "new",
            "core/brew",
        );
        desired = with_package(desired, "core/keep", "keep", "core/brew");

        let state = state_with_package("core/old", "old", "core/brew");
        // Also add "core/keep" to state as-is.
        let _ = state.components.clone(); // just for clarity; state_with_package creates fresh state
        let mut state2 = state_with_package("core/old", "old", "core/brew");
        state2.components.insert(
            "core/keep".to_string(),
            ComponentState {
                resources: vec![Resource {
                    id: "package:keep".to_string(),
                    kind: ResourceKind::Package {
                        backend: backend("core/brew"),
                        package: PackageDetails {
                            name: "keep".to_string(),
                            version: None,
                        },
                    },
                }],
            },
        );

        let order = vec![cid("core/new"), cid("core/keep")];
        let p = plan(&desired, &state2, &order).unwrap();
        assert_eq!(p.summary.create, 1);
        assert_eq!(p.summary.destroy, 1);
        assert_eq!(p.noops.len(), 1);
    }

    // --- ordering: destroy respects dependency order ---

    #[test]
    fn destroy_ordering_dependent_before_dependency() {
        // Both core/neovim (depends on core/git) and core/git are being destroyed
        // (neither is in desired). The full_order from resolve_extended contains both,
        // with git before neovim (install order). Reverse for destroy: neovim must be
        // destroyed before git.
        let desired = empty_desired(&[]);
        let mut state = state_with_package("core/git", "git", "core/brew");
        state.components.insert(
            "core/neovim".to_string(),
            ComponentState {
                resources: vec![Resource {
                    id: "package:neovim".to_string(),
                    kind: ResourceKind::Package {
                        backend: backend("core/brew"),
                        package: PackageDetails {
                            name: "neovim".to_string(),
                            version: None,
                        },
                    },
                }],
            },
        );

        // Simulate full_order from resolve_extended: git at position 0, neovim at position 1
        // (install order: git before neovim because neovim depends on git).
        let full_order = vec![cid("core/git"), cid("core/neovim")];
        let p = plan(&desired, &state, &full_order).unwrap();

        assert_eq!(p.summary.destroy, 2);
        let ops: Vec<&str> = p
            .actions
            .iter()
            .filter(|a| a.operation == Operation::Destroy)
            .map(|a| a.component.as_str())
            .collect();
        let git_pos = ops.iter().position(|&s| s == "core/git").unwrap();
        let nvim_pos = ops.iter().position(|&s| s == "core/neovim").unwrap();
        assert!(
            nvim_pos < git_pos,
            "neovim (dependent) must be destroyed before git (dependency); got ops={ops:?}"
        );
    }

    #[test]
    fn destroy_ordering_without_full_order_falls_back_to_alphabetical_descending() {
        // When the full_order does NOT contain the state-only components (e.g. their yaml
        // was deleted and resolve_extended excluded them), destroy order falls back to
        // alphabetical descending. This is the pre-existing best-effort behaviour.
        let desired = empty_desired(&[]);
        let mut state = state_with_package("core/aaa", "aaa", "core/brew");
        state.components.insert(
            "core/zzz".to_string(),
            ComponentState {
                resources: vec![Resource {
                    id: "package:zzz".to_string(),
                    kind: ResourceKind::Package {
                        backend: backend("core/brew"),
                        package: PackageDetails {
                            name: "zzz".to_string(),
                            version: None,
                        },
                    },
                }],
            },
        );

        // Empty order (both yamls gone; resolve_extended skipped them).
        let full_order: ResolvedComponentOrder = vec![];
        let p = plan(&desired, &state, &full_order).unwrap();

        assert_eq!(p.summary.destroy, 2);
        let ops: Vec<&str> = p
            .actions
            .iter()
            .filter(|a| a.operation == Operation::Destroy)
            .map(|a| a.component.as_str())
            .collect();
        // Alphabetical descending: zzz before aaa.
        assert_eq!(ops[0], "core/zzz");
        assert_eq!(ops[1], "core/aaa");
    }

    // ── tool resource helpers ─────────────────────────────────────────────────

    fn make_tool_verify(
        command: &str,
        path: &str,
        constraint: Option<&str>,
    ) -> model::tool::ToolVerifyContract {
        use model::tool::{
            OneOf, ToolIdentityVerify, ToolVerifyContract, ToolVersionVerify, VersionParseRule,
        };
        ToolVerifyContract {
            identity: ToolIdentityVerify::ResolvedCommand {
                command: command.to_string(),
                expected_path: OneOf {
                    one_of: vec![path.to_string()],
                },
            },
            version: constraint.map(|c| ToolVersionVerify {
                command: command.to_string(),
                args: vec!["--version".to_string()],
                parse: VersionParseRule {
                    first_line_regex: "^([0-9]+\\.[0-9]+\\.[0-9]+)".to_string(),
                },
                constraint: Some(c.to_string()),
            }),
        }
    }

    fn desired_with_tool(
        component: &str,
        res_id: &str,
        name: &str,
        verify: model::tool::ToolVerifyContract,
    ) -> DesiredResourceGraph {
        let mut g = empty_desired(&[component]);
        g.components
            .get_mut(component)
            .unwrap()
            .resources
            .push(DesiredResource {
                id: res_id.to_string(),
                kind: DesiredResourceKind::Tool {
                    name: name.to_string(),
                    verify,
                },
            });
        g
    }

    fn state_with_tool(
        component: &str,
        res_id: &str,
        name: &str,
        verify: model::tool::ToolVerifyContract,
    ) -> State {
        use model::tool::{ToolObservedFacts, ToolResource};
        let mut s = State::empty();
        s.components.insert(
            component.to_string(),
            ComponentState {
                resources: vec![Resource {
                    id: res_id.to_string(),
                    kind: ResourceKind::Tool {
                        tool: ToolResource {
                            name: name.to_string(),
                            verify,
                            observed: ToolObservedFacts {
                                resolved_path: Some(
                                    "/home/linuxbrew/.linuxbrew/bin/brew".to_string(),
                                ),
                                version: None,
                            },
                        },
                    },
                }],
            },
        );
        s
    }

    // ── tool: create / destroy / noop ─────────────────────────────────────────

    #[test]
    fn tool_create_when_not_in_state() {
        let verify = make_tool_verify("brew", "/home/linuxbrew/.linuxbrew/bin/brew", None);
        let desired = desired_with_tool("core/brew", "tool:brew", "brew", verify);
        let state = State::empty();
        let order = vec![cid("core/brew")];
        let p = plan(&desired, &state, &order).unwrap();
        assert_eq!(p.actions.len(), 1);
        assert_eq!(p.actions[0].operation, Operation::Create);
    }

    #[test]
    fn tool_noop_when_identity_contract_identical() {
        let verify = make_tool_verify("brew", "/home/linuxbrew/.linuxbrew/bin/brew", None);
        let desired = desired_with_tool("core/brew", "tool:brew", "brew", verify.clone());
        let state = state_with_tool("core/brew", "tool:brew", "brew", verify);
        let order = vec![cid("core/brew")];
        let p = plan(&desired, &state, &order).unwrap();
        assert!(p.actions.is_empty());
        assert_eq!(p.noops.len(), 1);
    }

    #[test]
    fn tool_destroy_when_not_in_desired() {
        let verify = make_tool_verify("brew", "/home/linuxbrew/.linuxbrew/bin/brew", None);
        let desired = empty_desired(&[]);
        let state = state_with_tool("core/brew", "tool:brew", "brew", verify);
        let order: ResolvedComponentOrder = vec![];
        let p = plan(&desired, &state, &order).unwrap();
        assert_eq!(p.actions.len(), 1);
        assert_eq!(p.actions[0].operation, Operation::Destroy);
    }

    // ── tool: replace triggers ────────────────────────────────────────────────

    #[test]
    fn tool_replace_on_identity_contract_change() {
        // Change the expected_path in identity → replace.
        use model::tool::{OneOf, ToolIdentityVerify, ToolVerifyContract};
        let old_verify = make_tool_verify("brew", "/home/linuxbrew/.linuxbrew/bin/brew", None);
        let new_verify = ToolVerifyContract {
            identity: ToolIdentityVerify::ResolvedCommand {
                command: "brew".to_string(),
                expected_path: OneOf {
                    one_of: vec!["/opt/homebrew/bin/brew".to_string()],
                },
            },
            version: None,
        };
        let desired = desired_with_tool("core/brew", "tool:brew", "brew", new_verify);
        let state = state_with_tool("core/brew", "tool:brew", "brew", old_verify);
        let order = vec![cid("core/brew")];
        let p = plan(&desired, &state, &order).unwrap();
        assert_eq!(p.actions[0].operation, Operation::Replace);
    }

    #[test]
    fn tool_replace_on_version_constraint_added() {
        // No constraint → with constraint: replace.
        let old_verify = make_tool_verify("brew", "/home/linuxbrew/.linuxbrew/bin/brew", None);
        let new_verify = make_tool_verify(
            "brew",
            "/home/linuxbrew/.linuxbrew/bin/brew",
            Some(">=4.0.0"),
        );
        let desired = desired_with_tool("core/brew", "tool:brew", "brew", new_verify);
        let state = state_with_tool("core/brew", "tool:brew", "brew", old_verify);
        let order = vec![cid("core/brew")];
        let p = plan(&desired, &state, &order).unwrap();
        assert_eq!(p.actions[0].operation, Operation::Replace);
    }

    #[test]
    fn tool_replace_on_version_constraint_removed() {
        // With constraint → no constraint: replace.
        let old_verify = make_tool_verify(
            "brew",
            "/home/linuxbrew/.linuxbrew/bin/brew",
            Some(">=4.0.0"),
        );
        let new_verify = make_tool_verify("brew", "/home/linuxbrew/.linuxbrew/bin/brew", None);
        let desired = desired_with_tool("core/brew", "tool:brew", "brew", new_verify);
        let state = state_with_tool("core/brew", "tool:brew", "brew", old_verify);
        let order = vec![cid("core/brew")];
        let p = plan(&desired, &state, &order).unwrap();
        assert_eq!(p.actions[0].operation, Operation::Replace);
    }

    #[test]
    fn tool_replace_on_version_constraint_changed() {
        // Constraint value change: replace with from/to version info.
        let old_verify = make_tool_verify(
            "brew",
            "/home/linuxbrew/.linuxbrew/bin/brew",
            Some(">=4.0.0"),
        );
        let new_verify = make_tool_verify(
            "brew",
            "/home/linuxbrew/.linuxbrew/bin/brew",
            Some(">=5.0.0"),
        );
        let desired = desired_with_tool("core/brew", "tool:brew", "brew", new_verify);
        let state = state_with_tool("core/brew", "tool:brew", "brew", old_verify);
        let order = vec![cid("core/brew")];
        let p = plan(&desired, &state, &order).unwrap();
        assert_eq!(p.actions[0].operation, Operation::Replace);
        // Version info should be propagated for display.
        match &p.actions[0].details {
            Some(ActionDetails::Replace(d)) => {
                assert_eq!(d.from_version.as_deref(), Some(">=4.0.0"));
                assert_eq!(d.to_version.as_deref(), Some(">=5.0.0"));
            }
            _ => panic!("expected replace details with version info"),
        }
    }

    #[test]
    fn tool_noop_when_only_version_command_differs() {
        // Only version.command/args/parse changes; constraint is the same → noop.
        use model::tool::{
            OneOf, ToolIdentityVerify, ToolVerifyContract, ToolVersionVerify, VersionParseRule,
        };
        let base_identity = ToolIdentityVerify::ResolvedCommand {
            command: "brew".to_string(),
            expected_path: OneOf {
                one_of: vec!["/home/linuxbrew/.linuxbrew/bin/brew".to_string()],
            },
        };
        let old_verify = ToolVerifyContract {
            identity: base_identity.clone(),
            version: Some(ToolVersionVerify {
                command: "brew".to_string(),
                args: vec!["--version".to_string()],
                parse: VersionParseRule {
                    first_line_regex: "^old (.+)".to_string(),
                },
                constraint: Some(">=4.0.0".to_string()),
            }),
        };
        let new_verify = ToolVerifyContract {
            identity: base_identity,
            version: Some(ToolVersionVerify {
                command: "brew".to_string(),
                args: vec!["--version".to_string()],
                parse: VersionParseRule {
                    first_line_regex: "^Homebrew (.+)".to_string(),
                },
                constraint: Some(">=4.0.0".to_string()), // same constraint, different regex
            }),
        };
        let desired = desired_with_tool("core/brew", "tool:brew", "brew", new_verify);
        let state = state_with_tool("core/brew", "tool:brew", "brew", old_verify);
        let order = vec![cid("core/brew")];
        let p = plan(&desired, &state, &order).unwrap();
        // Only constraint is compared; parse regex difference is not a replace trigger.
        assert!(p.actions.is_empty(), "expected noop, got: {:?}", p.actions);
        assert_eq!(p.noops.len(), 1);
    }

    // ── managed_script: strengthen → replace normalization ───────────────────

    #[test]
    fn managed_script_tool_addition_produces_replace_not_strengthen() {
        // State has one tool resource; desired adds a second tool resource.
        // Because the component has tool resources, strengthen must be normalized to replace.
        use model::tool::{ToolIdentityVerify, ToolVerifyContract};

        let verify_brew = make_tool_verify("brew", "/home/linuxbrew/.linuxbrew/bin/brew", None);
        let verify_deno = ToolVerifyContract {
            identity: ToolIdentityVerify::File {
                path: "/home/user/.deno/bin/deno".to_string(),
                executable: true,
            },
            version: None,
        };

        // desired: brew + deno
        let mut desired = desired_with_tool("core/tools", "tool:brew", "brew", verify_brew.clone());
        desired
            .components
            .get_mut("core/tools")
            .unwrap()
            .resources
            .push(DesiredResource {
                id: "tool:deno".to_string(),
                kind: DesiredResourceKind::Tool {
                    name: "deno".to_string(),
                    verify: verify_deno,
                },
            });

        // state: only brew
        let state = state_with_tool("core/tools", "tool:brew", "brew", verify_brew);
        let order = vec![cid("core/tools")];
        let p = plan(&desired, &state, &order).unwrap();
        assert_eq!(p.actions.len(), 1);
        assert_eq!(
            p.actions[0].operation,
            Operation::Replace,
            "tool addition in managed_script must produce Replace, not Strengthen"
        );
    }

    #[test]
    fn non_tool_component_still_produces_strengthen() {
        // A declarative component (no tool resources) with a new resource → strengthen.
        // This confirms the strengthen→replace normalization is scoped to tool-containing components.
        let mut desired =
            with_package(empty_desired(&["core/git"]), "core/git", "git", "core/brew");
        desired
            .components
            .get_mut("core/git")
            .unwrap()
            .resources
            .push(DesiredResource {
                id: "fs:gitconfig".to_string(),
                kind: DesiredResourceKind::Fs {
                    source: dummy_source("/tmp/test/files/.gitconfig"),
                    source_fingerprint: None,
                    path: "~/.gitconfig".to_string(),
                    entry_type: FsEntryType::File,
                    op: FsOp::Link,
                },
            });
        let state = state_with_package("core/git", "git", "core/brew");
        let order = vec![cid("core/git")];
        let p = plan(&desired, &state, &order).unwrap();
        assert_eq!(p.actions[0].operation, Operation::Strengthen);
    }

    // ---------------------------------------------------------------------------
    // fs compatibility: correspondence table, source comparison, fingerprint
    // ---------------------------------------------------------------------------

    /// Helper: build a desired graph with a single fs resource.
    fn desired_with_fs(
        path: &str,
        source: ConcreteFsSource,
        entry_type: FsEntryType,
        op: FsOp,
        fingerprint: Option<String>,
    ) -> DesiredResourceGraph {
        let mut g = empty_desired(&["core/git"]);
        g.components
            .get_mut("core/git")
            .unwrap()
            .resources
            .push(DesiredResource {
                id: "fs:test".to_string(),
                kind: DesiredResourceKind::Fs {
                    source,
                    source_fingerprint: fingerprint,
                    path: path.to_string(),
                    entry_type,
                    op,
                },
            });
        g
    }

    /// Helper: build a state with a single fs resource.
    fn state_with_fs(
        path: &str,
        entry_type: SFsEntryType,
        op: SFsOp,
        source: Option<ConcreteFsSource>,
        fingerprint: Option<String>,
    ) -> State {
        let mut s = State::empty();
        s.components.insert(
            "core/git".to_string(),
            ComponentState {
                resources: vec![Resource {
                    id: "fs:test".to_string(),
                    kind: ResourceKind::Fs {
                        fs: FsDetails {
                            path: path.to_string(),
                            entry_type,
                            op,
                            source,
                            source_fingerprint: fingerprint,
                        },
                    },
                }],
            },
        );
        s
    }

    // --- correspondence table ---

    #[test]
    fn fs_noop_file_link_symlink() {
        // desired: file+link, state: symlink → compatible
        let src = dummy_source("/tmp/files/config");
        let desired = desired_with_fs(
            "/home/user/.config",
            src.clone(),
            FsEntryType::File,
            FsOp::Link,
            None,
        );
        let state = state_with_fs(
            "/home/user/.config",
            SFsEntryType::Symlink,
            SFsOp::Link,
            Some(src),
            None,
        );
        let order = vec![cid("core/git")];
        let p = plan(&desired, &state, &order).unwrap();
        assert!(p.actions.is_empty(), "expected noop, got {:?}", p.actions);
        assert_eq!(p.noops.len(), 1);
    }

    #[test]
    fn fs_replace_file_link_vs_file() {
        // desired: file+link, state: file → incompatible (file was copied, now want symlink)
        let src = dummy_source("/tmp/files/config");
        let desired = desired_with_fs(
            "/home/user/.config",
            src.clone(),
            FsEntryType::File,
            FsOp::Link,
            None,
        );
        let state = state_with_fs(
            "/home/user/.config",
            SFsEntryType::File,
            SFsOp::Copy,
            Some(src),
            None,
        );
        let order = vec![cid("core/git")];
        let p = plan(&desired, &state, &order).unwrap();
        assert_eq!(p.actions[0].operation, Operation::Replace);
    }

    #[test]
    fn fs_noop_dir_link_junction() {
        // desired: dir+link, state: junction → compatible (Windows)
        let src = dummy_source("/tmp/files/nvim");
        let desired = desired_with_fs(
            "/home/user/.config/nvim",
            src.clone(),
            FsEntryType::Dir,
            FsOp::Link,
            None,
        );
        let state = state_with_fs(
            "/home/user/.config/nvim",
            SFsEntryType::Junction,
            SFsOp::Link,
            Some(src),
            None,
        );
        let order = vec![cid("core/git")];
        let p = plan(&desired, &state, &order).unwrap();
        assert!(p.actions.is_empty(), "dir+link vs junction should be noop");
        assert_eq!(p.noops.len(), 1);
    }

    #[test]
    fn fs_replace_dir_link_vs_dir() {
        // desired: dir+link, state: dir → incompatible (was copied, now want link/junction)
        let src = dummy_source("/tmp/files/nvim");
        let desired = desired_with_fs(
            "/home/user/.config/nvim",
            src.clone(),
            FsEntryType::Dir,
            FsOp::Link,
            None,
        );
        let state = state_with_fs(
            "/home/user/.config/nvim",
            SFsEntryType::Dir,
            SFsOp::Copy,
            Some(src),
            None,
        );
        let order = vec![cid("core/git")];
        let p = plan(&desired, &state, &order).unwrap();
        assert_eq!(p.actions[0].operation, Operation::Replace);
    }

    #[test]
    fn fs_noop_file_copy() {
        // desired: file+copy, state: file → compatible
        let src = dummy_source("/tmp/files/config");
        let desired = desired_with_fs(
            "/home/user/.config",
            src.clone(),
            FsEntryType::File,
            FsOp::Copy,
            None,
        );
        let state = state_with_fs(
            "/home/user/.config",
            SFsEntryType::File,
            SFsOp::Copy,
            Some(src),
            None,
        );
        let order = vec![cid("core/git")];
        let p = plan(&desired, &state, &order).unwrap();
        assert!(p.actions.is_empty(), "file+copy vs file should be noop");
        assert_eq!(p.noops.len(), 1);
    }

    #[test]
    fn fs_noop_dir_copy() {
        // desired: dir+copy, state: dir → compatible
        let src = dummy_source("/tmp/files/nvim");
        let desired = desired_with_fs(
            "/home/user/.config/nvim",
            src.clone(),
            FsEntryType::Dir,
            FsOp::Copy,
            None,
        );
        let state = state_with_fs(
            "/home/user/.config/nvim",
            SFsEntryType::Dir,
            SFsOp::Copy,
            Some(src),
            None,
        );
        let order = vec![cid("core/git")];
        let p = plan(&desired, &state, &order).unwrap();
        assert!(p.actions.is_empty(), "dir+copy vs dir should be noop");
        assert_eq!(p.noops.len(), 1);
    }

    // --- source comparison ---

    #[test]
    fn fs_replace_on_source_path_change() {
        // Same target path and entry_type/op, but source.resolved changed → replace.
        let desired = desired_with_fs(
            "/home/user/.gitconfig",
            ConcreteFsSource::component_relative(PathBuf::from("/tmp/compA/files/.gitconfig")),
            FsEntryType::File,
            FsOp::Link,
            None,
        );
        let state = state_with_fs(
            "/home/user/.gitconfig",
            SFsEntryType::Symlink,
            SFsOp::Link,
            Some(ConcreteFsSource::component_relative(PathBuf::from(
                "/tmp/compB/files/.gitconfig",
            ))),
            None,
        );
        let order = vec![cid("core/git")];
        let p = plan(&desired, &state, &order).unwrap();
        assert_eq!(p.actions[0].operation, Operation::Replace);
    }

    #[test]
    fn fs_noop_legacy_state_no_source() {
        // State has no source recorded (legacy). Source comparison is skipped → noop.
        let src = dummy_source("/tmp/files/.gitconfig");
        let desired = desired_with_fs(
            "/home/user/.gitconfig",
            src,
            FsEntryType::File,
            FsOp::Link,
            None,
        );
        let state = state_with_fs(
            "/home/user/.gitconfig",
            SFsEntryType::Symlink,
            SFsOp::Link,
            None, // legacy: no source recorded
            None,
        );
        let order = vec![cid("core/git")];
        let p = plan(&desired, &state, &order).unwrap();
        assert!(
            p.actions.is_empty(),
            "legacy state without source should not force replace"
        );
    }

    // --- fingerprint comparison ---

    #[test]
    fn fs_noop_fingerprint_match() {
        // Both fingerprints present and equal → noop.
        let src = ConcreteFsSource::component_relative(PathBuf::from("/tmp/files/marker"));
        let fp = Some("sha256:abc123".to_string());
        let desired = desired_with_fs(
            "/home/user/.marker",
            src.clone(),
            FsEntryType::File,
            FsOp::Copy,
            fp.clone(),
        );
        let state = state_with_fs(
            "/home/user/.marker",
            SFsEntryType::File,
            SFsOp::Copy,
            Some(src),
            fp,
        );
        let order = vec![cid("core/git")];
        let p = plan(&desired, &state, &order).unwrap();
        assert!(
            p.actions.is_empty(),
            "matching fingerprints should produce noop"
        );
        assert_eq!(p.noops.len(), 1);
    }

    #[test]
    fn fs_replace_fingerprint_mismatch() {
        // Both fingerprints present but different → replace.
        let src = ConcreteFsSource::component_relative(PathBuf::from("/tmp/files/marker"));
        let desired = desired_with_fs(
            "/home/user/.marker",
            src.clone(),
            FsEntryType::File,
            FsOp::Copy,
            Some("sha256:aaaaaa".to_string()),
        );
        let state = state_with_fs(
            "/home/user/.marker",
            SFsEntryType::File,
            SFsOp::Copy,
            Some(src),
            Some("sha256:bbbbbb".to_string()),
        );
        let order = vec![cid("core/git")];
        let p = plan(&desired, &state, &order).unwrap();
        assert_eq!(p.actions[0].operation, Operation::Replace);
    }

    #[test]
    fn fs_noop_fingerprint_desired_none() {
        // Desired has no fingerprint (e.g., home_relative source) → skip comparison → noop.
        let src = ConcreteFsSource::home_relative(PathBuf::from("/home/user/external.conf"));
        let desired = desired_with_fs(
            "/home/user/.config",
            src.clone(),
            FsEntryType::File,
            FsOp::Copy,
            None,
        );
        let state = state_with_fs(
            "/home/user/.config",
            SFsEntryType::File,
            SFsOp::Copy,
            Some(src),
            Some("sha256:recorded".to_string()),
        );
        let order = vec![cid("core/git")];
        let p = plan(&desired, &state, &order).unwrap();
        assert!(
            p.actions.is_empty(),
            "None desired fingerprint must not force replace"
        );
    }

    #[test]
    fn fs_noop_fingerprint_state_none() {
        // State has no fingerprint (legacy state for copy resource) → skip → noop.
        let src = ConcreteFsSource::component_relative(PathBuf::from("/tmp/files/marker"));
        let desired = desired_with_fs(
            "/home/user/.marker",
            src.clone(),
            FsEntryType::File,
            FsOp::Copy,
            Some("sha256:current".to_string()),
        );
        let state = state_with_fs(
            "/home/user/.marker",
            SFsEntryType::File,
            SFsOp::Copy,
            Some(src),
            None,
        );
        let order = vec![cid("core/git")];
        let p = plan(&desired, &state, &order).unwrap();
        assert!(
            p.actions.is_empty(),
            "None state fingerprint must not force replace"
        );
    }
}
