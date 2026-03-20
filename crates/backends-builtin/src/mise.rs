//! Mise backend — manages language runtimes via `mise` (all platforms).
//!
//! Supported resource kinds: `runtime`.
//! Package resources are not supported (use brew / apt / scoop backends).
//!
//! Versions are specified as `name@version` (e.g. `node@22.17.1`).
//! If no version is given, `mise install <name>` installs based on the nearest
//! `.tool-versions` / `.mise.toml` file, falling back to the latest available.

use backend_host::{Backend, BackendError, Resource, ResourceState};

use crate::cmd;

// ---------------------------------------------------------------------------
// MiseBackend
// ---------------------------------------------------------------------------

/// Installs runtime versions via mise (`mise install / uninstall`).
#[derive(Debug)]
pub struct MiseBackend;

impl Backend for MiseBackend {
    fn apply(&self, resource: &Resource) -> Result<(), BackendError> {
        let spec = cmd::runtime_spec(resource)?;
        if is_installed(resource)? {
            return Ok(());
        }
        cmd::require_ok(cmd::run("mise", &["install", &spec])?)
    }

    fn remove(&self, resource: &Resource) -> Result<(), BackendError> {
        let spec = cmd::runtime_spec(resource)?;
        if !is_installed(resource)? {
            return Ok(());
        }
        cmd::require_ok(cmd::run("mise", &["uninstall", &spec])?)
    }

    fn status(&self, resource: &Resource) -> Result<ResourceState, BackendError> {
        if is_installed(resource)? {
            Ok(ResourceState::Installed)
        } else {
            Ok(ResourceState::NotInstalled)
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return `true` if the runtime (and version) is installed under mise.
///
/// `mise ls <name>` lists installed versions for a tool.
/// We grep the output for the specific version string; an empty version
/// means any installed version counts.
fn is_installed(resource: &Resource) -> Result<bool, BackendError> {
    let (name, version) = cmd::runtime_name_version(resource)?;

    let out = cmd::run("mise", &["ls", name])?;
    if !out.success {
        return Ok(false);
    }

    if version.is_empty() {
        // No version specified: treat as installed if mise knows about it with any version.
        Ok(!out.stdout.trim().is_empty())
    } else {
        Ok(out.stdout.contains(version))
    }
}
