//! Component host: subprocess execution of script-mode component scripts.
//!
//! Responsibilities:
//! - Execute `install.sh` / `uninstall.sh` from a component's `source_dir``
//! - Inject context via environment variables (not JSON stdin/stdout)
//! - Capture stderr for logging; propagate exit code
//!
//! Component scripts must NOT be given write access to state. This crate must not
//! import or call `state::commit`. State updates are the executor's responsibility.
//!
//! Script interface:
//! - Input:  environment variables (see [`ComponentEnv`])
//! - Output: exit code 0 = success, non-0 = failure; stderr captured for logging
//!
//! See: `docs/specs/api/component-host.md` and `20260315_crate.md`

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use model::component_index::ComponentMeta;
use model::id::CanonicalComponentId;
pub use platform::{Dirs, Platform};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// The output of a successfully executed component script.
#[derive(Debug, Clone)]
pub struct ComponentOutput {
    /// Lines written to stdout by the component script.
    pub stdout: String,
    /// Lines written to stderr by the component script.
    pub stderr: String,
}

/// The operation to perform on a component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentOp {
    Install,
    Uninstall,
}

impl ComponentOp {
    /// The script filename for this operation, platform-appropriate.
    fn script_name(&self, platform: &Platform) -> String {
        let base = match self {
            ComponentOp::Install => "install",
            ComponentOp::Uninstall => "uninstall",
        };
        let ext = match platform {
            Platform::Windows => "ps1",
            Platform::Linux | Platform::Wsl => "sh",
        };
        format!("{base}.{ext}")
    }
}

/// Errors produced by the component host.
#[derive(Debug, thiserror::Error)]
pub enum ComponentHostError {
    /// The required script (`install.sh` or `uninstall.sh`) is absent.
    #[error("component script not found: {path}")]
    ScriptNotFound { path: String },

    /// The component's `source_dir` directory does not exist.
    #[error("component source directory not found: {path}")]
    SourceDirNotFound { path: String },

    /// The script process could not be spawned (e.g. `sh` not in PATH).
    #[error("failed to spawn component script: {reason}")]
    SpawnFailed { reason: String },

