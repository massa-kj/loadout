//! Backend host: trait definition, registry, and script plugin runtime.
//!
//! Responsibilities:
//! - Define the `Backend` trait (the executor-facing contract)
//! - `BackendRegistry`: maps `CanonicalBackendId` → `Box<dyn Backend>`
//! - `ScriptBackend`: subprocess-based backend that loads `apply.sh` / `remove.sh` /
//!   `status.sh` from a directory and invokes them with the resource as JSON on stdin
//!
//! Plugin isolation: backends must not read strategy or state directly.
//! Resource routing is handled by the executor via the registry.
//!
//! See: `docs/specs/api/backend.md`

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use model::desired_resource_graph::DesiredResource;
use model::id::CanonicalBackendId;
use platform::Platform;
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Re-exports for convenience
// ---------------------------------------------------------------------------

pub use model::desired_resource_graph::{DesiredResource as Resource, DesiredResourceKind};
pub use model::id::CanonicalBackendId as BackendId;

// ---------------------------------------------------------------------------
// ResourceState
// ---------------------------------------------------------------------------

/// Installation status of a resource as reported by a backend.
///
/// Used by the Planner for noop detection: if desired resource matches installed state,
/// no action is needed.
///
/// Phase 4 uses three values only. Version-aware comparison is deferred to Phase 5.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceState {
    /// The resource is present and correctly installed.
    Installed,
    /// The resource is absent.
    NotInstalled,
    /// The backend could not determine the state (e.g. backend not available).
    ///
    /// Planner treats Unknown as NotInstalled for safety (re-installation is idempotent).
    Unknown,
}

// ---------------------------------------------------------------------------
// BackendError
// ---------------------------------------------------------------------------

/// Errors produced by backend operations.
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    /// The backend registry does not contain an entry for this ID.
    ///
    /// Should be unreachable if ComponentCompiler validated backends correctly.
    #[error("unknown backend: {id}")]
    UnknownBackend { id: String },

    /// The backend directory does not exist or is not a directory.
    ///
    /// Occurs when backend discovery failed or backend was removed after registration.
    #[error("backend directory not found: {path}")]
    DirNotFound { path: String },

    /// A required script file (`apply.sh`, `remove.sh`, `status.sh`) is missing.
    ///
    /// Indicates incomplete backend implementation.
    #[error("backend script not found: {path}")]
    ScriptNotFound { path: String },

    /// `backend.yaml` is absent or could not be parsed.
    ///
    /// Indicates malformed backend metadata.
    #[error("invalid backend.yaml at {path}: {reason}")]
    InvalidMeta { path: String, reason: String },

    /// The backend API version is not supported.
    ///
    /// Occurs when backend.yaml declares api_version that loadout does not recognize.
    #[error("unsupported backend api_version {version} at {path}")]
    UnsupportedApiVersion { version: u32, path: String },

    /// A backend script exited with a non-zero exit code.
    ///
    /// Example: `apply.sh package:foo` failed because package does not exist in repository.
    #[error("backend script failed (exit {exit_code}): {stderr}")]
    ScriptFailed { exit_code: i32, stderr: String },

    /// The script process could not be spawned.
    ///
    /// Indicates OS-level failure (e.g., permissions, resource exhaustion).
    #[error("failed to spawn backend script: {reason}")]
    SpawnFailed { reason: String },

    /// The `status` script produced output that could not be interpreted.
    ///
    /// Expected format: "installed", "not_installed", or "unknown".
    #[error("unrecognised status output from backend: {output:?}")]
    UnrecognisedStatus { output: String },

    /// The resource kind is not supported by this backend.
    ///
    /// Example: npm backend does not support runtime resources.
    #[error("resource kind not supported by this backend: {kind}")]
    NotSupported { kind: String },

    /// The `env_pre.sh` or `env_post.sh` script produced output that could not
    /// be parsed as a valid env delta JSON payload.
    ///
    /// See: `docs/specs/api/backend.md` for the expected JSON wire format.
    #[error("env script output could not be parsed: {reason}")]
    EnvScriptParseFailed { reason: String },
}

