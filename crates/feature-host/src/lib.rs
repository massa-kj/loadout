//! Feature host: subprocess execution of script-mode feature scripts.
//!
//! Responsibilities:
//! - Execute `install.sh` / `uninstall.sh` from a feature's `source_dir`
//! - Inject context via environment variables (not JSON stdin/stdout)
//! - Capture stderr for logging; propagate exit code
//!
//! Feature scripts must NOT be given write access to state. This crate must not
//! import or call `state::commit`. State updates are the executor's responsibility.
//!
//! Script interface:
//! - Input:  environment variables (see [`FeatureEnv`])
//! - Output: exit code 0 = success, non-0 = failure; stderr captured for logging
//!
//! See: `docs/specs/api/feature-host.md` and `20260315_crate.md`

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use model::feature_index::FeatureMeta;
use model::id::CanonicalFeatureId;
pub use platform::Dirs;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// The output of a successfully executed feature script.
#[derive(Debug, Clone)]
pub struct FeatureOutput {
    /// Lines written to stdout by the feature script.
    pub stdout: String,
    /// Lines written to stderr by the feature script.
    pub stderr: String,
}

/// The operation to perform on a feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureOp {
    Install,
    Uninstall,
}

impl FeatureOp {
    /// The script filename for this operation.
    fn script_name(&self) -> &'static str {
        match self {
            FeatureOp::Install => "install.sh",
            FeatureOp::Uninstall => "uninstall.sh",
        }
    }
}

/// Errors produced by the feature host.
#[derive(Debug, thiserror::Error)]
pub enum FeatureHostError {
    /// The required script (`install.sh` or `uninstall.sh`) is absent.
    #[error("feature script not found: {path}")]
    ScriptNotFound { path: String },

    /// The feature's `source_dir` directory does not exist.
    #[error("feature source directory not found: {path}")]
    SourceDirNotFound { path: String },

    /// The script process could not be spawned (e.g. `sh` not in PATH).
    #[error("failed to spawn feature script: {reason}")]
    SpawnFailed { reason: String },

