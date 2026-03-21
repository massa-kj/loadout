//! Scoop backend — manages packages via `scoop` (Windows).
//!
//! Supported resource kinds: `package`.
//! Runtime resources are not supported (use `MiseBackend`).
//!
//! Status is determined by `scoop list <name>`:
//! exit 0 with non-empty output indicates the package is installed.

use backend_host::{Backend, BackendError, Resource, ResourceState};

use crate::cmd;

// ---------------------------------------------------------------------------
// ScoopBackend
// ---------------------------------------------------------------------------

/// Installs packages via Scoop (`scoop install / uninstall`).
#[derive(Debug)]
pub struct ScoopBackend;

impl Backend for ScoopBackend {
    fn apply(&self, resource: &Resource) -> Result<(), BackendError> {
        let name = cmd::package_name(resource)?;
        if is_installed(name)? {
            return Ok(());
        }
        cmd::require_ok(cmd::run("scoop", &["install", name])?)
    }

    fn remove(&self, resource: &Resource) -> Result<(), BackendError> {
        let name = cmd::package_name(resource)?;
        if !is_installed(name)? {
            return Ok(());
        }
        cmd::require_ok(cmd::run("scoop", &["uninstall", name])?)
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

/// Return `true` if `name` is listed in `scoop list`.
fn is_installed(name: &str) -> Result<bool, BackendError> {
    let out = cmd::run("scoop", &["list", name])?;
    // `scoop list <name>` exits 0 whether or not the package is installed,
    // but returns empty / header-only output when not found.
    // We look for the name in stdout to determine actual presence.
    Ok(out.success
        && out
            .stdout
            .to_ascii_lowercase()
            .contains(&name.to_ascii_lowercase()))
}
