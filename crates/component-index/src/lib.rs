//! Component Index Builder — reads `component.yaml` files and produces a `ComponentIndex`.
//!
//! # Responsibilities
//!
//! 1. Scan source directories to discover available components.
//! 2. Load `component.yaml` (base) and optionally merge `component.<platform>.yaml` on top.
//! 3. Validate `spec_version` — abort on missing or unsupported version.
//! 4. Normalize `dep.depends` bare names to `<source_id>/<name>`.
//! 5. Reject `declarative` mode components that declare no resources.
//! 6. Produce a fully validated [`ComponentIndex`] for consumption by Resolver and ComponentCompiler.
//!
//! # Merge Semantics
//!
//! Platform overrides (`component.<platform>.yaml`) replace individual top-level fields.
//! Arrays are **replaced**, not appended. Fields absent in the override file are inherited
//! from the base file unchanged.
//!
//! # Phase 3 Contract
//!
//! Platform path resolution (XDG, AppData) is not performed here.
//! Callers supply [`SourceRoot`] values with absolute paths resolved by the caller.
//!
//! See: `docs/specs/data/component_index.md`

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use model::component_index::{
    CapabilityRef, ComponentIndex, ComponentMeta, ComponentMode, ComponentSpec, DepSpec,
    ScriptSpec, SpecResource, SpecResourceKind, COMPONENT_INDEX_SCHEMA_VERSION,
};
use serde::Deserialize;
use thiserror::Error;

/// The currently supported `spec_version` in `component.yaml`.
const SUPPORTED_SPEC_VERSION: u32 = 1;

/// Errors produced by the Component Index Builder.
#[derive(Debug, Error)]
pub enum ComponentIndexError {
    /// `spec_version` is missing or not `1`.
    #[error(
        "component '{component_id}': unsupported spec_version (found {found:?}, expected {expected})"
    )]
    UnsupportedSpecVersion {
        component_id: String,
        found: Option<u32>,
        expected: u32,
    },

    /// A `declarative` mode component has no `resources` list.
    #[error("component '{component_id}': mode is declarative but no resources are declared")]
    DeclarativeMissingResources { component_id: String },

    /// A `managed_script` component has no `resources` list or declares no `tool` resources.
    #[error(
        "component '{component_id}': mode is managed_script but no tool resources are declared"
    )]
    ManagedScriptMissingResources { component_id: String },

    /// A `managed_script` component declares a resource kind other than `tool`.
    #[error(
        "component '{component_id}': mode is managed_script but resource '{resource_id}' \
         has kind '{kind}' (only 'tool' is allowed in managed_script components)"
    )]
    ManagedScriptInvalidResourceKind {
        component_id: String,
        resource_id: String,
        kind: String,
    },

    /// A `managed_script` (or `script`) component does not declare `scripts.install`/`scripts.uninstall`.
    #[error(
        "component '{component_id}': mode is {mode} but 'scripts.install' and \
         'scripts.uninstall' are required"
    )]
    ScriptsMissing { component_id: String, mode: String },

    /// A `tool` resource has no identity verify (only `versioned_command`-only verify is invalid).
    #[error(
        "component '{component_id}': tool resource '{resource_id}' \
         must declare an identity verify (resolved_command, file, symlink_target, or directory). \
         versioned_command alone is insufficient."
    )]
    ToolMissingIdentityVerify {
        component_id: String,
        resource_id: String,
    },

    /// A `dep.depends` entry uses a multi-slash form (normalized form must be `<source>/<name>`).
    #[error(
        "component '{component_id}': dep.depends entry '{entry}' has more than one '/' \
         (must be bare name or single-slash canonical ID)"
    )]
    InvalidDependsEntry { component_id: String, entry: String },

    /// I/O or YAML parse error while reading a component file.
    #[error("component '{component_id}': failed to read '{path}': {source}")]
    ReadError {
        component_id: String,
        path: PathBuf,
        #[source]
        source: Box<io::IoError>,
    },

    /// A component directory could not be scanned.
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
    /// Returns the platform suffix used in `component.<platform>.yaml` filenames.
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
    pub components_dir: PathBuf,
}

