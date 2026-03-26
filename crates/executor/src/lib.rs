//! Executor: executes a Plan by calling feature-host and backend-host, then commits state.
//!
//! Responsibilities:
//! - Process `Plan.actions` in order
//! - For each action, dispatch to `feature-host` (script mode) or `backend-host` (declarative)
//! - Maintain per-feature atomicity: commit state only if all resources succeed
//! - Emit `Event`s so callers (CLI / app) can show progress without coupling to I/O
//!
//! Error strategy:
//! - Resource failure  → `Event::ResourceFailed` + feature aborts → `Event::FeatureFailed` → continue
//! - State commit fail → `ExecutorError` (fatal, stops execution)
//!
//! Fs resources are handled directly by the executor in Phase 4.
//! They will be extracted to a builtin `core/fs` backend in Phase 5.
//!
//! See: `docs/architecture/boundaries.md` (planner/executor boundary)

use std::path::{Path, PathBuf};

use backend_host::{BackendError, BackendRegistry};
use feature_host::Dirs;
use model::desired_resource_graph::{DesiredResource, DesiredResourceGraph, DesiredResourceKind};
use model::feature_index::{FeatureIndex, FeatureMode};
use model::id::CanonicalFeatureId;
use model::plan::{Operation, Plan, StrengthenDetails};
use model::state::{
    FeatureState, FsDetails, FsEntryType, FsOp, PackageDetails, Resource, ResourceKind,
    RuntimeDetails, State,
};
use platform::Platform;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A progress event emitted during execution.
///
/// The caller (app / CLI) receives these via the `on_event` callback.
/// Events are informational; they do not affect the execution flow.
#[derive(Debug, Clone)]
pub enum Event {
    /// A feature is about to be processed.
    FeatureStart { id: String },
    /// A feature was processed successfully.
    FeatureDone { id: String },
    /// A feature failed; execution continues to the next feature.
    FeatureFailed { id: String, error: String },
    /// A single resource failed within a feature.
    ResourceFailed {
        feature_id: String,
        resource_id: String,
        error: String,
    },
}

/// Fatal executor errors (unrecoverable; stop all execution).
///
/// These represent invariant violations or I/O failures that make it unsafe
/// to continue modifying state.
#[derive(Debug, thiserror::Error)]
pub enum ExecutorError {
    /// State could not be committed after a successful feature execution.
    /// The state file may be corrupt or the filesystem is unavailable.
    #[error("state commit failed: {reason}")]
    StateCommitFailed { reason: String },

    /// A feature referenced in the plan is absent from the feature index.
    /// This is a programming error: plan and index must be consistent.
    #[error("feature not found in index: {id}")]
    FeatureNotInIndex { id: String },

    /// A resource in the desired graph was not found for a feature that needs it.
    #[error("desired resources not found for feature: {id}")]
    DesiredResourcesNotFound { id: String },
}

/// Result of a feature that executed successfully.
#[derive(Debug, Clone)]
pub struct ExecutedFeature {
    pub id: String,
    pub operation: String,
}

/// Result of a feature that failed.
#[derive(Debug, Clone)]
pub struct FailedFeature {
    pub id: String,
    pub operation: String,
    pub error: String,
}

/// Summary report produced by `execute()`.
#[derive(Debug, Default)]
pub struct ExecutorReport {
    pub executed: Vec<ExecutedFeature>,
    pub failed: Vec<FailedFeature>,
}

/// All inputs required for a single execution run.
///
/// The executor does not own mutable state; the caller passes `state` as `&mut`
/// so the app can inspect it after execution.
pub struct ExecutionContext<'a> {
    pub plan: &'a Plan,
    pub graph: &'a DesiredResourceGraph,
    pub index: &'a FeatureIndex,
    pub registry: &'a BackendRegistry,
    pub dirs: &'a Dirs,
    pub platform: &'a Platform,
    pub state_path: &'a Path,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Execute a plan, mutating `state` in place on each successful feature commit.