// ---------------------------------------------------------------------------
// Backend trait
// ---------------------------------------------------------------------------

/// Result returned by a successful [`Backend::apply`] call.
///
/// Contains keys of post-action env contributors that the executor should
/// evaluate after this apply completes, and whose deltas should be merged into
/// the running [`ExecutionEnvContext`] before the next action runs.
///
/// Most backends return an empty `post_contributors` list. Backends that
/// provide new tools to later actions (e.g. `core/mise` after installing a
/// runtime) populate this list so the executor can update PATH and similar
/// variables automatically.
#[derive(Debug, Default)]
pub struct BackendApplyResult {
    /// Keys into `ContributorRegistry::named` to evaluate after this apply.
    pub post_contributors: Vec<String>,
}

impl BackendApplyResult {
    /// Construct a result with no post-action contributors (the common case).
    pub fn none() -> Self {
        Self::default()
    }

    /// Construct a result advertising a single named post-action contributor.
    pub fn with_contributor(key: impl Into<String>) -> Self {
        Self {
            post_contributors: vec![key.into()],
        }
    }
}

/// The executor-facing interface for all backend implementations.
///
/// Implementations must be `Send + Sync` to allow future async or multi-threaded use.
/// Backends receive the full `DesiredResource` (including resolved `desired_backend`)
/// and are responsible only for the operation; routing is handled by the registry.
pub trait Backend: Send + Sync {
    /// Install or update the resource so that it is present.
    ///
    /// Returns a [`BackendApplyResult`] that advertises any post-action env
    /// contributors the executor should evaluate to keep the env context current.
    fn apply(&self, resource: &DesiredResource) -> Result<BackendApplyResult, BackendError>;

    /// Remove the resource so that it is no longer present.
    fn remove(&self, resource: &DesiredResource) -> Result<(), BackendError>;

    /// Query the current installation state of the resource.
    fn status(&self, resource: &DesiredResource) -> Result<ResourceState, BackendError>;

    /// Probe and return env mutations needed **before** `apply` is called.
    ///
    /// The executor merges the returned delta into the running env context and
    /// exports updated variables to the subprocess environment before invoking
    /// `apply`, `remove`, or `status`.
    ///
    /// Script backends implement this via an optional `env_pre.sh` /
    /// `env_pre.ps1` file in the backend directory. The script:
    /// - receives the same environment variables as `apply.sh` (no JSON stdin)
    /// - writes a JSON [`EnvDeltaPayload`] to **stdout** (empty stdout = no-op)
    /// - must exit 0 on success; non-zero exit → [`BackendError::ScriptFailed`]
    ///
    /// Non-fatal: the executor emits a [`ContributorWarning`] on failure and
    /// continues. Default: `Ok(None)` (no pre-action env setup).
    ///
    /// [`ContributorWarning`]: (see executor crate Event enum)
    fn env_pre(
        &self,
        resource: &DesiredResource,
    ) -> Result<Option<model::env::ExecutionEnvDelta>, BackendError> {
        let _ = resource;
        Ok(None)
    }

    /// Probe and return env mutations needed **after** a successful `apply`.
    ///
    /// Used to expose newly installed tools to subsequent backend calls in the
    /// same apply session (e.g. mise shims after installing a runtime).
    ///
    /// Same script contract as `env_pre`: optional `env_post.sh` / `env_post.ps1`,
    /// JSON stdout, no JSON stdin, exit 0 = success.
    ///
    /// Default: `Ok(None)` (no post-action env setup).
    fn env_post(
        &self,
        resource: &DesiredResource,
    ) -> Result<Option<model::env::ExecutionEnvDelta>, BackendError> {
        let _ = resource;
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// BackendRegistry
// ---------------------------------------------------------------------------

/// Maps canonical backend IDs to `Backend` implementations.
///
/// The executor owns one registry for the duration of an apply run.
/// Backends are registered before execution begins; no dynamic loading during execution.
pub struct BackendRegistry {
    backends: HashMap<String, Box<dyn Backend>>,
}

impl BackendRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            backends: HashMap::new(),
        }
    }

    /// Register a backend under the given canonical ID.
    ///
    /// If a backend with the same ID is already registered, it is replaced.
    pub fn register(&mut self, id: CanonicalBackendId, backend: Box<dyn Backend>) {
        self.backends.insert(id.as_str().to_string(), backend);
    }

    /// Look up a backend by canonical ID.
    ///
    /// Returns [`BackendError::UnknownBackend`] if not registered.
    pub fn get(&self, id: &CanonicalBackendId) -> Result<&dyn Backend, BackendError> {
        self.backends
            .get(id.as_str())
            .map(|b| b.as_ref())
            .ok_or_else(|| BackendError::UnknownBackend {
                id: id.as_str().to_string(),
            })
    }
}

