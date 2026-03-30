//! Feature Index Builder — reads `feature.yaml` files and produces a `FeatureIndex`.
//!
//! # Responsibilities
//!
//! 1. Scan source directories to discover available features.
//! 2. Load `feature.yaml` (base) and optionally merge `feature.<platform>.yaml` on top.
//! 3. Validate `spec_version` — abort on missing or unsupported version.
//! 4. Normalize `dep.depends` bare names to `<source_id>/<name>`.
//! 5. Reject `declarative` mode features that declare no resources.
//! 6. Produce a fully validated [`FeatureIndex`] for consumption by Resolver and FeatureCompiler.
//!
//! # Merge Semantics
//!
//! Platform overrides (`feature.<platform>.yaml`) replace individual top-level fields.
//! Arrays are **replaced**, not appended. Fields absent in the override file are inherited
//! from the base file unchanged.
//!
//! # Phase 3 Contract
//!
//! Platform path resolution (XDG, AppData) is not performed here.
//! Callers supply [`SourceRoot`] values with absolute paths resolved by the caller.
//!
//! See: `docs/specs/data/feature_index.md`

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use model::feature_index::{
    CapabilityRef, DepSpec, FeatureIndex, FeatureMeta, FeatureMode, FeatureSpec, SpecResource,
    SpecResourceKind, FEATURE_INDEX_SCHEMA_VERSION,
};
use serde::Deserialize;
use thiserror::Error;

/// The currently supported `spec_version` in `feature.yaml`.
const SUPPORTED_SPEC_VERSION: u32 = 1;

/// Errors produced by the Feature Index Builder.
#[derive(Debug, Error)]
pub enum FeatureIndexError {
    /// `spec_version` is missing or not `1`.
    #[error(
        "feature '{feature_id}': unsupported spec_version (found {found:?}, expected {expected})"
    )]
    UnsupportedSpecVersion {
        feature_id: String,
        found: Option<u32>,
        expected: u32,
    },

    /// A `declarative` mode feature has no `resources` list.
    #[error("feature '{feature_id}': mode is declarative but no resources are declared")]
    DeclarativeMissingResources { feature_id: String },

    /// A `dep.depends` entry uses a multi-slash form (normalized form must be `<source>/<name>`).
    #[error(
        "feature '{feature_id}': dep.depends entry '{entry}' has more than one '/' \
         (must be bare name or single-slash canonical ID)"
    )]
    InvalidDependsEntry { feature_id: String, entry: String },

    /// I/O or YAML parse error while reading a feature file.
    #[error("feature '{feature_id}': failed to read '{path}': {source}")]
    ReadError {
        feature_id: String,
        path: PathBuf,
        #[source]
        source: Box<io::IoError>,
    },

    /// A feature directory could not be scanned.
    #[error("failed to scan source directory '{dir}': {source}")]
    ScanError {
        dir: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Target platform for platform-specific override merging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Platform {
    Linux,
    Windows,
    Wsl,
}

impl Platform {
    /// Returns the platform suffix used in `feature.<platform>.yaml` filenames.
    pub fn file_suffix(&self) -> &'static str {
        match self {
            Platform::Linux => "linux",
            Platform::Windows => "windows",
            Platform::Wsl => "wsl",
        }
    }
}

/// A source root for feature discovery: a source ID and its `features/` directory.
///
/// For implicit sources:
/// - `core` → `<repo_root>/features`
/// - `local` → `<config_home>/features`
///
/// For external sources:
/// - `<ext>` → `<data_home>/sources/<ext>/features`
#[derive(Debug, Clone)]
pub struct SourceRoot {
    /// Source identifier (e.g. `core`, `local`, `community`).
    pub source_id: String,
    /// Absolute path to the `features/` directory for this source.
    pub features_dir: PathBuf,
}