    /// The script exited with a non-zero exit code.
    #[error("feature script failed (exit {exit_code}): {stderr}")]
    ScriptFailed { exit_code: i32, stderr: String },
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Execute the `install.sh` script for a script-mode feature.
///
/// `feature_id` and `dirs` are injected as environment variables so the script
/// can locate config/state roots without hard-coded paths.
///
/// # Errors
///
/// Returns [`FeatureHostError::ScriptNotFound`] if `install.sh` is absent,
/// or [`FeatureHostError::ScriptFailed`] on non-zero exit.
pub fn run_install(
    meta: &FeatureMeta,
    feature_id: &CanonicalFeatureId,
    dirs: &Dirs,
) -> Result<FeatureOutput, FeatureHostError> {
    run_op(meta, feature_id, dirs, FeatureOp::Install)
}

/// Execute the `uninstall.sh` script for a script-mode feature.
///
/// Same contract as [`run_install`].
pub fn run_uninstall(
    meta: &FeatureMeta,
    feature_id: &CanonicalFeatureId,
    dirs: &Dirs,
) -> Result<FeatureOutput, FeatureHostError> {
    run_op(meta, feature_id, dirs, FeatureOp::Uninstall)
}

// ---------------------------------------------------------------------------
// Internal implementation
// ---------------------------------------------------------------------------

fn run_op(
    meta: &FeatureMeta,
    feature_id: &CanonicalFeatureId,
    dirs: &Dirs,
    op: FeatureOp,
) -> Result<FeatureOutput, FeatureHostError> {
    let source_dir = PathBuf::from(&meta.source_dir);

    if !source_dir.is_dir() {
        return Err(FeatureHostError::SourceDirNotFound {
            path: meta.source_dir.clone(),
        });
    }

    let script = source_dir.join(op.script_name());
    if !script.is_file() {
        return Err(FeatureHostError::ScriptNotFound {
            path: script.display().to_string(),
        });
    }

    execute_script(&script, feature_id, dirs)
}

/// Spawn the script and wait for completion.
fn execute_script(
    script: &Path,
    feature_id: &CanonicalFeatureId,
    dirs: &Dirs,
) -> Result<FeatureOutput, FeatureHostError> {
    let output = Command::new("bash")
        .arg(script)
        .env("LOADOUT_FEATURE_ID", feature_id.as_str())
        .env("LOADOUT_CONFIG_HOME", &dirs.config_home)
        .env("LOADOUT_DATA_HOME", &dirs.data_home)
        .env("LOADOUT_STATE_HOME", &dirs.state_home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| FeatureHostError::SpawnFailed {
            reason: e.to_string(),
        })?;

    if output.status.success() {
        Ok(FeatureOutput {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    } else {
        Err(FeatureHostError::ScriptFailed {
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
    use model::feature_index::{DepSpec, FeatureMeta, FeatureMode};
    use model::id::CanonicalFeatureId;
    use platform::Dirs;
    use std::fs;
    use tempfile::TempDir;

    // --- Helpers ------------------------------------------------------------

    fn make_feature_id(s: &str) -> CanonicalFeatureId {
        CanonicalFeatureId::new(s).unwrap()
    }

    fn make_meta(source_dir: &str) -> FeatureMeta {
        FeatureMeta {
            spec_version: 1,
            mode: FeatureMode::Script,
            description: None,
            source_dir: source_dir.to_string(),
            dep: DepSpec::default(),
            spec: None,
        }
    }

    fn make_dirs(tmp: &TempDir) -> Dirs {
        Dirs {
            config_home: tmp.path().join("config"),
            data_home: tmp.path().join("data"),
            state_home: tmp.path().join("state"),
        }
    }

    /// Write a minimal install.sh that exits 0 and prints text.
    fn write_ok_script(dir: &Path, name: &str, body: &str) {
        let content = format!("#!/usr/bin/env sh\n{body}\n");
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        // make executable (needed on Linux/macOS)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    // --- run_install tests --------------------------------------------------

    #[test]
    fn install_success_exits_zero() {
        let tmp = tempfile::tempdir().unwrap();
        write_ok_script(tmp.path(), "install.sh", "exit 0");
        let meta = make_meta(tmp.path().to_str().unwrap());
        let dirs = make_dirs(&tmp);

        let result = run_install(&meta, &make_feature_id("core/brew"), &dirs);
        assert!(result.is_ok(), "expected ok, got: {result:?}");
    }

    #[test]
    fn install_script_stdout_captured() {
        let tmp = tempfile::tempdir().unwrap();
        write_ok_script(tmp.path(), "install.sh", "echo hello_from_install");
        let meta = make_meta(tmp.path().to_str().unwrap());
        let dirs = make_dirs(&tmp);

        let out = run_install(&meta, &make_feature_id("core/brew"), &dirs).unwrap();
        assert!(out.stdout.contains("hello_from_install"));
    }

    #[test]
    fn install_script_stderr_captured_on_success() {
        let tmp = tempfile::tempdir().unwrap();
        write_ok_script(tmp.path(), "install.sh", "echo warn >&2");
        let meta = make_meta(tmp.path().to_str().unwrap());
        let dirs = make_dirs(&tmp);

        let out = run_install(&meta, &make_feature_id("core/mise"), &dirs).unwrap();
        assert!(out.stderr.contains("warn"));
    }

    #[test]
    fn install_script_nonzero_exit_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        write_ok_script(
            tmp.path(),
            "install.sh",
            "echo 'install failed' >&2\nexit 2",
        );
        let meta = make_meta(tmp.path().to_str().unwrap());
        let dirs = make_dirs(&tmp);

        let err = run_install(&meta, &make_feature_id("core/brew"), &dirs).unwrap_err();
        assert!(matches!(
            err,
            FeatureHostError::ScriptFailed { exit_code: 2, .. }
        ));
    }

    #[test]
    fn install_script_missing_returns_script_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        // No install.sh written
        let meta = make_meta(tmp.path().to_str().unwrap());
        let dirs = make_dirs(&tmp);

        let err = run_install(&meta, &make_feature_id("core/brew"), &dirs).unwrap_err();
        assert!(matches!(err, FeatureHostError::ScriptNotFound { .. }));
    }

    #[test]
    fn install_source_dir_missing_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let meta = make_meta("/nonexistent/feature/dir");
        let dirs = make_dirs(&tmp);

        let err = run_install(&meta, &make_feature_id("core/brew"), &dirs).unwrap_err();
        assert!(matches!(err, FeatureHostError::SourceDirNotFound { .. }));
    }

    // --- run_uninstall tests ------------------------------------------------

    #[test]
    fn uninstall_success_exits_zero() {
        let tmp = tempfile::tempdir().unwrap();
        write_ok_script(tmp.path(), "uninstall.sh", "exit 0");
        let meta = make_meta(tmp.path().to_str().unwrap());
        let dirs = make_dirs(&tmp);

        let result = run_uninstall(&meta, &make_feature_id("core/brew"), &dirs);
        assert!(result.is_ok());
    }

    #[test]
    fn uninstall_script_missing_returns_script_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        // No uninstall.sh written (only install.sh)
        write_ok_script(tmp.path(), "install.sh", "exit 0");
        let meta = make_meta(tmp.path().to_str().unwrap());
        let dirs = make_dirs(&tmp);

        let err = run_uninstall(&meta, &make_feature_id("core/brew"), &dirs).unwrap_err();
        assert!(matches!(err, FeatureHostError::ScriptNotFound { .. }));
    }

    #[test]
    fn uninstall_nonzero_exit_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        write_ok_script(tmp.path(), "uninstall.sh", "exit 3");
        let meta = make_meta(tmp.path().to_str().unwrap());
        let dirs = make_dirs(&tmp);

        let err = run_uninstall(&meta, &make_feature_id("core/brew"), &dirs).unwrap_err();
        assert!(matches!(
            err,
            FeatureHostError::ScriptFailed { exit_code: 3, .. }
        ));
    }

    // --- Environment variable injection tests -------------------------------

    #[test]
    fn env_vars_are_injected_into_script() {
        let tmp = tempfile::tempdir().unwrap();
        // Script prints the env vars; we verify they are present in stdout
        write_ok_script(
            tmp.path(),
            "install.sh",
            "printf '%s\\n' \"$LOADOUT_FEATURE_ID\" \"$LOADOUT_CONFIG_HOME\" \
             \"$LOADOUT_DATA_HOME\" \"$LOADOUT_STATE_HOME\"",
        );
        let meta = make_meta(tmp.path().to_str().unwrap());
        let dirs = Dirs {
            config_home: PathBuf::from("/tmp/cfg/loadout"),
            data_home: PathBuf::from("/tmp/data/loadout"),
            state_home: PathBuf::from("/tmp/state/loadout"),
        };

        let out = run_install(&meta, &make_feature_id("core/git"), &dirs).unwrap();
        assert!(
            out.stdout.contains("core/git"),
            "LOADOUT_FEATURE_ID missing"
        );
        assert!(
            out.stdout.contains("/tmp/cfg/loadout"),
            "LOADOUT_CONFIG_HOME missing"
        );
        assert!(
            out.stdout.contains("/tmp/data/loadout"),
            "LOADOUT_DATA_HOME missing"
        );
        assert!(
            out.stdout.contains("/tmp/state/loadout"),
            "LOADOUT_STATE_HOME missing"
        );
    }

    // --- FeatureHostError display -------------------------------------------

    #[test]
    fn error_messages_are_nonempty() {
        let errors: &[FeatureHostError] = &[
            FeatureHostError::ScriptNotFound {
                path: "/tmp/install.sh".to_string(),
            },
            FeatureHostError::SourceDirNotFound {
                path: "/tmp/feat".to_string(),
            },
            FeatureHostError::SpawnFailed {
                reason: "no sh".to_string(),
            },
            FeatureHostError::ScriptFailed {
                exit_code: 1,
                stderr: "boom".to_string(),
            },
        ];
        for e in errors {
            assert!(!e.to_string().is_empty(), "empty message: {e:?}");
        }
    }
}