/// Build a [`ComponentIndex`] by discovering and parsing all components under the given source roots.
///
/// For each source root, every subdirectory within `components_dir` is treated as one component.
/// If `components_dir` does not exist, it is silently skipped (local/external sources may be absent).
///
/// # Errors
///
/// Returns the first [`ComponentIndexError`] encountered. All components in all sources are attempted
/// before returning, with errors accumulated and returned for the first failure.
pub fn build(
    sources: &[SourceRoot],
    platform: &Platform,
) -> Result<ComponentIndex, ComponentIndexError> {
    let mut components: HashMap<String, ComponentMeta> = HashMap::new();

    for source in sources {
        if !source.components_dir.exists() {
            continue;
        }

        let entries = std::fs::read_dir(&source.components_dir).map_err(|e| {
            ComponentIndexError::ScanError {
                dir: source.components_dir.clone(),
                source: e,
            }
        })?;

        for entry in entries {
            let entry = entry.map_err(|e| ComponentIndexError::ScanError {
                dir: source.components_dir.clone(),
                source: e,
            })?;

            let component_dir = entry.path();
            if !component_dir.is_dir() {
                continue;
            }

            let name = component_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();

            let component_id = format!("{}/{}", source.source_id, name);

            let meta = build_one(&component_id, &component_dir, &source.source_id, platform)?;
            components.insert(component_id, meta);
        }
    }

    Ok(ComponentIndex {
        schema_version: COMPONENT_INDEX_SCHEMA_VERSION,
        components,
    })
}

