//! Application service layer — orchestrates the loadout pipeline.
//!
//! This crate corresponds to `cmd/* + core/lib/orchestrator.sh` in the shell
//! implementation.  It assembles and sequences all pipeline stages:
//!
//! ```text
//! load_sources → SourcesSpec
//! load_profile → Profile
//! load_policy  → Policy
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

pub use executor::{Event, ExecutorReport};
pub use model::plan::Plan;

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
    /// The profile YAML file was not found.
    #[error("profile not found: {}", path.display())]
    ProfileNotFound { path: PathBuf },

    /// A configuration file (profile, policy, sources) could not be loaded.
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
}

// ---------------------------------------------------------------------------
// AppContext
// ---------------------------------------------------------------------------

/// Stable, run-level context shared by all use cases.
///
/// Contains the repository root, platform, and resolved base directories.
/// Use-case-specific paths (e.g. the profile file) are passed as arguments.
pub struct AppContext {
    /// Repository root — where `features/`, `backends/`, `policies/` live.
    pub repo_root: PathBuf,

    /// Platform variant: Linux, Windows, or WSL.
    pub platform: platform::Platform,

    /// Resolved XDG / AppData base directories.
    pub dirs: platform::Dirs,
}

impl AppContext {
    /// Create an `AppContext` from explicit parts.
    pub fn new(repo_root: PathBuf, platform: platform::Platform, dirs: platform::Dirs) -> Self {
        Self { repo_root, platform, dirs }
    }

    /// Absolute path to the authoritative state file: `{state_home}/state.json`.
    pub fn state_path(&self) -> PathBuf {
        self.dirs.state_home.join("state.json")
    }

    /// Absolute path to the platform default policy: `{repo_root}/policies/default.{platform}.yaml`.
    pub fn default_policy_path(&self) -> PathBuf {
        let suffix = match &self.platform {
            platform::Platform::Linux   => "linux",
            platform::Platform::Windows => "windows",
            platform::Platform::Wsl     => "wsl",
        };
        self.repo_root.join("policies").join(format!("default.{suffix}.yaml"))
    }

    /// Absolute path to the user sources file: `{config_home}/sources.yaml`.
    /// May not exist; callers should treat absence as an empty `SourcesSpec`.
    pub fn sources_path(&self) -> PathBuf {
        self.dirs.config_home.join("sources.yaml")
    }
}

// ---------------------------------------------------------------------------
// plan() use case
// ---------------------------------------------------------------------------

/// Compute the plan for the given profile without executing any actions.
///
/// Returns the [`Plan`] that describes what `apply()` would do.
/// All stages are read-only; no state is modified.
pub fn plan(ctx: &AppContext, profile_path: &Path) -> Result<Plan, AppError> {
    let PipelineOutput { order, graph, state, .. } = run_pipeline(ctx, profile_path)?;
    let p = planner::plan(&graph, &state, &order)?;
    Ok(p)
}

// ---------------------------------------------------------------------------
// apply() use case
// ---------------------------------------------------------------------------

/// Execute the plan: install, update, and remove features as needed.
///
/// Feature-level failures do not abort the run; they are reported via `on_event`
/// and collected in [`ExecutorReport::failed`].
///
/// Returns `Err` only for fatal conditions (state commit failure, invariant
/// violation, or a pipeline stage failure before execution begins).
pub fn apply(
    ctx: &AppContext,
    profile_path: &Path,
    on_event: &mut dyn FnMut(Event),
) -> Result<ExecutorReport, AppError> {
    let PipelineOutput { index, order, graph, mut state, .. } =
        run_pipeline(ctx, profile_path)?;

    let p = planner::plan(&graph, &state, &order)?;

    // Build backend registry by scanning backends/ directories on disk.
    // Backends that cannot be loaded are silently skipped (non-fatal during
    // the migration from the shell implementation to the new layout).
    let registry = build_backend_registry(ctx);

    let exec_ctx = executor::ExecutionContext {
        plan: &p,
        graph: &graph,
        index: &index,
        registry: &registry,
        dirs: &ctx.dirs,
        state_path: &ctx.state_path(),
    };

    let report = executor::execute(&exec_ctx, &mut state, on_event)?;
    Ok(report)
}

// ---------------------------------------------------------------------------
// Shared pipeline (plan + apply share the read-only stages)
// ---------------------------------------------------------------------------

/// Outputs from the common read-only pipeline stages.
struct PipelineOutput {
    #[allow(dead_code)]
    profile: config::Profile,
    #[allow(dead_code)]
    policy: config::Policy,
    index: model::FeatureIndex,
    order: model::ResolvedFeatureOrder,
    graph: model::desired_resource_graph::DesiredResourceGraph,
    state: state::State,
}