impl Default for BackendRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// BackendMeta (backend.yaml)
// ---------------------------------------------------------------------------

/// Supported backend API version.
pub const BACKEND_API_VERSION: u32 = 1;

/// Parsed contents of a `backend.yaml` file in a script backend directory.
#[derive(Debug, Deserialize)]
struct BackendMeta {
    /// Must equal [`BACKEND_API_VERSION`].
    api_version: u32,
}

// ---------------------------------------------------------------------------
// ScriptBackend
// ---------------------------------------------------------------------------

/// A backend implemented as a set of shell scripts in a directory.
///
/// Expected directory layout:
/// ```text
/// <backend_dir>/
///   backend.yaml       # metadata (api_version)
///   apply.sh           # install or upgrade  (required)
///   remove.sh          # uninstall           (required)
///   status.sh          # query state         (required)
///   env_pre.sh         # pre-action env      (optional)
///   env_post.sh        # post-action env     (optional)
/// ```
///
/// Protocol for `apply.sh` / `remove.sh` / `status.sh`:
/// - Resource data passed via **environment variables** (primary).
/// - Optionally also available as JSON on **stdin** (for `jq` / complex parsing).
/// - Exit 0 = success; non-zero = failure (stderr captured).
/// - `status.sh` writes `installed`, `not_installed`, or `unknown` to stdout.
///
/// Protocol for `env_pre.sh` / `env_post.sh` (optional; api_version 1+):
/// - Receive the same environment variables as `apply.sh` (no JSON stdin).
/// - Write a JSON [`EnvDeltaPayload`] to **stdout** (empty stdout = no-op).
/// - Exit 0 = success; non-zero → [`BackendError::ScriptFailed`].
/// - Failure is non-fatal: executor emits a warning and continues.
#[derive(Debug)]
pub struct ScriptBackend {
    platform: Platform,
    backend_dir: PathBuf,
    apply_script: PathBuf,
    remove_script: PathBuf,
    status_script: PathBuf,
    /// Pre-action env script (`env_pre.sh`), present only if the file exists.
    env_pre_script: Option<PathBuf>,
    /// Post-action env script (`env_post.sh`), present only if the file exists.
    env_post_script: Option<PathBuf>,
}

impl ScriptBackend {
    /// Load and validate a script backend from `backend_dir`.
    ///
    /// Checks that the directory exists, `backend.yaml` is valid and at a supported
    /// `api_version`, and that all three scripts are present.
    ///
    /// Does **not** execute any scripts.
    pub fn load(platform: Platform, backend_dir: PathBuf) -> Result<Self, BackendError> {
        if !backend_dir.is_dir() {
            return Err(BackendError::DirNotFound {
                path: backend_dir.display().to_string(),
            });
        }

        // Parse backend.yaml.
        let meta_path = backend_dir.join("backend.yaml");
        let meta = load_meta(&meta_path)?;
        if meta.api_version != BACKEND_API_VERSION {
            return Err(BackendError::UnsupportedApiVersion {
                version: meta.api_version,
                path: backend_dir.display().to_string(),
            });
        }

        // Verify all required scripts exist (platform-specific extension).
        let script_ext = match platform {
            Platform::Windows => "ps1",
            Platform::Linux | Platform::Wsl => "sh",
        };
        let apply_script = backend_dir.join(format!("apply.{}", script_ext));
        let remove_script = backend_dir.join(format!("remove.{}", script_ext));
        let status_script = backend_dir.join(format!("status.{}", script_ext));

        for script in [&apply_script, &remove_script, &status_script] {
            if !script.is_file() {
                return Err(BackendError::ScriptNotFound {
                    path: script.display().to_string(),
                });
            }
        }

        // Detect optional env_pre / env_post scripts (api_version 1+).
        let env_pre_script = {
            let p = backend_dir.join(format!("env_pre.{}", script_ext));
            if p.is_file() {
                Some(p)
            } else {
                None
            }
        };
        let env_post_script = {
            let p = backend_dir.join(format!("env_post.{}", script_ext));
            if p.is_file() {
                Some(p)
            } else {
                None
            }
        };

        Ok(Self {
            platform,
            backend_dir,
            apply_script,
            remove_script,
            status_script,
            env_pre_script,
            env_post_script,
        })
    }

