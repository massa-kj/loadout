// Scaffold use cases: create a new local component or backend directory from a template.
//
// Files are always created with `create_new` (fails if path exists) so that
// accidental overwrites cannot happen.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::context::{AppContext, AppError};

// ── Template / platform choices ──────────────────────────────────────────────

/// Content template for `component new`.
#[derive(Debug, Clone, Copy)]
pub enum ComponentTemplate {
    /// Declarative component: `component.yaml` with a `resources:` skeleton.
    Declarative,
    /// Script component: `component.yaml` + stub `install.sh` / `uninstall.sh`.
    Script,
}

/// Target platform for `backend new`.
#[derive(Debug, Clone, Copy)]
pub enum BackendPlatform {
    /// Generate `.sh` scripts only (Linux / macOS / WSL).
    Unix,
    /// Generate both `.sh` and `.ps1` scripts.
    UnixWindows,
}

// ── Static templates ─────────────────────────────────────────────────────────

const COMPONENT_YAML_DECLARATIVE: &str = "\
spec_version: 1
# mode: declarative  (default; uncomment to make explicit once resources are declared)
description: TODO

resources:
  # - kind: package
  #   id: package:example
  #   name: example
  #
  # - kind: runtime
  #   id: runtime:example
  #   name: example
  #   version: \"1.0.0\"
  #
  # - kind: fs
  #   id: fs:example
  #   source: files/example
  #   path: ~/.example
  #   entry_type: file
  #   op: link
";

const COMPONENT_YAML_SCRIPT: &str = "\
spec_version: 1
mode: script
description: TODO

# Uncomment to declare dependencies:
# depends:
#   - other-component
";

const INSTALL_SH: &str = "\
#!/usr/bin/env bash
set -euo pipefail
# Available environment variables:
#   LOADOUT_COMPONENT_ID      — canonical component ID (e.g. local/mycomponent)
#   LOADOUT_CONFIG_HOME     — loadout config directory
#   LOADOUT_DATA_HOME       — loadout data directory
#   LOADOUT_STATE_HOME      — loadout state directory
#
# Working directory: this component's source directory.

echo \"Installing ${LOADOUT_COMPONENT_ID}...\"

# TODO: implement install logic
";

const UNINSTALL_SH: &str = "\
#!/usr/bin/env bash
set -euo pipefail

echo \"Uninstalling ${LOADOUT_COMPONENT_ID}...\"

# TODO: implement uninstall logic
";

const BACKEND_YAML: &str = "\
api_version: 1
";

const APPLY_SH: &str = "\
#!/usr/bin/env bash
set -euo pipefail
# Available environment variables:
#   LOADOUT_RESOURCE_ID       — stable resource identifier
#   LOADOUT_RESOURCE_KIND     — package | runtime | fs
#   LOADOUT_RESOURCE_NAME     — package / runtime name
#   LOADOUT_RESOURCE_VERSION  — pinned version (runtime only; may be empty)
#
# Resource data is also available as JSON on stdin for complex parsing.

echo \"Applying ${LOADOUT_RESOURCE_ID}...\"

# TODO: implement apply (install/upgrade) logic
";

const REMOVE_SH: &str = "\
#!/usr/bin/env bash
set -euo pipefail

echo \"Removing ${LOADOUT_RESOURCE_ID}...\"

# TODO: implement remove (uninstall) logic
";

const STATUS_SH: &str = "\
#!/usr/bin/env bash
set -euo pipefail
# Must print exactly one of: installed | not_installed | unknown

# TODO: implement status check
echo \"unknown\"
";

const APPLY_PS1: &str = "\
# Available environment variables:
#   $env:LOADOUT_RESOURCE_ID      — stable resource identifier
#   $env:LOADOUT_RESOURCE_KIND    — package | runtime | fs
#   $env:LOADOUT_RESOURCE_NAME    — package / runtime name
#   $env:LOADOUT_RESOURCE_VERSION — pinned version (runtime only; may be empty)

Write-Host \"Applying $env:LOADOUT_RESOURCE_ID...\"

# TODO: implement apply (install/upgrade) logic
";

const REMOVE_PS1: &str = "\
Write-Host \"Removing $env:LOADOUT_RESOURCE_ID...\"

# TODO: implement remove (uninstall) logic
";

const STATUS_PS1: &str = "\
# Must print exactly one of: installed | not_installed | unknown

# TODO: implement status check
Write-Output \"unknown\"
";

// ── component new ──────────────────────────────────────────────────────────────

