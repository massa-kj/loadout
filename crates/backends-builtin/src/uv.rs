//! uv backend — manages Python tools via `uv tool` (all platforms).
//!
//! Supported resource kinds: `package`.
//! Runtime resources are not supported (use mise for Python version management).
//!
//! Status is determined by scanning `uv tool list` output for the package name.

use backend_host::{Backend, BackendError, Resource, ResourceState};

use crate::cmd;

// ---------------------------------------------------------------------------
// UvBackend
// ---------------------------------------------------------------------------

/// Installs Python tools globally via uv (`uv tool install / uninstall`).
#[derive(Debug)]
pub struct UvBackend;

impl Backend for UvBackend {
    fn apply(&self, resource: &Resource) -> Result<(), BackendError> {
        let name = cmd::package_name(resource)?;
        if is_installed(name)? {
            return Ok(());
        }
        cmd::require_ok(cmd::run("uv", &["tool", "install", name])?)
    }

    fn remove(&self, resource: &Resource) -> Result<(), BackendError> {
        let name = cmd::package_name(resource)?;
        if !is_installed(name)? {
            return Ok(());
        }
        cmd::require_ok(cmd::run("uv", &["tool", "uninstall", name])?)
    }

    fn status(&self, resource: &Resource) -> Result<ResourceState, BackendError> {
        let name = cmd::package_name(resource)?;
        if is_installed(name)? {
            Ok(ResourceState::Installed)
        } else {
            Ok(ResourceState::NotInstalled)
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return `true` if a tool named `name` appears in `uv tool list`.
///
/// `uv tool list` prints one line per installed tool: `<name> v<version>`.
fn is_installed(name: &str) -> Result<bool, BackendError> {
    let out = cmd::run("uv", &["tool", "list"])?;
    if !out.success {
        return Ok(false);
    }
    // Check if any line starts with the tool name (case-insensitive to handle aliases).
    Ok(out.stdout.lines().any(|line| {
        let tok = line.split_whitespace().next().unwrap_or("");
        tok.eq_ignore_ascii_case(name)
    }))
}