/// Build a [`FeatureIndex`] by discovering and parsing all features under the given source roots.
///
/// For each source root, every subdirectory within `features_dir` is treated as one feature.
/// If `features_dir` does not exist, it is silently skipped (local/external sources may be absent).
///
/// # Errors
///
/// Returns the first [`FeatureIndexError`] encountered. All features in all sources are attempted
/// before returning, with errors accumulated and returned for the first failure.
pub fn build(
    sources: &[SourceRoot],
    platform: &Platform,
) -> Result<FeatureIndex, FeatureIndexError> {
    let mut features: HashMap<String, FeatureMeta> = HashMap::new();

    for source in sources {
        if !source.features_dir.exists() {
            continue;
        }

        let entries =
            std::fs::read_dir(&source.features_dir).map_err(|e| FeatureIndexError::ScanError {
                dir: source.features_dir.clone(),
                source: e,
            })?;

        for entry in entries {
            let entry = entry.map_err(|e| FeatureIndexError::ScanError {
                dir: source.features_dir.clone(),
                source: e,
            })?;

            let feature_dir = entry.path();
            if !feature_dir.is_dir() {
                continue;
            }

            let name = feature_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();

            let feature_id = format!("{}/{}", source.source_id, name);

            let meta = build_one(&feature_id, &feature_dir, &source.source_id, platform)?;
            features.insert(feature_id, meta);
        }
    }

    Ok(FeatureIndex {
        schema_version: FEATURE_INDEX_SCHEMA_VERSION,
        features,
    })
}

/// Build one [`FeatureMeta`] from a feature directory.
fn build_one(
    feature_id: &str,
    feature_dir: &Path,
    source_id: &str,
    platform: &Platform,
) -> Result<FeatureMeta, FeatureIndexError> {
    // Load base feature.yaml.
    let base_path = feature_dir.join("feature.yaml");
    let base: RawFeatureYaml =
        io::load_yaml(&base_path).map_err(|e| FeatureIndexError::ReadError {
            feature_id: feature_id.to_string(),
            path: base_path,
            source: Box::new(e),
        })?;

    // Load and merge platform override if present.
    let override_path = feature_dir.join(format!("feature.{}.yaml", platform.file_suffix()));
    let merged = if override_path.exists() {
        let overlay: RawFeatureYaml =
            io::load_yaml(&override_path).map_err(|e| FeatureIndexError::ReadError {
                feature_id: feature_id.to_string(),
                path: override_path,
                source: Box::new(e),
            })?;
        merge(base, overlay)
    } else {
        base
    };

    // Validate spec_version.
    match merged.spec_version {
        Some(v) if v == SUPPORTED_SPEC_VERSION => {}
        found => {
            return Err(FeatureIndexError::UnsupportedSpecVersion {
                feature_id: feature_id.to_string(),
                found,
                expected: SUPPORTED_SPEC_VERSION,
            });
        }
    }

    // Determine mode. Default is declarative; script must be declared explicitly.
    let mode = match merged.mode.as_deref() {
        Some("script") => FeatureMode::Script,
        _ => FeatureMode::Declarative,
    };

    // Normalize dep.depends bare names.
    let depends = normalize_depends(feature_id, source_id, merged.depends.unwrap_or_default())?;

    let dep = DepSpec {
        depends,
        requires: merged
            .requires
            .unwrap_or_default()
            .into_iter()
            .map(|r| CapabilityRef { name: r.name })
            .collect(),
        provides: merged
            .provides
            .unwrap_or_default()
            .into_iter()
            .map(|p| CapabilityRef { name: p.name })
            .collect(),
    };

    // Build spec from raw resources.
    let spec = merged.resources.map(|resources| FeatureSpec {
        resources: resources.into_iter().map(convert_resource).collect(),
    });

    // Reject declarative features that *explicitly* declare `mode: declarative`
    // but provide no resources — this is almost certainly a mistake.
    // Omitting `mode:` with no resources is allowed (produces a no-op declarative feature).
    if matches!(merged.mode.as_deref(), Some("declarative")) {
        let empty = match &spec {
            None => true,
            Some(s) => s.resources.is_empty(),
        };
        if empty {
            return Err(FeatureIndexError::DeclarativeMissingResources {
                feature_id: feature_id.to_string(),
            });
        }
    }

    Ok(FeatureMeta {
        spec_version: SUPPORTED_SPEC_VERSION,
        mode,
        description: merged.description,
        source_dir: feature_dir.to_string_lossy().into_owned(),
        dep,
        spec,
    })
}

