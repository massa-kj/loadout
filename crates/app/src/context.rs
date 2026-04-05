// Application-level context and error types shared by all use cases.

use std::path::PathBuf;

/// All pipeline-level errors.
///
/// These are returned as `Err` only for fatal, run-aborting conditions.
/// Feature-level failures during `apply()` are reported via [`executor::Event::FeatureFailed`]
/// and collected in [`executor::ExecutorReport::failed`], not surfaced as `AppError`.
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

    /// Feature not found in the index.
    #[error("feature '{id}' not found")]
    FeatureNotFound { id: String },

    /// Backend not found in any source root.
    #[error("backend '{id}' not found")]
    BackendNotFound { id: String },

    /// Source not found in sources.yaml.
    #[error("source '{id}' not found")]
    SourceNotFound { id: String },

    /// No cached env plan found; `loadout apply` must be run first.
    #[error("no cached env plan — run 'loadout apply' first")]
    EnvPlanNotFound,

    /// Failed to read the env plan cache file.
    #[error("failed to read env plan cache: {0}")]
    EnvPlanIo(std::io::Error),

    /// Failed to deserialize the env plan cache.
    #[error("failed to deserialize env plan cache: {0}")]
    EnvPlanDeserialize(serde_json::Error),

    /// No config specified and no context is currently set.
    #[error("no config specified and no context is set — use 'loadout context set <name>'")]
    NoActiveContext,

    /// The target directory or file already exists.
    #[error("already exists: {}", path.display())]
    AlreadyExists { path: PathBuf },

    /// I/O error during a scaffold (create-file) operation.
    #[error("I/O error at {}: {source}", path.display())]
    ScaffoldIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

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

    /// Return the currently active context name, or `None` if not set.
    ///
    /// The context is stored as a bare config name in `{config_home}/current`.
    pub fn current_context(&self) -> Option<String> {
        let path = self.dirs.config_home.join("current");
        std::fs::read_to_string(&path)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Resolve a config name-or-path string to a `PathBuf`.
    ///
    /// - Value containing `.yaml` or `.yml` → literal path.
    /// - Otherwise → `{config_home}/configs/{value}.yaml`.
    pub fn resolve_config_path(&self, value: &str) -> PathBuf {
        if value.contains(".yaml") || value.contains(".yml") {
            PathBuf::from(value)
        } else {
            self.dirs
                .config_home
                .join("configs")
                .join(format!("{value}.yaml"))
        }
    }
}
