//! Shared command execution helpers for builtin backends.
//!
//! Each builtin backend delegates to an external package manager CLI.
//! This module provides a uniform way to run those commands and convert
//! their exit status / stderr into [`BackendError`] values.

use std::process::{Command, Stdio};

use backend_host::BackendError;
use model::desired_resource_graph::DesiredResourceKind;

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

/// Raw output from a spawned command.
pub struct Output {
    pub exit_code: i32,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

// ---------------------------------------------------------------------------
// Runners
// ---------------------------------------------------------------------------

/// Run a command, capture stdout/stderr, return an [`Output`].
///
/// Returns [`BackendError::SpawnFailed`] if the process cannot be started.
pub fn run(program: &str, args: &[&str]) -> Result<Output, BackendError> {
    let output = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| BackendError::SpawnFailed {
            reason: format!("{program}: {e}"),
        })?;

    Ok(Output {
        exit_code: output.status.code().unwrap_or(-1),
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    })
}

/// Run a command that is expected to succeed.
///
/// Converts a non-zero exit into [`BackendError::ScriptFailed`].
pub fn require_ok(out: Output) -> Result<(), BackendError> {
    if out.success {
        Ok(())
    } else {
        Err(BackendError::ScriptFailed {
            exit_code: out.exit_code,
            stderr: out.stderr,
        })
    }
}

/// Run `program args` only to check its exit code (stdout/stderr discarded).
pub fn check(program: &str, args: &[&str]) -> Result<bool, BackendError> {
    let status = Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| BackendError::SpawnFailed {
            reason: format!("{program}: {e}"),
        })?;
    Ok(status.success())
}

// ---------------------------------------------------------------------------
// Resource helpers
// ---------------------------------------------------------------------------

/// Human-readable label for a resource kind (used in `NotSupported` errors).
pub fn kind_label(k: &DesiredResourceKind) -> &'static str {
    match k {
        DesiredResourceKind::Package { .. } => "package",
        DesiredResourceKind::Runtime { .. } => "runtime",
        DesiredResourceKind::Fs { .. } => "fs",
    }
}

/// Extract the package name from a resource, or return `NotSupported`.
pub fn package_name(resource: &backend_host::Resource) -> Result<&str, BackendError> {
    match &resource.kind {
        DesiredResourceKind::Package { name, .. } => Ok(name.as_str()),
        k => Err(BackendError::NotSupported {
            kind: kind_label(k).to_string(),
        }),
    }
}

/// Extract runtime `(name, version_spec)` from a resource, or return `NotSupported`.
///
/// Returns a `name@version` string suitable for passing to mise.
pub fn runtime_spec(resource: &backend_host::Resource) -> Result<String, BackendError> {
    match &resource.kind {
        DesiredResourceKind::Runtime { name, version, .. } => {
            if version.is_empty() {
                Ok(name.clone())
            } else {
                Ok(format!("{name}@{version}"))
            }
        }
        k => Err(BackendError::NotSupported {
            kind: kind_label(k).to_string(),
        }),
    }
}

/// Extract runtime `(name, version)` pair from a resource.
pub fn runtime_name_version(
    resource: &backend_host::Resource,
) -> Result<(&str, &str), BackendError> {
    match &resource.kind {
        DesiredResourceKind::Runtime { name, version, .. } => {
            Ok((name.as_str(), version.as_str()))
        }
        k => Err(BackendError::NotSupported {
            kind: kind_label(k).to_string(),
        }),
    }
}
