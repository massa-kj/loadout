//! Tool resource types shared between DesiredResourceGraph and State.
//!
//! A `tool` resource represents an external tool that is introduced via an install script
//! and is not managed by any backend. Core is responsible for verification and state updates;
//! the script is responsible only for installation and removal.
//!
//! See: `docs/specs/data/state.md` (tool resource section),
//!      `docs/specs/data/desired_resource_graph.md` (tool resource section)

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Verify contract (shared between Desired and State)
// ---------------------------------------------------------------------------

/// The complete verification contract declared for a tool resource.
///
/// `identity` is always required. `version` is optional and, if present,
/// is used by the planner for compatibility checks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolVerifyContract {
    /// Required. Describes how to confirm the tool is present at the expected location.
    ///
    /// Must use one of the identity-bearing verify types (`resolved_command`, `file`,
    /// `symlink_target`). A `versioned_command`-only verify is not valid.
    pub identity: ToolIdentityVerify,

    /// Optional. If present, the planner includes version constraint in compatibility checks.
    ///
    /// When absent, version differences do not affect classification.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<ToolVersionVerify>,
}

/// Identity verification for a tool resource.
///
/// Each variant confirms that the tool exists at a known, determinate location —
/// not merely that a command of the same name is reachable via PATH.
///
/// # Variants
///
/// - `ResolvedCommand`: the command resolves to one of the expected absolute paths.
/// - `File`: an absolute file path exists (optionally confirmed executable).
/// - `Directory`: an absolute directory path exists.
/// - `SymlinkTarget`: a symlink at `path` points to `expected_target`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolIdentityVerify {
    /// Resolve `command` in the executor's environment and check that the resulting
    /// absolute path is one of the candidates in `expected_path.one_of`.
    ///
    /// Resolution is platform-specific:
    /// - POSIX: PATH search with executable bit check. Shell aliases and functions are excluded.
    /// - Windows: PATHEXT-aware command resolution. Shell aliases and functions are excluded.
    ResolvedCommand {
        /// The command name to resolve (e.g., `"brew"`, `"deno"`).
        command: String,
        /// Set of acceptable absolute paths for the resolved binary.
        expected_path: OneOf,
    },

    /// Confirm that an absolute path exists as a regular file.
    ///
    /// If `executable` is `true`, also confirms the file has execute permission
    /// (on POSIX) or is a recognized executable extension (on Windows).
    File {
        /// Absolute path to the file.
        path: String,
        /// If `true`, also verify the file is executable. Defaults to `false`.
        #[serde(default)]
        executable: bool,
    },

    /// Confirm that an absolute path exists as a directory.
    Directory {
        /// Absolute path to the directory.
        path: String,
    },

    /// Confirm that a symlink at `path` resolves to `expected_target`.
    SymlinkTarget {
        /// Absolute path to the symlink itself.
        path: String,
        /// Absolute path that the symlink must point to.
        expected_target: String,
    },
}

/// A set of candidate paths, any one of which satisfies the identity contract.
///
/// During verification the executor resolves the actual path and checks it against
/// this set. The first match is recorded in `observed.resolved_path`.
/// Multiple candidates being present simultaneously is not a failure condition.
///
/// During uninstall the absence check is performed only against `observed.resolved_path`,
/// not against all candidates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OneOf {
    /// Candidate absolute paths (at least one required).
    pub one_of: Vec<String>,
}

/// Optional version verification for a tool resource.
///
/// When declared, the planner includes this constraint in compatibility checks.
/// Absence means version differences are ignored during classification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolVersionVerify {
    /// Command to run for version output (e.g., `"brew"`, `"/home/user/.bun/bin/bun"`).
    pub command: String,
    /// Arguments to pass to the command (e.g., `["--version"]`).
    #[serde(default)]
    pub args: Vec<String>,
    /// Rules for extracting the version string from command output.
    pub parse: VersionParseRule,
    /// Optional semver constraint string (e.g., `">=4.0.0"`, `">=2.0.0 <3.0.0"`).
    ///
    /// If `None`, version is observed and recorded but not used for compatibility.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constraint: Option<String>,
}

/// Rule for extracting a semver string from a command's stdout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionParseRule {
    /// Regex applied to the first non-empty line of stdout.
    ///
    /// Must contain exactly one capture group that yields the version string
    /// (e.g., `"^Homebrew\\s+([0-9]+\\.[0-9]+\\.[0-9]+)"`).
    pub first_line_regex: String,
}

// ---------------------------------------------------------------------------
// State-only: observed facts recorded after successful install
// ---------------------------------------------------------------------------