    /// Return the backend directory path (used for diagnostics).
    pub fn backend_dir(&self) -> &PathBuf {
        &self.backend_dir
    }
}

impl Backend for ScriptBackend {
    fn apply(&self, resource: &DesiredResource) -> Result<BackendApplyResult, BackendError> {
        let json = serialise_resource(resource);
        run_script(self.platform, &self.apply_script, resource, &json)?;
        Ok(BackendApplyResult::none())
    }

    fn remove(&self, resource: &DesiredResource) -> Result<(), BackendError> {
        let json = serialise_resource(resource);
        run_script(self.platform, &self.remove_script, resource, &json)?;
        Ok(())
    }

    fn status(&self, resource: &DesiredResource) -> Result<ResourceState, BackendError> {
        let json = serialise_resource(resource);
        let stdout = run_script_with_output(self.platform, &self.status_script, resource, &json)?;
        match stdout.trim() {
            "installed" => Ok(ResourceState::Installed),
            "not_installed" => Ok(ResourceState::NotInstalled),
            "unknown" => Ok(ResourceState::Unknown),
            other => Err(BackendError::UnrecognisedStatus {
                output: other.to_string(),
            }),
        }
    }

    fn env_pre(
        &self,
        resource: &DesiredResource,
    ) -> Result<Option<model::env::ExecutionEnvDelta>, BackendError> {
        match &self.env_pre_script {
            Some(script) => run_env_script(self.platform, script, resource).map(Some),
            None => Ok(None),
        }
    }