/// Merge platform override on top of base. Each `Some` field in overlay replaces the base field.
fn merge(mut base: RawFeatureYaml, overlay: RawFeatureYaml) -> RawFeatureYaml {
    if overlay.spec_version.is_some() {
        base.spec_version = overlay.spec_version;
    }
    if overlay.mode.is_some() {
        base.mode = overlay.mode;
    }
    if overlay.description.is_some() {
        base.description = overlay.description;
    }
    if overlay.depends.is_some() {
        base.depends = overlay.depends;
    }
    if overlay.requires.is_some() {
        base.requires = overlay.requires;
    }
    if overlay.provides.is_some() {
        base.provides = overlay.provides;
    }
    if overlay.resources.is_some() {
        base.resources = overlay.resources;
    }
    base
}

/// Normalize `dep.depends` entries:
/// - bare name (`git`) → `<source_id>/git`
/// - already canonical (`core/git`) → preserved unchanged
/// - multi-slash → error
fn normalize_depends(
    feature_id: &str,
    source_id: &str,
    depends: Vec<String>,
) -> Result<Vec<String>, FeatureIndexError> {
    depends
        .into_iter()
        .map(|entry| {
            let slash_count = entry.chars().filter(|&c| c == '/').count();
            match slash_count {
                0 => Ok(format!("{source_id}/{entry}")),
                1 => Ok(entry),
                _ => Err(FeatureIndexError::InvalidDependsEntry {
                    feature_id: feature_id.to_string(),
                    entry,
                }),
            }
        })
        .collect()
}

/// Convert a raw spec resource into the model's `SpecResource`.
fn convert_resource(raw: RawSpecResource) -> SpecResource {
    SpecResource {
        id: raw.id,
        kind: raw.kind,
    }
}

// ─── Raw YAML types (internal only) ─────────────────────────────────────────

/// Raw representation of `feature.yaml` / `feature.<platform>.yaml`.
///
/// All fields are `Option` so the platform override file can contain only the fields it overrides.
#[derive(Debug, Default, Deserialize)]
struct RawFeatureYaml {
    spec_version: Option<u32>,
    mode: Option<String>,
    description: Option<String>,
    depends: Option<Vec<String>>,
    requires: Option<Vec<RawCapRef>>,
    provides: Option<Vec<RawCapRef>>,
    resources: Option<Vec<RawSpecResource>>,
}

/// Raw capability reference (`{ name: "..." }`).
#[derive(Debug, Deserialize)]
struct RawCapRef {
    name: String,
}

/// Raw spec resource — reuses the model's `SpecResourceKind` for the type tag dispatch,
/// but needs the `id` field alongside it.
#[derive(Debug, Deserialize)]
struct RawSpecResource {
    id: String,
    #[serde(flatten)]
    kind: SpecResourceKind,
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::feature_index::FeatureMode;

    // ─── Helpers ─────────────────────────────────────────────────────────────

