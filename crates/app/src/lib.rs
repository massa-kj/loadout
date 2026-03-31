//! Application service layer — orchestrates the loadout pipeline.
//!
//! This crate corresponds to `cmd/* + core/lib/orchestrator.sh` in the shell
//! implementation.  It assembles and sequences all pipeline stages:
//!
//! ```text
//! load_sources → SourcesSpec
//! load_profile → Profile
//! load_strategy  → Strategy
//! build_feature_index → FeatureIndex
//! filter_desired_features → Vec<CanonicalFeatureId>
//! resolver::resolve → ResolvedFeatureOrder
//! compiler::compile → DesiredResourceGraph
//! state::load → State
//! planner::plan → Plan
//! (apply only) build_backend_registry + executor::execute
//! ```
//!
//! The only state mutation happens inside `executor::execute`, which atomically
//! commits state after each successful feature.  Every other stage is read-only.
//!
//! See: `docs/architecture/layers.md` (cmd / app layer)

use std::path::{Path, PathBuf};

pub use executor::activate::ShellKind;
pub use executor::{Event, ExecutorReport};
pub use model::plan::Plan;

// ---------------------------------------------------------------------------
// ExecutionPlan
// ---------------------------------------------------------------------------

/// All data required to execute a plan.
///
/// Returned by `prepare_execution()` and consumed by `execute()`.
/// This type allows the CLI layer to inspect the plan, display it to the user,
/// and request confirmation before execution begins.
pub struct ExecutionPlan {
    pub plan: Plan,
    pub graph: model::desired_resource_graph::DesiredResourceGraph,
    pub index: model::FeatureIndex,
    pub order: model::ResolvedFeatureOrder,
    pub registry: backend_host::BackendRegistry,
    pub state: state::State,
}

// ---------------------------------------------------------------------------
// AppError
// ---------------------------------------------------------------------------