///
/// `on_event` receives progress events as they occur (non-blocking).
/// Returns a report of executed and failed features.
/// Returns `Err` only on unrecoverable failures (state commit I/O, invariant violation).
pub fn execute(
    ctx: &ExecutionContext<'_>,
    state: &mut State,
    on_event: &mut dyn FnMut(Event),
) -> Result<ExecutorReport, ExecutorError> {
    let mut report = ExecutorReport::default();

    for action in &ctx.plan.actions {
        let id_str = action.feature.as_str().to_string();
        on_event(Event::FeatureStart { id: id_str.clone() });

        let result = execute_action(
            ctx,
            state,
            &action.feature,
            &action.operation,
            &action.details,
        );

        match result {
            Ok(()) => {
                // Commit state after each successful feature.
                state::commit(ctx.state_path, state).map_err(|e| {
                    ExecutorError::StateCommitFailed {
                        reason: e.to_string(),
                    }
                })?;
                on_event(Event::FeatureDone { id: id_str.clone() });
                report.executed.push(ExecutedFeature {
                    id: id_str,
                    operation: format!("{:?}", action.operation),
                });
            }
            Err(FeatureError::Resource { resource_id, error }) => {
                on_event(Event::ResourceFailed {
                    feature_id: id_str.clone(),
                    resource_id,
                    error: error.clone(),
                });
                on_event(Event::FeatureFailed {
                    id: id_str.clone(),
                    error: error.clone(),
                });
                report.failed.push(FailedFeature {
                    id: id_str,
                    operation: format!("{:?}", action.operation),
                    error,
                });
            }
            Err(FeatureError::Feature { error }) => {
                on_event(Event::FeatureFailed {
                    id: id_str.clone(),
                    error: error.clone(),
                });
                report.failed.push(FailedFeature {
                    id: id_str,
                    operation: format!("{:?}", action.operation),
                    error,
                });
            }
        }
    }

    Ok(report)
}

// ---------------------------------------------------------------------------
// Internal error types
// ---------------------------------------------------------------------------

/// Non-fatal per-feature error variants.
enum FeatureError {
    Resource { resource_id: String, error: String },
    Feature { error: String },
}