/// Run the read-only stages common to both `plan()` and `apply()`.
///
/// Steps:
///   1. Validate profile file exists.
///   2. Load profile → `Profile`.
///   3. Load policy (optional) → `Policy`.
///   4. Load sources (optional) → `SourcesSpec`.
///   5. Build `FeatureIndex` from source roots.
///   6. Map profile features to `CanonicalFeatureId`s (skip unknown).
///   7. Resolve dependency order.
///   8. Compile: `FeatureIndex + Policy + order` → `DesiredResourceGraph`.
///   9. Load (or initialise) state.
fn run_pipeline(ctx: &AppContext, profile_path: &Path) -> Result<PipelineOutput, AppError> {
    // Step 1: profile file must exist.
    if !profile_path.exists() {
        return Err(AppError::ProfileNotFound { path: profile_path.to_path_buf() });
    }

    // Step 2: load profile (normalises bare names to "core/<name>").
    let profile = config::load_profile(profile_path)?;

    // Step 3: load policy if present; default to empty (no backend overrides).
    let policy = load_policy_optional(ctx)?;

    // Step 4: load sources if present; default to empty (core + user only).
    let sources = load_sources_optional(ctx)?;

    // Step 5: build feature index from all source roots.
    let source_roots = build_source_roots(ctx, &sources);
    let fi_platform = to_fi_platform(&ctx.platform);
    let index = feature_index::build(&source_roots, &fi_platform)?;

    // Step 6: convert profile keys to CanonicalFeatureIds; skip those absent from index.
    // An empty desired list is valid: it means "uninstall everything in state".
    let desired_ids = profile_to_desired_ids(&profile, &index);

    // Step 7: resolve dependency order (topological sort).
    let order = resolver::resolve(&index, &desired_ids)?;

    // Step 8: compile desired resource graph.
    let graph = compiler::compile(&index, &policy, &order)?;

    // Step 9: load state (state::load returns empty state if file absent).
    let state = state::load(&ctx.state_path())?;

    Ok(PipelineOutput { profile, policy, index, order, graph, state })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Map `platform::Platform` → `feature_index::Platform`.
fn to_fi_platform(p: &platform::Platform) -> feature_index::Platform {
    match p {
        platform::Platform::Linux   => feature_index::Platform::Linux,
        platform::Platform::Windows => feature_index::Platform::Windows,
        platform::Platform::Wsl     => feature_index::Platform::Wsl,
    }
}

/// Build the list of feature source roots for the feature-index scanner.
///
/// Implicit sources:
/// - `core` → `{repo_root}/features/`
/// - `user` → `{config_home}/features/`
///
/// External sources (from `sources.yaml`):
/// - `{id}` → `{data_home}/sources/{id}/features/`
fn build_source_roots(
    ctx: &AppContext,
    sources: &config::SourcesSpec,
) -> Vec<feature_index::SourceRoot> {
    let mut roots = vec![
        feature_index::SourceRoot {
            source_id: "core".into(),
            features_dir: ctx.repo_root.join("features"),
        },
        feature_index::SourceRoot {
            source_id: "user".into(),
            features_dir: ctx.dirs.config_home.join("features"),
        },
    ];

    for entry in &sources.sources {
        roots.push(feature_index::SourceRoot {
            source_id: entry.id.clone(),
            features_dir: ctx.dirs.data_home
                .join("sources")
                .join(&entry.id)
                .join("features"),
        });
    }

    roots
}

/// Load the platform policy; return an empty `Policy` if the file is absent.
fn load_policy_optional(ctx: &AppContext) -> Result<config::Policy, AppError> {
    let path = ctx.default_policy_path();
    if path.exists() {
        Ok(config::load_policy(&path)?)
    } else {
        Ok(config::Policy::default())
    }
}

/// Load the sources spec; return an empty `SourcesSpec` if the file is absent.
fn load_sources_optional(ctx: &AppContext) -> Result<config::SourcesSpec, AppError> {
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

/// Scan `{repo_root}/backends/` and `{config_home}/backends/` for subdirectories
/// containing a `backend.yaml` (new layout), and register them as script backends.
///
/// - Backends under `{repo_root}/backends/` → registered as `core/<name>`.
/// - Backends under `{config_home}/backends/` → registered as `user/<name>`.
///
/// Flat `.sh` files (old shell layout) and directories that fail to load are
/// skipped silently to remain resilient during the migration period.
fn build_backend_registry(ctx: &AppContext) -> backend_host::BackendRegistry {
    let mut registry = backend_host::BackendRegistry::new();
    load_backends_from_dir(&mut registry, &ctx.repo_root.join("backends"), "core");
    load_backends_from_dir(&mut registry, &ctx.dirs.config_home.join("backends"), "user");
    registry
}

/// Scan a single directory for script backend subdirectories and register each.
fn load_backends_from_dir(
    registry: &mut backend_host::BackendRegistry,
    backends_dir: &Path,
    source_id: &str,
) {
    let Ok(entries) = std::fs::read_dir(backends_dir) else { return };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue; // Skip flat .sh files (old shell layout).
        }

        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        let backend_id_str = format!("{source_id}/{name}");
        let Ok(backend_id) = model::CanonicalBackendId::new(&backend_id_str) else { continue };

        match backend_host::ScriptBackend::load(path.clone()) {
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

    /// Build an AppContext whose repo_root and dirs all point inside `tmp`.
    fn make_ctx(tmp: &TempDir) -> AppContext {
        let root = tmp.path().to_path_buf();
        AppContext {
            repo_root: root.clone(),
            platform: platform::Platform::Linux,
            dirs: platform::Dirs {
                config_home: root.join("config"),
                data_home: root.join("data"),
                state_home: root.join("state"),
            },
        }
    }

    /// Write a minimal script-mode feature to `{repo_root}/features/{name}/`.
    fn write_script_feature(root: &Path, name: &str) {
        let feat_dir = root.join("features").join(name);
        write(
            &feat_dir.join("feature.yaml"),
            "spec_version: 1\nmode: script\n",
        );
        let install_sh = feat_dir.join("install.sh");
        write(&install_sh, "#!/usr/bin/env sh\nexit 0\n");
        make_executable(&install_sh);
        let uninstall_sh = feat_dir.join("uninstall.sh");
        write(&uninstall_sh, "#!/usr/bin/env sh\nexit 0\n");
        make_executable(&uninstall_sh);
    }

    /// Write a minimal profile YAML referencing the given feature names.
    fn write_profile(dir: &Path, filename: &str, features: &[&str]) -> PathBuf {
        let content = format!(
            "features:\n{}",
            features.iter().map(|f| format!("  {f}: {{}}\n")).collect::<String>()
        );
        let path = dir.join(filename);
        write(&path, &content);
        path
    }

    /// Collect all events emitted during apply.
    fn collect_apply(
        ctx: &AppContext,
        profile_path: &Path,
    ) -> (Result<ExecutorReport, AppError>, Vec<Event>) {
        let mut events = vec![];
        let result = apply(ctx, profile_path, &mut |e| events.push(e));
        (result, events)
    }

    // --- Tests ---

    /// Missing profile returns ProfileNotFound.
    #[test]
    fn plan_missing_profile_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = make_ctx(&tmp);
        let profile_path = tmp.path().join("nonexistent.yaml");

        let err = plan(&ctx, &profile_path).unwrap_err();
        assert!(
            matches!(err, AppError::ProfileNotFound { .. }),
            "expected ProfileNotFound, got {err:?}"
        );
    }

    /// Profile with unrecognised features: recognised list is empty → plan has no actions.
    #[test]
    fn plan_unknown_features_produce_empty_plan() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = make_ctx(&tmp);
        // Feature referenced in profile does not exist in index → desired IDs empty.
        let profile_path = write_profile(tmp.path(), "profile.yaml", &["core/nonexistent"]);

        // Should succeed: empty desired produces a plan with no actions.
        let p = plan(&ctx, &profile_path).unwrap();
        assert!(p.actions.is_empty(), "plan should have no actions for unknown features");
    }

    /// plan() with a valid script feature returns a Plan with a Create action.
    #[test]
    fn plan_script_feature_returns_create_action() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = make_ctx(&tmp);
        write_script_feature(tmp.path(), "git");
        let profile_path = write_profile(tmp.path(), "profile.yaml", &["core/git"]);

        let p = plan(&ctx, &profile_path).unwrap();
        assert_eq!(p.actions.len(), 1);
        let action = &p.actions[0];
        assert_eq!(action.feature.as_str(), "core/git");
        assert!(matches!(action.operation, model::plan::Operation::Create));
    }

    /// apply() installs a script feature and commits state.
    #[test]
    fn apply_script_feature_commits_state() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = make_ctx(&tmp);
        write_script_feature(tmp.path(), "git");
        let profile_path = write_profile(tmp.path(), "profile.yaml", &["core/git"]);

        let (result, events) = collect_apply(&ctx, &profile_path);

        let report = result.unwrap();
        assert_eq!(report.executed.len(), 1, "expected one feature executed");
        assert!(report.failed.is_empty());

        // State file must be committed.
        assert!(ctx.state_path().exists(), "state.json must be written");

        // Events: FeatureStart + FeatureDone.
        let starts = events.iter().filter(|e| matches!(e, Event::FeatureStart { .. })).count();
        let dones  = events.iter().filter(|e| matches!(e, Event::FeatureDone { .. })).count();
        assert_eq!(starts, 1);
        assert_eq!(dones, 1);
    }

    /// apply() a second time on an already-installed feature emits no actions (noop).
    #[test]
    fn apply_already_installed_feature_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = make_ctx(&tmp);
        write_script_feature(tmp.path(), "git");
        let profile_path = write_profile(tmp.path(), "profile.yaml", &["core/git"]);

        // First apply: installs.
        let (r1, _) = collect_apply(&ctx, &profile_path);
        r1.unwrap();

        // Second apply: state already reflects desired; should be a noop.
        let (r2, events2) = collect_apply(&ctx, &profile_path);
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

    /// apply() missing profile propagates ProfileNotFound.
    #[test]
    fn apply_missing_profile_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = make_ctx(&tmp);
        let profile_path = tmp.path().join("does_not_exist.yaml");

        let (result, _) = collect_apply(&ctx, &profile_path);
        assert!(matches!(result.unwrap_err(), AppError::ProfileNotFound { .. }));
    }

    /// apply() two script features: both install, state has both.
    #[test]
    fn apply_multiple_features_all_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = make_ctx(&tmp);
        write_script_feature(tmp.path(), "git");
        write_script_feature(tmp.path(), "node");
        let profile_path =
            write_profile(tmp.path(), "profile.yaml", &["core/git", "core/node"]);

        let (result, _) = collect_apply(&ctx, &profile_path);
        let report = result.unwrap();

        assert_eq!(report.executed.len(), 2);
        assert!(report.failed.is_empty());
    }

    /// apply() removes a feature that is in state but not in the profile.
    #[test]
    fn apply_removes_undesired_feature_from_state() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = make_ctx(&tmp);
        write_script_feature(tmp.path(), "git");
        write_script_feature(tmp.path(), "node");

        // First apply: install git + node.
        let profile_both =
            write_profile(tmp.path(), "both.yaml", &["core/git", "core/node"]);
        collect_apply(&ctx, &profile_both).0.unwrap();

        // Second apply: only git desired → node should be destroyed.
        let profile_git_only = write_profile(tmp.path(), "git_only.yaml", &["core/git"]);
        let (result, _) = collect_apply(&ctx, &profile_git_only);
        let report = result.unwrap();

        // One action executed (Destroy node).
        assert_eq!(report.executed.len(), 1);
        assert!(report.failed.is_empty());

        // Reload state from disk and verify.
        let state = state::load(&ctx.state_path()).unwrap();
        assert!(state.features.contains_key("core/git"), "git must still be in state");
        assert!(!state.features.contains_key("core/node"), "node must be removed from state");
    }

    /// AppContext::default_policy_path returns the correct path for Linux.
    #[test]
    fn app_context_policy_path_linux() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = make_ctx(&tmp);
        let expected = tmp.path().join("policies").join("default.linux.yaml");
        assert_eq!(ctx.default_policy_path(), expected);
    }

    /// Optional policy missing → empty policy (no error).
    #[test]
    fn plan_with_no_policy_file_uses_default_policy() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = make_ctx(&tmp);
        write_script_feature(tmp.path(), "git");
        let profile_path = write_profile(tmp.path(), "profile.yaml", &["core/git"]);

        // No policies/ dir → load_policy_optional should return empty policy.
        let p = plan(&ctx, &profile_path).unwrap();
        assert_eq!(p.actions.len(), 1);
    }

    /// apply() with a script feature whose uninstall fails is non-fatal;
    /// other features in the same run still succeed.
    #[test]
    fn apply_failing_uninstall_is_non_fatal() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = make_ctx(&tmp);

        // Feature with a failing uninstall.sh.
        let feat_dir = tmp.path().join("features").join("badfeature");
        write(
            &feat_dir.join("feature.yaml"),
            "spec_version: 1\nmode: script\n",
        );
        let install_sh = feat_dir.join("install.sh");
        write(&install_sh, "#!/usr/bin/env sh\nexit 0\n");
        make_executable(&install_sh);
        let uninstall_sh = feat_dir.join("uninstall.sh");
        write(&uninstall_sh, "#!/usr/bin/env sh\nexit 1\n"); // Always fails.
        make_executable(&uninstall_sh);

        // A good feature that succeeds.
        write_script_feature(tmp.path(), "git");

        // First apply: install both.
        let profile_both =
            write_profile(tmp.path(), "both.yaml", &["core/badfeature", "core/git"]);
        collect_apply(&ctx, &profile_both).0.unwrap();

        // Second apply: only git desired → badfeature must be destroyed (fails), git is noop.
        let profile_git_only = write_profile(tmp.path(), "git.yaml", &["core/git"]);
        let (result, events) = collect_apply(&ctx, &profile_git_only);
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
