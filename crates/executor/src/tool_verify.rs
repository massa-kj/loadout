//! Tool resource verify subsystem.
//!
//! Provides two public entry points:
//! - [`verify_tool`]: run after a managed_script install to confirm the declared tool is present
//!   and, if a version constraint was declared, to confirm it is satisfied.
//! - [`check_absence`]: run after a managed_script uninstall to confirm the previously recorded
//!   path no longer exists.
//!
//! Platform dispatch for `resolved_command`:
//! - POSIX: PATH search with executable-bit check. Shell aliases and functions are excluded.
//! - Windows: PATHEXT-aware command resolution. Shell aliases and functions are excluded.
//!
//! See design doc: `tmp/202604xx_script_feature.md` (verify type table, absence check rules)

use std::path::Path;

use model::tool::{
    ToolIdentityVerify, ToolObservedFacts, ToolVerifyContract, ToolVersionVerify,
};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during tool resource verification.
#[derive(Debug, thiserror::Error)]
pub enum ToolVerifyError {
    /// The tool was not found at any expected location (install verify / identity check).
    #[error("tool '{resource_id}' identity check failed: {reason}")]
    IdentityNotFound { resource_id: String, reason: String },

    /// The resolved path exists but is not in the expected set declared in `one_of`.
    #[error(
        "tool '{resource_id}' resolved to '{resolved}' which is not in expected set: {candidates:?}"
    )]
    PathNotInExpectedSet {
        resource_id: String,
        resolved: String,
        candidates: Vec<String>,
    },

    /// The version constraint declared in the verify contract is not satisfied.
    #[error(
        "tool '{resource_id}' version constraint '{constraint}' not met: found '{found}'"
    )]
    VersionConstraintNotMet {
        resource_id: String,
        found: String,
        constraint: String,
    },

    /// Version output could not be parsed using the declared regex.
    #[error("tool '{resource_id}' version parse failed: regex='{regex}', output='{output}'")]
    VersionParseFailed {
        resource_id: String,
        output: String,
        regex: String,
    },

    /// The path recorded in `observed.resolved_path` still exists after uninstall.
    #[error("tool '{resource_id}' absence check failed: '{path}' still exists")]
    AbsenceCheckFailed { resource_id: String, path: String },

    /// An I/O error occurred while performing the verification.
    #[error("tool '{resource_id}' I/O error: {reason}")]
    IoError { resource_id: String, reason: String },
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Verify a tool resource after a managed_script install.
///
/// Steps:
/// 1. Confirm identity (via the appropriate [`ToolIdentityVerify`] variant).
/// 2. If version verify is declared, run the command, parse stdout, and evaluate the constraint.
/// 3. Return observed facts (`resolved_path`, `version`) on success.
///
/// On failure, returns a [`ToolVerifyError`] describing the problem. The caller
/// must treat any error as an operation failure and must not commit state.
pub fn verify_tool(
    resource_id: &str,
    verify: &ToolVerifyContract,
) -> Result<ToolObservedFacts, ToolVerifyError> {
    // Step 1: identity check.
    let resolved_path = verify_identity(resource_id, &verify.identity)?;

    // Step 2: version check (optional).
    let version = if let Some(version_verify) = &verify.version {
        Some(verify_version(resource_id, version_verify)?)
    } else {
        None
    };

    Ok(ToolObservedFacts {
        resolved_path,
        version,
    })
}

