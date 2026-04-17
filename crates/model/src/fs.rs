//! Filesystem resource types shared across pipeline stages.
//!
//! `ConcreteFsSource` represents a fully resolved source reference. It is produced
//! by the materialize stage and consumed by compiler, planner, and executor.
//!
//! See: `docs/specs/data/desired_resource_graph.md`

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Origin kind of a filesystem source.
///
/// Determines validation rules and fingerprint eligibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FsSourceKind {
    /// Relative to the component directory (e.g. `files/.gitconfig`).
    ///
    /// Must not escape the component root via `..`.
    ComponentRelative,
    /// Relative to the user's home directory (e.g. `~/.ssh/config`).
    HomeRelative,
    /// An absolute filesystem path.
    Absolute,
}

/// A fully resolved filesystem source reference.
///
/// Produced by the materialize stage (impure) and immutable thereafter.
/// The `resolved` path is always absolute and ready for direct filesystem access.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConcreteFsSource {
    /// Origin kind — determines validation rules and fingerprint eligibility.
    pub kind: FsSourceKind,
    /// Absolute path to the source file or directory.
    ///
    /// This is the concrete path that executor will use for apply operations.
    pub resolved: PathBuf,
}

impl ConcreteFsSource {
    /// Create a component-relative source with a pre-resolved absolute path.
    pub fn component_relative(resolved: PathBuf) -> Self {
        Self {
            kind: FsSourceKind::ComponentRelative,
            resolved,
        }
    }

    /// Create a home-relative source with a pre-resolved absolute path.
    pub fn home_relative(resolved: PathBuf) -> Self {
        Self {
            kind: FsSourceKind::HomeRelative,
            resolved,
        }
    }

    /// Create an absolute source.
    pub fn absolute(resolved: PathBuf) -> Self {
        Self {
            kind: FsSourceKind::Absolute,
            resolved,
        }
    }

    /// Returns `true` if this source is eligible for content fingerprinting.
    ///
    /// Phase 1 only fingerprints `component_relative` sources because they are
    /// managed assets under loadout's control not subject to external mutation.
    pub fn is_fingerprint_eligible(&self) -> bool {
        self.kind == FsSourceKind::ComponentRelative
    }
}

/// Validate that a component-relative source path does not escape the component root.
///
/// Returns `Err` with a human-readable message if the path contains `..` components
/// that would escape the component directory.
pub fn validate_component_relative_source(
    source_rel: &str,
    component_dir: &Path,
) -> Result<PathBuf, String> {
    let resolved = component_dir.join(source_rel);

    // Canonicalize is not used here because the path may not exist yet at validation time.
    // Instead, check that the normalized path starts with the component directory.
    let normalized = normalize_path(&resolved);
    let normalized_root = normalize_path(component_dir);

    if !normalized.starts_with(&normalized_root) {
        return Err(format!(
            "component-relative source '{}' escapes component directory '{}'",
            source_rel,
            component_dir.display()
        ));
    }

    Ok(resolved)
}

/// Simple path normalization that resolves `.` and `..` without touching the filesystem.
///
/// This is a logical normalization only; it does not follow symlinks.
/// Exposed publicly so that the executor can perform the same normalization
/// for its defensive boundary check on already-resolved absolute paths.
pub fn normalize_path_pub(path: &Path) -> PathBuf {
    normalize_path(path)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                // Only pop if we have a normal component to pop.
                if matches!(components.last(), Some(std::path::Component::Normal(_))) {
                    components.pop();
                } else {
                    components.push(component);
                }
            }
            std::path::Component::CurDir => {
                // Skip `.`
            }
            _ => {
                components.push(component);
            }
        }
    }
    components.iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_relative_valid() {
        let dir = Path::new("/home/user/loadout/components/git");
        let result = validate_component_relative_source("files/.gitconfig", dir);
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            PathBuf::from("/home/user/loadout/components/git/files/.gitconfig")
        );
    }

    #[test]
    fn component_relative_subdir_valid() {
        let dir = Path::new("/home/user/loadout/components/git");
        let result = validate_component_relative_source("files/config/init", dir);
        assert!(result.is_ok());
    }

    #[test]
    fn component_relative_escape_rejected() {
        let dir = Path::new("/home/user/loadout/components/git");
        let result = validate_component_relative_source("../other/secret", dir);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("escapes component directory"));
    }

    #[test]
    fn component_relative_deeply_nested_escape_rejected() {
        let dir = Path::new("/home/user/loadout/components/git");
        let result = validate_component_relative_source("files/../../other/secret", dir);
        assert!(result.is_err());
    }

    #[test]
    fn component_relative_dot_within_component() {
        let dir = Path::new("/home/user/loadout/components/git");
        let result = validate_component_relative_source("files/./config", dir);
        assert!(result.is_ok());
    }

    #[test]
    fn normalize_removes_dotdot() {
        let p = Path::new("/a/b/../c");
        assert_eq!(normalize_path(p), PathBuf::from("/a/c"));
    }

    #[test]
    fn normalize_removes_dot() {
        let p = Path::new("/a/./b/c");
        assert_eq!(normalize_path(p), PathBuf::from("/a/b/c"));
    }

    #[test]
    fn concrete_fs_source_fingerprint_eligibility() {
        let comp = ConcreteFsSource::component_relative(PathBuf::from("/a/b"));
        assert!(comp.is_fingerprint_eligible());

        let home = ConcreteFsSource::home_relative(PathBuf::from("/home/user/.config"));
        assert!(!home.is_fingerprint_eligible());

        let abs = ConcreteFsSource::absolute(PathBuf::from("/etc/config"));
        assert!(!abs.is_fingerprint_eligible());
    }
}