/// Facts observed by the executor during a successful install verify.
///
/// Recorded in state alongside the verify contract. Used for:
/// - Uninstall absence check (check only `resolved_path`, not all `one_of` candidates)
/// - Diagnostics and drift detection
/// - Future migration support
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolObservedFacts {
    /// The absolute path that was observed during install verify.
    ///
    /// For `resolved_command`: the resolved binary path (the matched candidate from `one_of`).
    /// For `file` / `directory` / `symlink_target`: the checked path.
    ///
    /// `None` if the identity type does not produce a single resolvable path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_path: Option<String>,

    /// The version string observed during install verify, if version verify was declared.
    ///
    /// `None` if no `version` verify was declared, or if version could not be parsed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

// ---------------------------------------------------------------------------
// State resource payload
// ---------------------------------------------------------------------------

/// State payload for a recorded tool resource.
///
/// Combines the verify contract snapshot (for drift detection and uninstall)
/// with the observed facts recorded at install time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResource {
    /// Tool name as declared in the component (e.g., `"brew"`, `"deno"`).
    pub name: String,

    /// Snapshot of the verify contract at install time.
    ///
    /// Stored so that future runs can detect contract drift without re-reading
    /// the component source.
    pub verify: ToolVerifyContract,

    /// Facts observed by the executor during install verify.
    pub observed: ToolObservedFacts,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_resolved_command() {
        let json = r#"{
            "identity": {
                "type": "resolved_command",
                "command": "brew",
                "expected_path": {
                    "one_of": [
                        "/home/linuxbrew/.linuxbrew/bin/brew",
                        "/opt/homebrew/bin/brew"
                    ]
                }
            },
            "version": {
                "command": "brew",
                "args": ["--version"],
                "parse": {
                    "first_line_regex": "^Homebrew\\s+([0-9]+\\.[0-9]+\\.[0-9]+)"
                },
                "constraint": ">=4.0.0"
            }
        }"#;
        let contract: ToolVerifyContract = serde_json::from_str(json).unwrap();
        match &contract.identity {
            ToolIdentityVerify::ResolvedCommand {
                command,
                expected_path,
            } => {
                assert_eq!(command, "brew");
                assert_eq!(expected_path.one_of.len(), 2);
                assert_eq!(
                    expected_path.one_of[0],
                    "/home/linuxbrew/.linuxbrew/bin/brew"
                );
            }
            _ => panic!("expected resolved_command"),
        }
        let version = contract.version.as_ref().unwrap();
        assert_eq!(version.constraint.as_deref(), Some(">=4.0.0"));
        // Verify round-trip
        let serialized = serde_json::to_string(&contract).unwrap();
        let deserialized: ToolVerifyContract = serde_json::from_str(&serialized).unwrap();
        assert_eq!(contract, deserialized);
    }

    #[test]
    fn round_trip_file_identity() {
        let json = r#"{
            "identity": {
                "type": "file",
                "path": "/home/user/.bun/bin/bun",
                "executable": true
            }
        }"#;
        let contract: ToolVerifyContract = serde_json::from_str(json).unwrap();
        match &contract.identity {
            ToolIdentityVerify::File { path, executable } => {
                assert_eq!(path, "/home/user/.bun/bin/bun");
                assert!(executable);
            }
            _ => panic!("expected file"),
        }
        assert!(contract.version.is_none());
    }

    #[test]
    fn round_trip_tool_resource_in_state() {
        let json = r#"{
            "name": "brew",
            "verify": {
                "identity": {
                    "type": "resolved_command",
                    "command": "brew",
                    "expected_path": {
                        "one_of": [
                            "/home/linuxbrew/.linuxbrew/bin/brew",
                            "/opt/homebrew/bin/brew"
                        ]
                    }
                }
            },
            "observed": {
                "resolved_path": "/opt/homebrew/bin/brew",
                "version": "4.3.12"
            }
        }"#;
        let tool: ToolResource = serde_json::from_str(json).unwrap();
        assert_eq!(tool.name, "brew");
        assert_eq!(
            tool.observed.resolved_path.as_deref(),
            Some("/opt/homebrew/bin/brew")
        );
        assert_eq!(tool.observed.version.as_deref(), Some("4.3.12"));
        // Round-trip
        let serialized = serde_json::to_string(&tool).unwrap();
        let deserialized: ToolResource = serde_json::from_str(&serialized).unwrap();
        assert_eq!(tool, deserialized);
    }

    #[test]
    fn file_identity_defaults_executable_false() {
        let json = r#"{
            "identity": {
                "type": "file",
                "path": "/usr/local/bin/mytool"
            }
        }"#;
        let contract: ToolVerifyContract = serde_json::from_str(json).unwrap();
        match &contract.identity {
            ToolIdentityVerify::File { executable, .. } => {
                assert!(!executable, "executable should default to false");
            }
            _ => panic!("expected file"),
        }
    }

    #[test]
    fn version_verify_no_constraint() {
        let json = r#"{
            "identity": {
                "type": "resolved_command",
                "command": "deno",
                "expected_path": { "one_of": ["/home/user/.deno/bin/deno"] }
            },
            "version": {
                "command": "deno",
                "args": ["--version"],
                "parse": {
                    "first_line_regex": "^deno\\s+([0-9]+\\.[0-9]+\\.[0-9]+)"
                }
            }
        }"#;
        let contract: ToolVerifyContract = serde_json::from_str(json).unwrap();
        let version = contract.version.as_ref().unwrap();
        assert!(version.constraint.is_none());
    }

    #[test]
    fn observed_facts_optional_fields() {
        // Both fields absent — valid for identity types without a single resolvable path
        let json = r#"{"resolved_path": null, "version": null}"#;
        // serde should handle null by treating as None
        let facts: ToolObservedFacts =
            serde_json::from_str(r#"{"resolved_path": null, "version": null}"#).unwrap();
        assert!(facts.resolved_path.is_none());
        assert!(facts.version.is_none());

        // skip_serializing_if: absent fields should not appear in output
        let empty = ToolObservedFacts {
            resolved_path: None,
            version: None,
        };
        let serialized = serde_json::to_string(&empty).unwrap();
        assert_eq!(serialized, "{}");

        let _ = json; // suppress unused warning
    }
}
