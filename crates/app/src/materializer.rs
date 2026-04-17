//! Materializer — resolves fs.source references before compilation.
//!
//! The materializer is the impure pre-compilation stage that:
//! - Resolves `source` field defaults (`files/<basename(path)>`)
//! - Classifies source kind (`component_relative`, `home_relative`, `absolute`)
//! - Validates component-relative paths do not escape the component directory
//! - Computes content fingerprints for eligible sources (Phase 1C)
//!
//! After materialization, all `ConcreteFsSource` values are fully resolved and
//! ready for consumption by the pure compiler stage.
//!
//! See: `docs/specs/data/desired_resource_graph.md`

use model::component_index::{ComponentIndex, FsOp, SpecFsEntryType, SpecResourceKind};
use model::fs::{validate_component_relative_source, ConcreteFsSource, FsSourceKind};
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
/// 4. Compute content fingerprint if eligible (Phase 1C)
pub(crate) fn materialize_fs_sources(
    index: &ComponentIndex,
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

            // Step 3: compute fingerprint for eligible sources (Phase 1C).
            let source_fingerprint = compute_fingerprint_if_eligible(&concrete, entry_type, op);

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

    // Absolute path
    if Path::new(raw_source).is_absolute() {
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
/// Phase 1C: only `component_relative + copy + file` sources are fingerprinted.
/// Returns `None` for all other combinations or if the file cannot be read.
fn compute_fingerprint_if_eligible(
    source: &ConcreteFsSource,
    entry_type: &SpecFsEntryType,
    op: &FsOp,
) -> Option<String> {
    if source.kind != FsSourceKind::ComponentRelative {
        return None;
    }
    if *op != FsOp::Copy {
        return None;
    }
    if *entry_type != SpecFsEntryType::File {
        return None;
    }

    compute_file_fingerprint(&source.resolved)
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

        let result = materialize_fs_sources(&index).unwrap();
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

        let result = materialize_fs_sources(&index).unwrap();
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

        let result = materialize_fs_sources(&index).unwrap();
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

        let result = materialize_fs_sources(&index).unwrap();
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

        let result = materialize_fs_sources(&index);
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

        let result = materialize_fs_sources(&index).unwrap();
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
            },
        );
        let index = ComponentIndex {
            schema_version: COMPONENT_INDEX_SCHEMA_VERSION,
            components,
        };

        let result = materialize_fs_sources(&index).unwrap();
        assert!(result.is_empty());
    }
}