/// Check that a previously recorded tool path no longer exists after an uninstall.
///
/// Only the `observed.resolved_path` is checked — absence of all `one_of` candidates
/// is not required.
///
/// If `resolved_path` is `None` (tool had no path-bearing identity), the check
/// is considered satisfied (nothing to verify).
///
/// Returns [`ToolVerifyError::AbsenceCheckFailed`] if the path still exists.
pub fn check_absence(
    resource_id: &str,
    observed: &ToolObservedFacts,
) -> Result<(), ToolVerifyError> {
    let path = match &observed.resolved_path {
        Some(p) => p,
        None => return Ok(()), // No recorded path; nothing to check.
    };

    if Path::new(path).exists() {
        return Err(ToolVerifyError::AbsenceCheckFailed {
            resource_id: resource_id.to_string(),
            path: path.clone(),
        });
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Identity verification
// ---------------------------------------------------------------------------

/// Verify identity and return the observed resolved path (may be None for types
/// that do not produce a single path, though currently all types do).
fn verify_identity(
    resource_id: &str,
    identity: &ToolIdentityVerify,
) -> Result<Option<String>, ToolVerifyError> {
    match identity {
        ToolIdentityVerify::ResolvedCommand {
            command,
            expected_path,
        } => {
            let resolved = resolve_command(command).ok_or_else(|| ToolVerifyError::IdentityNotFound {
                resource_id: resource_id.to_string(),
                reason: format!("command '{}' not found in PATH", command),
            })?;

            let resolved_str = resolved
                .to_str()
                .ok_or_else(|| ToolVerifyError::IoError {
                    resource_id: resource_id.to_string(),
                    reason: format!("resolved path for '{}' is not valid UTF-8", command),
                })?
                .to_string();

            if !expected_path.one_of.contains(&resolved_str) {
                return Err(ToolVerifyError::PathNotInExpectedSet {
                    resource_id: resource_id.to_string(),
                    resolved: resolved_str,
                    candidates: expected_path.one_of.clone(),
                });
            }

            Ok(Some(resolved_str))
        }

        ToolIdentityVerify::File { path, executable } => {
            let p = Path::new(path);
            if !p.exists() {
                return Err(ToolVerifyError::IdentityNotFound {
                    resource_id: resource_id.to_string(),
                    reason: format!("file '{}' does not exist", path),
                });
            }
            if !p.is_file() {
                return Err(ToolVerifyError::IdentityNotFound {
                    resource_id: resource_id.to_string(),
                    reason: format!("'{}' exists but is not a regular file", path),
                });
            }
            if *executable {
                check_executable(resource_id, p)?;
            }
            Ok(Some(path.clone()))
        }

        ToolIdentityVerify::Directory { path } => {
            let p = Path::new(path);
            if !p.is_dir() {
                return Err(ToolVerifyError::IdentityNotFound {
                    resource_id: resource_id.to_string(),
                    reason: format!("directory '{}' does not exist", path),
                });
            }
            Ok(Some(path.clone()))
        }

        ToolIdentityVerify::SymlinkTarget {
            path,
            expected_target,
        } => {
            let p = Path::new(path);
            let target = std::fs::read_link(p).map_err(|e| ToolVerifyError::IdentityNotFound {
                resource_id: resource_id.to_string(),
                reason: format!("'{}' is not a symlink or cannot be read: {}", path, e),
            })?;
            let target_str = target.to_str().ok_or_else(|| ToolVerifyError::IoError {
                resource_id: resource_id.to_string(),
                reason: format!("symlink target for '{}' is not valid UTF-8", path),
            })?;
            if target_str != expected_target.as_str() {
                return Err(ToolVerifyError::IdentityNotFound {
                    resource_id: resource_id.to_string(),
                    reason: format!(
                        "'{}' points to '{}', expected '{}'",
                        path, target_str, expected_target
                    ),
                });
            }
            Ok(Some(path.clone()))
        }
    }
}

// ---------------------------------------------------------------------------
// Version verification
// ---------------------------------------------------------------------------

/// Run the version command, parse the output, evaluate the constraint, and return
/// the observed version string.
fn verify_version(
    resource_id: &str,
    version_verify: &ToolVersionVerify,
) -> Result<String, ToolVerifyError> {
    // Run the version command and capture stdout.
    let output = std::process::Command::new(&version_verify.command)
        .args(&version_verify.args)
        .output()
        .map_err(|e| ToolVerifyError::IoError {
            resource_id: resource_id.to_string(),
            reason: format!(
                "failed to run version command '{}': {}",
                version_verify.command, e
            ),
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let version_str = parse_version_output(
        resource_id,
        &stdout,
        &version_verify.parse.first_line_regex,
    )?;

    // Evaluate the constraint if one was declared.
    if let Some(constraint_str) = &version_verify.constraint {
        check_version_constraint(resource_id, &version_str, constraint_str)?;
    }

    Ok(version_str)
}

/// Extract a version string from command output using the declared regex.
///
/// Applies the regex to the first non-empty line of stdout.
/// The regex must contain exactly one capture group.
fn parse_version_output(
    resource_id: &str,
    stdout: &str,
    first_line_regex: &str,
) -> Result<String, ToolVerifyError> {
    let re = regex::Regex::new(first_line_regex).map_err(|e| ToolVerifyError::IoError {
        resource_id: resource_id.to_string(),
        reason: format!("invalid version regex '{}': {}", first_line_regex, e),
    })?;

    // Find the first non-empty line and apply the regex.
    let first_line = stdout.lines().find(|l| !l.trim().is_empty()).unwrap_or("");

    let caps = re
        .captures(first_line)
        .ok_or_else(|| ToolVerifyError::VersionParseFailed {
            resource_id: resource_id.to_string(),
            output: first_line.to_string(),
            regex: first_line_regex.to_string(),
        })?;

    // Capture group 1 must exist and yield the version string.
    caps.get(1)
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| ToolVerifyError::VersionParseFailed {
            resource_id: resource_id.to_string(),
            output: first_line.to_string(),
            regex: first_line_regex.to_string(),
        })
}

/// Evaluate a semver constraint against an observed version string.
fn check_version_constraint(
    resource_id: &str,
    version_str: &str,
    constraint_str: &str,
) -> Result<(), ToolVerifyError> {
    let version =
        semver::Version::parse(version_str).map_err(|e| ToolVerifyError::VersionParseFailed {
            resource_id: resource_id.to_string(),
            output: version_str.to_string(),
            regex: format!("(semver parse error: {})", e),
        })?;

    let req = semver::VersionReq::parse(constraint_str).map_err(|e| ToolVerifyError::IoError {
        resource_id: resource_id.to_string(),
        reason: format!(
            "invalid semver constraint '{}': {}",
            constraint_str, e
        ),
    })?;

    if !req.matches(&version) {
        return Err(ToolVerifyError::VersionConstraintNotMet {
            resource_id: resource_id.to_string(),
            found: version_str.to_string(),
            constraint: constraint_str.to_string(),
        });
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Platform-specific command resolution
// ---------------------------------------------------------------------------

/// Resolve `command` to an absolute path using the executor's `PATH`.
///
/// Shell aliases and functions are excluded — only real files are matched.
/// Returns `None` if the command cannot be resolved.
fn resolve_command(command: &str) -> Option<std::path::PathBuf> {
    #[cfg(unix)]
    return resolve_command_posix(command);

    #[cfg(windows)]
    return resolve_command_windows(command);

    // Fallback for platforms that are neither unix nor windows (should not occur in practice).
    #[cfg(not(any(unix, windows)))]
    return resolve_command_fallback(command);
}

/// POSIX command resolution: search PATH directories for an executable file.
///
/// Excludes shell aliases and built-ins by requiring a real file with execute permission.
#[cfg(unix)]
fn resolve_command_posix(command: &str) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    // If command already looks like a path, just verify it directly.
    if command.contains('/') {
        let p = std::path::PathBuf::from(command);
        if p.is_file() {
            let meta = p.metadata().ok()?;
            if meta.permissions().mode() & 0o111 != 0 {
                return Some(p);
            }
        }
        return None;
    }

    let path_var = std::env::var("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(command);
        if candidate.is_file() {
            if let Ok(meta) = candidate.metadata() {
                if meta.permissions().mode() & 0o111 != 0 {
                    // Resolve symlinks to get the canonical path reported to callers.
                    // If canonicalize fails (e.g. dangling), use the un-canonicalized path.
                    let resolved = candidate.canonicalize().unwrap_or(candidate);
                    return Some(resolved);
                }
            }
        }
    }
    None
}

/// Windows command resolution: search PATH directories with PATHEXT extensions.
///
/// Excludes shell aliases and built-ins by requiring a real file.
#[cfg(windows)]
fn resolve_command_windows(command: &str) -> Option<std::path::PathBuf> {
    // If command already looks like a path, verify it directly.
    if command.contains('\\') || command.contains('/') {
        let p = std::path::PathBuf::from(command);
        if p.is_file() {
            return Some(p.canonicalize().unwrap_or(p));
        }
        return None;
    }

    let path_var = std::env::var("PATH").unwrap_or_default();
    let pathext = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    let extensions: Vec<&str> = pathext.split(';').collect();

    for dir in std::env::split_paths(&path_var) {
        // Try the command as-is (may already have extension).
        let direct = dir.join(command);
        if direct.is_file() {
            return Some(direct.canonicalize().unwrap_or(direct));
        }
        // Try appending each PATHEXT extension (case-insensitive on Windows).
        let command_upper = command.to_uppercase();
        // Only append extension if command doesn't already have one.
        let has_ext = extensions.iter().any(|ext| {
            command_upper.ends_with(&ext.to_uppercase())
        });
        if !has_ext {
            for ext in &extensions {
                let candidate = dir.join(format!("{}{}", command, ext));
                if candidate.is_file() {
                    return Some(candidate.canonicalize().unwrap_or(candidate));
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Executable check (POSIX)
// ---------------------------------------------------------------------------

/// Check that a file has at least one execute bit set (POSIX only).
/// On Windows, file presence is taken as sufficient.
fn check_executable(resource_id: &str, path: &Path) -> Result<(), ToolVerifyError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = path.metadata().map_err(|e| ToolVerifyError::IoError {
            resource_id: resource_id.to_string(),
            reason: format!("failed to stat '{}': {}", path.display(), e),
        })?;
        if meta.permissions().mode() & 0o111 == 0 {
            return Err(ToolVerifyError::IdentityNotFound {
                resource_id: resource_id.to_string(),
                reason: format!("'{}' is not executable", path.display()),
            });
        }
    }
    #[cfg(not(unix))]
    {
        // On Windows, any file that resolves via PATHEXT is considered "executable".
        // No extra permission check needed at this layer.
        let _ = (resource_id, path);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use model::tool::{OneOf, ToolIdentityVerify, ToolVerifyContract};

    // -----------------------------------------------------------------------
    // parse_version_output
    // -----------------------------------------------------------------------

    #[test]
    fn parse_version_brew_style() {
        let result = parse_version_output(
            "test:brew",
            "Homebrew 4.3.12\nHomebrew/homebrew-core (git revision abc123; last commit 2025-01-01)\n",
            r"^Homebrew\s+([0-9]+\.[0-9]+\.[0-9]+)",
        );
        assert_eq!(result.unwrap(), "4.3.12");
    }

    #[test]
    fn parse_version_plain_semver() {
        let result = parse_version_output(
            "test:deno",
            "2.1.4\n",
            r"^([0-9]+\.[0-9]+\.[0-9]+)",
        );
        assert_eq!(result.unwrap(), "2.1.4");
    }

    #[test]
    fn parse_version_no_match_returns_error() {
        let result = parse_version_output(
            "test:tool",
            "unexpected output\n",
            r"^Homebrew\s+([0-9]+\.[0-9]+\.[0-9]+)",
        );
        assert!(matches!(result, Err(ToolVerifyError::VersionParseFailed { .. })));
    }

    #[test]
    fn parse_version_skips_leading_empty_lines() {
        let result = parse_version_output(
            "test:tool",
            "\n\ndeno 1.2.3\n",
            r"^deno\s+([0-9]+\.[0-9]+\.[0-9]+)",
        );
        assert_eq!(result.unwrap(), "1.2.3");
    }

    // -----------------------------------------------------------------------
    // check_version_constraint
    // -----------------------------------------------------------------------

    #[test]
    fn version_constraint_satisfied() {
        let result = check_version_constraint("test:tool", "4.3.12", ">=4.0.0");
        assert!(result.is_ok());
    }

    #[test]
    fn version_constraint_not_met() {
        let result = check_version_constraint("test:tool", "3.9.0", ">=4.0.0");
        assert!(matches!(
            result,
            Err(ToolVerifyError::VersionConstraintNotMet { .. })
        ));
    }

    #[test]
    fn version_constraint_range_satisfied() {
        let result = check_version_constraint("test:tool", "2.1.4", ">=2.0.0, <3.0.0");
        assert!(result.is_ok());
    }

    #[test]
    fn version_constraint_range_not_met() {
        let result = check_version_constraint("test:tool", "3.0.0", ">=2.0.0, <3.0.0");
        assert!(matches!(
            result,
            Err(ToolVerifyError::VersionConstraintNotMet { .. })
        ));
    }

    // -----------------------------------------------------------------------
    // check_absence
    // -----------------------------------------------------------------------

    #[test]
    fn absence_check_passes_when_no_path() {
        let observed = ToolObservedFacts {
            resolved_path: None,
            version: None,
        };
        assert!(check_absence("test:tool", &observed).is_ok());
    }

    #[test]
    fn absence_check_passes_when_path_gone() {
        // Use a path that definitely does not exist.
        let observed = ToolObservedFacts {
            resolved_path: Some("/tmp/__loadout_test_absent_binary_xyz__".to_string()),
            version: None,
        };
        assert!(check_absence("test:tool", &observed).is_ok());
    }

    #[test]
    fn absence_check_fails_when_path_exists() {
        // /tmp always exists as a directory on Linux/macOS.
        // Use a known-present file: /usr/bin/env should exist on most Unix systems.
        #[cfg(unix)]
        {
            let observed = ToolObservedFacts {
                resolved_path: Some("/usr/bin/env".to_string()),
                version: None,
            };
            let result = check_absence("test:tool", &observed);
            assert!(matches!(
                result,
                Err(ToolVerifyError::AbsenceCheckFailed { .. })
            ));
        }
    }

    // -----------------------------------------------------------------------
    // verify_identity: File variant
    // -----------------------------------------------------------------------

    #[test]
    fn file_identity_fails_for_nonexistent() {
        let identity = ToolIdentityVerify::File {
            path: "/tmp/__loadout_nonexistent_file__".to_string(),
            executable: false,
        };
        let result = verify_identity("test:tool", &identity);
        assert!(matches!(result, Err(ToolVerifyError::IdentityNotFound { .. })));
    }

    #[test]
    fn file_identity_succeeds_for_existing_file() {
        // Create a temporary file.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path_str = tmp.path().to_str().unwrap().to_string();
        let identity = ToolIdentityVerify::File {
            path: path_str.clone(),
            executable: false,
        };
        let result = verify_identity("test:tool", &identity);
        assert_eq!(result.unwrap(), Some(path_str));
    }

    // -----------------------------------------------------------------------
    // verify_identity: Directory variant
    // -----------------------------------------------------------------------

    #[test]
    fn directory_identity_succeeds_for_existing_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path_str = tmp.path().to_str().unwrap().to_string();
        let identity = ToolIdentityVerify::Directory { path: path_str.clone() };
        let result = verify_identity("test:tool", &identity);
        assert_eq!(result.unwrap(), Some(path_str));
    }

    #[test]
    fn directory_identity_fails_for_nonexistent() {
        let identity = ToolIdentityVerify::Directory {
            path: "/tmp/__loadout_nonexistent_dir__".to_string(),
        };
        let result = verify_identity("test:tool", &identity);
        assert!(matches!(result, Err(ToolVerifyError::IdentityNotFound { .. })));
    }

    // -----------------------------------------------------------------------
    // verify_identity: SymlinkTarget variant
    // -----------------------------------------------------------------------

    #[cfg(unix)]
    #[test]
    fn symlink_target_identity_succeeds() {
        let dir = tempfile::TempDir::new().unwrap();
        let target = dir.path().join("real_binary");
        std::fs::write(&target, b"binary").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let identity = ToolIdentityVerify::SymlinkTarget {
            path: link.to_str().unwrap().to_string(),
            expected_target: target.to_str().unwrap().to_string(),
        };
        let result = verify_identity("test:tool", &identity);
        assert_eq!(result.unwrap(), Some(link.to_str().unwrap().to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_target_identity_fails_wrong_target() {
        let dir = tempfile::TempDir::new().unwrap();
        let target = dir.path().join("real_binary");
        std::fs::write(&target, b"binary").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let identity = ToolIdentityVerify::SymlinkTarget {
            path: link.to_str().unwrap().to_string(),
            expected_target: "/wrong/path".to_string(),
        };
        let result = verify_identity("test:tool", &identity);
        assert!(matches!(result, Err(ToolVerifyError::IdentityNotFound { .. })));
    }

    // -----------------------------------------------------------------------
    // verify_identity: ResolvedCommand variant
    // -----------------------------------------------------------------------

    #[cfg(unix)]
    #[test]
    fn resolved_command_fails_when_not_in_expected_set() {
        // Use a command that definitely exists on CI (env) but point at a wrong expected path.
        let identity = ToolIdentityVerify::ResolvedCommand {
            command: "env".to_string(),
            expected_path: OneOf {
                one_of: vec!["/nonexistent/path/env".to_string()],
            },
        };
        let result = verify_identity("test:tool", &identity);
        // Either IdentityNotFound (if env not in PATH) or PathNotInExpectedSet.
        assert!(matches!(
            result,
            Err(ToolVerifyError::PathNotInExpectedSet { .. }) | Err(ToolVerifyError::IdentityNotFound { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn resolved_command_succeeds_when_path_in_expected_set() {
        // Resolve `env` and whatever we get, put it in the expected set.
        if let Some(resolved) = resolve_command("env") {
            let resolved_str = resolved.to_str().unwrap().to_string();
            let identity = ToolIdentityVerify::ResolvedCommand {
                command: "env".to_string(),
                expected_path: OneOf {
                    one_of: vec![resolved_str.clone()],
                },
            };
            let result = verify_identity("test:tool", &identity);
            assert_eq!(result.unwrap(), Some(resolved_str));
        }
        // If `env` is not found, skip (would be a very unusual CI environment).
    }

    // -----------------------------------------------------------------------
    // verify_tool: full happy path (File identity, no version)
    // -----------------------------------------------------------------------

    #[test]
    fn verify_tool_file_identity_no_version() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path_str = tmp.path().to_str().unwrap().to_string();

        let contract = ToolVerifyContract {
            identity: ToolIdentityVerify::File {
                path: path_str.clone(),
                executable: false,
            },
            version: None,
        };
        let facts = verify_tool("test:tool", &contract).unwrap();
        assert_eq!(facts.resolved_path, Some(path_str));
        assert_eq!(facts.version, None);
    }
}