    fn write(dir: &Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).unwrap();
    }

    fn make_feature_dir(root: &Path, name: &str) -> PathBuf {
        let d = root.join(name);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn source_root(source_id: &str, dir: &Path) -> SourceRoot {
        SourceRoot {
            source_id: source_id.to_string(),
            features_dir: dir.to_path_buf(),
        }
    }

    fn simple_script_yaml() -> &'static str {
        "spec_version: 1\n"
    }

    fn simple_declarative_yaml() -> &'static str {
        "spec_version: 1\nmode: declarative\nresources:\n  - id: pkg:jq\n    kind: package\n    name: jq\n"
    }

    // ─── build — happy path ───────────────────────────────────────────────────

    #[test]
    fn build_empty_source_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let features_dir = tmp.path().join("features");
        std::fs::create_dir_all(&features_dir).unwrap();
        let index = build(&[source_root("core", &features_dir)], &Platform::Linux).unwrap();
        assert!(index.features.is_empty());
    }

    #[test]
    fn build_missing_source_dir_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("nonexistent");
        let index = build(&[source_root("local", &missing)], &Platform::Linux).unwrap();
        assert!(index.features.is_empty());
    }

    #[test]
    fn build_default_mode_is_declarative() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_feature_dir(tmp.path(), "git");
        write(
            &fdir,
            "feature.yaml",
            "spec_version: 1\ndescription: Git VCS\n",
        );

        let index = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap();
        let meta = index
            .features
            .get("core/git")
            .expect("core/git must be in index");
        // No `mode:` field → default is Declarative.
        assert_eq!(meta.mode, FeatureMode::Declarative);
        assert_eq!(meta.description.as_deref(), Some("Git VCS"));
        assert!(meta.spec.is_none());
    }

    #[test]
    fn build_declarative_feature() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_feature_dir(tmp.path(), "jq");
        write(&fdir, "feature.yaml", simple_declarative_yaml());

        let index = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap();
        let meta = index.features.get("core/jq").unwrap();
        assert_eq!(meta.mode, FeatureMode::Declarative);
        let spec = meta.spec.as_ref().unwrap();
        assert_eq!(spec.resources.len(), 1);
        assert_eq!(spec.resources[0].id, "pkg:jq");
    }

    #[test]
    fn build_source_dir_set_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_feature_dir(tmp.path(), "git");
        write(&fdir, "feature.yaml", simple_script_yaml());

        let index = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap();
        let meta = &index.features["core/git"];
        assert_eq!(meta.source_dir, fdir.to_string_lossy());
    }

    // ─── depends normalization ────────────────────────────────────────────────

    #[test]
    fn bare_depends_normalized_to_same_source() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_feature_dir(tmp.path(), "neovim");
        write(
            &fdir,
            "feature.yaml",
            "spec_version: 1\ndepends:\n  - git\n",
        );

        let index = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap();
        let deps = &index.features["core/neovim"].dep.depends;
        assert_eq!(deps, &["core/git"]);
    }

    #[test]
    fn canonical_depends_preserved() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_feature_dir(tmp.path(), "neovim");
        write(
            &fdir,
            "feature.yaml",
            "spec_version: 1\ndepends:\n  - community/plugin\n",
        );

        let index = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap();
        let deps = &index.features["core/neovim"].dep.depends;
        assert_eq!(deps, &["community/plugin"]);
    }

    // ─── platform merge ───────────────────────────────────────────────────────

    #[test]
    fn platform_override_replaces_depends() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_feature_dir(tmp.path(), "mise");
        write(
            &fdir,
            "feature.yaml",
            "spec_version: 1\ndepends:\n  - brew\n",
        );
        write(&fdir, "feature.linux.yaml", "depends:\n  - apt\n");

        let index = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap();
        let deps = &index.features["core/mise"].dep.depends;
        // Platform override replaces the array.
        assert_eq!(deps, &["core/apt"]);
    }

    #[test]
    fn platform_override_not_applied_on_different_platform() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_feature_dir(tmp.path(), "mise");
        write(
            &fdir,
            "feature.yaml",
            "spec_version: 1\ndepends:\n  - brew\n",
        );
        write(&fdir, "feature.linux.yaml", "depends:\n  - apt\n");

        let index = build(&[source_root("core", tmp.path())], &Platform::Windows).unwrap();
        let deps = &index.features["core/mise"].dep.depends;
        // Windows platform: linux override is not applied.
        assert_eq!(deps, &["core/brew"]);
    }

    #[test]
    fn platform_override_inherits_base_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_feature_dir(tmp.path(), "mise");
        write(
            &fdir,
            "feature.yaml",
            "spec_version: 1\ndescription: Base desc\ndepends:\n  - brew\n",
        );
        write(&fdir, "feature.linux.yaml", "depends:\n  - apt\n");

        let index = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap();
        let meta = &index.features["core/mise"];
        // description is inherited from base
        assert_eq!(meta.description.as_deref(), Some("Base desc"));
        // depends is replaced by override
        assert_eq!(&meta.dep.depends, &["core/apt"]);
    }

    // ─── validation errors ────────────────────────────────────────────────────

    #[test]
    fn missing_spec_version_is_error() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_feature_dir(tmp.path(), "bad");
        write(&fdir, "feature.yaml", "description: no version\n");

        let err = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap_err();
        assert!(matches!(
            err,
            FeatureIndexError::UnsupportedSpecVersion { .. }
        ));
    }

    #[test]
    fn wrong_spec_version_is_error() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_feature_dir(tmp.path(), "bad");
        write(&fdir, "feature.yaml", "spec_version: 99\n");

        let err = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap_err();
        assert!(matches!(
            err,
            FeatureIndexError::UnsupportedSpecVersion { .. }
        ));
    }

    #[test]
    fn declarative_with_no_resources_is_error() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_feature_dir(tmp.path(), "nodecl");
        write(
            &fdir,
            "feature.yaml",
            "spec_version: 1\nmode: declarative\n",
        );

        let err = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap_err();
        assert!(matches!(
            err,
            FeatureIndexError::DeclarativeMissingResources { .. }
        ));
    }

    #[test]
    fn declarative_with_empty_resources_is_error() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_feature_dir(tmp.path(), "empty");
        write(
            &fdir,
            "feature.yaml",
            "spec_version: 1\nmode: declarative\nresources: []\n",
        );

        let err = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap_err();
        assert!(matches!(
            err,
            FeatureIndexError::DeclarativeMissingResources { .. }
        ));
    }

    #[test]
    fn invalid_multi_slash_depends_is_error() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_feature_dir(tmp.path(), "bad");
        write(
            &fdir,
            "feature.yaml",
            "spec_version: 1\ndepends:\n  - a/b/c\n",
        );

        let err = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap_err();
        assert!(matches!(err, FeatureIndexError::InvalidDependsEntry { .. }));
    }

    // ─── multiple sources ─────────────────────────────────────────────────────

    #[test]
    fn multiple_sources_merged_into_one_index() {
        let tmp = tempfile::tempdir().unwrap();
        let core_dir = tmp.path().join("core");
        let local_dir = tmp.path().join("local");
        let cgit = make_feature_dir(&core_dir, "git");
        let umyvim = make_feature_dir(&local_dir, "myvim");
        write(&cgit, "feature.yaml", simple_script_yaml());
        write(&umyvim, "feature.yaml", simple_script_yaml());

        let index = build(
            &[
                source_root("core", &core_dir),
                source_root("local", &local_dir),
            ],
            &Platform::Linux,
        )
        .unwrap();

        assert!(index.features.contains_key("core/git"));
        assert!(index.features.contains_key("local/myvim"));
    }

    // ─── requires/provides ───────────────────────────────────────────────────

    #[test]
    fn capabilities_parsed_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_feature_dir(tmp.path(), "mise");
        write(
            &fdir,
            "feature.yaml",
            "spec_version: 1\nprovides:\n  - name: runtime_manager\n  - name: package_manager\n\
             requires:\n  - name: shell\n",
        );

        let index = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap();
        let dep = &index.features["core/mise"].dep;
        assert_eq!(dep.provides.len(), 2);
        assert_eq!(dep.provides[0].name, "runtime_manager");
        assert_eq!(dep.provides[1].name, "package_manager");
        assert_eq!(dep.requires.len(), 1);
        assert_eq!(dep.requires[0].name, "shell");
    }
}
