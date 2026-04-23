// prepare_execution(), execute(), and apply() use cases.

use std::path::Path;

use crate::context::{AppContext, AppError};
use crate::pipeline::{run_pipeline, PipelineOutput};

/// All data required to execute a plan.
///
/// Returned by `prepare_execution()` and consumed by `execute()`.
/// This type allows the CLI layer to inspect the plan, display it to the user,
/// and request confirmation before execution begins.
pub struct ExecutionPlan {
    pub plan: model::plan::Plan,
    pub graph: model::desired_resource_graph::DesiredResourceGraph,
    pub index: model::ComponentIndex,
    pub order: model::ResolvedComponentOrder,
    pub registry: backend_host::BackendRegistry,
    pub state: state::State,
}

/// Prepare execution: load config, resolve dependencies, compile, and plan.
///
/// This is the read-only portion of `apply()`, extracted to allow the CLI
/// layer to inspect the plan, display it, and request user confirmation
/// before execution begins.
///
/// Returns an `ExecutionPlan` containing all data needed by `execute()`.
///
/// All stages are read-only except for reading the state file.
pub fn prepare_execution(ctx: &AppContext, config_path: &Path) -> Result<ExecutionPlan, AppError> {
    let PipelineOutput {
        index,
        order,
        full_order,
        graph,
        state,
        ..
    } = run_pipeline(ctx, config_path)?;

    // Use full_order for the planner so destroy ordering is correct when both a component
    // and its dependency are removed simultaneously. The executor receives desired-only `order`.
    let plan = planner::plan(&graph, &state, &full_order)?;
    let registry = build_backend_registry(ctx);

    Ok(ExecutionPlan {
        plan,
        graph,
        index,
        order,
        registry,
        state,
    })
}

/// Execute a prepared plan.
///
/// Takes ownership of `ExecutionPlan` and performs all side-effecting operations:
/// - Calls component-host (script mode) or backend-host (declarative mode)
/// - Commits state after each successful component
///
/// Component-level failures are non-fatal and reported via `on_event` +
/// `ExecutorReport::failed`.
///
/// Returns `Err` only for fatal conditions (state commit failure, invariant violation).
pub fn execute(
    ctx: &AppContext,
    execution_plan: ExecutionPlan,
    on_event: &mut dyn FnMut(executor::Event),
) -> Result<executor::ExecutorReport, AppError> {
    let mut state = execution_plan.state;

    let mut contributors = executor::ContributorRegistry::new();
    backends_builtin::register_contributors(&mut contributors, &ctx.platform);

    let exec_ctx = executor::ExecutionContext {
        plan: &execution_plan.plan,
        graph: &execution_plan.graph,
        index: &execution_plan.index,
        registry: &execution_plan.registry,
        dirs: &ctx.dirs,
        platform: &ctx.platform,
        state_path: &ctx.state_path(),
        contributors: &contributors,
    };

    let report = executor::execute(&exec_ctx, &mut state, on_event)?;

    // Save the env plan cache for `loadout activate`. This is best-effort;
    // a failure does not abort the apply or affect the returned report.
    let _ = save_env_plan_cache(&report.final_env_plan, &ctx.dirs);

    Ok(report)
}

/// Execute the plan: install, update, and remove components as needed.
///
/// This is a convenience wrapper around `prepare_execution()` + `execute()`.
/// For use cases that require user confirmation or plan inspection, use
/// `prepare_execution()` followed by `execute()` directly.
///
/// `config_path` must point to a unified `config.yaml` containing both the
/// `profile` and (optionally) the `strategy` section.
///
/// Component-level failures do not abort the run; they are reported via `on_event`
/// and collected in [`executor::ExecutorReport::failed`].
///
/// Returns `Err` only for fatal conditions (state commit failure, invariant
/// violation, or a pipeline stage failure before execution begins).
pub fn apply(
    ctx: &AppContext,
    config_path: &Path,
    on_event: &mut dyn FnMut(executor::Event),
) -> Result<executor::ExecutorReport, AppError> {
    let execution_plan = prepare_execution(ctx, config_path)?;
    execute(ctx, execution_plan, on_event)
}

/// Serialize the env plan to the cache file.
///
/// Creates the parent directory if it does not exist.
/// Called by `execute()` on successful apply; failures are ignored (best-effort).
fn save_env_plan_cache(
    plan: &model::env::ExecutionEnvPlan,
    dirs: &platform::Dirs,
) -> Result<(), std::io::Error> {
    let cache_path = dirs.cache_home.join("env_plan.json");
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(plan).map_err(std::io::Error::other)?;
    std::fs::write(&cache_path, json)
}

/// Build the backend registry from builtins and local script backends.
///
/// - Builtin Rust backends are registered first (embedded in the binary).
/// - Script backends under `{local_root}/backends/` are registered as `local/<name>`
///   and can override builtins for local customisation.
///
/// Directories that fail to load are skipped silently.
fn build_backend_registry(ctx: &AppContext) -> backend_host::BackendRegistry {
    let mut registry = backend_host::BackendRegistry::new();
    // 1. Register builtin Rust backends for the current platform.
    backends_builtin::register_builtins(&mut registry, &ctx.platform);
    // 2. Script backends from the local source can override / extend builtins.
    load_backends_from_dir(
        &mut registry,
        &ctx.local_root.join("backends"),
        "local",
        ctx.platform,
    );
    registry
}

/// Scan a single directory for script backend subdirectories and register each.
fn load_backends_from_dir(
    registry: &mut backend_host::BackendRegistry,
    backends_dir: &std::path::Path,
    source_id: &str,
    platform: platform::Platform,
) {
    let Ok(entries) = std::fs::read_dir(backends_dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue; // Skip flat .sh files (old shell layout).
        }

        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let backend_id_str = format!("{source_id}/{name}");
        let Ok(backend_id) = model::CanonicalBackendId::new(&backend_id_str) else {
            continue;
        };

        match backend_host::ScriptBackend::load(platform, path.clone()) {
            Ok(backend) => {
                registry.register(backend_id, Box::new(backend));
            }
            Err(_) => {
                // Skip; backend may not have been migrated to the new layout yet.
            }
        }
    }
}
