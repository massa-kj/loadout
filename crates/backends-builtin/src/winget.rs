//! winget backend — manages packages via Windows Package Manager (`winget`).
//!
//! Supported resource kinds: `package`.
//! Runtime resources are not supported (use `MiseBackend`).
//!
//! Package `name` in feature.yaml should be the winget package ID
//! (e.g. `Microsoft.VCRedist.2015+.x64`). winget matching is case-insensitive.

use backend_host::{Backend, BackendError, Resource, ResourceState};

use crate::cmd;

// ---------------------------------------------------------------------------
// WingetBackend
// ---------------------------------------------------------------------------

/// Installs packages via winget (`winget install / uninstall`).
#[derive(Debug)]
pub struct WingetBackend;

impl Backend for WingetBackend {
    fn apply(&self, resource: &Resource) -> Result<(), BackendError> {
        let name = cmd::package_name(resource)?;
        if is_installed(name)? {
            return Ok(());
        }
        cmd::require_ok(cmd::run(
            "winget",
            &[
                "install",
                "--exact",
                "--id",
                name,
                "--accept-package-agreements",
                "--accept-source-agreements",
                "--silent",
            ],
        )?)
    }

    fn remove(&self, resource: &Resource) -> Result<(), BackendError> {
        let name = cmd::package_name(resource)?;
        if !is_installed(name)? {
            return Ok(());
        }
        cmd::require_ok(cmd::run(
            "winget",
            &["uninstall", "--exact", "--id", name, "--silent"],
        )?)
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

/// Return `true` if the package `id` is listed in `winget list`.
fn is_installed(id: &str) -> Result<bool, BackendError> {
    // `winget list --exact --id <id>` exits 0 when found.
    cmd::check("winget", &["list", "--exact", "--id", id, "--accept-source-agreements"])
}
