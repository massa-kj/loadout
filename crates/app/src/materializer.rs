//! Materializer — resolves fs.source references before compilation.
//!
//! The materializer is the impure pre-compilation stage that:
//! - Resolves `source` field defaults (`files/<basename(path)>`)
//! - Classifies source kind (`component_relative`, `home_relative`, `absolute`)
//! - Validates component-relative paths do not escape the component directory
//! - Computes content fingerprints for eligible sources (Phase 1C: file, Phase 2: dir)
//!
//! After materialization, all `ConcreteFsSource` values are fully resolved and
//! ready for consumption by the pure compiler stage.
//!
//! See: `docs/specs/data/desired_resource_graph.md`

use model::component_index::{ComponentIndex, FsOp, SpecFsEntryType, SpecResourceKind};
use model::fs::{validate_component_relative_source, ConcreteFsSource, FsSourceKind};
use model::strategy::FingerprintPolicy;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Materialized fs source data keyed by `(component_id, resource_id)`.
///
/// Consumed by the compiler to populate `DesiredResourceGraph.fs.source`.
pub(crate) type MaterializedSources = HashMap<(String, String), MaterializedFsResource>;

/// Materialized data for a single fs resource.
#[derive(Debug)]
pub(crate) struct MaterializedFsResource {
    /// Fully resolved source reference.
    pub source: ConcreteFsSource,
    /// Content fingerprint if eligible and computed (Phase 1C).
    pub source_fingerprint: Option<String>,
    /// Target path with `~` expanded to the user home directory.
    ///
    /// This must be used by the compiler so that `DesiredResourceGraph.fs.path`
    /// is absolute and matches the absolute path recorded in state.
    pub expanded_path: String,
}

/// Errors produced during materialization.
#[derive(Debug, thiserror::Error)]
pub enum MaterializeError {
    #[error("[{component_id}] resource '{resource_id}': {message}")]
    Validation {
        component_id: String,
        resource_id: String,
        message: String,
    },

    #[error(
        "[{component_id}] resource '{resource_id}': cannot determine basename of path '{path}'"
    )]
    NoBasename {
        component_id: String,
        resource_id: String,
        path: String,
    },
}

/// Materialize all fs resource sources in the component index.
///
/// For each fs resource:
/// 1. Resolve source default if omitted (`files/<basename(path)>`)
/// 2. Classify source kind and resolve to absolute path
/// 3. Validate component-relative paths
/// 4. Compute content fingerprint according to `fingerprint_policy`
pub(crate) fn materialize_fs_sources(
    index: &ComponentIndex,
    fingerprint_policy: FingerprintPolicy,
) -> Result<MaterializedSources, MaterializeError> {
    let mut result = MaterializedSources::new();

    for (component_id, meta) in &index.components {
        let Some(spec) = &meta.spec else {
            continue;
        };

        let comp_dir = Path::new(&meta.source_dir);

        for resource in &spec.resources {
            let (source_str, path, entry_type, op) = match &resource.kind {
                SpecResourceKind::Fs {
                    source,
                    path,
                    entry_type,
                    op,
                } => (source, path, entry_type, op),
                _ => continue,
            };

            // Step 1: resolve default source if omitted.
            let raw_source = match source_str {
                Some(s) => s.clone(),
                None => {
                    let target_expanded = expand_home(path);
                    let basename = target_expanded.file_name().ok_or_else(|| {
                        MaterializeError::NoBasename {
                            component_id: component_id.clone(),
                            resource_id: resource.id.clone(),
                            path: path.clone(),
                        }
                    })?;
                    format!("files/{}", basename.to_string_lossy())
                }
            };

            // Step 2: classify source kind and resolve to absolute path.
            let concrete = classify_and_resolve(&raw_source, comp_dir, component_id, &resource.id)?;

            // Step 3: compute fingerprint according to policy.
            let source_fingerprint =
                compute_fingerprint_if_eligible(&concrete, entry_type, op, fingerprint_policy);

            // Step 4: expand `~` in the target path so DesiredResourceGraph holds absolute paths.
            let expanded_path = expand_home(path).to_string_lossy().into_owned();

            result.insert(
                (component_id.clone(), resource.id.clone()),
                MaterializedFsResource {
                    source: concrete,
                    source_fingerprint,
                    expanded_path,
                },
            );
        }
    }

    Ok(result)
}