    /// The script exited with a non-zero exit code.
    #[error("component script failed (exit {exit_code}): {stderr}")]
    ScriptFailed { exit_code: i32, stderr: String },
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Execute the install script for a script-mode component.
///
/// Uses `install.sh` on Linux/WSL or `install.ps1` on Windows.
///
/// `component_id` and `dirs` are injected as environment variables so the script
/// can locate config/state roots without hard-coded paths.
///
/// # Errors
///
/// Returns [`ComponentHostError::ScriptNotFound`] if the install script is absent,
/// or [`ComponentHostError::ScriptFailed`] on non-zero exit.
pub fn run_install(
    meta: &ComponentMeta,
    component_id: &CanonicalComponentId,
    dirs: &Dirs,
    platform: &Platform,
) -> Result<ComponentOutput, ComponentHostError> {
    run_op(meta, component_id, dirs, platform, ComponentOp::Install)
}

/// Execute the uninstall script for a script-mode component.
///
/// Uses `uninstall.sh` on Linux/WSL or `uninstall.ps1` on Windows.
///
/// Same contract as [`run_install`].
pub fn run_uninstall(
    meta: &ComponentMeta,
    component_id: &CanonicalComponentId,
    dirs: &Dirs,
    platform: &Platform,
) -> Result<ComponentOutput, ComponentHostError> {
    run_op(meta, component_id, dirs, platform, ComponentOp::Uninstall)
}

// ---------------------------------------------------------------------------
// Internal implementation
// ---------------------------------------------------------------------------

fn run_op(
    meta: &ComponentMeta,
    component_id: &CanonicalComponentId,
    dirs: &Dirs,
    platform: &Platform,
    op: ComponentOp,
) -> Result<ComponentOutput, ComponentHostError> {
    let source_dir = PathBuf::from(&meta.source_dir);

    if !source_dir.is_dir() {
        return Err(ComponentHostError::SourceDirNotFound {
            path: meta.source_dir.clone(),
        });
    }

    let script = source_dir.join(op.script_name(platform));
    if !script.is_file() {
        return Err(ComponentHostError::ScriptNotFound {
            path: script.display().to_string(),
        });
    }

    execute_script(&script, component_id, dirs, platform)
}

/// Spawn the script and wait for completion.
fn execute_script(
    script: &Path,
    component_id: &CanonicalComponentId,
    dirs: &Dirs,
    platform: &Platform,
) -> Result<ComponentOutput, ComponentHostError> {
    let mut cmd = match platform {
        Platform::Windows => {
            let mut c = Command::new("powershell");
            c.arg("-ExecutionPolicy").arg("Bypass");
            c.arg("-File").arg(script);
            c
        }
        Platform::Linux | Platform::Wsl => {
            let mut c = Command::new("bash");
            c.arg(script);
            c
        }
    };

    let output = cmd
        .env("LOADOUT_COMPONENT_ID", component_id.as_str())
        .env("LOADOUT_CONFIG_HOME", &dirs.config_home)
        .env("LOADOUT_DATA_HOME", &dirs.data_home)
        .env("LOADOUT_STATE_HOME", &dirs.state_home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| ComponentHostError::SpawnFailed {
            reason: e.to_string(),
        })?;

    if output.status.success() {
        Ok(ComponentOutput {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    } else {
        Err(ComponentHostError::ScriptFailed {
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
    use model::component_index::{ComponentMeta, ComponentMode, DepSpec};
    use model::id::CanonicalComponentId;
    use platform::Dirs;
    use std::fs;
    use tempfile::TempDir;

    // --- Helpers ------------------------------------------------------------

    fn make_component_id(s: &str) -> CanonicalComponentId {
        CanonicalComponentId::new(s).unwrap()
    }

    fn make_meta(source_dir: &str) -> ComponentMeta {
        ComponentMeta {
            spec_version: 1,
            mode: ComponentMode::Script,
            description: None,
            source_dir: source_dir.to_string(),
            dep: DepSpec::default(),
            spec: None,
            scripts: None,
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

    fn current_platform() -> Platform {
        platform::detect_platform()
    }

    /// Write a platform-appropriate script that exits 0 and prints text.
    /// On Unix: .sh with shebang; on Windows: .ps1 without shebang.
    fn write_ok_script(dir: &Path, name: &str, body: &str) {
        let platform = current_platform();
        let (filename, content) = match platform {
            Platform::Windows => {
                // PowerShell script
                let ps_name = name.replace(".sh", ".ps1");
                (ps_name, body.to_string())
            }
            Platform::Linux | Platform::Wsl => {
                // Shell script
                (name.to_string(), format!("#!/usr/bin/env sh\n{body}\n"))
            }
        };
        let path = dir.join(&filename);
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
        let platform = current_platform();

        let result = run_install(&meta, &make_component_id("core/brew"), &dirs, &platform);
        assert!(result.is_ok(), "expected ok, got: {result:?}");
    }

    #[test]
    fn install_script_stdout_captured() {
        let tmp = tempfile::tempdir().unwrap();
        write_ok_script(tmp.path(), "install.sh", "echo hello_from_install");
        let meta = make_meta(tmp.path().to_str().unwrap());
        let dirs = make_dirs(&tmp);
        let platform = current_platform();

        let out = run_install(&meta, &make_component_id("core/brew"), &dirs, &platform).unwrap();
        assert!(out.stdout.contains("hello_from_install"));
    }

    #[test]
    fn install_script_stderr_captured_on_success() {
        let tmp = tempfile::tempdir().unwrap();
        let platform = current_platform();
        let body = match platform {
            Platform::Windows => "[Console]::Error.WriteLine('warn')",
            Platform::Linux | Platform::Wsl => "echo warn >&2",
        };
        write_ok_script(tmp.path(), "install.sh", body);
        let meta = make_meta(tmp.path().to_str().unwrap());
        let dirs = make_dirs(&tmp);

        let out = run_install(&meta, &make_component_id("core/mise"), &dirs, &platform).unwrap();
        assert!(out.stderr.contains("warn"));
    }

    #[test]
    fn install_script_nonzero_exit_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let platform = current_platform();
        let body = match platform {
            Platform::Windows => "[Console]::Error.WriteLine('install failed')\nexit 2",
            Platform::Linux | Platform::Wsl => "echo 'install failed' >&2\nexit 2",
        };
        write_ok_script(tmp.path(), "install.sh", body);
        let meta = make_meta(tmp.path().to_str().unwrap());
        let dirs = make_dirs(&tmp);

        let err =
            run_install(&meta, &make_component_id("core/brew"), &dirs, &platform).unwrap_err();
        assert!(matches!(
            err,
            ComponentHostError::ScriptFailed { exit_code: 2, .. }
        ));
    }

    #[test]
    fn install_script_missing_returns_script_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        // No install.sh written
        let meta = make_meta(tmp.path().to_str().unwrap());
        let dirs = make_dirs(&tmp);
        let platform = current_platform();

        let err =
            run_install(&meta, &make_component_id("core/brew"), &dirs, &platform).unwrap_err();
        assert!(matches!(err, ComponentHostError::ScriptNotFound { .. }));
    }

    #[test]
    fn install_source_dir_missing_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let meta = make_meta("/nonexistent/component/dir");
        let dirs = make_dirs(&tmp);
        let platform = current_platform();

        let err =
            run_install(&meta, &make_component_id("core/brew"), &dirs, &platform).unwrap_err();
        assert!(matches!(err, ComponentHostError::SourceDirNotFound { .. }));
    }

    // --- run_uninstall tests ------------------------------------------------

    #[test]
    fn uninstall_success_exits_zero() {
        let tmp = tempfile::tempdir().unwrap();
        write_ok_script(tmp.path(), "uninstall.sh", "exit 0");
        let meta = make_meta(tmp.path().to_str().unwrap());
        let dirs = make_dirs(&tmp);
        let platform = current_platform();

        let result = run_uninstall(&meta, &make_component_id("core/brew"), &dirs, &platform);
        assert!(result.is_ok());
    }

    #[test]
    fn uninstall_script_missing_returns_script_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        // No uninstall.sh written (only install.sh)
        write_ok_script(tmp.path(), "install.sh", "exit 0");
        let meta = make_meta(tmp.path().to_str().unwrap());
        let dirs = make_dirs(&tmp);
        let platform = current_platform();

        let err =
            run_uninstall(&meta, &make_component_id("core/brew"), &dirs, &platform).unwrap_err();
        assert!(matches!(err, ComponentHostError::ScriptNotFound { .. }));
    }

    #[test]
    fn uninstall_nonzero_exit_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        write_ok_script(tmp.path(), "uninstall.sh", "exit 3");
        let meta = make_meta(tmp.path().to_str().unwrap());
        let dirs = make_dirs(&tmp);
        let platform = current_platform();

        let err =
            run_uninstall(&meta, &make_component_id("core/brew"), &dirs, &platform).unwrap_err();
        assert!(matches!(
            err,
            ComponentHostError::ScriptFailed { exit_code: 3, .. }
        ));
    }

    // --- Environment variable injection tests -------------------------------

    #[test]
    fn env_vars_are_injected_into_script() {
        let tmp = tempfile::tempdir().unwrap();
        let platform = current_platform();

        // Write platform-appropriate script to print env vars
        let script_body = match platform {
            Platform::Windows => {
                // PowerShell: Write env vars to output
                r#"Write-Output "$env:LOADOUT_COMPONENT_ID"
Write-Output "$env:LOADOUT_CONFIG_HOME"
Write-Output "$env:LOADOUT_DATA_HOME"
Write-Output "$env:LOADOUT_STATE_HOME""#
            }
            Platform::Linux | Platform::Wsl => {
                // Bash: printf with env vars
                r#"printf '%s\n' "$LOADOUT_COMPONENT_ID" "$LOADOUT_CONFIG_HOME" "$LOADOUT_DATA_HOME" "$LOADOUT_STATE_HOME""#
            }
        };

        write_ok_script(tmp.path(), "install.sh", script_body);
        let meta = make_meta(tmp.path().to_str().unwrap());

        // Use platform-appropriate paths
        let (cfg_path, data_path, state_path) = match platform {
            Platform::Windows => (
                PathBuf::from("C:\\tmp\\cfg\\loadout"),
                PathBuf::from("C:\\tmp\\data\\loadout"),
                PathBuf::from("C:\\tmp\\state\\loadout"),
            ),
            Platform::Linux | Platform::Wsl => (
                PathBuf::from("/tmp/cfg/loadout"),
                PathBuf::from("/tmp/data/loadout"),
                PathBuf::from("/tmp/state/loadout"),
            ),
        };

        let dirs = Dirs {
            config_home: cfg_path.clone(),
            data_home: data_path.clone(),
            state_home: state_path.clone(),
            cache_home: tmp.path().join("cache"),
        };

        let out = run_install(&meta, &make_component_id("core/git"), &dirs, &platform).unwrap();
        assert!(
            out.stdout.contains("core/git"),
            "LOADOUT_COMPONENT_ID missing"
        );
        assert!(
            out.stdout.contains(&cfg_path.to_string_lossy().to_string()),
            "LOADOUT_CONFIG_HOME missing"
        );
        assert!(
            out.stdout
                .contains(&data_path.to_string_lossy().to_string()),
            "LOADOUT_DATA_HOME missing"
        );
        assert!(
            out.stdout
                .contains(&state_path.to_string_lossy().to_string()),
            "LOADOUT_STATE_HOME missing"
        );
    }

    // --- ComponentHostError display -------------------------------------------

    #[test]
    fn error_messages_are_nonempty() {
        let errors: &[ComponentHostError] = &[
            ComponentHostError::ScriptNotFound {
                path: "/tmp/install.sh".to_string(),
            },
            ComponentHostError::SourceDirNotFound {
                path: "/tmp/feat".to_string(),
            },
            ComponentHostError::SpawnFailed {
                reason: "no sh".to_string(),
            },
            ComponentHostError::ScriptFailed {
                exit_code: 1,
                stderr: "boom".to_string(),
            },
        ];
        for e in errors {
            assert!(!e.to_string().is_empty(), "empty message: {e:?}");
        }
    }
}