/// Build one [`ComponentMeta`] from a component directory.
fn build_one(
    component_id: &str,
    component_dir: &Path,
    source_id: &str,
    platform: &Platform,
) -> Result<ComponentMeta, ComponentIndexError> {
    // Load base component.yaml.
    let base_path = component_dir.join("component.yaml");
    let base: RawComponentYaml =
        io::load_yaml(&base_path).map_err(|e| ComponentIndexError::ReadError {
            component_id: component_id.to_string(),
            path: base_path,
            source: Box::new(e),
        })?;

    // Load and merge platform override if present.
    let override_path = component_dir.join(format!("component.{}.yaml", platform.file_suffix()));
    let merged = if override_path.exists() {
        let overlay: RawComponentYaml =
            io::load_yaml(&override_path).map_err(|e| ComponentIndexError::ReadError {
                component_id: component_id.to_string(),
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
            return Err(ComponentIndexError::UnsupportedSpecVersion {
                component_id: component_id.to_string(),
                found,
                expected: SUPPORTED_SPEC_VERSION,
            });
        }
    }

    // Determine mode. Default is declarative; script/managed_script must be declared explicitly.
    let mode = match merged.mode.as_deref() {
        Some("script") => ComponentMode::Script,
        Some("managed_script") => ComponentMode::ManagedScript,
        _ => ComponentMode::Declarative,
    };

    // Normalize dep.depends bare names.
    let depends = normalize_depends(component_id, source_id, merged.depends.unwrap_or_default())?;

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
    let spec = merged.resources.map(|resources| ComponentSpec {
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
            return Err(ComponentIndexError::DeclarativeMissingResources {
                component_id: component_id.to_string(),
            });
        }
    }

    // `managed_script` mode requires scripts.install and scripts.uninstall.
    // `script` mode supports implicit file-name discovery by component-host, so scripts: is optional.
    if matches!(mode, ComponentMode::ManagedScript) {
        let scripts_ok = merged
            .scripts
            .as_ref()
            .is_some_and(|s| s.install.is_some() && s.uninstall.is_some());
        if !scripts_ok {
            return Err(ComponentIndexError::ScriptsMissing {
                component_id: component_id.to_string(),
                mode: "managed_script".to_string(),
            });
        }
    }

    // `managed_script` mode: all resources must be `kind: tool`, and at least one must exist.
    if matches!(mode, ComponentMode::ManagedScript) {
        let has_resources = spec.as_ref().is_some_and(|s| !s.resources.is_empty());
        if !has_resources {
            return Err(ComponentIndexError::ManagedScriptMissingResources {
                component_id: component_id.to_string(),
            });
        }
        for resource in spec.as_ref().unwrap().resources.iter() {
            if !matches!(resource.kind, SpecResourceKind::Tool { .. }) {
                let kind_str = match &resource.kind {
                    SpecResourceKind::Package { .. } => "package",
                    SpecResourceKind::Runtime { .. } => "runtime",
                    SpecResourceKind::Fs { .. } => "fs",
                    SpecResourceKind::Tool { .. } => unreachable!(),
                };
                return Err(ComponentIndexError::ManagedScriptInvalidResourceKind {
                    component_id: component_id.to_string(),
                    resource_id: resource.id.clone(),
                    kind: kind_str.to_string(),
                });
            }
        }
    }

    // Build ScriptSpec from raw scripts fields.
    // For managed_script: already validated present above.
    // For script: optional (component-host falls back to convention-based file discovery).
    let scripts = merged.scripts.and_then(|raw| {
        let install = raw.install?;
        let uninstall = raw.uninstall?;
        Some(ScriptSpec { install, uninstall })
    });

    Ok(ComponentMeta {
        spec_version: SUPPORTED_SPEC_VERSION,
        mode,
        description: merged.description,
        source_dir: component_dir.to_string_lossy().into_owned(),
        dep,
        spec,
        scripts,
    })
}

/// Merge platform override on top of base. Each `Some` field in overlay replaces the base field.
fn merge(mut base: RawComponentYaml, overlay: RawComponentYaml) -> RawComponentYaml {
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
    if overlay.scripts.is_some() {
        base.scripts = overlay.scripts;
    }
    base
}

/// Normalize `dep.depends` entries:
/// - bare name (`git`) → `<source_id>/git`
/// - already canonical (`core/git`) → preserved unchanged
/// - multi-slash → error
fn normalize_depends(
    component_id: &str,
    source_id: &str,
    depends: Vec<String>,
) -> Result<Vec<String>, ComponentIndexError> {
    depends
        .into_iter()
        .map(|entry| {
            let slash_count = entry.chars().filter(|&c| c == '/').count();
            match slash_count {
                0 => Ok(format!("{source_id}/{entry}")),
                1 => Ok(entry),
                _ => Err(ComponentIndexError::InvalidDependsEntry {
                    component_id: component_id.to_string(),
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

/// Raw representation of `component.yaml` / `component.<platform>.yaml`.
///
/// All fields are `Option` so the platform override file can contain only the fields it overrides.
#[derive(Debug, Default, Deserialize)]
struct RawComponentYaml {
    spec_version: Option<u32>,
    mode: Option<String>,
    description: Option<String>,
    depends: Option<Vec<String>>,
    requires: Option<Vec<RawCapRef>>,
    provides: Option<Vec<RawCapRef>>,
    resources: Option<Vec<RawSpecResource>>,
    scripts: Option<RawScriptSpec>,
}

/// Raw script entry points from `component.yaml`.
#[derive(Debug, Default, Deserialize)]
struct RawScriptSpec {
    install: Option<String>,
    uninstall: Option<String>,
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
    use model::component_index::ComponentMode;

    // ─── Helpers ─────────────────────────────────────────────────────────────

    fn write(dir: &Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).unwrap();
    }

    fn make_component_dir(root: &Path, name: &str) -> PathBuf {
        let d = root.join(name);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn source_root(source_id: &str, dir: &Path) -> SourceRoot {
        SourceRoot {
            source_id: source_id.to_string(),
            components_dir: dir.to_path_buf(),
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
        let components_dir = tmp.path().join("components");
        std::fs::create_dir_all(&components_dir).unwrap();
        let index = build(&[source_root("core", &components_dir)], &Platform::Linux).unwrap();
        assert!(index.components.is_empty());
    }

    #[test]
    fn build_missing_source_dir_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("nonexistent");
        let index = build(&[source_root("local", &missing)], &Platform::Linux).unwrap();
        assert!(index.components.is_empty());
    }

    #[test]
    fn build_default_mode_is_declarative() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_component_dir(tmp.path(), "git");
        write(
            &fdir,
            "component.yaml",
            "spec_version: 1\ndescription: Git VCS\n",
        );

        let index = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap();
        let meta = index
            .components
            .get("core/git")
            .expect("core/git must be in index");
        // No `mode:` field → default is Declarative.
        assert_eq!(meta.mode, ComponentMode::Declarative);
        assert_eq!(meta.description.as_deref(), Some("Git VCS"));
        assert!(meta.spec.is_none());
    }

    #[test]
    fn build_declarative_component() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_component_dir(tmp.path(), "jq");
        write(&fdir, "component.yaml", simple_declarative_yaml());

        let index = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap();
        let meta = index.components.get("core/jq").unwrap();
        assert_eq!(meta.mode, ComponentMode::Declarative);
        let spec = meta.spec.as_ref().unwrap();
        assert_eq!(spec.resources.len(), 1);
        assert_eq!(spec.resources[0].id, "pkg:jq");
    }

    #[test]
    fn build_source_dir_set_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_component_dir(tmp.path(), "git");
        write(&fdir, "component.yaml", simple_script_yaml());

        let index = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap();
        let meta = &index.components["core/git"];
        assert_eq!(meta.source_dir, fdir.to_string_lossy());
    }

    // ─── depends normalization ────────────────────────────────────────────────

    #[test]
    fn bare_depends_normalized_to_same_source() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_component_dir(tmp.path(), "neovim");
        write(
            &fdir,
            "component.yaml",
            "spec_version: 1\ndepends:\n  - git\n",
        );

        let index = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap();
        let deps = &index.components["core/neovim"].dep.depends;
        assert_eq!(deps, &["core/git"]);
    }

    #[test]
    fn canonical_depends_preserved() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_component_dir(tmp.path(), "neovim");
        write(
            &fdir,
            "component.yaml",
            "spec_version: 1\ndepends:\n  - community/plugin\n",
        );

        let index = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap();
        let deps = &index.components["core/neovim"].dep.depends;
        assert_eq!(deps, &["community/plugin"]);
    }

    // ─── platform merge ───────────────────────────────────────────────────────

    #[test]
    fn platform_override_replaces_depends() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_component_dir(tmp.path(), "mise");
        write(
            &fdir,
            "component.yaml",
            "spec_version: 1\ndepends:\n  - brew\n",
        );
        write(&fdir, "component.linux.yaml", "depends:\n  - apt\n");

        let index = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap();
        let deps = &index.components["core/mise"].dep.depends;
        // Platform override replaces the array.
        assert_eq!(deps, &["core/apt"]);
    }

    #[test]
    fn platform_override_not_applied_on_different_platform() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_component_dir(tmp.path(), "mise");
        write(
            &fdir,
            "component.yaml",
            "spec_version: 1\ndepends:\n  - brew\n",
        );
        write(&fdir, "component.linux.yaml", "depends:\n  - apt\n");

        let index = build(&[source_root("core", tmp.path())], &Platform::Windows).unwrap();
        let deps = &index.components["core/mise"].dep.depends;
        // Windows platform: linux override is not applied.
        assert_eq!(deps, &["core/brew"]);
    }

    #[test]
    fn platform_override_inherits_base_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_component_dir(tmp.path(), "mise");
        write(
            &fdir,
            "component.yaml",
            "spec_version: 1\ndescription: Base desc\ndepends:\n  - brew\n",
        );
        write(&fdir, "component.linux.yaml", "depends:\n  - apt\n");

        let index = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap();
        let meta = &index.components["core/mise"];
        // description is inherited from base
        assert_eq!(meta.description.as_deref(), Some("Base desc"));
        // depends is replaced by override
        assert_eq!(&meta.dep.depends, &["core/apt"]);
    }

    // ─── validation errors ────────────────────────────────────────────────────

    #[test]
    fn missing_spec_version_is_error() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_component_dir(tmp.path(), "bad");
        write(&fdir, "component.yaml", "description: no version\n");

        let err = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap_err();
        assert!(matches!(
            err,
            ComponentIndexError::UnsupportedSpecVersion { .. }
        ));
    }

    #[test]
    fn wrong_spec_version_is_error() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_component_dir(tmp.path(), "bad");
        write(&fdir, "component.yaml", "spec_version: 99\n");

        let err = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap_err();
        assert!(matches!(
            err,
            ComponentIndexError::UnsupportedSpecVersion { .. }
        ));
    }

    #[test]
    fn declarative_with_no_resources_is_error() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_component_dir(tmp.path(), "nodecl");
        write(
            &fdir,
            "component.yaml",
            "spec_version: 1\nmode: declarative\n",
        );

        let err = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap_err();
        assert!(matches!(
            err,
            ComponentIndexError::DeclarativeMissingResources { .. }
        ));
    }

    #[test]
    fn declarative_with_empty_resources_is_error() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_component_dir(tmp.path(), "empty");
        write(
            &fdir,
            "component.yaml",
            "spec_version: 1\nmode: declarative\nresources: []\n",
        );

        let err = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap_err();
        assert!(matches!(
            err,
            ComponentIndexError::DeclarativeMissingResources { .. }
        ));
    }

    #[test]
    fn invalid_multi_slash_depends_is_error() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_component_dir(tmp.path(), "bad");
        write(
            &fdir,
            "component.yaml",
            "spec_version: 1\ndepends:\n  - a/b/c\n",
        );

        let err = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap_err();
        assert!(matches!(
            err,
            ComponentIndexError::InvalidDependsEntry { .. }
        ));
    }

    // ─── multiple sources ─────────────────────────────────────────────────────

    #[test]
    fn multiple_sources_merged_into_one_index() {
        let tmp = tempfile::tempdir().unwrap();
        let core_dir = tmp.path().join("core");
        let local_dir = tmp.path().join("local");
        let cgit = make_component_dir(&core_dir, "git");
        let umyvim = make_component_dir(&local_dir, "myvim");
        write(&cgit, "component.yaml", simple_script_yaml());
        write(&umyvim, "component.yaml", simple_script_yaml());

        let index = build(
            &[
                source_root("core", &core_dir),
                source_root("local", &local_dir),
            ],
            &Platform::Linux,
        )
        .unwrap();

        assert!(index.components.contains_key("core/git"));
        assert!(index.components.contains_key("local/myvim"));
    }

    // ─── requires/provides ───────────────────────────────────────────────────

    #[test]
    fn capabilities_parsed_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_component_dir(tmp.path(), "mise");
        write(
            &fdir,
            "component.yaml",
            "spec_version: 1\nprovides:\n  - name: runtime_manager\n  - name: package_manager\n\
             requires:\n  - name: shell\n",
        );

        let index = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap();
        let dep = &index.components["core/mise"].dep;
        assert_eq!(dep.provides.len(), 2);
        assert_eq!(dep.provides[0].name, "runtime_manager");
        assert_eq!(dep.provides[1].name, "package_manager");
        assert_eq!(dep.requires.len(), 1);
        assert_eq!(dep.requires[0].name, "shell");
    }

    // ─── managed_script mode ─────────────────────────────────────────────────

    #[test]
    fn managed_script_component_parsed() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_component_dir(tmp.path(), "brew");
        let yaml = concat!(
            "spec_version: 1\n",
            "mode: managed_script\n",
            "scripts:\n",
            "  install: install.sh\n",
            "  uninstall: uninstall.sh\n",
            "resources:\n",
            "  - id: tool:brew\n",
            "    kind: tool\n",
            "    name: brew\n",
            "    verify:\n",
            "      identity:\n",
            "        type: resolved_command\n",
            "        command: brew\n",
            "        expected_path:\n",
            "          one_of:\n",
            "            - /home/linuxbrew/.linuxbrew/bin/brew\n",
        );
        write(&fdir, "component.yaml", yaml);

        let index = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap();
        let meta = index.components.get("core/brew").unwrap();
        assert_eq!(meta.mode, ComponentMode::ManagedScript);
        let scripts = meta.scripts.as_ref().unwrap();
        assert_eq!(scripts.install, "install.sh");
        assert_eq!(scripts.uninstall, "uninstall.sh");
        let spec = meta.spec.as_ref().unwrap();
        assert_eq!(spec.resources.len(), 1);
        assert_eq!(spec.resources[0].id, "tool:brew");
        assert!(matches!(
            spec.resources[0].kind,
            model::component_index::SpecResourceKind::Tool { .. }
        ));
    }

    #[test]
    fn managed_script_missing_scripts_is_error() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_component_dir(tmp.path(), "brew");
        let yaml = concat!(
            "spec_version: 1\n",
            "mode: managed_script\n",
            "resources:\n",
            "  - id: tool:brew\n",
            "    kind: tool\n",
            "    name: brew\n",
            "    verify:\n",
            "      identity:\n",
            "        type: resolved_command\n",
            "        command: brew\n",
            "        expected_path:\n",
            "          one_of:\n",
            "            - /home/linuxbrew/.linuxbrew/bin/brew\n",
        );
        write(&fdir, "component.yaml", yaml);

        let err = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap_err();
        assert!(matches!(err, ComponentIndexError::ScriptsMissing { .. }));
    }

    #[test]
    fn managed_script_missing_resources_is_error() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_component_dir(tmp.path(), "brew");
        let yaml = concat!(
            "spec_version: 1\n",
            "mode: managed_script\n",
            "scripts:\n",
            "  install: install.sh\n",
            "  uninstall: uninstall.sh\n",
        );
        write(&fdir, "component.yaml", yaml);

        let err = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap_err();
        assert!(matches!(
            err,
            ComponentIndexError::ManagedScriptMissingResources { .. }
        ));
    }

    #[test]
    fn managed_script_non_tool_resource_is_error() {
        let tmp = tempfile::tempdir().unwrap();
        let fdir = make_component_dir(tmp.path(), "brew");
        let yaml = concat!(
            "spec_version: 1\n",
            "mode: managed_script\n",
            "scripts:\n",
            "  install: install.sh\n",
            "  uninstall: uninstall.sh\n",
            "resources:\n",
            "  - id: pkg:git\n",
            "    kind: package\n",
            "    name: git\n",
        );
        write(&fdir, "component.yaml", yaml);

        let err = build(&[source_root("core", tmp.path())], &Platform::Linux).unwrap_err();
        assert!(matches!(
            err,
            ComponentIndexError::ManagedScriptInvalidResourceKind { .. }
        ));
    }
}