/// Classify a raw source string and resolve it to a `ConcreteFsSource`.
fn classify_and_resolve(
    raw_source: &str,
    comp_dir: &Path,
    component_id: &str,
    resource_id: &str,
) -> Result<ConcreteFsSource, MaterializeError> {
    // Home-relative: starts with ~/
    if raw_source.starts_with("~/") || raw_source.starts_with("~\\") {
        let resolved = expand_home(raw_source);
        return Ok(ConcreteFsSource::home_relative(resolved));
    }

    // Absolute path (use has_root() to also handle Unix-style paths on Windows,
    // where `/foo` is rooted but not considered absolute by the OS).
    if Path::new(raw_source).has_root() {
        return Ok(ConcreteFsSource::absolute(PathBuf::from(raw_source)));
    }

    // Component-relative (default): validate no escape
    let resolved = validate_component_relative_source(raw_source, comp_dir).map_err(|message| {
        MaterializeError::Validation {
            component_id: component_id.to_string(),
            resource_id: resource_id.to_string(),
            message,
        }
    })?;

    Ok(ConcreteFsSource::component_relative(resolved))
}

/// Compute a content fingerprint for eligible sources.
///
/// Eligibility depends on `fingerprint_policy`:
/// - `AllCopy` — all source kinds when `op = copy`.
/// - `ManagedOnly` — only `component_relative` sources when `op = copy`.
/// - `None` — always returns `None`.
///
/// Returns `None` for non-copy operations or if the source cannot be read.
fn compute_fingerprint_if_eligible(
    source: &ConcreteFsSource,
    entry_type: &SpecFsEntryType,
    op: &FsOp,
    policy: FingerprintPolicy,
) -> Option<String> {
    if policy == FingerprintPolicy::None {
        return None;
    }
    if *op != FsOp::Copy {
        return None;
    }
    if policy == FingerprintPolicy::ManagedOnly && source.kind != FsSourceKind::ComponentRelative {
        return None;
    }
    match entry_type {
        SpecFsEntryType::File => compute_file_fingerprint(&source.resolved),
        SpecFsEntryType::Dir => compute_dir_fingerprint(&source.resolved),
    }
}

/// Compute the SHA-256 fingerprint of a file's contents.
///
/// Returns `None` if the file cannot be read (e.g., does not exist yet).
fn compute_file_fingerprint(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};

    let content = std::fs::read(path).ok()?;
    let hash = Sha256::digest(&content);
    Some(format!("sha256:{:x}", hash))
}

/// Compute a deterministic tree hash for a directory.
///
/// Algorithm:
/// 1. Recursively walk the directory, skipping symlinks.
/// 2. Collect records:
///    - Files: `file:<forward-slash-rel-path>:<sha256-of-content>`
///    - Empty directories: `dir:<forward-slash-rel-path>`
/// 3. Sort records lexicographically.
/// 4. SHA-256 hash the newline-joined records.
///
/// A completely empty root directory produces a single `dir:` sentinel record.
///
/// Returns `None` if the directory cannot be read, or if any file cannot be hashed.
fn compute_dir_fingerprint(dir_path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};

    let mut records: Vec<String> = Vec::new();
    collect_dir_records(dir_path, dir_path, &mut records)?;
    records.sort();

    // Represent a completely empty directory with a sentinel so different empty dirs
    // and non-empty dirs do not collide in hash space.
    if records.is_empty() {
        records.push("dir:".to_string());
    }

    let combined = records.join("\n");
    let hash = Sha256::digest(combined.as_bytes());
    Some(format!("sha256:{:x}", hash))
}

