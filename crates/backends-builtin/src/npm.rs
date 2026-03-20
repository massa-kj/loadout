//! npm backend — manages global Node.js packages via `npm` (all platforms).
//!
//! Supported resource kinds: `package`.
//! Runtime resources are not supported.
//!
//! Status is determined by `npm list -g --depth=0 <name>`:
//! exit 0 indicates the package is globally installed.

use backend_host::{Backend, BackendError, Resource, ResourceState};

use crate::cmd;

// ---------------------------------------------------------------------------
// NpmBackend
// ---------------------------------------------------------------------------

/// Installs global packages via npm (`npm install -g / uninstall -g`).
#[derive(Debug)]
pub struct NpmBackend;

impl Backend for NpmBackend {
    fn apply(&self, resource: &Resource) -> Result<(), BackendError> {
        let name = cmd::package_name(resource)?;
        if is_installed(name)? {
            return Ok(());
        }
        cmd::require_ok(cmd::run("npm", &["install", "-g", name])?)
    }

    fn remove(&self, resource: &Resource) -> Result<(), BackendError> {
        let name = cmd::package_name(resource)?;
        if !is_installed(name)? {
            return Ok(());
        }
        cmd::require_ok(cmd::run("npm", &["uninstall", "-g", name])?)
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

/// Return `true` if the package is present in the global npm package tree.
fn is_installed(name: &str) -> Result<bool, BackendError> {
    // `npm list -g --depth=0 <name>` exits 0 if installed.
    cmd::check("npm", &["list", "-g", "--depth=0", name])
}
