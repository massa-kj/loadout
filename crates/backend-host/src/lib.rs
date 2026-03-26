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
    /// Should be unreachable if FeatureCompiler validated backends correctly.
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
}

// ---------------------------------------------------------------------------
// Backend trait
// ---------------------------------------------------------------------------

/// The executor-facing interface for all backend implementations.
///
/// Implementations must be `Send + Sync` to allow future async or multi-threaded use.
/// Backends receive the full `DesiredResource` (including resolved `desired_backend`)
/// and are responsible only for the operation; routing is handled by the registry.
pub trait Backend: Send + Sync {
    /// Install or update the resource so that it is present.
    fn apply(&self, resource: &DesiredResource) -> Result<(), BackendError>;

    /// Remove the resource so that it is no longer present.
    fn remove(&self, resource: &DesiredResource) -> Result<(), BackendError>;

    /// Query the current installation state of the resource.
    fn status(&self, resource: &DesiredResource) -> Result<ResourceState, BackendError>;
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
///   backend.yaml     # metadata (api_version)
///   apply.sh         # install / update
///   remove.sh        # uninstall
///   status.sh        # query current state
/// ```
///
/// Protocol:
/// - The resource is serialised as JSON and written to the script's **stdin**.
/// - `apply.sh` / `remove.sh`: exit 0 = success, non-0 = failure (stderr captured).
/// - `status.sh`: exit 0 + stdout one of `installed`, `not_installed`, `unknown`.
#[derive(Debug)]
pub struct ScriptBackend {
    platform: Platform,
    backend_dir: PathBuf,
    apply_script: PathBuf,
    remove_script: PathBuf,
    status_script: PathBuf,
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

        Ok(Self {
            platform,
            backend_dir,
            apply_script,
            remove_script,
            status_script,
        })
    }

    /// Return the backend directory path (used for diagnostics).
    pub fn backend_dir(&self) -> &PathBuf {
        &self.backend_dir
    }
}

impl Backend for ScriptBackend {
    fn apply(&self, resource: &DesiredResource) -> Result<(), BackendError> {
        let json = serialise_resource(resource);
        run_script(self.platform, &self.apply_script, resource, &json)?;
        Ok(())
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
        fn apply(&self, _r: &DesiredResource) -> Result<(), BackendError> {
            Ok(())
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