/// All pipeline-level errors.
///
/// These are returned as `Err` only for fatal, run-aborting conditions.
/// Feature-level failures during `apply()` are reported via [`Event::FeatureFailed`]
/// and collected in [`ExecutorReport::failed`], not surfaced as `AppError`.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// The config YAML file was not found.
    #[error("config not found: {}", path.display())]
    ConfigNotFound { path: PathBuf },

    /// A configuration file (profile, strategy, sources) could not be loaded.
    #[error("config error: {0}")]
    Config(#[from] config::ConfigError),

    /// Feature index construction failed (e.g. unreadable feature.yaml).
    #[error("feature index error: {0}")]
    FeatureIndex(#[from] feature_index::FeatureIndexError),

    /// Dependency resolution failed (missing dependency or cycle).
    #[error("resolver error: {0}")]
    Resolver(#[from] resolver::ResolverError),

    /// Compiler failed to build the desired resource graph.
    #[error("compiler error: {0}")]
    Compiler(#[from] compiler::CompilerError),

    /// Planner could not produce a plan.
    #[error("planner error: {0}")]
    Planner(#[from] planner::PlannerError),

    /// State I/O or invariant error.
    #[error("state error: {0}")]
    State(#[from] state::StateError),

    /// Fatal executor error (state commit failure or invariant violation).
    #[error("executor error: {0}")]
    Executor(#[from] executor::ExecutorError),

    /// No cached env plan found; `loadout apply` must be run first.
    #[error("no cached env plan — run 'loadout apply' first")]
    EnvPlanNotFound,

    /// Failed to read the env plan cache file.
    #[error("failed to read env plan cache: {0}")]
    EnvPlanIo(std::io::Error),

    /// Failed to deserialize the env plan cache.
    #[error("failed to deserialize env plan cache: {0}")]
    EnvPlanDeserialize(serde_json::Error),
}

// ---------------------------------------------------------------------------
// AppContext
// ---------------------------------------------------------------------------

/// Stable, run-level context shared by all use cases.
///
/// Contains the platform, resolved base directories, and the local source root.
/// Use-case-specific paths (e.g. the config file) are passed as arguments.
pub struct AppContext {
    /// Platform variant: Linux, Windows, or WSL.
    pub platform: platform::Platform,

    /// Resolved XDG / AppData base directories.
    pub dirs: platform::Dirs,

    /// Base directory for the `local` source (features/ and backends/).
    ///
    /// Defaults to `dirs.config_home`. Can be overridden via the `LOADOUT_ROOT`
    /// environment variable for development use only.
    pub local_root: PathBuf,

    /// Optional override for the sources spec path.
    /// When `Some`, this path is used instead of `{config_home}/sources.yaml`.
    /// Intended for CI / verification use only (mirrors `--sources` CLI flag).
    pub sources_override: Option<PathBuf>,
}

impl AppContext {
    /// Create an `AppContext` from explicit parts.
    ///
    /// `local_root` defaults to `dirs.config_home`.
    /// Use [`AppContext::with_local_root`] to override it for development.
    pub fn new(platform: platform::Platform, dirs: platform::Dirs) -> Self {
        let local_root = dirs.config_home.clone();
        Self {
            platform,
            dirs,
            local_root,
            sources_override: None,
        }
    }

    /// Override the `local` source root. Used by the CLI when `LOADOUT_ROOT` is set.
    pub fn with_local_root(mut self, path: PathBuf) -> Self {
        self.local_root = path;
        self
    }

    /// Absolute path to the authoritative state file: `{state_home}/state.json`.
    pub fn state_path(&self) -> PathBuf {
        self.dirs.state_home.join("state.json")
    }

    /// Absolute path to the user sources file: `{config_home}/sources.yaml`.
    /// May not exist; callers should treat absence as an empty `SourcesSpec`.
    pub fn sources_path(&self) -> PathBuf {
        self.dirs.config_home.join("sources.yaml")
    }

    /// Absolute path to the ephemeral env plan cache: `{cache_home}/env_plan.json`.
    ///
    /// Written by `execute()` on successful apply; read by `activate()`.
    /// Not part of the authoritative state — callers must handle absence gracefully.
    pub fn env_plan_cache_path(&self) -> PathBuf {
        self.dirs.cache_home.join("env_plan.json")
    }
}

// ---------------------------------------------------------------------------
// plan() use case
// ---------------------------------------------------------------------------

/// Compute the plan for the given config without executing any actions.
///
/// `config_path` must point to a unified `config.yaml` containing both the
/// `profile` and (optionally) the `strategy` section.
///
/// Returns the [`Plan`] that describes what `apply()` would do.
/// All stages are read-only; no state is modified.
pub fn plan(ctx: &AppContext, config_path: &Path) -> Result<Plan, AppError> {
    let PipelineOutput {
        order,
        graph,
        state,
        ..
    } = run_pipeline(ctx, config_path)?;
    let p = planner::plan(&graph, &state, &order)?;
    Ok(p)
}

// ---------------------------------------------------------------------------
// apply() use case
// ---------------------------------------------------------------------------

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
        graph,
        state,
        ..
    } = run_pipeline(ctx, config_path)?;

    let plan = planner::plan(&graph, &state, &order)?;
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
/// - Calls feature-host (script mode) or backend-host (declarative mode)
/// - Commits state after each successful feature
///
/// Feature-level failures are non-fatal and reported via `on_event` +
/// `ExecutorReport::failed`.
///
/// Returns `Err` only for fatal conditions (state commit failure, invariant violation).
pub fn execute(
    ctx: &AppContext,
    execution_plan: ExecutionPlan,
    on_event: &mut dyn FnMut(Event),
) -> Result<ExecutorReport, AppError> {
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

/// Execute the plan: install, update, and remove features as needed.
///
/// This is a convenience wrapper around `prepare_execution()` + `execute()`.
/// For use cases that require user confirmation or plan inspection, use
/// `prepare_execution()` followed by `execute()` directly.
///
/// `config_path` must point to a unified `config.yaml` containing both the
/// `profile` and (optionally) the `strategy` section.
///
/// Feature-level failures do not abort the run; they are reported via `on_event`
/// and collected in [`ExecutorReport::failed`].
///
/// Returns `Err` only for fatal conditions (state commit failure, invariant
/// violation, or a pipeline stage failure before execution begins).
pub fn apply(
    ctx: &AppContext,
    config_path: &Path,
    on_event: &mut dyn FnMut(Event),
) -> Result<ExecutorReport, AppError> {
    let execution_plan = prepare_execution(ctx, config_path)?;
    execute(ctx, execution_plan, on_event)
}

// ---------------------------------------------------------------------------
// activate() use case
// ---------------------------------------------------------------------------

/// Generate a shell activation script from the last apply's env plan.
///
/// Reads the env plan cache written by `execute()` and returns a shell script
/// suitable for evaluation in the target shell.
///
/// # Usage
///
/// ```text
/// eval "$(loadout activate)"               # bash / zsh
/// loadout activate --shell fish | source   # fish
/// Invoke-Expression (loadout activate --shell pwsh)  # PowerShell
/// ```
///
/// # Errors
///
/// Returns [`AppError::EnvPlanNotFound`] if no cache exists — the user must
/// run `loadout apply` first.
pub fn activate(ctx: &AppContext, shell: ShellKind) -> Result<String, AppError> {
    let cache_path = ctx.env_plan_cache_path();
    if !cache_path.exists() {
        return Err(AppError::EnvPlanNotFound);
    }
    let json = std::fs::read_to_string(&cache_path).map_err(AppError::EnvPlanIo)?;
    let plan: model::env::ExecutionEnvPlan =
        serde_json::from_str(&json).map_err(AppError::EnvPlanDeserialize)?;
    Ok(executor::activate::generate_activation(&plan, shell))
}

// ---------------------------------------------------------------------------
// save_env_plan_cache (private)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Shared pipeline (plan + apply share the read-only stages)
// ---------------------------------------------------------------------------

/// Outputs from the common read-only pipeline stages.
struct PipelineOutput {
    #[allow(dead_code)]
    profile: config::Profile,
    #[allow(dead_code)]
    strategy: config::Strategy,
    index: model::FeatureIndex,
    order: model::ResolvedFeatureOrder,
    graph: model::desired_resource_graph::DesiredResourceGraph,
    state: state::State,
}

/// Run the read-only stages common to both `plan()` and `apply()`.
///
/// Steps:
///   1. Validate config file exists.
///   2. Load config → `Profile` + `Strategy` via `config::load_config`.
///   3. Load sources (optional) → `SourcesSpec`.
///   4. Build `FeatureIndex` from source roots.
///   5. Map profile features to `CanonicalFeatureId`s (skip unknown).
///   6. Resolve dependency order.
///   7. Compile: `FeatureIndex + Strategy + order` → `DesiredResourceGraph`.
///   8. Load (or initialise) state.
fn run_pipeline(ctx: &AppContext, config_path: &Path) -> Result<PipelineOutput, AppError> {
    // Step 1: config file must exist.
    if !config_path.exists() {
        return Err(AppError::ConfigNotFound {
            path: config_path.to_path_buf(),
        });
    }

    // Step 2: load config — profile is required, strategy is optional (defaults to
    // Strategy::default() if the 'strategy' section is absent from the file).
    let (profile, strategy) = config::load_config(config_path)?;

    // Step 3: load sources if present; default to empty (core + local only).
    let sources = load_sources_optional(ctx)?;

    // Step 4: build feature index from all source roots.
    let source_roots = build_source_roots(ctx, &sources);
    let fi_platform = to_fi_platform(&ctx.platform);
    let index = feature_index::build(&source_roots, &fi_platform)?;

    // Step 5: convert profile keys to CanonicalFeatureIds; skip those absent from index.
    // An empty desired list is valid: it means "uninstall everything in state".
    let desired_ids = profile_to_desired_ids(&profile, &index);

    // Step 6: resolve dependency order (topological sort).
    let order = resolver::resolve(&index, &desired_ids)?;

    // Step 7: compile desired resource graph.
    let graph = compiler::compile(&index, &strategy, &order)?;

    // Step 8: load state (state::load returns empty state if file absent).
    let state = state::load(&ctx.state_path())?;

    Ok(PipelineOutput {
        profile,
        strategy,
        index,
        order,
        graph,
        state,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Map `platform::Platform` → `feature_index::Platform`.
fn to_fi_platform(p: &platform::Platform) -> feature_index::Platform {
    match p {
        platform::Platform::Linux => feature_index::Platform::Linux,
        platform::Platform::Windows => feature_index::Platform::Windows,
        platform::Platform::Wsl => feature_index::Platform::Wsl,
    }
}

/// Build the list of feature source roots for the feature-index scanner.
///
/// Implicit sources:
/// - `local` → `{local_root}/features/`
///
/// (The `core` source is embedded in the binary; no filesystem path applies.)
///
/// External sources (from `sources.yaml`):
/// - `{id}` → `{data_home}/sources/{id}/features/`
fn build_source_roots(
    ctx: &AppContext,
    sources: &config::SourcesSpec,
) -> Vec<feature_index::SourceRoot> {
    let mut roots = vec![feature_index::SourceRoot {
        source_id: "local".into(),
        features_dir: ctx.local_root.join("features"),
    }];

    for entry in &sources.sources {
        roots.push(feature_index::SourceRoot {
            source_id: entry.id.clone(),
            features_dir: ctx
                .dirs
                .data_home
                .join("sources")
                .join(&entry.id)
                .join("features"),
        });
    }

    roots
}

/// Load the sources spec; return an empty `SourcesSpec` if the file is absent.
///
/// If `ctx.sources_override` is set, that path is used exclusively (no fallback).
/// This mirrors the `--sources` CLI flag, intended for CI / verification use only.
fn load_sources_optional(ctx: &AppContext) -> Result<config::SourcesSpec, AppError> {
    if let Some(ref path) = ctx.sources_override {
        return Ok(config::load_sources(path)?);
    }
    let path = ctx.sources_path();
    if path.exists() {
        Ok(config::load_sources(&path)?)
    } else {
        Ok(config::SourcesSpec::default())
    }
}

/// Map profile feature keys (already normalised by `config::load_profile`) to
/// `CanonicalFeatureId`s, keeping only those present in the feature index.
///
/// Features absent from the index may belong to a source that has not been
/// cloned yet; they are silently skipped rather than returning an error,
/// so that a machine with a partial set of sources can still make progress.
fn profile_to_desired_ids(
    profile: &config::Profile,
    index: &model::FeatureIndex,
) -> Vec<model::CanonicalFeatureId> {
    let mut ids: Vec<model::CanonicalFeatureId> = profile
        .features
        .keys()
        .filter(|k| index.features.contains_key(k.as_str()))
        .filter_map(|k| model::CanonicalFeatureId::new(k).ok())
        .collect();

    // Sort for deterministic order (profile is a HashMap, iteration order varies).
    ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    ids
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
    backends_dir: &Path,
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    // --- Fixture helpers ---

    /// Write a file, creating all parent directories.
    fn write(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    /// Make a file executable on Unix.
    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(not(unix))]
    fn make_executable(_path: &Path) {}

    /// Build an AppContext whose dirs and local_root all point inside `tmp`.
    fn make_ctx(tmp: &TempDir) -> AppContext {
        let root = tmp.path().to_path_buf();
        let config_home = root.join("config");
        AppContext {
            platform: platform::detect_platform(),
            local_root: config_home.clone(),
            dirs: platform::Dirs {
                config_home,
                data_home: root.join("data"),
                state_home: root.join("state"),
                cache_home: root.join("cache"),
            },
            sources_override: None,
        }
    }

    /// Write a minimal script-mode feature to `{local_root}/features/{name}/`.
    /// Creates platform-appropriate scripts: .sh on Linux/WSL, .ps1 on Windows.
    fn write_script_feature(root: &Path, name: &str) {
        let feat_dir = root.join("config").join("features").join(name);
        write(
            &feat_dir.join("feature.yaml"),
            "spec_version: 1\nmode: script\n",
        );

        let platform = platform::detect_platform();
        match platform {
            platform::Platform::Windows => {
                // PowerShell scripts
                let install_ps1 = feat_dir.join("install.ps1");
                write(&install_ps1, "exit 0\n");
                let uninstall_ps1 = feat_dir.join("uninstall.ps1");
                write(&uninstall_ps1, "exit 0\n");
            }
            platform::Platform::Linux | platform::Platform::Wsl => {
                // Shell scripts
                let install_sh = feat_dir.join("install.sh");
                write(&install_sh, "#!/usr/bin/env sh\nexit 0\n");
                make_executable(&install_sh);
                let uninstall_sh = feat_dir.join("uninstall.sh");
                write(&uninstall_sh, "#!/usr/bin/env sh\nexit 0\n");
                make_executable(&uninstall_sh);
            }
        }
    }

    /// Write a minimal config.yaml referencing the given feature names.
    /// Features must be canonical `source_id/name` form; they are grouped by source_id.
    /// No strategy section is written (uses Strategy::default()).
    fn write_config(dir: &Path, filename: &str, features: &[&str]) -> PathBuf {
        // Group features by source_id.
        let mut grouped: std::collections::BTreeMap<&str, Vec<&str>> =
            std::collections::BTreeMap::new();
        for f in features {
            let (source, name) = f
                .split_once('/')
                .expect("feature must be canonical source/name");
            grouped.entry(source).or_default().push(name);
        }
        let mut features_str = String::new();
        for (source, names) in &grouped {
            features_str.push_str(&format!("    {source}:\n"));
            for name in names {
                features_str.push_str(&format!("      {name}: {{}}\n"));
            }
        }
        let content = format!("profile:\n  features:\n{features_str}");
        let path = dir.join(filename);
        write(&path, &content);
        path
    }

    /// Collect all events emitted during apply.
    fn collect_apply(
        ctx: &AppContext,
        config_path: &Path,
    ) -> (Result<ExecutorReport, AppError>, Vec<Event>) {
        let mut events = vec![];
        let result = apply(ctx, config_path, &mut |e| events.push(e));
        (result, events)
    }

    // --- Tests ---

    /// Missing config returns ConfigNotFound.
    #[test]
    fn plan_missing_config_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = make_ctx(&tmp);
        let config_path = tmp.path().join("nonexistent.yaml");

        let err = plan(&ctx, &config_path).unwrap_err();
        assert!(
            matches!(err, AppError::ConfigNotFound { .. }),
            "expected ConfigNotFound, got {err:?}"
        );
    }

    /// Config with unrecognised features: recognised list is empty → plan has no actions.
    #[test]
    fn plan_unknown_features_produce_empty_plan() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = make_ctx(&tmp);
        // Feature referenced in config does not exist in index → desired IDs empty.
        let config_path = write_config(tmp.path(), "config.yaml", &["local/nonexistent"]);

        // Should succeed: empty desired produces a plan with no actions.
        let p = plan(&ctx, &config_path).unwrap();
        assert!(
            p.actions.is_empty(),
            "plan should have no actions for unknown features"
        );
    }

    /// plan() with a valid script feature returns a Plan with a Create action.
    #[test]
    fn plan_script_feature_returns_create_action() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = make_ctx(&tmp);
        write_script_feature(tmp.path(), "git");
        let config_path = write_config(tmp.path(), "config.yaml", &["local/git"]);

        let p = plan(&ctx, &config_path).unwrap();
        assert_eq!(p.actions.len(), 1);
        let action = &p.actions[0];
        assert_eq!(action.feature.as_str(), "local/git");
        assert!(matches!(action.operation, model::plan::Operation::Create));
    }

    /// apply() installs a script feature and commits state.
    #[test]
    fn apply_script_feature_commits_state() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = make_ctx(&tmp);
        write_script_feature(tmp.path(), "git");
        let config_path = write_config(tmp.path(), "config.yaml", &["local/git"]);

        let (result, events) = collect_apply(&ctx, &config_path);

        let report = result.unwrap();
        assert_eq!(report.executed.len(), 1, "expected one feature executed");
        assert!(report.failed.is_empty());

        // State file must be committed.
        assert!(ctx.state_path().exists(), "state.json must be written");

        // Events: FeatureStart + FeatureDone.
        let starts = events
            .iter()
            .filter(|e| matches!(e, Event::FeatureStart { .. }))
            .count();
        let dones = events
            .iter()
            .filter(|e| matches!(e, Event::FeatureDone { .. }))
            .count();
        assert_eq!(starts, 1);
        assert_eq!(dones, 1);
    }

    /// apply() a second time on an already-installed feature emits no actions (noop).
    #[test]
    fn apply_already_installed_feature_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = make_ctx(&tmp);
        write_script_feature(tmp.path(), "git");
        let config_path = write_config(tmp.path(), "config.yaml", &["local/git"]);

        // First apply: installs.
        let (r1, _) = collect_apply(&ctx, &config_path);
        r1.unwrap();

        // Second apply: state already reflects desired; should be a noop.
        let (r2, events2) = collect_apply(&ctx, &config_path);
        let report2 = r2.unwrap();

        // No actions executed: feature is already in state.
        assert!(
            report2.executed.is_empty(),
            "second apply should have no executed features"
        );
        // No events at all (no actions → no FeatureStart/Done).
        let start_count = events2
            .iter()
            .filter(|e| matches!(e, Event::FeatureStart { .. }))
            .count();
        assert_eq!(start_count, 0, "no FeatureStart events on noop");
    }

    /// apply() missing config propagates ConfigNotFound.
    #[test]
    fn apply_missing_config_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = make_ctx(&tmp);
        let config_path = tmp.path().join("does_not_exist.yaml");

        let (result, _) = collect_apply(&ctx, &config_path);
        assert!(matches!(
            result.unwrap_err(),
            AppError::ConfigNotFound { .. }
        ));
    }

    /// apply() two script features: both install, state has both.
    #[test]
    fn apply_multiple_features_all_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = make_ctx(&tmp);
        write_script_feature(tmp.path(), "git");
        write_script_feature(tmp.path(), "node");
        let config_path = write_config(tmp.path(), "config.yaml", &["local/git", "local/node"]);

        let (result, _) = collect_apply(&ctx, &config_path);
        let report = result.unwrap();

        assert_eq!(report.executed.len(), 2);
        assert!(report.failed.is_empty());
    }

    /// apply() removes a feature that is in state but not in the config.
    #[test]
    fn apply_removes_undesired_feature_from_state() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = make_ctx(&tmp);
        write_script_feature(tmp.path(), "git");
        write_script_feature(tmp.path(), "node");

        // First apply: install git + node.
        let config_both = write_config(tmp.path(), "both.yaml", &["local/git", "local/node"]);
        collect_apply(&ctx, &config_both).0.unwrap();

        // Second apply: only git desired → node should be destroyed.
        let config_git_only = write_config(tmp.path(), "git_only.yaml", &["local/git"]);
        let (result, _) = collect_apply(&ctx, &config_git_only);
        let report = result.unwrap();

        // One action executed (Destroy node).
        assert_eq!(report.executed.len(), 1);
        assert!(report.failed.is_empty());

        // Reload state from disk and verify.
        let state = state::load(&ctx.state_path()).unwrap();
        assert!(
            state.features.contains_key("local/git"),
            "git must still be in state"
        );
        assert!(
            !state.features.contains_key("local/node"),
            "node must be removed from state"
        );
    }

    /// Config without a strategy section → Strategy::default() is used (no error).
    #[test]
    fn plan_without_strategy_section_uses_default_strategy() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = make_ctx(&tmp);
        write_script_feature(tmp.path(), "git");
        let config_path = write_config(tmp.path(), "config.yaml", &["local/git"]);

        // write_config omits the strategy section → Strategy::default() is used.
        let p = plan(&ctx, &config_path).unwrap();
        assert_eq!(p.actions.len(), 1);
    }

    /// apply() with a script feature whose uninstall fails is non-fatal;
    /// other features in the same run still succeed.
    #[test]
    fn apply_failing_uninstall_is_non_fatal() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = make_ctx(&tmp);

        // Feature with a failing uninstall script.
        let feat_dir = tmp
            .path()
            .join("config")
            .join("features")
            .join("badfeature");
        write(
            &feat_dir.join("feature.yaml"),
            "spec_version: 1\nmode: script\n",
        );

        let platform = platform::detect_platform();
        match platform {
            platform::Platform::Windows => {
                // PowerShell scripts
                let install_ps1 = feat_dir.join("install.ps1");
                write(&install_ps1, "exit 0\n");
                let uninstall_ps1 = feat_dir.join("uninstall.ps1");
                write(&uninstall_ps1, "exit 1\n"); // Always fails
            }
            platform::Platform::Linux | platform::Platform::Wsl => {
                // Shell scripts
                let install_sh = feat_dir.join("install.sh");
                write(&install_sh, "#!/usr/bin/env sh\nexit 0\n");
                make_executable(&install_sh);
                let uninstall_sh = feat_dir.join("uninstall.sh");
                write(&uninstall_sh, "#!/usr/bin/env sh\nexit 1\n"); // Always fails
                make_executable(&uninstall_sh);
            }
        }

        // A good feature that succeeds.
        write_script_feature(tmp.path(), "git");

        // First apply: install both.
        let config_both = write_config(tmp.path(), "both.yaml", &["local/badfeature", "local/git"]);
        collect_apply(&ctx, &config_both).0.unwrap();

        // Second apply: only git desired → badfeature must be destroyed (fails), git is noop.
        let config_git_only = write_config(tmp.path(), "git.yaml", &["local/git"]);
        let (result, events) = collect_apply(&ctx, &config_git_only);
        let report = result.unwrap(); // Must not be a fatal error.

        // badfeature destruction failed → shows up in failed list.
        assert_eq!(report.failed.len(), 1, "badfeature uninstall should fail");
        // git was already installed; no new action.
        assert!(report.executed.is_empty(), "git is already installed");

        // A FeatureFailed event is emitted.
        let ff_count = events
            .iter()
            .filter(|e| matches!(e, Event::FeatureFailed { .. }))
            .count();
        assert_eq!(ff_count, 1);
    }
}