/// Recursively collect file and empty-directory records relative to `root`.
///
/// Symlinks are skipped to avoid infinite loops and non-determinism.
/// Returns `None` if any directory entry or file read fails.
fn collect_dir_records(root: &Path, dir: &Path, records: &mut Vec<String>) -> Option<()> {
    let mut rd_entries: Vec<_> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .collect();
    // Sort by filename instead of relying on OS order.
    rd_entries.sort_by_key(|e| e.file_name());

    for entry in rd_entries {
        let file_type = entry.file_type().ok()?;
        // Skip symlinks — they introduce platform variance and potential cycles.
        if file_type.is_symlink() {
            continue;
        }
        let abs_path = entry.path();
        let rel_path = abs_path.strip_prefix(root).ok()?;
        // Normalise path separator to `/` for cross-platform hash stability.
        let rel_str = rel_path.to_string_lossy().replace('\\', "/");

        if file_type.is_file() {
            let fingerprint = compute_file_fingerprint(&abs_path)?;
            records.push(format!("file:{}:{}", rel_str, fingerprint));
        } else if file_type.is_dir() {
            let before = records.len();
            collect_dir_records(root, &abs_path, records)?;
            if records.len() == before {
                // Subdirectory produced no records: it is empty (or contains only symlinks).
                records.push(format!("dir:{}", rel_str));
            }
        }
    }
    Some(())
}