    fn env_post(
        &self,
        resource: &DesiredResource,
    ) -> Result<Option<model::env::ExecutionEnvDelta>, BackendError> {
        match &self.env_post_script {
            Some(script) => run_env_script(self.platform, script, resource).map(Some),
            None => Ok(None),
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Parse a `backend.yaml` file.
fn load_meta(path: &std::path::Path) -> Result<BackendMeta, BackendError> {
    let content = std::fs::read_to_string(path).map_err(|e| BackendError::InvalidMeta {
        path: path.display().to_string(),
        reason: e.to_string(),
    })?;
    serde_yaml::from_str::<BackendMeta>(&content).map_err(|e| BackendError::InvalidMeta {
        path: path.display().to_string(),
        reason: e.to_string(),
    })
}

/// Serialise a `DesiredResource` to JSON for script stdin.
fn serialise_resource(resource: &DesiredResource) -> String {
    serde_json::to_string(resource).unwrap_or_else(|_| "{}".to_string())
}

/// Build a Command with environment variables set from the resource.
///
/// Scripts receive parameters via environment variables (primary protocol)
/// and optionally via JSON on stdin (for complex cases requiring jq).
fn build_command_with_env(
    platform: Platform,
    script: &std::path::Path,
    resource: &DesiredResource,
) -> Command {
    let mut cmd = match platform {
        Platform::Windows => {
            let mut c = Command::new("powershell");
            c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]);
            c.arg(script);
            c
        }
        Platform::Linux | Platform::Wsl => {
            let mut c = Command::new("bash");
            c.arg(script);
            c
        }
    };

    // Common environment variables
    cmd.env("LOADOUT_RESOURCE_ID", &resource.id);

    // Kind-specific environment variables
    match &resource.kind {
        DesiredResourceKind::Package {
            name,
            desired_backend,
        } => {
            cmd.env("LOADOUT_RESOURCE_KIND", "Package");
            cmd.env("LOADOUT_PACKAGE_NAME", name);
            cmd.env("LOADOUT_BACKEND_ID", desired_backend.as_str());
        }
        DesiredResourceKind::Runtime {
            name,
            version,
            desired_backend,
        } => {
            cmd.env("LOADOUT_RESOURCE_KIND", "Runtime");
            cmd.env("LOADOUT_RUNTIME_NAME", name);
            cmd.env("LOADOUT_RUNTIME_VERSION", version);
            cmd.env("LOADOUT_BACKEND_ID", desired_backend.as_str());
        }
        DesiredResourceKind::Fs {
            source,
            path,
            entry_type,
            op,
        } => {
            cmd.env("LOADOUT_RESOURCE_KIND", "Fs");
            cmd.env("LOADOUT_FS_PATH", path);
            if let Some(src) = source {
                cmd.env("LOADOUT_FS_SOURCE", src);
            }
            cmd.env("LOADOUT_FS_ENTRY_TYPE", format!("{:?}", entry_type));
            cmd.env("LOADOUT_FS_OP", format!("{:?}", op));
        }
        DesiredResourceKind::Tool { .. } => {
            // Tool resources are installed/removed by managed_script component scripts,
            // not by any backend. Reaching this path indicates a logic error in the executor.
            panic!("backend-host must not be invoked for tool resources");
        }
    }

    cmd
}

/// Run a script with environment variables and JSON on stdin. Returns `Err` on non-zero exit.
fn run_script(
    platform: Platform,
    script: &std::path::Path,
    resource: &DesiredResource,
    json: &str,
) -> Result<(), BackendError> {
    let mut cmd = build_command_with_env(platform, script, resource);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| BackendError::SpawnFailed {
        reason: e.to_string(),
    })?;

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(json.as_bytes());
    }

    let output = child
        .wait_with_output()
        .map_err(|e| BackendError::SpawnFailed {
            reason: e.to_string(),
        })?;

    if output.status.success() {
        Ok(())
    } else {
        Err(BackendError::ScriptFailed {
            exit_code: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

/// Run a script with environment variables and JSON on stdin. Returns stdout on success.
fn run_script_with_output(
    platform: Platform,
    script: &std::path::Path,
    resource: &DesiredResource,
    json: &str,
) -> Result<String, BackendError> {
    let mut cmd = build_command_with_env(platform, script, resource);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| BackendError::SpawnFailed {
        reason: e.to_string(),
    })?;

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(json.as_bytes());
    }

    let output = child
        .wait_with_output()
        .map_err(|e| BackendError::SpawnFailed {
            reason: e.to_string(),
        })?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(BackendError::ScriptFailed {
            exit_code: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// Env delta wire format (env_pre / env_post script protocol)
// ---------------------------------------------------------------------------

/// Expected `schema_version` for env delta JSON payloads.
const ENV_DELTA_SCHEMA_VERSION: u32 = 1;

/// Top-level JSON envelope produced by `env_pre.sh` / `env_post.sh`.
///
/// Example:
/// ```json
/// {
///   "schema_version": 1,
///   "mutations": [
///     { "op": "prepend_path", "key": "PATH", "entries": ["/opt/homebrew/bin"] }
///   ],
///   "evidence": { "kind": "probed", "command": "brew --prefix" }
/// }
/// ```
#[derive(Debug, serde::Deserialize)]
struct EnvDeltaPayload {
    schema_version: u32,
    mutations: Vec<EnvMutationEntry>,
    #[serde(default)]
    evidence: EnvEvidenceEntry,
}

/// Individual mutation entry as delivered by the script.
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum EnvMutationEntry {
    Set { key: String, value: String },
    Unset { key: String },
    PrependPath { key: String, entries: Vec<String> },
    AppendPath { key: String, entries: Vec<String> },
    RemovePath { key: String, entries: Vec<String> },
}

/// Evidence entry describing how the env value was determined.
#[derive(Debug, serde::Deserialize, Default)]
struct EnvEvidenceEntry {
    #[serde(default = "EnvEvidenceEntry::default_kind")]
    kind: String,
    command: Option<String>,
    path: Option<String>,
}

impl EnvEvidenceEntry {
    fn default_kind() -> String {
        "static_default".to_string()
    }
}

/// Parse a trimmed JSON string produced by an env lifecycle script into an
/// [`ExecutionEnvDelta`].
///
/// Returns [`BackendError::EnvScriptParseFailed`] when the JSON is invalid or
/// when the `schema_version` does not match [`ENV_DELTA_SCHEMA_VERSION`].
fn parse_env_delta(stdout: &str) -> Result<model::env::ExecutionEnvDelta, BackendError> {
    let payload: EnvDeltaPayload =
        serde_json::from_str(stdout).map_err(|e| BackendError::EnvScriptParseFailed {
            reason: e.to_string(),
        })?;
    if payload.schema_version != ENV_DELTA_SCHEMA_VERSION {
        return Err(BackendError::EnvScriptParseFailed {
            reason: format!(
                "unsupported schema_version: {} (expected {})",
                payload.schema_version, ENV_DELTA_SCHEMA_VERSION
            ),
        });
    }
    let mutations = payload
        .mutations
        .into_iter()
        .map(|m| match m {
            EnvMutationEntry::Set { key, value } => model::env::EnvMutation::Set { key, value },
            EnvMutationEntry::Unset { key } => model::env::EnvMutation::Unset { key },
            EnvMutationEntry::PrependPath { key, entries } => {
                model::env::EnvMutation::PrependPath {
                    key,
                    entries: entries
                        .into_iter()
                        .map(model::env::PathEntry::new)
                        .collect(),
                }
            }
            EnvMutationEntry::AppendPath { key, entries } => model::env::EnvMutation::AppendPath {
                key,
                entries: entries
                    .into_iter()
                    .map(model::env::PathEntry::new)
                    .collect(),
            },
            EnvMutationEntry::RemovePath { key, entries } => model::env::EnvMutation::RemovePath {
                key,
                entries: entries
                    .into_iter()
                    .map(model::env::PathEntry::new)
                    .collect(),
            },
        })
        .collect();
    let evidence = match payload.evidence.kind.as_str() {
        "probed" => model::env::EnvEvidence::Probed {
            command: payload.evidence.command.unwrap_or_default(),
        },
        "config_file" => model::env::EnvEvidence::ConfigFile {
            path: std::path::PathBuf::from(payload.evidence.path.unwrap_or_default()),
        },
        _ => model::env::EnvEvidence::StaticDefault,
    };
    Ok(model::env::ExecutionEnvDelta {
        mutations,
        evidence,
    })
}

/// Run an optional env lifecycle script (`env_pre.sh` / `env_post.sh`) and
/// parse its JSON output into an [`ExecutionEnvDelta`].
///
/// - The script receives the same environment variables as `apply.sh`.
/// - No JSON is written to stdin (scripts must not rely on it).
/// - Empty stdout (exit 0) is silently treated as an empty delta.
/// - Non-zero exit → [`BackendError::ScriptFailed`].
fn run_env_script(
    platform: Platform,
    script: &std::path::Path,
    resource: &DesiredResource,
) -> Result<model::env::ExecutionEnvDelta, BackendError> {
    let mut cmd = build_command_with_env(platform, script, resource);
    // No JSON stdin for env scripts; stderr is forwarded to the user.
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    let output = cmd.output().map_err(|e| BackendError::SpawnFailed {
        reason: e.to_string(),
    })?;

    if !output.status.success() {
        return Err(BackendError::ScriptFailed {
            exit_code: output.status.code().unwrap_or(-1),
            // stderr is inherited (visible to user); no need to echo it here.
            stderr: String::new(),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        // Empty output is a valid no-op.
        return Ok(model::env::ExecutionEnvDelta::empty());
    }
    parse_env_delta(trimmed)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use model::desired_resource_graph::{DesiredResource, DesiredResourceKind};
    use model::id::CanonicalBackendId;
    use std::fs;
    use tempfile::TempDir;

    // --- Test helpers -------------------------------------------------------

    fn backend_id(s: &str) -> CanonicalBackendId {
        CanonicalBackendId::new(s).unwrap()
    }

    fn package_resource(name: &str, backend: &str) -> DesiredResource {
        DesiredResource {
            id: format!("package:{name}"),
            kind: DesiredResourceKind::Package {
                name: name.to_string(),
                desired_backend: CanonicalBackendId::new(backend).unwrap(),
            },
        }
    }

    /// Minimal backend that always succeeds and reports Installed.
    struct AlwaysOkBackend;

    impl Backend for AlwaysOkBackend {
        fn apply(&self, _r: &DesiredResource) -> Result<BackendApplyResult, BackendError> {
            Ok(BackendApplyResult::none())
        }
        fn remove(&self, _r: &DesiredResource) -> Result<(), BackendError> {
            Ok(())
        }
        fn status(&self, _r: &DesiredResource) -> Result<ResourceState, BackendError> {
            Ok(ResourceState::Installed)
        }
    }

    /// Backend that always fails every operation.
    struct AlwaysFailBackend;

    impl Backend for AlwaysFailBackend {
        fn apply(&self, _r: &DesiredResource) -> Result<BackendApplyResult, BackendError> {
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
        fn status(&self, _r: &DesiredResource) -> Result<ResourceState, BackendError> {
            Ok(ResourceState::NotInstalled)
        }
    }

    fn make_script_backend_dir(api_version: u32, scripts: &[&str]) -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        let meta = format!("api_version: {api_version}\n");
        fs::write(dir.path().join("backend.yaml"), meta).unwrap();
        for script in scripts {
            fs::write(dir.path().join(script), "#!/usr/bin/env sh\necho ok\n").unwrap();
        }
        dir
    }

    // --- BackendRegistry tests ----------------------------------------------

    #[test]
    fn registry_starts_empty() {
        let reg = BackendRegistry::new();
        let result = reg.get(&backend_id("core/brew"));
        assert!(result.is_err());
        let err = result.err().expect("expected error");
        assert!(matches!(err, BackendError::UnknownBackend { .. }));
    }

    #[test]
    fn registry_default_is_empty() {
        let reg = BackendRegistry::default();
        assert!(reg.get(&backend_id("core/apt")).is_err());
    }

    #[test]
    fn registry_register_and_get() {
        let mut reg = BackendRegistry::new();
        reg.register(backend_id("core/brew"), Box::new(AlwaysOkBackend));
        let backend = reg.get(&backend_id("core/brew")).unwrap();
        let r = package_resource("git", "core/brew");
        assert!(backend.apply(&r).is_ok());
    }

    #[test]
    fn registry_unknown_id_returns_error() {
        let mut reg = BackendRegistry::new();
        reg.register(backend_id("core/brew"), Box::new(AlwaysOkBackend));
        let result = reg.get(&backend_id("core/apt"));
        assert!(result.is_err());
        let err = result.err().expect("expected error");
        assert!(matches!(err, BackendError::UnknownBackend { .. }));
    }

    #[test]
    fn registry_replace_existing_backend() {
        let mut reg = BackendRegistry::new();
        reg.register(backend_id("core/brew"), Box::new(AlwaysOkBackend));
        reg.register(backend_id("core/brew"), Box::new(AlwaysFailBackend));
        let backend = reg.get(&backend_id("core/brew")).unwrap();
        let r = package_resource("git", "core/brew");
        // AlwaysFailBackend.apply returns Err
        assert!(backend.apply(&r).is_err());
    }

    #[test]
    fn registry_multiple_backends_dispatch_correctly() {
        let mut reg = BackendRegistry::new();
        reg.register(backend_id("core/brew"), Box::new(AlwaysOkBackend));
        reg.register(backend_id("core/apt"), Box::new(AlwaysFailBackend));

        let r = package_resource("git", "core/brew");
        assert!(reg.get(&backend_id("core/brew")).unwrap().apply(&r).is_ok());
        assert!(reg.get(&backend_id("core/apt")).unwrap().apply(&r).is_err());
    }

    // --- ResourceState tests ------------------------------------------------

    #[test]
    fn resource_state_equality() {
        assert_eq!(ResourceState::Installed, ResourceState::Installed);
        assert_ne!(ResourceState::Installed, ResourceState::NotInstalled);
        assert_ne!(ResourceState::NotInstalled, ResourceState::Unknown);
    }

    #[test]
    fn mock_backend_status_variants() {
        let r = package_resource("git", "core/brew");
        assert_eq!(
            AlwaysOkBackend.status(&r).unwrap(),
            ResourceState::Installed
        );
        assert_eq!(
            AlwaysFailBackend.status(&r).unwrap(),
            ResourceState::NotInstalled
        );
    }

    // --- ScriptBackend::load tests ------------------------------------------

    #[test]
    fn script_backend_load_success() {
        let dir = make_script_backend_dir(1, &["apply.sh", "remove.sh", "status.sh"]);
        let backend = ScriptBackend::load(Platform::Linux, dir.path().to_path_buf()).unwrap();
        assert_eq!(backend.backend_dir(), &dir.path().to_path_buf());
    }

    #[test]
    fn script_backend_load_dir_not_found() {
        let err = ScriptBackend::load(Platform::Linux, PathBuf::from("/nonexistent/backend"))
            .unwrap_err();
        assert!(matches!(err, BackendError::DirNotFound { .. }));
    }

    #[test]
    fn script_backend_load_missing_backend_yaml() {
        let dir = tempfile::tempdir().unwrap();
        // No backend.yaml written
        let err = ScriptBackend::load(Platform::Linux, dir.path().to_path_buf()).unwrap_err();
        assert!(matches!(err, BackendError::InvalidMeta { .. }));
    }

    #[test]
    fn script_backend_load_unsupported_api_version() {
        let dir = make_script_backend_dir(99, &["apply.sh", "remove.sh", "status.sh"]);
        let err = ScriptBackend::load(Platform::Linux, dir.path().to_path_buf()).unwrap_err();
        assert!(matches!(
            err,
            BackendError::UnsupportedApiVersion { version: 99, .. }
        ));
    }

    #[test]
    fn script_backend_load_missing_script() {
        // apply.sh and remove.sh present, status.sh missing
        let dir = make_script_backend_dir(1, &["apply.sh", "remove.sh"]);
        let err = ScriptBackend::load(Platform::Linux, dir.path().to_path_buf()).unwrap_err();
        assert!(matches!(err, BackendError::ScriptNotFound { .. }));
    }

    // --- BackendError display -----------------------------------------------

    #[test]
    fn backend_error_messages_are_nonempty() {
        let errors: &[BackendError] = &[
            BackendError::UnknownBackend {
                id: "core/brew".to_string(),
            },
            BackendError::DirNotFound {
                path: "/tmp/x".to_string(),
            },
            BackendError::ScriptNotFound {
                path: "/tmp/apply.sh".to_string(),
            },
            BackendError::InvalidMeta {
                path: "/tmp/backend.yaml".to_string(),
                reason: "bad".to_string(),
            },
            BackendError::UnsupportedApiVersion {
                version: 2,
                path: "/tmp".to_string(),
            },
            BackendError::ScriptFailed {
                exit_code: 1,
                stderr: "oops".to_string(),
            },
            BackendError::SpawnFailed {
                reason: "no sh".to_string(),
            },
            BackendError::UnrecognisedStatus {
                output: "??".to_string(),
            },
            BackendError::NotSupported {
                kind: "runtime".to_string(),
            },
        ];
        for e in errors {
            assert!(!e.to_string().is_empty(), "empty error message for {e:?}");
        }
    }
}