impl From<feature_host::FeatureHostError> for FeatureError {
    fn from(e: feature_host::FeatureHostError) -> Self {
        FeatureError::Feature {
            error: e.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Per-action dispatch
// ---------------------------------------------------------------------------

fn execute_action(
    ctx: &ExecutionContext<'_>,
    state: &mut State,
    feature_id: &CanonicalFeatureId,
    op: &Operation,
    details: &Option<model::plan::ActionDetails>,
) -> Result<(), FeatureError> {
    let id_str = feature_id.as_str();

    let meta = ctx.index.features.get(id_str).ok_or_else(|| {
        // This is a programming error but we surface it as a feature-level failure
        // so execution can continue. The caller will see FeatureFailed.
        FeatureError::Feature {
            error: format!("feature not found in index: {id_str}"),
        }
    })?;

    match op {
        Operation::Create => match meta.mode {
            FeatureMode::Script => {
                feature_host::run_install(meta, feature_id, ctx.dirs, ctx.platform)
                    .map_err(FeatureError::from)?;
                // Script features are recorded with empty resources.
                state
                    .features
                    .insert(id_str.to_string(), FeatureState { resources: vec![] });
            }
            FeatureMode::Declarative => {
                let desired =
                    ctx.graph
                        .features
                        .get(id_str)
                        .ok_or_else(|| FeatureError::Feature {
                            error: format!("desired resources not found for: {id_str}"),
                        })?;

                let resources = apply_resources(ctx, id_str, &desired.resources)?;
                state
                    .features
                    .insert(id_str.to_string(), FeatureState { resources });
            }
        },

        Operation::Destroy => {
            match meta.mode {
                FeatureMode::Script => {
                    feature_host::run_uninstall(meta, feature_id, ctx.dirs, ctx.platform)
                        .map_err(FeatureError::from)?;
                }
                FeatureMode::Declarative => {
                    // Remove resources using the backend recorded in state (authoritative).
                    if let Some(feat_state) = state.features.get(id_str) {
                        remove_state_resources(ctx, id_str, &feat_state.resources.clone())?;
                    }
                }
            }
            state.features.remove(id_str);
        }

        Operation::Replace => {
            // Destroy old, then create new.
            match meta.mode {
                FeatureMode::Script => {
                    feature_host::run_uninstall(meta, feature_id, ctx.dirs, ctx.platform)
                        .map_err(FeatureError::from)?;
                    feature_host::run_install(meta, feature_id, ctx.dirs, ctx.platform)
                        .map_err(FeatureError::from)?;
                    state
                        .features
                        .insert(id_str.to_string(), FeatureState { resources: vec![] });
                }
                FeatureMode::Declarative => {
                    if let Some(feat_state) = state.features.get(id_str) {
                        remove_state_resources(ctx, id_str, &feat_state.resources.clone())?;
                    }
                    let desired =
                        ctx.graph
                            .features
                            .get(id_str)
                            .ok_or_else(|| FeatureError::Feature {
                                error: format!("desired resources not found for: {id_str}"),
                            })?;
                    let resources = apply_resources(ctx, id_str, &desired.resources)?;
                    state
                        .features
                        .insert(id_str.to_string(), FeatureState { resources });
                }
            }
        }

        Operation::ReplaceBackend => {
            // Remove via old backend (from state), apply via new backend (from graph).
            // Script features don't have a backend concept; treat as Replace.
            if meta.mode == FeatureMode::Script {
                feature_host::run_uninstall(meta, feature_id, ctx.dirs, ctx.platform)
                    .map_err(FeatureError::from)?;
                feature_host::run_install(meta, feature_id, ctx.dirs, ctx.platform)
                    .map_err(FeatureError::from)?;
                state
                    .features
                    .insert(id_str.to_string(), FeatureState { resources: vec![] });
            } else {
                if let Some(feat_state) = state.features.get(id_str) {
                    remove_state_resources(ctx, id_str, &feat_state.resources.clone())?;
                }
                let desired =
                    ctx.graph
                        .features
                        .get(id_str)
                        .ok_or_else(|| FeatureError::Feature {
                            error: format!("desired resources not found for: {id_str}"),
                        })?;
                let resources = apply_resources(ctx, id_str, &desired.resources)?;
                state
                    .features
                    .insert(id_str.to_string(), FeatureState { resources });
            }
        }

        Operation::Strengthen => {
            // Apply only the add_resources listed in the plan details.
            // Script features do not have strengthen; treat as noop with warning.
            if meta.mode == FeatureMode::Script {
                return Ok(());
            }

            let add = match details {
                Some(model::plan::ActionDetails::Strengthen(StrengthenDetails {
                    add_resources,
                })) => add_resources,
                _ => {
                    return Err(FeatureError::Feature {
                        error: "strengthen action missing add_resources details".to_string(),
                    });
                }
            };

            let desired = ctx
                .graph
                .features
                .get(id_str)
                .ok_or_else(|| FeatureError::Feature {
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
            )?;

            // Merge new resources into existing state.
            let feat_state = state
                .features
                .entry(id_str.to_string())
                .or_insert_with(|| FeatureState { resources: vec![] });
            feat_state.resources.extend(new_resources);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Resource-level helpers
// ---------------------------------------------------------------------------

/// Apply all desired resources for a declarative feature.
/// Returns the resulting state resources, or a FeatureError on the first failure.
fn apply_resources(
    ctx: &ExecutionContext<'_>,
    feature_id: &str,
    desired: &[DesiredResource],
) -> Result<Vec<Resource>, FeatureError> {
    let mut resources = Vec::with_capacity(desired.len());

    for dr in desired {
        match apply_one_resource(ctx, dr) {
            Ok(state_resource) => resources.push(state_resource),
            Err(e) => {
                return Err(FeatureError::Resource {
                    resource_id: dr.id.clone(),
                    error: format!("[{feature_id}] resource '{}' failed: {e}", dr.id),
                });
            }
        }
    }

    Ok(resources)
}

/// Apply a single desired resource, returning its state representation.
fn apply_one_resource(
    ctx: &ExecutionContext<'_>,
    dr: &DesiredResource,
) -> Result<Resource, String> {
    match &dr.kind {
        DesiredResourceKind::Package {
            name,
            desired_backend,
        } => {
            let backend = ctx
                .registry
                .get(desired_backend)
                .map_err(|e| e.to_string())?;
            backend.apply(dr).map_err(|e| e.to_string())?;
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
            let backend = ctx
                .registry
                .get(desired_backend)
                .map_err(|e| e.to_string())?;
            backend.apply(dr).map_err(|e| e.to_string())?;
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
            apply_fs(path, entry_type, op)?;
            Ok(Resource {
                id: dr.id.clone(),
                kind: ResourceKind::Fs {
                    fs: FsDetails {
                        path: path.clone(),
                        entry_type: map_fs_entry_type(entry_type, op),
                        op: map_fs_op(op),
                    },
                },
            })
        }
    }
}

/// Remove state resources using the backend recorded at install time (authoritative).
fn remove_state_resources(
    ctx: &ExecutionContext<'_>,
    feature_id: &str,
    resources: &[Resource],
) -> Result<(), FeatureError> {
    for res in resources {
        match remove_one_state_resource(ctx, res) {
            Ok(()) => {}
            Err(e) => {
                return Err(FeatureError::Resource {
                    resource_id: res.id.clone(),
                    error: format!("[{feature_id}] resource '{}' remove failed: {e}", res.id),
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
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
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
    use model::desired_resource_graph::{
        DesiredResource, DesiredResourceGraph, DesiredResourceKind, FeatureDesiredResources,
        DESIRED_RESOURCE_GRAPH_SCHEMA_VERSION,
    };
    use model::feature_index::{
        DepSpec, FeatureIndex, FeatureMeta, FeatureMode, FEATURE_INDEX_SCHEMA_VERSION,
    };
    use model::id::{CanonicalBackendId, CanonicalFeatureId};
    use model::plan::{ActionDetails, PlanAction, PlanSummary, StrengthenDetails};
    use model::state::State;
    use platform::Dirs;

    use tempfile::TempDir;

    // --- Test doubles -------------------------------------------------------

    struct OkBackend;
    impl Backend for OkBackend {
        fn apply(&self, _r: &DesiredResource) -> Result<(), BackendError> {
            Ok(())
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
        fn apply(&self, _r: &DesiredResource) -> Result<(), BackendError> {
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

    fn feature_id(s: &str) -> CanonicalFeatureId {
        CanonicalFeatureId::new(s).unwrap()
    }

    fn declarative_meta() -> FeatureMeta {
        FeatureMeta {
            spec_version: 1,
            mode: FeatureMode::Declarative,
            description: None,
            source_dir: "/tmp".to_string(),
            dep: DepSpec::default(),
            spec: None,
        }
    }

    fn script_meta(source_dir: &str) -> FeatureMeta {
        FeatureMeta {
            spec_version: 1,
            mode: FeatureMode::Script,
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

    fn make_index(entries: Vec<(&str, FeatureMeta)>) -> FeatureIndex {
        FeatureIndex {
            schema_version: FEATURE_INDEX_SCHEMA_VERSION,
            features: entries
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        }
    }

    fn make_graph(entries: Vec<(&str, Vec<DesiredResource>)>) -> DesiredResourceGraph {
        DesiredResourceGraph {
            schema_version: DESIRED_RESOURCE_GRAPH_SCHEMA_VERSION,
            features: entries
                .into_iter()
                .map(|(k, v)| (k.to_string(), FeatureDesiredResources { resources: v }))
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

    fn make_action(feature: &str, op: Operation) -> PlanAction {
        PlanAction {
            feature: feature_id(feature),
            operation: op,
            details: None,
        }
    }

    fn make_dirs(tmp: &TempDir) -> Dirs {
        Dirs {
            config_home: tmp.path().join("config"),
            data_home: tmp.path().join("data"),
            state_home: tmp.path().join("state"),
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
                Event::FeatureStart { id } => Some(id.clone()),
                _ => None,
            })
            .collect();
        let dones: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::FeatureDone { id } => Some(id.clone()),
                _ => None,
            })
            .collect();
        let failed: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::FeatureFailed { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect();
        (starts, dones, failed)
    }

    // --- Tests --------------------------------------------------------------

    /// Create declarative feature: resources applied, state committed.
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
        };

        let mut events = vec![];
        let report = execute(&ctx, &mut state, &mut |e| events.push(e)).unwrap();

        assert_eq!(report.executed.len(), 1);
        assert!(report.failed.is_empty());
        assert!(state.features.contains_key("core/git"));
        assert_eq!(state.features["core/git"].resources.len(), 1);
        assert!(state_path.exists(), "state file should be committed");

        let (starts, dones, failed) = collect_events(&events);
        assert_eq!(starts, ["core/git"]);
        assert_eq!(dones, ["core/git"]);
        assert!(failed.is_empty());
    }

    /// Create declarative feature with failing backend: FeatureFailed emitted, state unchanged.
    #[test]
    fn create_declarative_resource_fail_emits_feature_failed() {
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
        };

        let mut events = vec![];
        let report = execute(&ctx, &mut state, &mut |e| events.push(e)).unwrap();

        assert!(report.executed.is_empty());
        assert_eq!(report.failed.len(), 1);
        // State must not contain the feature.
        assert!(!state.features.contains_key("core/git"));

        let resource_failed = events
            .iter()
            .any(|e| matches!(e, Event::ResourceFailed { .. }));
        let feature_failed = events
            .iter()
            .any(|e| matches!(e, Event::FeatureFailed { .. }));
        assert!(resource_failed, "expected ResourceFailed event");
        assert!(feature_failed, "expected FeatureFailed event");
    }

    /// Destroy declarative feature: resources removed, state cleared.
    #[test]
    fn destroy_declarative_success_removes_state() {
        let tmp = tempfile::tempdir().unwrap();
        let state_path = tmp.path().join("state.json");

        // Pre-populate state.
        let mut state = State::empty();
        state.features.insert(
            "core/git".to_string(),
            FeatureState {
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
        };

        let mut events = vec![];
        let report = execute(&ctx, &mut state, &mut |e| events.push(e)).unwrap();

        assert_eq!(report.executed.len(), 1);
        assert!(
            !state.features.contains_key("core/git"),
            "feature must be removed from state"
        );
    }

    /// Destroy declarative feature with failing backend: state unchanged.
    #[test]
    fn destroy_resource_fail_leaves_state_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let state_path = tmp.path().join("state.json");

        let mut state = State::empty();
        state.features.insert(
            "core/git".to_string(),
            FeatureState {
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
        };

        let mut events = vec![];
        let report = execute(&ctx, &mut state, &mut |e| events.push(e)).unwrap();

        assert!(report.failed.len() == 1);
        // State must still have the feature.
        assert!(state.features.contains_key("core/git"));
    }

    /// Multiple features: failed feature does not stop subsequent features.
    #[test]
    fn failed_feature_does_not_stop_next_feature() {
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
        };

        let mut events = vec![];
        let report = execute(&ctx, &mut state, &mut |e| events.push(e)).unwrap();

        assert_eq!(report.executed.len(), 1, "core/node should succeed");
        assert_eq!(report.failed.len(), 1, "core/git should fail");
        assert!(state.features.contains_key("core/node"));
        assert!(!state.features.contains_key("core/git"));
    }

    /// Strengthen adds only the listed resources to existing state.
    #[test]
    fn strengthen_adds_resources_to_existing_state() {
        let tmp = tempfile::tempdir().unwrap();
        let state_path = tmp.path().join("state.json");

        // Start with one already-installed resource.
        let mut state = State::empty();
        state.features.insert(
            "core/tools".to_string(),
            FeatureState {
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
            feature: feature_id("core/tools"),
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
        };

        let mut events = vec![];
        let report = execute(&ctx, &mut state, &mut |e| events.push(e)).unwrap();

        assert_eq!(report.executed.len(), 1);
        let feat = &state.features["core/tools"];
        assert_eq!(
            feat.resources.len(),
            2,
            "both git and ripgrep should be in state"
        );
    }

    /// Create script feature: install.sh executed, empty resources recorded in state.
    #[test]
    fn create_script_feature_records_empty_resources() {
        let tmp = tempfile::tempdir().unwrap();
        let state_path = tmp.path().join("state.json");
        let feat_dir = tmp.path().join("feat");
        std::fs::create_dir_all(&feat_dir).unwrap();

        // Write a minimal install.sh.
        let script = feat_dir.join("install.sh");
        std::fs::write(&script, "#!/usr/bin/env sh\nexit 0\n").unwrap();
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
        let platform = Platform::Linux;
        let ctx = ExecutionContext {
            plan: &plan,
            graph: &graph,
            index: &index,
            registry: &registry,
            dirs: &dirs,
            platform: &platform,
            state_path: &state_path,
        };

        let mut events = vec![];
        let report = execute(&ctx, &mut state, &mut |e| events.push(e)).unwrap();

        assert_eq!(report.executed.len(), 1);
        assert!(state.features.contains_key("core/brew"));
        assert!(state.features["core/brew"].resources.is_empty());
    }

    /// Replace declarative: old resources removed, new resources applied.
    #[test]
    fn replace_declarative_updates_state() {
        let tmp = tempfile::tempdir().unwrap();
        let state_path = tmp.path().join("state.json");

        let mut state = State::empty();
        state.features.insert(
            "core/git".to_string(),
            FeatureState {
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
        };

        let mut events = vec![];
        let report = execute(&ctx, &mut state, &mut |e| events.push(e)).unwrap();

        assert_eq!(report.executed.len(), 1);
        // Backend should now be core/apt.
        let feat = &state.features["core/git"];
        match &feat.resources[0].kind {
            ResourceKind::Package { backend, .. } => {
                assert_eq!(backend.as_str(), "core/apt");
            }
            _ => panic!("expected package"),
        }
    }
}