/// Expand `~` prefix to the user's home directory.
///
/// Supports `~/` (Unix) and `~\` (Windows). Falls back to unchanged path
/// if HOME/USERPROFILE is not set.
fn expand_home(path: &str) -> PathBuf {
    let rest = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\"));
    if let Some(rest) = rest {
        let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"));
        if let Ok(home) = home {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::component_index::*;
    use std::collections::HashMap;

    fn make_index_with_fs(
        component_id: &str,
        source_dir: &str,
        resources: Vec<SpecResource>,
    ) -> ComponentIndex {
        let mut components = HashMap::new();
        components.insert(
            component_id.to_string(),
            ComponentMeta {
                spec_version: 1,
                mode: ComponentMode::Declarative,
                description: None,
                source_dir: source_dir.to_string(),
                dep: DepSpec {
                    depends: vec![],
                    requires: vec![],
                    provides: vec![],
                },
                spec: Some(ComponentSpec { resources }),
                scripts: None,
                params_schema: None,
            },
        );
        ComponentIndex {
            schema_version: COMPONENT_INDEX_SCHEMA_VERSION,
            components,
        }
    }

    #[test]
    fn default_source_from_basename() {
        let index = make_index_with_fs(
            "core/git",
            "/tmp/components/git",
            vec![SpecResource {
                id: "fs:gitconfig".to_string(),
                kind: SpecResourceKind::Fs {
                    source: None,
                    path: "~/.gitconfig".to_string(),
                    entry_type: SpecFsEntryType::File,
                    op: FsOp::Link,
                },
            }],
        );

        let result = materialize_fs_sources(&index, FingerprintPolicy::default()).unwrap();
        let key = ("core/git".to_string(), "fs:gitconfig".to_string());
        let mat = result.get(&key).unwrap();
        assert_eq!(mat.source.kind, FsSourceKind::ComponentRelative);
        assert_eq!(
            mat.source.resolved,
            PathBuf::from("/tmp/components/git/files/.gitconfig")
        );
    }

    #[test]
    fn explicit_component_relative_source() {
        let index = make_index_with_fs(
            "core/git",
            "/tmp/components/git",
            vec![SpecResource {
                id: "fs:gitconfig".to_string(),
                kind: SpecResourceKind::Fs {
                    source: Some("files/custom.conf".to_string()),
                    path: "~/.gitconfig".to_string(),
                    entry_type: SpecFsEntryType::File,
                    op: FsOp::Link,
                },
            }],
        );

        let result = materialize_fs_sources(&index, FingerprintPolicy::default()).unwrap();
        let key = ("core/git".to_string(), "fs:gitconfig".to_string());
        let mat = result.get(&key).unwrap();
        assert_eq!(mat.source.kind, FsSourceKind::ComponentRelative);
        assert_eq!(
            mat.source.resolved,
            PathBuf::from("/tmp/components/git/files/custom.conf")
        );
    }

    #[test]
    fn home_relative_source() {
        let index = make_index_with_fs(
            "core/git",
            "/tmp/components/git",
            vec![SpecResource {
                id: "fs:gitconfig".to_string(),
                kind: SpecResourceKind::Fs {
                    source: Some("~/shared/.gitconfig".to_string()),
                    path: "~/.gitconfig".to_string(),
                    entry_type: SpecFsEntryType::File,
                    op: FsOp::Link,
                },
            }],
        );

        let result = materialize_fs_sources(&index, FingerprintPolicy::default()).unwrap();
        let key = ("core/git".to_string(), "fs:gitconfig".to_string());
        let mat = result.get(&key).unwrap();
        assert_eq!(mat.source.kind, FsSourceKind::HomeRelative);
    }

    #[test]
    fn absolute_source() {
        let index = make_index_with_fs(
            "core/git",
            "/tmp/components/git",
            vec![SpecResource {
                id: "fs:gitconfig".to_string(),
                kind: SpecResourceKind::Fs {
                    source: Some("/etc/gitconfig".to_string()),
                    path: "~/.gitconfig".to_string(),
                    entry_type: SpecFsEntryType::File,
                    op: FsOp::Link,
                },
            }],
        );

        let result = materialize_fs_sources(&index, FingerprintPolicy::default()).unwrap();
        let key = ("core/git".to_string(), "fs:gitconfig".to_string());
        let mat = result.get(&key).unwrap();
        assert_eq!(mat.source.kind, FsSourceKind::Absolute);
        assert_eq!(mat.source.resolved, PathBuf::from("/etc/gitconfig"));
    }

    #[test]
    fn component_relative_escape_rejected() {
        let index = make_index_with_fs(
            "core/evil",
            "/tmp/components/evil",
            vec![SpecResource {
                id: "fs:bad".to_string(),
                kind: SpecResourceKind::Fs {
                    source: Some("../../../etc/shadow".to_string()),
                    path: "~/.evil".to_string(),
                    entry_type: SpecFsEntryType::File,
                    op: FsOp::Link,
                },
            }],
        );

        let result = materialize_fs_sources(&index, FingerprintPolicy::default());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("escapes component directory"), "{}", err);
    }

    #[test]
    fn non_fs_resources_skipped() {
        let index = make_index_with_fs(
            "core/git",
            "/tmp/components/git",
            vec![SpecResource {
                id: "package:git".to_string(),
                kind: SpecResourceKind::Package {
                    name: "git".to_string(),
                },
            }],
        );

        let result = materialize_fs_sources(&index, FingerprintPolicy::default()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn script_component_skipped() {
        let mut components = HashMap::new();
        components.insert(
            "core/script".to_string(),
            ComponentMeta {
                spec_version: 1,
                mode: ComponentMode::Script,
                description: None,
                source_dir: "/tmp/components/script".to_string(),
                dep: DepSpec {
                    depends: vec![],
                    requires: vec![],
                    provides: vec![],
                },
                spec: None,
                scripts: Some(ScriptSpec {
                    install: "install.sh".to_string(),
                    uninstall: "uninstall.sh".to_string(),
                }),
                params_schema: None,
            },
        );
        let index = ComponentIndex {
            schema_version: COMPONENT_INDEX_SCHEMA_VERSION,
            components,
        };

        let result = materialize_fs_sources(&index, FingerprintPolicy::default()).unwrap();
        assert!(result.is_empty());
    }

    // --- compute_dir_fingerprint unit tests ---

    #[test]
    fn dir_fingerprint_stable_across_calls() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"hello").unwrap();
        std::fs::write(dir.path().join("b.txt"), b"world").unwrap();

        let fp1 = compute_dir_fingerprint(dir.path()).unwrap();
        let fp2 = compute_dir_fingerprint(dir.path()).unwrap();
        assert_eq!(fp1, fp2);
        assert!(fp1.starts_with("sha256:"), "{}", fp1);
    }

    #[test]
    fn dir_fingerprint_changes_on_content_change() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file.txt"), b"original").unwrap();
        let fp1 = compute_dir_fingerprint(dir.path()).unwrap();

        std::fs::write(dir.path().join("file.txt"), b"modified").unwrap();
        let fp2 = compute_dir_fingerprint(dir.path()).unwrap();
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn dir_fingerprint_changes_on_added_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"existing").unwrap();
        let fp1 = compute_dir_fingerprint(dir.path()).unwrap();

        std::fs::write(dir.path().join("new.txt"), b"new").unwrap();
        let fp2 = compute_dir_fingerprint(dir.path()).unwrap();
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn dir_fingerprint_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        // An empty directory should still return a fingerprint (not None).
        let fp = compute_dir_fingerprint(dir.path()).unwrap();
        assert!(fp.starts_with("sha256:"), "{}", fp);
    }

    #[test]
    fn dir_fingerprint_empty_subdir_represented() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();
        let fp_with_empty_subdir = compute_dir_fingerprint(dir.path()).unwrap();

        // Removing the subdir should change the fingerprint.
        std::fs::remove_dir(dir.path().join("subdir")).unwrap();
        let fp_without_subdir = compute_dir_fingerprint(dir.path()).unwrap();
        assert_ne!(fp_with_empty_subdir, fp_without_subdir);
    }

    #[test]
    fn dir_fingerprint_nested_files() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("deep.txt"), b"deep content").unwrap();
        std::fs::write(dir.path().join("root.txt"), b"root content").unwrap();

        let fp = compute_dir_fingerprint(dir.path()).unwrap();
        assert!(fp.starts_with("sha256:"), "{}", fp);

        // Changing a nested file should alter the fingerprint.
        std::fs::write(sub.join("deep.txt"), b"different").unwrap();
        let fp2 = compute_dir_fingerprint(dir.path()).unwrap();
        assert_ne!(fp, fp2);
    }

    #[test]
    fn dir_fingerprint_two_empty_dirs_identical() {
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        assert_eq!(
            compute_dir_fingerprint(dir1.path()),
            compute_dir_fingerprint(dir2.path()),
        );
    }

    #[test]
    fn compute_fingerprint_dir_copy_component_relative() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file.txt"), b"content").unwrap();

        let source = ConcreteFsSource::component_relative(dir.path().to_path_buf());
        let fp = compute_fingerprint_if_eligible(
            &source,
            &SpecFsEntryType::Dir,
            &FsOp::Copy,
            FingerprintPolicy::default(),
        );
        assert!(fp.is_some());
        assert!(fp.unwrap().starts_with("sha256:"));
    }

    #[test]
    fn compute_fingerprint_dir_link_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let source = ConcreteFsSource::component_relative(dir.path().to_path_buf());
        // link operations are not fingerprinted.
        let fp = compute_fingerprint_if_eligible(
            &source,
            &SpecFsEntryType::Dir,
            &FsOp::Link,
            FingerprintPolicy::default(),
        );
        assert!(fp.is_none());
    }

    #[test]
    fn compute_fingerprint_dir_home_relative_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let source = ConcreteFsSource::home_relative(dir.path().to_path_buf());
        // home_relative sources are not fingerprinted under managed_only policy.
        let fp = compute_fingerprint_if_eligible(
            &source,
            &SpecFsEntryType::Dir,
            &FsOp::Copy,
            FingerprintPolicy::ManagedOnly,
        );
        assert!(fp.is_none());
    }

    #[test]
    fn fingerprint_policy_all_copy_enables_home_relative() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"data").unwrap();
        let source = ConcreteFsSource::home_relative(dir.path().to_path_buf());
        let fp = compute_fingerprint_if_eligible(
            &source,
            &SpecFsEntryType::Dir,
            &FsOp::Copy,
            FingerprintPolicy::AllCopy,
        );
        assert!(fp.is_some());
    }

    #[test]
    fn fingerprint_policy_all_copy_enables_absolute() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, b"data").unwrap();
        let source = ConcreteFsSource::absolute(file);
        let fp = compute_fingerprint_if_eligible(
            &source,
            &SpecFsEntryType::File,
            &FsOp::Copy,
            FingerprintPolicy::AllCopy,
        );
        assert!(fp.is_some());
    }

    #[test]
    fn fingerprint_policy_none_disables_all() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"data").unwrap();
        let source = ConcreteFsSource::component_relative(dir.path().to_path_buf());
        let fp = compute_fingerprint_if_eligible(
            &source,
            &SpecFsEntryType::Dir,
            &FsOp::Copy,
            FingerprintPolicy::None,
        );
        assert!(fp.is_none());
    }
}
