//! Homebrew backend — manages packages via `brew` (Linux / macOS / WSL).
//!
//! Supported resource kinds: `package`.
//! Runtime resources are not supported (use `MiseBackend`).
//!
//! The backend queries brew before installing/removing to stay idempotent
//! and avoid noisy brew output on repeated runs.

use backend_host::{Backend, BackendError, Resource, ResourceState};

use crate::cmd;

// ---------------------------------------------------------------------------
// BrewBackend
// ---------------------------------------------------------------------------

/// Installs packages via Homebrew (`brew install / uninstall`).
#[derive(Debug)]
pub struct BrewBackend;

impl Backend for BrewBackend {
    fn apply(&self, resource: &Resource) -> Result<(), BackendError> {
        let name = cmd::package_name(resource)?;
        if is_installed(name)? {
            return Ok(());
        }
        cmd::require_ok(run_brew(&["install", name])?)
    }

    fn remove(&self, resource: &Resource) -> Result<(), BackendError> {
        let name = cmd::package_name(resource)?;
        if !is_installed(name)? {
            return Ok(());
        }
        cmd::require_ok(run_brew(&["uninstall", name])?)
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

/// Run `brew` with the given arguments, trying PATH first, then the standard install location.
fn run_brew(args: &[&str]) -> Result<cmd::Output, BackendError> {
    // Try PATH first
    match cmd::run("brew", args) {
        Ok(output) => Ok(output),
        Err(BackendError::SpawnFailed { .. }) => {
            // If brew is not in PATH, try the standard Linuxbrew location
            cmd::run("/home/linuxbrew/.linuxbrew/bin/brew", args)
        }
        Err(e) => Err(e),
    }
}

/// Check if `brew` command is available, trying PATH first, then the standard location.
fn check_brew(args: &[&str]) -> Result<bool, BackendError> {
    match cmd::check("brew", args) {
        Ok(result) => Ok(result),
        Err(BackendError::SpawnFailed { .. }) => {
            cmd::check("/home/linuxbrew/.linuxbrew/bin/brew", args)
        }
        Err(e) => Err(e),
    }
}

/// Return `true` if `name` is installed as a Homebrew formula.
fn is_installed(name: &str) -> Result<bool, BackendError> {
    // `brew list --formula <name>` exits 0 if installed, 1 if not.
    check_brew(&["list", "--formula", name])
}
