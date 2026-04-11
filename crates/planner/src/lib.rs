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
    /// A component appears in `resolved_component_order` but not in the DesiredResourceGraph.
    ///
    /// This is always a programming error caused by a mismatch between resolver and compiler output.
    /// Should be unreachable in normal operation if the pipeline is correctly sequenced.
    #[error("feature '{id}' is in the resolved order but not in the DesiredResourceGraph")]
    ComponentOrderMismatch { id: String },
}

/// Feature-level classification produced by the diff phase.
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
/// - `desired`: compiled desired resources (FeatureCompiler output, includes resolved backends)
/// - `state`: current authoritative state (components installed, resources recorded)
/// - `resolved_order`: topologically sorted component IDs (resolver output, defines install order)
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

    // Validate: every component in resolved_order must be in desired.
    for id in resolved_order {
        if !desired_ids.contains(id.as_str()) {
            return Err(PlannerError::ComponentOrderMismatch {
                id: id.as_str().into(),
            });
        }
    }

    // Build ordered list of all components to consider.
    // Features in desired but not in resolved_order (shouldn't happen in normal use)
    // are appended at the end in sorted order to remain deterministic.
    let ordered_desired: Vec<&str> = {
        let mut v: Vec<&str> = resolved_order.iter().map(|id| id.as_str()).collect();
        let in_order: HashSet<&str> = v.iter().copied().collect();
        let mut extras: Vec<&str> = desired_ids.difference(&in_order).copied().collect();
        extras.sort_unstable();
        v.extend(extras);
        v
    };

    // Features in state but not in desired → destroy (reverse order of install).
    let destroy_ids: Vec<&str> = {
        let mut v: Vec<&str> = state_ids.difference(&desired_ids).copied().collect();
        // Reverse topological order for destroy: reverse of resolved_order for known components,
        // then alphabetical for unknown. Use a position map from resolved_order.
        let pos: HashMap<&str, usize> = resolved_order
            .iter()
            .enumerate()
            .map(|(i, id)| (id.as_str(), i))
            .collect();
        v.sort_unstable_by(|a, b| {
            let pa = pos.get(a).copied().unwrap_or(usize::MAX);
            let pb = pos.get(b).copied().unwrap_or(usize::MAX);
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
            // Feature not in state → create.
            Classification::Create
        } else {
            // Feature in both → diff resources.
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
        // New resources to add → strengthen.
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
                path: dp,
                entry_type: det,
                op: dop,
                ..
            },
            S::Fs { fs: sf },
        ) => {
            let et_match = matches!(
                (det, &sf.entry_type),
                (
                    model::desired_resource_graph::FsEntryType::File,
                    FsEntryType::File
                ) | (
                    model::desired_resource_graph::FsEntryType::Dir,
                    FsEntryType::Dir
                )
            );
            let op_match = matches!(
                (dop, &sf.op),
                (model::desired_resource_graph::FsOp::Link, FsOp::Link)
                    | (model::desired_resource_graph::FsOp::Copy, FsOp::Copy)
            );
            if dp != &sf.path || !et_match || !op_match {
                return Compatibility::Incompatible {
                    from_version: None,
                    to_version: None,
                };
            }
            Compatibility::Compatible
        }
        // Kind mismatch (e.g. package in desired, runtime in state) → replace.
        _ => Compatibility::Incompatible {
            from_version: None,
            to_version: None,
        },
    }
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{
        desired_resource_graph::{
            ComponentDesiredResources, DesiredResource, DesiredResourceGraph, DesiredResourceKind,
            FsEntryType, FsOp,
        },
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
                    source: None,
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
                    source: None,
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
}
