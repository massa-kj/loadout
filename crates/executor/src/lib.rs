//! Executor: executes a Plan by calling component-host and backend-host, then commits state.
//!
//! Responsibilities:
//! - Process `Plan.actions` in order
//! - For each action, dispatch to `component-host` (script mode) or `backend-host` (declarative)
//! - Maintain per-component atomicity: commit state only if all resources succeed
//! - Emit `Event`s so callers (CLI / app) can show progress without coupling to I/O
//! - Accumulate `ExecutionEnvContext` across actions so PATH and similar variables
//!   propagate from earlier actions (e.g. brew) to later ones (e.g. npm)
//!
//! Error strategy:
//! - Resource failure  → `Event::ResourceFailed` + component aborts → `Event::ComponentFailed` → continue
//! - State commit fail → `ExecutorError` (fatal, stops execution)
//! - Required contributor fail → `ExecutorError` (fatal, stops execution)
//! - Optional contributor fail → `Event::ContributorWarning`, execution continues
//!
//! Fs resources are handled directly by the executor in Phase 4.
//! They will be extracted to a builtin `core/fs` backend in Phase 5.
//!
//! See: `docs/architecture/boundaries.md` (planner/executor boundary)

pub mod activate;
pub use activate::{generate_activation, ShellKind};

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use backend_host::{BackendError, BackendRegistry};
use component_host::Dirs;
use model::component_index::{ComponentIndex, ComponentMode};
use model::desired_resource_graph::{DesiredResource, DesiredResourceGraph, DesiredResourceKind};
use model::env::{ExecutionEnvContext, ExecutionEnvDelta, ExecutionEnvPlan};
use model::id::CanonicalComponentId;
use model::plan::{Operation, Plan, StrengthenDetails};
use model::state::{
    ComponentState, FsDetails, FsEntryType, FsOp, PackageDetails, Resource, ResourceKind,
    RuntimeDetails, State,
};
use platform::Platform;

// ---------------------------------------------------------------------------
// ExecutionEnvContributor trait
// ---------------------------------------------------------------------------

/// Context passed to [`ExecutionEnvContributor::execution_env_delta`].
///
/// Carries the platform and a snapshot of the current accumulated environment
/// so contributors can probe the system or compute deltas based on what is
/// already available.
pub struct ExecutionEnvQuery<'a> {
    pub platform: Platform,
    /// Current state of accumulated env vars (read-only snapshot).
    pub current_vars: &'a std::collections::BTreeMap<String, String>,
}

/// A source of execution environment mutations.
///
/// Implemented by:
/// - builtin backend-adjacent structs (e.g. `BrewContributor`, `MiseContributor`)
/// - runtime provider structs
/// - component contributors that have opted in via `component.yaml`
///
/// Constraints for component contributors:
/// - May only contribute `PrependPath` / `AppendPath` mutations (command/runtime exposure)
/// - Must not mutate global scalar variables (`Set` / `Unset` on shared keys is forbidden)
/// - All mutations must be reversible (i.e. can be undone by `RemovePath`)
///
/// Contributors must **never** return shell fragments. Structured data only.
pub trait ExecutionEnvContributor: Send + Sync {
    /// Probe the system and return the environment delta this contributor provides.
    fn execution_env_delta(
        &self,
        query: &ExecutionEnvQuery<'_>,
    ) -> Result<ExecutionEnvDelta, ContributorError>;