/// Scaffold a new local component directory under `{local_root}/components/<name>/`.
///
/// Creates `component.yaml` from the chosen template. For `Script` mode, also
/// creates stub `install.sh` / `uninstall.sh` and makes them executable on Unix.
///
/// Returns the path of the created directory. Fails with [`AppError::AlreadyExists`]
/// if the directory already exists.
pub fn component_new(
    ctx: &AppContext,
    name: &str,
    template: ComponentTemplate,
) -> Result<PathBuf, AppError> {
    let dir = ctx.local_root.join("components").join(name);
    if dir.exists() {
        return Err(AppError::AlreadyExists { path: dir });
    }
    std::fs::create_dir_all(&dir).map_err(|e| AppError::ScaffoldIo {
        path: dir.clone(),
        source: e,
    })?;

    let component_yaml = match template {
        ComponentTemplate::Declarative => COMPONENT_YAML_DECLARATIVE,
        ComponentTemplate::Script => COMPONENT_YAML_SCRIPT,
    };
    write_new_file(&dir.join("component.yaml"), component_yaml)?;

    if matches!(template, ComponentTemplate::Script) {
        let install = dir.join("install.sh");
        let uninstall = dir.join("uninstall.sh");
        write_new_file(&install, INSTALL_SH)?;
        write_new_file(&uninstall, UNINSTALL_SH)?;
        #[cfg(unix)]
        {
            make_executable(&install)?;
            make_executable(&uninstall)?;
        }
    }

    Ok(dir)
}

// ── backend new ──────────────────────────────────────────────────────────────

/// Scaffold a new local backend directory under `{local_root}/backends/<name>/`.
///
/// Always creates `backend.yaml` and `.sh` scripts. When `platform` is
/// [`BackendPlatform::UnixWindows`], `.ps1` scripts are also created.
/// On Unix, all `.sh` scripts are made executable.
///
/// Returns the path of the created directory. Fails with [`AppError::AlreadyExists`]
/// if the directory already exists.
pub fn backend_new(
    ctx: &AppContext,
    name: &str,
    platform: BackendPlatform,
) -> Result<PathBuf, AppError> {
    let dir = ctx.local_root.join("backends").join(name);
    if dir.exists() {
        return Err(AppError::AlreadyExists { path: dir });
    }
    std::fs::create_dir_all(&dir).map_err(|e| AppError::ScaffoldIo {
        path: dir.clone(),
        source: e,
    })?;

    write_new_file(&dir.join("backend.yaml"), BACKEND_YAML)?;

    // Unix scripts (always created for Unix; also created for UnixWindows).
    let sh_scripts: &[(&str, &str)] = &[
        ("apply.sh", APPLY_SH),
        ("remove.sh", REMOVE_SH),
        ("status.sh", STATUS_SH),
    ];
    for (filename, content) in sh_scripts {
        let path = dir.join(filename);
        write_new_file(&path, content)?;
        #[cfg(unix)]
        make_executable(&path)?;
    }

    // Windows scripts (only for UnixWindows).
    if matches!(platform, BackendPlatform::UnixWindows) {
        let ps1_scripts: &[(&str, &str)] = &[
            ("apply.ps1", APPLY_PS1),
            ("remove.ps1", REMOVE_PS1),
            ("status.ps1", STATUS_PS1),
        ];
        for (filename, content) in ps1_scripts {
            write_new_file(&dir.join(filename), content)?;
        }
    }

    Ok(dir)
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// Write `content` to `path`, failing if the file already exists.
fn write_new_file(path: &Path, content: &str) -> Result<(), AppError> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| AppError::ScaffoldIo {
            path: path.to_path_buf(),
            source: e,
        })?;
    f.write_all(content.as_bytes())
        .map_err(|e| AppError::ScaffoldIo {
            path: path.to_path_buf(),
            source: e,
        })?;
    Ok(())
}

/// Add the executable bit to a file (Unix only).
#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), AppError> {
    use std::os::unix::fs::PermissionsExt as _;
    let meta = std::fs::metadata(path).map_err(|e| AppError::ScaffoldIo {
        path: path.to_path_buf(),
        source: e,
    })?;
    let mut perms = meta.permissions();
    let mode = perms.mode();
    perms.set_mode(mode | 0o111);
    std::fs::set_permissions(path, perms).map_err(|e| AppError::ScaffoldIo {
        path: path.to_path_buf(),
        source: e,
    })?;
    Ok(())
}
