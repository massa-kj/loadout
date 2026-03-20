//! APT backend — manages packages via `apt-get` (Debian / Ubuntu Linux).
//!
//! Supported resource kinds: `package`.
//! Runtime resources are not supported (use `MiseBackend`).
//!
//! Status is determined by `dpkg-query -W --showformat '${Status}' <name>`:
//! exit 0 + output containing `install ok installed` → [`ResourceState::Installed`].

use backend_host::{Backend, BackendError, Resource, ResourceState};

use crate::cmd;

// ---------------------------------------------------------------------------
// AptBackend
// ---------------------------------------------------------------------------

/// Installs packages via APT (`apt-get install / remove`).
#[derive(Debug)]
pub struct AptBackend;

impl Backend for AptBackend {
    fn apply(&self, resource: &Resource) -> Result<(), BackendError> {
        let name = cmd::package_name(resource)?;
        if is_installed(name)? {
            return Ok(());
        }
        cmd::require_ok(cmd::run(
            "apt-get",
            &["install", "-y", "--no-install-recommends", name],
        )?)
    }

    fn remove(&self, resource: &Resource) -> Result<(), BackendError> {
        let name = cmd::package_name(resource)?;
        if !is_installed(name)? {
            return Ok(());
        }
        cmd::require_ok(cmd::run("apt-get", &["remove", "-y", name])?)
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

/// Return `true` if `name` is installed and fully configured via dpkg.
fn is_installed(name: &str) -> Result<bool, BackendError> {
    let out =
        cmd::run("dpkg-query", &["-W", "--showformat=${Status}", name])?;
    // dpkg status for a properly installed package: "install ok installed"
    Ok(out.success && out.stdout.contains("install ok installed"))
}