    /// If `true`, a failure from this contributor is fatal (hard fail).
    /// If `false`, the failure is recorded as a warning and execution continues.
    fn is_required(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// ContributorError
// ---------------------------------------------------------------------------

/// Error produced by an [`ExecutionEnvContributor`] invocation.
#[derive(Debug, thiserror::Error)]
#[error("contributor error: {reason}")]
pub struct ContributorError {
    pub reason: String,
}

impl ContributorError {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// ContributorRegistry
// ---------------------------------------------------------------------------

/// Registry that maps backend keys to [`ExecutionEnvContributor`] implementations.
///
/// # Role: secondary Rust-only extension point
///
/// The **primary** mechanism for backend env contributions is `env_pre.sh` /
/// `env_post.sh` shell scripts in the backend plugin directory (see
/// `docs/specs/api/backend.md`). The executor calls `backend.env_pre()` and
/// `backend.env_post()` directly, bypassing this registry entirely.
///
/// This registry exists as a **secondary extension point** for cases where shell
/// scripts are not sufficient:
/// - Rust unit tests that inject mock contributors without touching the filesystem
/// - OS-level API probes that cannot be done in shell (e.g. Windows registry)
/// - Future internal contributors generated from loadout metadata at runtime
///
/// # Registration
///
/// - **Pre-action**: contributor runs before calling a backend with the matching ID.
/// - **Named (post-action)**: contributor runs when explicitly keyed in by e.g. a
///   future post-apply hook. Currently unused in the primary execute path.
///
/// See `backends-builtin::register_contributors` for the entry point.
#[derive(Default)]
pub struct ContributorRegistry {
    /// Backend-ID → contributor to evaluate *before* calling that backend.
    pre_action: HashMap<String, Box<dyn ExecutionEnvContributor>>,
    /// Named contributor key → contributor to evaluate *after* an action
    /// signals it via `BackendApplyResult.post_contributors`.
    named: HashMap<String, Box<dyn ExecutionEnvContributor>>,
}

impl ContributorRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a contributor to run before calls to `backend_id`.
    pub fn register_pre_action(
        &mut self,
        backend_id: impl Into<String>,
        contributor: Box<dyn ExecutionEnvContributor>,
    ) {
        self.pre_action.insert(backend_id.into(), contributor);
    }

    /// Register a named contributor for post-action lookup.
    pub fn register_named(
        &mut self,
        key: impl Into<String>,
        contributor: Box<dyn ExecutionEnvContributor>,
    ) {
        self.named.insert(key.into(), contributor);
    }

    /// Look up the pre-action contributor for a backend (may be absent).
    pub fn pre_for_backend(&self, backend_id: &str) -> Option<&dyn ExecutionEnvContributor> {
        self.pre_action.get(backend_id).map(|c| c.as_ref())
    }

    /// Look up a named contributor by key (may be absent).
    pub fn named(&self, key: &str) -> Option<&dyn ExecutionEnvContributor> {
        self.named.get(key).map(|c| c.as_ref())
    }
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A progress event emitted during execution.
///
/// The caller (app / CLI) receives these via the `on_event` callback.
/// Events are informational; they do not affect the execution flow.
#[derive(Debug, Clone)]
pub enum Event {
    /// A component is about to be processed.
    ComponentStart { id: String },
    /// A component was processed successfully.
    ComponentDone { id: String },
    /// A component failed; execution continues to the next component.
    ComponentFailed { id: String, error: String },
    /// A single resource failed within a component.
    ResourceFailed {
        component_id: String,
        resource_id: String,
        error: String,
    },
    /// An optional contributor failed; execution continues with a warning.
    ContributorWarning { backend_id: String, reason: String },
}

/// Fatal executor errors (unrecoverable; stop all execution).
///
/// These represent invariant violations or I/O failures that make it unsafe
/// to continue modifying state.
#[derive(Debug, thiserror::Error)]
pub enum ExecutorError {
    /// State could not be committed after a successful component execution.
    /// The state file may be corrupt or the filesystem is unavailable.
    #[error("state commit failed: {reason}")]
    StateCommitFailed { reason: String },

    /// A component referenced in the plan is absent from the component index.
    /// This is a programming error: plan and index must be consistent.
    #[error("component not found in index: {id}")]
    ComponentNotInIndex { id: String },

    /// A resource in the desired graph was not found for a component that needs it.
    #[error("desired resources not found for component: {id}")]
    DesiredResourcesNotFound { id: String },

    /// A required contributor failed, making it unsafe to continue execution.
    #[error("required contributor failed for backend '{backend_id}': {reason}")]
    RequiredContributorFailed { backend_id: String, reason: String },
}

/// Result of a component that executed successfully.
#[derive(Debug, Clone)]
pub struct ExecutedComponent {
    pub id: String,
    pub operation: String,
}

/// Result of a component that failed.
#[derive(Debug, Clone)]
pub struct FailedComponent {
    pub id: String,
    pub operation: String,
    pub error: String,
}

/// Per-action record of env contributor activity, included in the apply report.
///
/// Kept as an ephemeral artifact (not written to state) for debugging and
/// future `loadout activate --from-last-apply` support.
#[derive(Debug, Clone)]
pub struct EnvArtifact {
    /// Component (action) that generated this record.
    pub component_id: String,
    /// Phase: "pre" (before action) or "post" (after action).
    pub phase: String,
    /// Contributor key that was evaluated.
    pub contributor_key: String,
    /// The delta returned (mutations + evidence).
    pub delta: ExecutionEnvDelta,
    /// Variables that changed as a result of merging this delta.
    pub changed_vars: Vec<(String, String)>,
}

/// Summary report produced by `execute()`.
#[derive(Debug, Default)]
pub struct ExecutorReport {
    pub executed: Vec<ExecutedComponent>,
    pub failed: Vec<FailedComponent>,
    /// Env contributor invocation records for debugging (ephemeral, not in state).
    pub env_artifacts: Vec<EnvArtifact>,
    /// Final env context snapshot after all actions completed.
    pub final_env_plan: ExecutionEnvPlan,
}

/// All inputs required for a single execution run.
///
/// The executor does not own mutable state; the caller passes `state` as `&mut`
/// so the app can inspect it after execution.
pub struct ExecutionContext<'a> {
    pub plan: &'a Plan,
    pub graph: &'a DesiredResourceGraph,
    pub index: &'a ComponentIndex,
    pub registry: &'a BackendRegistry,
    pub contributors: &'a ContributorRegistry,
    pub dirs: &'a Dirs,
    pub platform: &'a Platform,
    pub state_path: &'a Path,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Execute a plan, mutating `state` in place on each successful component commit.
///
/// `on_event` receives progress events as they occur (non-blocking).
/// Returns a report of executed and failed components.
/// Returns `Err` only on unrecoverable failures (state commit I/O, invariant violation).
pub fn execute(
    ctx: &ExecutionContext<'_>,
    state: &mut State,
    on_event: &mut dyn FnMut(Event),
) -> Result<ExecutorReport, ExecutorError> {
    let mut report = ExecutorReport::default();
    // Seed the env context from the current process environment so that
    // PrependPath/AppendPath mutations correctly extend the existing
    // system PATH rather than replacing it with only the new entries.
    let mut env_ctx = ExecutionEnvContext::from_process_env();

    for action in &ctx.plan.actions {
        let id_str = action.component.as_str().to_string();
        on_event(Event::ComponentStart { id: id_str.clone() });

        let result = execute_action(
            ctx,
            state,
            &action.component,
            &action.operation,
            &action.details,
            &mut env_ctx,
            &mut report.env_artifacts,
            on_event,
        );

        match result {
            Ok(()) => {
                // Commit state after each successful component.
                state::commit(ctx.state_path, state).map_err(|e| {
                    ExecutorError::StateCommitFailed {
                        reason: e.to_string(),
                    }
                })?;
                on_event(Event::ComponentDone { id: id_str.clone() });
                report.executed.push(ExecutedComponent {
                    id: id_str,
                    operation: format!("{:?}", action.operation),
                });
            }
            Err(ComponentError::Resource { resource_id, error }) => {
                on_event(Event::ResourceFailed {
                    component_id: id_str.clone(),
                    resource_id,
                    error: error.clone(),
                });
                on_event(Event::ComponentFailed {
                    id: id_str.clone(),
                    error: error.clone(),
                });
                report.failed.push(FailedComponent {
                    id: id_str,
                    operation: format!("{:?}", action.operation),
                    error,
                });
            }
            Err(ComponentError::Component { error }) => {
                on_event(Event::ComponentFailed {
                    id: id_str.clone(),
                    error: error.clone(),
                });
                report.failed.push(FailedComponent {
                    id: id_str,
                    operation: format!("{:?}", action.operation),
                    error,
                });
            }
        }
    }

    report.final_env_plan = env_ctx.to_plan();
    Ok(report)
}

// ---------------------------------------------------------------------------
// Internal error types
// ---------------------------------------------------------------------------

/// Non-fatal per-component error variants.
enum ComponentError {
    Resource { resource_id: String, error: String },
    Component { error: String },
}

impl From<component_host::ComponentHostError> for ComponentError {
    fn from(e: component_host::ComponentHostError) -> Self {
        ComponentError::Component {
            error: e.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Per-action dispatch
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn execute_action(
    ctx: &ExecutionContext<'_>,
    state: &mut State,
    component_id: &CanonicalComponentId,
    op: &Operation,
    details: &Option<model::plan::ActionDetails>,
    env_ctx: &mut ExecutionEnvContext,
    artifacts: &mut Vec<EnvArtifact>,
    on_event: &mut dyn FnMut(Event),
) -> Result<(), ComponentError> {
    let id_str = component_id.as_str();

    let meta = ctx.index.components.get(id_str).ok_or_else(|| {
        // This is a programming error but we surface it as a component-level failure
        // so execution can continue. The caller will see ComponentFailed.
        ComponentError::Component {
            error: format!("component not found in index: {id_str}"),
        }
    })?;

    match op {
        Operation::Create => {
            match meta.mode {
                ComponentMode::Script => {
                    component_host::run_install(meta, component_id, ctx.dirs, ctx.platform)
                        .map_err(ComponentError::from)?;
                    // Script components are recorded with empty resources.
                    state
                        .components
                        .insert(id_str.to_string(), ComponentState { resources: vec![] });
                }
                ComponentMode::Declarative => {
                    let desired = ctx.graph.components.get(id_str).ok_or_else(|| {
                        ComponentError::Component {
                            error: format!("desired resources not found for: {id_str}"),
                        }
                    })?;

                    let resources = apply_resources(
                        ctx,
                        id_str,
                        &desired.resources,
                        env_ctx,
                        artifacts,
                        on_event,
                    )?;
                    state
                        .components
                        .insert(id_str.to_string(), ComponentState { resources });
                }
            }
        }

        Operation::Destroy => {
            match meta.mode {
                ComponentMode::Script => {
                    component_host::run_uninstall(meta, component_id, ctx.dirs, ctx.platform)
                        .map_err(ComponentError::from)?;
                }
                ComponentMode::Declarative => {
                    // Remove resources using the backend recorded in state (authoritative).
                    if let Some(comp_state) = state.components.get(id_str) {
                        remove_state_resources(ctx, id_str, &comp_state.resources.clone())?;
                    }
                }
            }
            state.components.remove(id_str);
        }

        Operation::Replace => {
            // Destroy old, then create new.
            match meta.mode {
                ComponentMode::Script => {
                    component_host::run_uninstall(meta, component_id, ctx.dirs, ctx.platform)
                        .map_err(ComponentError::from)?;
                    component_host::run_install(meta, component_id, ctx.dirs, ctx.platform)
                        .map_err(ComponentError::from)?;
                    state
                        .components
                        .insert(id_str.to_string(), ComponentState { resources: vec![] });
                }
                ComponentMode::Declarative => {
                    if let Some(comp_state) = state.components.get(id_str) {
                        remove_state_resources(ctx, id_str, &comp_state.resources.clone())?;
                    }
                    let desired = ctx.graph.components.get(id_str).ok_or_else(|| {
                        ComponentError::Component {
                            error: format!("desired resources not found for: {id_str}"),
                        }
                    })?;
                    let resources = apply_resources(
                        ctx,
                        id_str,
                        &desired.resources,
                        env_ctx,
                        artifacts,
                        on_event,
                    )?;
                    state
                        .components
                        .insert(id_str.to_string(), ComponentState { resources });
                }
            }
        }

        Operation::ReplaceBackend => {
            // Remove via old backend (from state), apply via new backend (from graph).
            // Script components don't have a backend concept; treat as Replace.
            if meta.mode == ComponentMode::Script {
                component_host::run_uninstall(meta, component_id, ctx.dirs, ctx.platform)
                    .map_err(ComponentError::from)?;
                component_host::run_install(meta, component_id, ctx.dirs, ctx.platform)
                    .map_err(ComponentError::from)?;
                state
                    .components
                    .insert(id_str.to_string(), ComponentState { resources: vec![] });
            } else {
                if let Some(comp_state) = state.components.get(id_str) {
                    remove_state_resources(ctx, id_str, &comp_state.resources.clone())?;
                }
                let desired =
                    ctx.graph
                        .components
                        .get(id_str)
                        .ok_or_else(|| ComponentError::Component {
                            error: format!("desired resources not found for: {id_str}"),
                        })?;
                let resources = apply_resources(
                    ctx,
                    id_str,
                    &desired.resources,
                    env_ctx,
                    artifacts,
                    on_event,
                )?;
                state
                    .components
                    .insert(id_str.to_string(), ComponentState { resources });
            }
        }

        Operation::Strengthen => {
            // Apply only the add_resources listed in the plan details.
            // Script components do not have strengthen; treat as noop with warning.
            if meta.mode == ComponentMode::Script {
                return Ok(());
            }

            let add = match details {
                Some(model::plan::ActionDetails::Strengthen(StrengthenDetails {
                    add_resources,
                })) => add_resources,
                _ => {
                    return Err(ComponentError::Component {
                        error: "strengthen action missing add_resources details".to_string(),
                    });
                }
            };

            let desired =
                ctx.graph
                    .components
                    .get(id_str)
                    .ok_or_else(|| ComponentError::Component {
                        error: format!("desired resources not found for: {id_str}"),
                    })?;

            // Filter desired resources to only those referenced in add_resources.
            let to_add: Vec<&DesiredResource> = desired
                .resources
                .iter()
                .filter(|r| add.iter().any(|ref_| ref_.id == r.id))
                .collect();

            let new_resources = apply_resources(
                ctx,
                id_str,
                &to_add.iter().map(|r| (*r).clone()).collect::<Vec<_>>(),
                env_ctx,
                artifacts,
                on_event,
            )?;

            // Merge new resources into existing state.
            let comp_state = state
                .components
                .entry(id_str.to_string())
                .or_insert_with(|| ComponentState { resources: vec![] });
            comp_state.resources.extend(new_resources);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Resource-level helpers
// ---------------------------------------------------------------------------

/// Apply all desired resources for a declarative component.
/// Returns the resulting state resources, or a ComponentError on the first failure.
fn apply_resources(
    ctx: &ExecutionContext<'_>,
    component_id: &str,
    desired: &[DesiredResource],
    env_ctx: &mut ExecutionEnvContext,
    artifacts: &mut Vec<EnvArtifact>,
    on_event: &mut dyn FnMut(Event),
) -> Result<Vec<Resource>, ComponentError> {
    let mut resources = Vec::with_capacity(desired.len());

    for dr in desired {
        match apply_one_resource(ctx, component_id, dr, env_ctx, artifacts, on_event) {
            Ok(state_resource) => resources.push(state_resource),
            Err(e) => {
                return Err(ComponentError::Resource {
                    resource_id: dr.id.clone(),
                    error: format!("[{component_id}] resource '{}' failed: {e}", dr.id),
                });
            }
        }
    }

    Ok(resources)
}

/// Apply a single desired resource and update the execution env context.
///
/// Before calling the backend, any registered pre-action contributor for that
/// backend is evaluated and its delta is merged into `env_ctx`. The current
/// env_ctx is then exported to subprocess-level env vars so the backend sees
/// the cumulative environment. After a successful apply, any post-contributors
/// signalled by [`BackendApplyResult::post_contributors`] are evaluated.
fn apply_one_resource(
    ctx: &ExecutionContext<'_>,
    component_id: &str,
    dr: &DesiredResource,
    env_ctx: &mut ExecutionEnvContext,
    artifacts: &mut Vec<EnvArtifact>,
    on_event: &mut dyn FnMut(Event),
) -> Result<Resource, String> {
    match &dr.kind {
        DesiredResourceKind::Package {
            name,
            desired_backend,
        } => {
            let backend_key = desired_backend.as_str();
            let backend = ctx
                .registry
                .get(desired_backend)
                .map_err(|e| e.to_string())?;

            // Query pre-action env delta from the backend plugin (non-fatal on failure).
            match backend.env_pre(dr) {
                Ok(Some(delta)) => {
                    merge_backend_delta(
                        delta,
                        "pre",
                        backend_key,
                        component_id,
                        env_ctx,
                        *ctx.platform,
                        artifacts,
                    );
                }
                Ok(None) => {}
                Err(e) => {
                    on_event(Event::ContributorWarning {
                        backend_id: backend_key.to_string(),
                        reason: format!("env_pre failed: {e}"),
                    });
                }
            }

            // Export accumulated env to subprocess environment.
            for (k, v) in &env_ctx.vars {
                // SAFETY: env mutation is intentional; executor controls the process.
                // On Windows this is also safe — std::env::set_var is cfg-guarded by stdlib.
                #[allow(unsafe_code)]
                unsafe {
                    std::env::set_var(k, v);
                }
            }

            backend.apply(dr).map_err(|e| e.to_string())?;

            // Query post-action env delta from the backend plugin (non-fatal on failure).
            match backend.env_post(dr) {
                Ok(Some(delta)) => {
                    merge_backend_delta(
                        delta,
                        "post",
                        backend_key,
                        component_id,
                        env_ctx,
                        *ctx.platform,
                        artifacts,
                    );
                }
                Ok(None) => {}
                Err(e) => {
                    on_event(Event::ContributorWarning {
                        backend_id: backend_key.to_string(),
                        reason: format!("env_post failed: {e}"),
                    });
                }
            }

            Ok(Resource {
                id: dr.id.clone(),
                kind: ResourceKind::Package {
                    backend: desired_backend.clone(),
                    package: PackageDetails {
                        name: name.clone(),
                        version: None,
                    },
                },
            })
        }
        DesiredResourceKind::Runtime {
            name,
            version,
            desired_backend,
        } => {
            let backend_key = desired_backend.as_str();
            let backend = ctx
                .registry
                .get(desired_backend)
                .map_err(|e| e.to_string())?;

            // Query pre-action env delta from the backend plugin (non-fatal on failure).
            match backend.env_pre(dr) {
                Ok(Some(delta)) => {
                    merge_backend_delta(
                        delta,
                        "pre",
                        backend_key,
                        component_id,
                        env_ctx,
                        *ctx.platform,
                        artifacts,
                    );
                }
                Ok(None) => {}
                Err(e) => {
                    on_event(Event::ContributorWarning {
                        backend_id: backend_key.to_string(),
                        reason: format!("env_pre failed: {e}"),
                    });
                }
            }

            // Export accumulated env to subprocess environment.
            for (k, v) in &env_ctx.vars {
                #[allow(unsafe_code)]
                unsafe {
                    std::env::set_var(k, v);
                }
            }

            backend.apply(dr).map_err(|e| e.to_string())?;

            // Query post-action env delta from the backend plugin (non-fatal on failure).
            match backend.env_post(dr) {
                Ok(Some(delta)) => {
                    merge_backend_delta(
                        delta,
                        "post",
                        backend_key,
                        component_id,
                        env_ctx,
                        *ctx.platform,
                        artifacts,
                    );
                }
                Ok(None) => {}
                Err(e) => {
                    on_event(Event::ContributorWarning {
                        backend_id: backend_key.to_string(),
                        reason: format!("env_post failed: {e}"),
                    });
                }
            }

            Ok(Resource {
                id: dr.id.clone(),
                kind: ResourceKind::Runtime {
                    backend: desired_backend.clone(),
                    runtime: RuntimeDetails {
                        name: name.clone(),
                        version: version.clone(),
                    },
                },
            })
        }
        DesiredResourceKind::Fs {
            path,
            entry_type,
            op,
            ..
        } => {
            // Phase 4: Fs operations are handled directly by the executor.
            // Phase 5 will extract this into a builtin `core/fs` backend.
            //
            // Expand `~` before executing AND before storing in state.
            // Storing the expanded (absolute) path is required for the state
            // invariant check (`fs.path must be absolute`) and ensures that
            // `remove_fs` later receives an absolute path from state.
            let expanded = expand_home(path);
            let expanded_str = expanded.to_string_lossy().into_owned();
            apply_fs(path, entry_type, op)?;
            Ok(Resource {
                id: dr.id.clone(),
                kind: ResourceKind::Fs {
                    fs: FsDetails {
                        path: expanded_str,
                        entry_type: map_fs_entry_type(entry_type, op),
                        op: map_fs_op(op),
                    },
                },
            })
        }
        DesiredResourceKind::Tool { .. } => {
            // Tool resources are installed by managed_script component scripts.
            // This path should not be reached: managed_script apply is handled by a
            // separate code path (Phase 6), not by apply_one_resource.
            Err(format!(
                "[{component_id}] resource '{}': tool resources must be handled by managed_script executor, not apply_one_resource",
                dr.id
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Backend env delta merge helper
// ---------------------------------------------------------------------------

/// Merge an [`ExecutionEnvDelta`] returned from `backend.env_pre()` or
/// `backend.env_post()` into `env_ctx` and record an [`EnvArtifact`].
///
/// This is the primary mechanism for plugins to contribute environment
/// variables to the execution session. Rust-based [`ExecutionEnvContributor`]
/// implementations are a secondary, extension-point-only mechanism.
#[allow(clippy::too_many_arguments)]
fn merge_backend_delta(
    delta: model::env::ExecutionEnvDelta,
    phase: &str,
    backend_key: &str,
    component_id: &str,
    env_ctx: &mut ExecutionEnvContext,
    platform: Platform,
    artifacts: &mut Vec<EnvArtifact>,
) {
    // Collect affected keys before consuming `delta` in merge().
    let affected_keys: Vec<String> = delta
        .mutations
        .iter()
        .map(|m| match m {
            model::env::EnvMutation::Set { key, .. }
            | model::env::EnvMutation::Unset { key }
            | model::env::EnvMutation::PrependPath { key, .. }
            | model::env::EnvMutation::AppendPath { key, .. }
            | model::env::EnvMutation::RemovePath { key, .. } => key.clone(),
        })
        .collect();
    env_ctx.merge(&delta, platform);
    let changed_vars: Vec<(String, String)> = affected_keys
        .into_iter()
        .map(|k| {
            let v = env_ctx.vars.get(&k).cloned().unwrap_or_default();
            (k, v)
        })
        .collect();
    artifacts.push(EnvArtifact {
        component_id: component_id.to_string(),
        phase: phase.to_string(),
        contributor_key: backend_key.to_string(),
        delta,
        changed_vars,
    });
}

// ---------------------------------------------------------------------------
// Contributor evaluation helper
// ---------------------------------------------------------------------------

/// Evaluate one contributor, merge its delta into `env_ctx`, and record the
/// artifact. Non-fatal contributors emit a [`Event::ContributorWarning`] on
/// failure; required contributors propagate a hard error.
///
/// This is a secondary Rust extension point. Primary env contributions are
/// handled via `backend.env_pre()` / `backend.env_post()` (see
/// `merge_backend_delta`). This function is kept for cases where a Rust
/// struct must contribute env without a corresponding script backend.
#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
fn evaluate_contributor(
    key: &str,
    contributor: &dyn ExecutionEnvContributor,
    phase: &str,
    component_id: &str,
    env_ctx: &mut ExecutionEnvContext,
    platform: Platform,
    artifacts: &mut Vec<EnvArtifact>,
    on_event: &mut dyn FnMut(Event),
) -> Result<(), String> {
    let query = ExecutionEnvQuery {
        platform,
        current_vars: &env_ctx.vars,
    };
    match contributor.execution_env_delta(&query) {
        Ok(delta) => {
            // Collect which keys are affected before merging.
            let affected_keys: Vec<String> = delta
                .mutations
                .iter()
                .map(|m| match m {
                    model::env::EnvMutation::Set { key, .. }
                    | model::env::EnvMutation::Unset { key }
                    | model::env::EnvMutation::PrependPath { key, .. }
                    | model::env::EnvMutation::AppendPath { key, .. }
                    | model::env::EnvMutation::RemovePath { key, .. } => key.clone(),
                })
                .collect();
            env_ctx.merge(&delta, platform);
            // Record (key, new_value) pairs after merging.
            let changed_vars: Vec<(String, String)> = affected_keys
                .into_iter()
                .map(|k| {
                    let v = env_ctx.vars.get(&k).cloned().unwrap_or_default();
                    (k, v)
                })
                .collect();
            artifacts.push(EnvArtifact {
                component_id: component_id.to_string(),
                phase: phase.to_string(),
                contributor_key: key.to_string(),
                delta,
                changed_vars,
            });
        }
        Err(e) => {
            if contributor.is_required() {
                return Err(format!("required contributor '{key}' failed: {e}"));
            } else {
                on_event(Event::ContributorWarning {
                    backend_id: key.to_string(),
                    reason: e.to_string(),
                });
            }
        }
    }
    Ok(())
}

/// Remove state resources using the backend recorded at install time (authoritative).
fn remove_state_resources(
    ctx: &ExecutionContext<'_>,
    component_id: &str,
    resources: &[Resource],
) -> Result<(), ComponentError> {
    for res in resources {
        match remove_one_state_resource(ctx, res) {
            Ok(()) => {}
            Err(e) => {
                return Err(ComponentError::Resource {
                    resource_id: res.id.clone(),
                    error: format!("[{component_id}] resource '{}' remove failed: {e}", res.id),
                });
            }
        }
    }
    Ok(())
}

fn remove_one_state_resource(ctx: &ExecutionContext<'_>, res: &Resource) -> Result<(), String> {
    match &res.kind {
        ResourceKind::Package { backend, package } => {
            // Build a minimal DesiredResource so the backend can identify what to remove.
            // The backend receives the same shape it would for apply.
            let dr = DesiredResource {
                id: res.id.clone(),
                kind: DesiredResourceKind::Package {
                    name: package.name.clone(),
                    desired_backend: backend.clone(),
                },
            };
            let b = ctx
                .registry
                .get(backend)
                .map_err(|e: BackendError| e.to_string())?;
            b.remove(&dr).map_err(|e| e.to_string())?;
        }
        ResourceKind::Runtime { backend, runtime } => {
            let dr = DesiredResource {
                id: res.id.clone(),
                kind: DesiredResourceKind::Runtime {
                    name: runtime.name.clone(),
                    version: runtime.version.clone(),
                    desired_backend: backend.clone(),
                },
            };
            let b = ctx
                .registry
                .get(backend)
                .map_err(|e: BackendError| e.to_string())?;
            b.remove(&dr).map_err(|e| e.to_string())?;
        }
        ResourceKind::Fs { fs } => {
            remove_fs(&fs.path, &fs.entry_type)?;
        }
        ResourceKind::Tool { .. } => {
            // Tool resources are removed by managed_script component scripts.
            // This path should not be reached: managed_script destroy is handled by a
            // separate code path (Phase 6), not by remove_one_state_resource.
            return Err(format!(
                "resource '{}': tool resources must be handled by managed_script executor, not remove_one_state_resource",
                res.id
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Fs operations (Phase 4 — will move to builtin backend in Phase 5)
// ---------------------------------------------------------------------------

use model::desired_resource_graph::{FsEntryType as DesiredFsEntryType, FsOp as DesiredFsOp};

/// Perform a filesystem apply operation (link or copy).
///
/// `path` supports `~` prefix which is expanded to `$HOME`.
fn apply_fs(path: &str, entry_type: &DesiredFsEntryType, op: &DesiredFsOp) -> Result<(), String> {
    let target = expand_home(path);

    // Ensure parent directory exists.
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create_dir_all {:?}: {e}", parent))?;
    }

    match op {
        DesiredFsOp::Link => {
            // Remove any existing entry at target before linking.
            if target.exists() || target.symlink_metadata().is_ok() {
                std::fs::remove_file(&target)
                    .or_else(|_| std::fs::remove_dir_all(&target))
                    .map_err(|e| format!("remove existing {:?}: {e}", target))?;
            }
            #[cfg(unix)]
            {
                // Source for symlink is not available here; the caller (script) sets it up.
                // Executor only records the operation; actual symlink creation would need source.
                // Phase 4 placeholder: we mark success without actually symlinking.
                // TODO Phase 5: pass source path and create symlink properly.
                let _ = (entry_type, &target);
            }
            #[cfg(not(unix))]
            {
                // Windows junction / symlink support deferred to Phase 5.
            }
        }
        DesiredFsOp::Copy => {
            // Phase 4 placeholder: actual copy logic deferred to Phase 5 builtin backend.
            let _ = (entry_type, &target);
        }
    }

    Ok(())
}

/// Remove a filesystem entry recorded in state.
fn remove_fs(path: &str, entry_type: &FsEntryType) -> Result<(), String> {
    let target = expand_home(path);
    if !target.exists() && target.symlink_metadata().is_err() {
        return Ok(()); // Already absent; idempotent.
    }
    match entry_type {
        FsEntryType::File | FsEntryType::Symlink | FsEntryType::Junction => {
            std::fs::remove_file(&target).map_err(|e| format!("remove file {:?}: {e}", target))?;
        }
        FsEntryType::Dir => {
            std::fs::remove_dir_all(&target)
                .map_err(|e| format!("remove dir {:?}: {e}", target))?;
        }
    }
    Ok(())
}

fn expand_home(path: &str) -> PathBuf {
    // Strip `~/` (Unix) or `~\` (Windows) prefix.
    let rest = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\"));
    if let Some(rest) = rest {
        // HOME is set on Linux/WSL and sometimes on Windows (Git Bash, etc.).
        // USERPROFILE is the canonical home variable on Windows.
        let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"));
        if let Ok(home) = home {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

fn map_fs_entry_type(et: &DesiredFsEntryType, op: &DesiredFsOp) -> FsEntryType {
    match op {
        DesiredFsOp::Link => FsEntryType::Symlink,
        DesiredFsOp::Copy => match et {
            DesiredFsEntryType::File => FsEntryType::File,
            DesiredFsEntryType::Dir => FsEntryType::Dir,
        },
    }
}

fn map_fs_op(op: &DesiredFsOp) -> FsOp {
    match op {
        DesiredFsOp::Link => FsOp::Link,
        DesiredFsOp::Copy => FsOp::Copy,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use backend_host::{Backend, BackendError, BackendRegistry};
    use model::component_index::{
        ComponentIndex, ComponentMeta, ComponentMode, DepSpec, COMPONENT_INDEX_SCHEMA_VERSION,
    };
    use model::desired_resource_graph::{
        ComponentDesiredResources, DesiredResource, DesiredResourceGraph, DesiredResourceKind,
        DESIRED_RESOURCE_GRAPH_SCHEMA_VERSION,
    };
    use model::id::{CanonicalBackendId, CanonicalComponentId};
    use model::plan::{ActionDetails, PlanAction, PlanSummary, StrengthenDetails};
    use model::state::State;
    use platform::Dirs;

    use tempfile::TempDir;

    // --- Test doubles -------------------------------------------------------

    struct OkBackend;
    impl Backend for OkBackend {
        fn apply(
            &self,
            _r: &DesiredResource,
        ) -> Result<backend_host::BackendApplyResult, BackendError> {
            Ok(backend_host::BackendApplyResult::none())
        }
        fn remove(&self, _r: &DesiredResource) -> Result<(), BackendError> {
            Ok(())
        }
        fn status(
            &self,
            _r: &DesiredResource,
        ) -> Result<backend_host::ResourceState, BackendError> {
            Ok(backend_host::ResourceState::Installed)
        }
    }

    struct FailBackend;
    impl Backend for FailBackend {
        fn apply(
            &self,
            _r: &DesiredResource,
        ) -> Result<backend_host::BackendApplyResult, BackendError> {
            Err(BackendError::ScriptFailed {
                exit_code: 1,
                stderr: "fail".to_string(),
            })
        }
        fn remove(&self, _r: &DesiredResource) -> Result<(), BackendError> {
            Err(BackendError::ScriptFailed {
                exit_code: 1,
                stderr: "fail".to_string(),
            })
        }
        fn status(
            &self,
            _r: &DesiredResource,
        ) -> Result<backend_host::ResourceState, BackendError> {
            Ok(backend_host::ResourceState::NotInstalled)
        }
    }

    // --- Builder helpers ---------------------------------------------------

    fn backend_id(s: &str) -> CanonicalBackendId {
        CanonicalBackendId::new(s).unwrap()
    }

    fn component_id(s: &str) -> CanonicalComponentId {
        CanonicalComponentId::new(s).unwrap()
    }

    fn declarative_meta() -> ComponentMeta {
        ComponentMeta {
            spec_version: 1,
            mode: ComponentMode::Declarative,
            description: None,
            source_dir: "/tmp".to_string(),
            dep: DepSpec::default(),
            spec: None,
        }
    }

    fn script_meta(source_dir: &str) -> ComponentMeta {
        ComponentMeta {
            spec_version: 1,
            mode: ComponentMode::Script,
            description: None,
            source_dir: source_dir.to_string(),
            dep: DepSpec::default(),
            spec: None,
        }
    }

    fn package_resource(id: &str, name: &str, backend: &str) -> DesiredResource {
        DesiredResource {
            id: id.to_string(),
            kind: DesiredResourceKind::Package {
                name: name.to_string(),
                desired_backend: backend_id(backend),
            },
        }
    }

    fn make_index(entries: Vec<(&str, ComponentMeta)>) -> ComponentIndex {
        ComponentIndex {
            schema_version: COMPONENT_INDEX_SCHEMA_VERSION,
            components: entries
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        }
    }

    fn make_graph(entries: Vec<(&str, Vec<DesiredResource>)>) -> DesiredResourceGraph {
        DesiredResourceGraph {
            schema_version: DESIRED_RESOURCE_GRAPH_SCHEMA_VERSION,
            components: entries
                .into_iter()
                .map(|(k, v)| (k.to_string(), ComponentDesiredResources { resources: v }))
                .collect(),
        }
    }

    fn make_plan(actions: Vec<PlanAction>) -> Plan {
        Plan {
            actions,
            noops: vec![],
            blocked: vec![],
            summary: PlanSummary::default(),
        }
    }

    fn make_action(component: &str, op: Operation) -> PlanAction {
        PlanAction {
            component: component_id(component),
            operation: op,
            details: None,
        }
    }

    fn make_dirs(tmp: &TempDir) -> Dirs {
        Dirs {
            config_home: tmp.path().join("config"),
            data_home: tmp.path().join("data"),
            state_home: tmp.path().join("state"),
            cache_home: tmp.path().join("cache"),
        }
    }

    fn make_registry_ok(backend_ids: &[&str]) -> BackendRegistry {
        let mut reg = BackendRegistry::new();
        for id in backend_ids {
            reg.register(backend_id(id), Box::new(OkBackend));
        }
        reg
    }

    fn collect_events(events: &[Event]) -> (Vec<String>, Vec<String>, Vec<String>) {
        let starts: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::ComponentStart { id } => Some(id.clone()),
                _ => None,
            })
            .collect();
        let dones: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::ComponentDone { id } => Some(id.clone()),
                _ => None,
            })
            .collect();
        let failed: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::ComponentFailed { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect();
        (starts, dones, failed)
    }

    // --- Tests --------------------------------------------------------------

    /// Create declarative component: resources applied, state committed.
    #[test]
    fn create_declarative_success_updates_state() {
        let tmp = tempfile::tempdir().unwrap();
        let state_path = tmp.path().join("state.json");

        let plan = make_plan(vec![make_action("core/git", Operation::Create)]);
        let graph = make_graph(vec![(
            "core/git",
            vec![package_resource("package:git", "git", "core/brew")],
        )]);
        let index = make_index(vec![("core/git", declarative_meta())]);
        let registry = make_registry_ok(&["core/brew"]);
        let mut state = State::empty();
        let dirs = make_dirs(&tmp);
        let platform = Platform::Linux;

        let ctx = ExecutionContext {
            plan: &plan,
            graph: &graph,
            index: &index,
            registry: &registry,
            dirs: &dirs,
            platform: &platform,
            state_path: &state_path,
            contributors: &ContributorRegistry::new(),
        };

        let mut events = vec![];
        let report = execute(&ctx, &mut state, &mut |e| events.push(e)).unwrap();

        assert_eq!(report.executed.len(), 1);
        assert!(report.failed.is_empty());
        assert!(state.components.contains_key("core/git"));
        assert_eq!(state.components["core/git"].resources.len(), 1);
        assert!(state_path.exists(), "state file should be committed");

        let (starts, dones, failed) = collect_events(&events);
        assert_eq!(starts, ["core/git"]);
        assert_eq!(dones, ["core/git"]);
        assert!(failed.is_empty());
    }

    /// Create declarative component with failing backend: ComponentFailed emitted, state unchanged.
    #[test]
    fn create_declarative_resource_fail_emits_component_failed() {
        let tmp = tempfile::tempdir().unwrap();
        let state_path = tmp.path().join("state.json");

        let plan = make_plan(vec![make_action("core/git", Operation::Create)]);
        let graph = make_graph(vec![(
            "core/git",
            vec![package_resource("package:git", "git", "core/brew")],
        )]);
        let index = make_index(vec![("core/git", declarative_meta())]);

        // Use FailBackend so apply returns Err.
        let mut registry = BackendRegistry::new();
        registry.register(backend_id("core/brew"), Box::new(FailBackend));

        let mut state = State::empty();
        let dirs = make_dirs(&tmp);
        let platform = Platform::Linux;
        let ctx = ExecutionContext {
            plan: &plan,
            graph: &graph,
            index: &index,
            registry: &registry,
            dirs: &dirs,
            platform: &platform,
            state_path: &state_path,
            contributors: &ContributorRegistry::new(),
        };

        let mut events = vec![];
        let report = execute(&ctx, &mut state, &mut |e| events.push(e)).unwrap();

        assert!(report.executed.is_empty());
        assert_eq!(report.failed.len(), 1);
        // State must not contain the component.
        assert!(!state.components.contains_key("core/git"));

        let resource_failed = events
            .iter()
            .any(|e| matches!(e, Event::ResourceFailed { .. }));
        let component_failed = events
            .iter()
            .any(|e| matches!(e, Event::ComponentFailed { .. }));
        assert!(resource_failed, "expected ResourceFailed event");
        assert!(component_failed, "expected ComponentFailed event");
    }

    /// Destroy declarative component: resources removed, state cleared.
    #[test]
    fn destroy_declarative_success_removes_state() {
        let tmp = tempfile::tempdir().unwrap();
        let state_path = tmp.path().join("state.json");

        // Pre-populate state.
        let mut state = State::empty();
        state.components.insert(
            "core/git".to_string(),
            ComponentState {
                resources: vec![Resource {
                    id: "package:git".to_string(),
                    kind: ResourceKind::Package {
                        backend: backend_id("core/brew"),
                        package: PackageDetails {
                            name: "git".to_string(),
                            version: None,
                        },
                    },
                }],
            },
        );

        let plan = make_plan(vec![make_action("core/git", Operation::Destroy)]);
        let graph = make_graph(vec![]);
        let index = make_index(vec![("core/git", declarative_meta())]);
        let registry = make_registry_ok(&["core/brew"]);
        let dirs = make_dirs(&tmp);
        let platform = Platform::Linux;
        let ctx = ExecutionContext {
            plan: &plan,
            graph: &graph,
            index: &index,
            registry: &registry,
            dirs: &dirs,
            platform: &platform,
            state_path: &state_path,
            contributors: &ContributorRegistry::new(),
        };

        let mut events = vec![];
        let report = execute(&ctx, &mut state, &mut |e| events.push(e)).unwrap();

        assert_eq!(report.executed.len(), 1);
        assert!(
            !state.components.contains_key("core/git"),
            "component must be removed from state"
        );
    }

    /// Destroy declarative component with failing backend: state unchanged.
    #[test]
    fn destroy_resource_fail_leaves_state_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let state_path = tmp.path().join("state.json");

        let mut state = State::empty();
        state.components.insert(
            "core/git".to_string(),
            ComponentState {
                resources: vec![Resource {
                    id: "package:git".to_string(),
                    kind: ResourceKind::Package {
                        backend: backend_id("core/brew"),
                        package: PackageDetails {
                            name: "git".to_string(),
                            version: None,
                        },
                    },
                }],
            },
        );

        let plan = make_plan(vec![make_action("core/git", Operation::Destroy)]);
        let graph = make_graph(vec![]);
        let index = make_index(vec![("core/git", declarative_meta())]);
        let mut registry = BackendRegistry::new();
        registry.register(backend_id("core/brew"), Box::new(FailBackend));

        let dirs = make_dirs(&tmp);
        let platform = Platform::Linux;
        let ctx = ExecutionContext {
            plan: &plan,
            graph: &graph,
            index: &index,
            registry: &registry,
            dirs: &dirs,
            platform: &platform,
            state_path: &state_path,
            contributors: &ContributorRegistry::new(),
        };

        let mut events = vec![];
        let report = execute(&ctx, &mut state, &mut |e| events.push(e)).unwrap();

        assert!(report.failed.len() == 1);
        // State must still have the component.
        assert!(state.components.contains_key("core/git"));
    }

    /// Multiple components: failed component does not stop subsequent components.
    #[test]
    fn failed_component_does_not_stop_next_component() {
        let tmp = tempfile::tempdir().unwrap();
        let state_path = tmp.path().join("state.json");

        let plan = make_plan(vec![
            make_action("core/git", Operation::Create),  // will fail
            make_action("core/node", Operation::Create), // should still run
        ]);
        let graph = make_graph(vec![
            (
                "core/git",
                vec![package_resource("package:git", "git", "core/fail")],
            ),
            (
                "core/node",
                vec![package_resource("package:node", "node", "core/brew")],
            ),
        ]);
        let index = make_index(vec![
            ("core/git", declarative_meta()),
            ("core/node", declarative_meta()),
        ]);

        let mut registry = BackendRegistry::new();
        registry.register(backend_id("core/fail"), Box::new(FailBackend));
        registry.register(backend_id("core/brew"), Box::new(OkBackend));

        let mut state = State::empty();
        let dirs = make_dirs(&tmp);
        let platform = Platform::Linux;
        let ctx = ExecutionContext {
            plan: &plan,
            graph: &graph,
            index: &index,
            registry: &registry,
            dirs: &dirs,
            platform: &platform,
            state_path: &state_path,
            contributors: &ContributorRegistry::new(),
        };

        let mut events = vec![];
        let report = execute(&ctx, &mut state, &mut |e| events.push(e)).unwrap();

        assert_eq!(report.executed.len(), 1, "core/node should succeed");
        assert_eq!(report.failed.len(), 1, "core/git should fail");
        assert!(state.components.contains_key("core/node"));
        assert!(!state.components.contains_key("core/git"));
    }

    /// Strengthen adds only the listed resources to existing state.
    #[test]
    fn strengthen_adds_resources_to_existing_state() {
        let tmp = tempfile::tempdir().unwrap();
        let state_path = tmp.path().join("state.json");

        // Start with one already-installed resource.
        let mut state = State::empty();
        state.components.insert(
            "core/tools".to_string(),
            ComponentState {
                resources: vec![Resource {
                    id: "package:git".to_string(),
                    kind: ResourceKind::Package {
                        backend: backend_id("core/brew"),
                        package: PackageDetails {
                            name: "git".to_string(),
                            version: None,
                        },
                    },
                }],
            },
        );

        // Plan adds ripgrep via strengthen.
        let strengthen_action = PlanAction {
            component: component_id("core/tools"),
            operation: Operation::Strengthen,
            details: Some(ActionDetails::Strengthen(StrengthenDetails {
                add_resources: vec![model::plan::ResourceRef {
                    kind: "package".to_string(),
                    id: "package:ripgrep".to_string(),
                }],
            })),
        };

        let plan = make_plan(vec![strengthen_action]);
        let graph = make_graph(vec![(
            "core/tools",
            vec![
                package_resource("package:git", "git", "core/brew"),
                package_resource("package:ripgrep", "ripgrep", "core/brew"),
            ],
        )]);
        let index = make_index(vec![("core/tools", declarative_meta())]);
        let registry = make_registry_ok(&["core/brew"]);
        let dirs = make_dirs(&tmp);
        let platform = Platform::Linux;
        let ctx = ExecutionContext {
            plan: &plan,
            graph: &graph,
            index: &index,
            registry: &registry,
            dirs: &dirs,
            platform: &platform,
            state_path: &state_path,
            contributors: &ContributorRegistry::new(),
        };

        let mut events = vec![];
        let report = execute(&ctx, &mut state, &mut |e| events.push(e)).unwrap();

        assert_eq!(report.executed.len(), 1);
        let feat = &state.components["core/tools"];
        assert_eq!(
            feat.resources.len(),
            2,
            "both git and ripgrep should be in state"
        );
    }

    /// Create script component: install script executed, empty resources recorded in state.
    #[test]
    fn create_script_component_records_empty_resources() {
        let tmp = tempfile::tempdir().unwrap();
        let state_path = tmp.path().join("state.json");
        let feat_dir = tmp.path().join("feat");
        std::fs::create_dir_all(&feat_dir).unwrap();

        // Write a minimal platform-appropriate install script.
        let platform = platform::detect_platform();
        let script_name = match platform {
            Platform::Windows => "install.ps1",
            Platform::Linux | Platform::Wsl => "install.sh",
        };
        let script_content = match platform {
            Platform::Windows => "exit 0\n",
            Platform::Linux | Platform::Wsl => "#!/usr/bin/env sh\nexit 0\n",
        };
        let script = feat_dir.join(script_name);
        std::fs::write(&script, script_content).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let plan = make_plan(vec![make_action("core/brew", Operation::Create)]);
        let graph = make_graph(vec![]);
        let index = make_index(vec![("core/brew", script_meta(feat_dir.to_str().unwrap()))]);
        let registry = BackendRegistry::new();
        let mut state = State::empty();
        let dirs = make_dirs(&tmp);
        let ctx = ExecutionContext {
            plan: &plan,
            graph: &graph,
            index: &index,
            registry: &registry,
            dirs: &dirs,
            platform: &platform,
            state_path: &state_path,
            contributors: &ContributorRegistry::new(),
        };

        let mut events = vec![];
        let report = execute(&ctx, &mut state, &mut |e| events.push(e)).unwrap();

        assert_eq!(report.executed.len(), 1);
        assert!(state.components.contains_key("core/brew"));
        assert!(state.components["core/brew"].resources.is_empty());
    }

    /// Replace declarative: old resources removed, new resources applied.
    #[test]
    fn replace_declarative_updates_state() {
        let tmp = tempfile::tempdir().unwrap();
        let state_path = tmp.path().join("state.json");

        let mut state = State::empty();
        state.components.insert(
            "core/git".to_string(),
            ComponentState {
                resources: vec![Resource {
                    id: "package:git".to_string(),
                    kind: ResourceKind::Package {
                        backend: backend_id("core/brew"),
                        package: PackageDetails {
                            name: "git".to_string(),
                            version: None,
                        },
                    },
                }],
            },
        );

        let plan = make_plan(vec![make_action("core/git", Operation::Replace)]);
        let graph = make_graph(vec![(
            "core/git",
            vec![package_resource("package:git", "git", "core/apt")],
        )]);
        let index = make_index(vec![("core/git", declarative_meta())]);
        let registry = make_registry_ok(&["core/brew", "core/apt"]);
        let dirs = make_dirs(&tmp);
        let platform = Platform::Linux;
        let ctx = ExecutionContext {
            plan: &plan,
            graph: &graph,
            index: &index,
            registry: &registry,
            dirs: &dirs,
            platform: &platform,
            state_path: &state_path,
            contributors: &ContributorRegistry::new(),
        };

        let mut events = vec![];
        let report = execute(&ctx, &mut state, &mut |e| events.push(e)).unwrap();

        assert_eq!(report.executed.len(), 1);
        // Backend should now be core/apt.
        let feat = &state.components["core/git"];
        match &feat.resources[0].kind {
            ResourceKind::Package { backend, .. } => {
                assert_eq!(backend.as_str(), "core/apt");
            }
            _ => panic!("expected package"),
        }
    }
}
