// Read-only use cases for CLI display commands.
//
// Each function here is a stand-alone use case that:
//   1. Loads whatever sources / index / state are needed.
//   2. Applies any filtering or lookup.
//   3. Returns a typed result ready for the CLI to format and display.
//
// CLI is responsible only for argument parsing, output formatting, and exit codes.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::context::{AppContext, AppError};
use crate::pipeline::{build_source_roots, load_sources_optional, to_ci_platform};

// ---------------------------------------------------------------------------
// Feature types
// ---------------------------------------------------------------------------

/// Summary of a single feature for list output.
#[derive(Debug, Serialize)]
pub struct ComponentSummary {
    pub id: String,
    pub mode: model::component_index::ComponentMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Full detail of a single feature for show output.
#[derive(Debug, Serialize)]
pub struct ComponentDetail {
    pub id: String,
    #[serde(flatten)]
    pub meta: model::component_index::ComponentMeta,
}

// ---------------------------------------------------------------------------
// Backend types
// ---------------------------------------------------------------------------

/// Summary of a single script backend for list output.
#[derive(Debug, Serialize)]
pub struct BackendSummary {
    pub id: String,
    pub source: String,
    pub dir: String,
    pub api_version: u32,
}

/// Script availability flags for a backend.
#[derive(Debug, Serialize)]
pub struct BackendScripts {
    pub apply: bool,
    pub remove: bool,
    pub status: bool,
    pub env_pre: bool,
    pub env_post: bool,
}

/// Full detail of a single script backend for show output.
#[derive(Debug, Serialize)]
pub struct BackendDetail {
    pub id: String,
    pub source: String,
    pub dir: String,
    pub api_version: u32,
    pub scripts: BackendScripts,
}

// ---------------------------------------------------------------------------
// Source types
// ---------------------------------------------------------------------------

/// Summary of a single source entry (implicit or external) for list/show output.
#[derive(Debug, Serialize)]
pub struct SourceSummary {
    pub id: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
    /// Human-readable ref label: `"branch:main"` / `"tag:v1.0"` / `"commit:<hash>"`.
    /// Set only for `type: git` sources that declare a `ref:` field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_spec: Option<String>,
    /// Full commit hash from `sources.lock.yaml`. `None` if the source is not locked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_commit: Option<String>,
    /// UTC RFC3339 timestamp of the last successful fetch. `None` if not yet locked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetched_at: Option<String>,
}

// ---------------------------------------------------------------------------
// Config types
// ---------------------------------------------------------------------------

/// Summary of a config file for list output.
#[derive(Debug, Serialize)]
pub struct ConfigSummary {
    pub name: String,
    pub path: String,
    pub active: bool,
}

/// Full detail of a config file for show output.
/// `profile` is not serializable directly; CLI builds its own serializable repr.
#[derive(Debug)]
pub struct ConfigDetail {
    pub name: String,
    pub path: PathBuf,
    pub profile: config::Profile,
}

// ---------------------------------------------------------------------------
// list_features
// ---------------------------------------------------------------------------

/// List all components visible from the current source roots.
///
/// If `source_filter` is `Some("local")`, only components whose ID starts with
/// `"local/"` are returned.  Pass `None` to return all components.
/// Results are sorted by component ID.
pub fn list_components(
    ctx: &AppContext,
    source_filter: Option<&str>,
) -> Result<Vec<ComponentSummary>, AppError> {
    let sources = load_sources_optional(ctx)?;
    let roots = build_source_roots(ctx, &sources);
    let fi_platform = to_ci_platform(&ctx.platform);
    let index = component_index::build(&roots, &fi_platform)?;

    let mut summaries: Vec<ComponentSummary> = index
        .components
        .into_iter()
        .filter(|(id, _)| match source_filter {
            Some(filter) => id.starts_with(&format!("{filter}/")),
            None => true,
        })
        .map(|(id, meta)| ComponentSummary {
            id,
            mode: meta.mode,
            description: meta.description,
        })
        .collect();

    summaries.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(summaries)
}

// ---------------------------------------------------------------------------
// show_feature
// ---------------------------------------------------------------------------

/// Load and return full detail for the component with the given canonical ID.
///
/// Returns [`AppError::ComponentNotFound`] if the ID is not in the index.
pub fn show_component(ctx: &AppContext, id: &str) -> Result<ComponentDetail, AppError> {
    let sources = load_sources_optional(ctx)?;
    let roots = build_source_roots(ctx, &sources);
    let fi_platform = to_ci_platform(&ctx.platform);
    let mut index = component_index::build(&roots, &fi_platform)?;

    let meta = index
        .components
        .remove(id)
        .ok_or_else(|| AppError::ComponentNotFound { id: id.to_string() })?;

    Ok(ComponentDetail {
        id: id.to_string(),
        meta,
    })
}

// ---------------------------------------------------------------------------
// list_backends
// ---------------------------------------------------------------------------

/// List all script backends visible from the current source roots.
///
/// If `source_filter` is `Some("local")`, only backends whose ID starts with
/// `"local/"` are returned.  Pass `None` to return all backends.
/// Results are sorted by backend ID.
pub fn list_backends(
    ctx: &AppContext,
    source_filter: Option<&str>,
) -> Result<Vec<BackendSummary>, AppError> {
    let sources = load_sources_optional(ctx)?;
    let dirs = scan_backend_dirs_impl(ctx, &sources);

    let mut summaries: Vec<BackendSummary> = dirs
        .into_iter()
        .filter(|(id, _)| match source_filter {
            Some(filter) => id.starts_with(&format!("{filter}/")),
            None => true,
        })
        .map(|(id, path)| {
            let source = id.split('/').next().unwrap_or("unknown").to_string();
            let api_version = read_api_version(&path);
            BackendSummary {
                id,
                source,
                dir: path.display().to_string(),
                api_version,
            }
        })
        .collect();

    summaries.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(summaries)
}

// ---------------------------------------------------------------------------
// show_backend
// ---------------------------------------------------------------------------

/// Load and return full detail for the backend with the given canonical ID.
///
/// Returns [`AppError::BackendNotFound`] if the ID is not found in any source root.
pub fn show_backend(ctx: &AppContext, id: &str) -> Result<BackendDetail, AppError> {
    // Validate format: must be `source/name`.
    let parts: Vec<&str> = id.splitn(2, '/').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        return Err(AppError::BackendNotFound { id: id.to_string() });
    }
    let source = parts[0].to_string();

    let sources = load_sources_optional(ctx)?;
    let dirs = scan_backend_dirs_impl(ctx, &sources);

    let (_, dir) = dirs
        .into_iter()
        .find(|(bid, _)| bid == id)
        .ok_or_else(|| AppError::BackendNotFound { id: id.to_string() })?;

    let api_version = read_api_version(&dir);
    let ext = platform_script_ext(&ctx.platform);
    let scripts = BackendScripts {
        apply: has_script(&dir, "apply", ext),
        remove: has_script(&dir, "remove", ext),
        status: has_script(&dir, "status", ext),
        env_pre: has_script(&dir, "env_pre", ext),
        env_post: has_script(&dir, "env_post", ext),
    };

    Ok(BackendDetail {
        id: id.to_string(),
        source,
        dir: dir.display().to_string(),
        api_version,
        scripts,
    })
}

// ---------------------------------------------------------------------------
// list_sources
// ---------------------------------------------------------------------------

/// List all sources: implicit (`core`, `local`) + declared external sources.
pub fn list_sources(ctx: &AppContext) -> Result<Vec<SourceSummary>, AppError> {
    let sources = load_sources_optional(ctx)?;
    let lock = load_lock_optional(ctx);

    let mut entries = vec![
        SourceSummary {
            id: "core".to_string(),
            kind: "implicit".to_string(),
            url: None,
            allow: Some("*".to_string()),
            local_path: None,
            ref_spec: None,
            resolved_commit: None,
            fetched_at: None,
        },
        SourceSummary {
            id: "local".to_string(),
            kind: "implicit".to_string(),
            url: None,
            allow: Some("*".to_string()),
            local_path: Some(ctx.local_root.display().to_string()),
            ref_spec: None,
            resolved_commit: None,
            fetched_at: None,
        },
    ];

    for entry in &sources.sources {
        entries.push(build_external_source_summary(ctx, entry, &lock));
    }

    Ok(entries)
}

// ---------------------------------------------------------------------------
// show_source
// ---------------------------------------------------------------------------

/// Return detail for a single source by ID.
///
/// `"core"` and `"local"` are always available as implicit sources.
/// External source IDs must be declared in `sources.yaml`.
/// Returns [`AppError::SourceNotFound`] if the ID is unknown.
pub fn show_source(ctx: &AppContext, id: &str) -> Result<SourceSummary, AppError> {
    match id {
        "core" => Ok(SourceSummary {
            id: "core".to_string(),
            kind: "implicit".to_string(),
            url: None,
            allow: Some("*".to_string()),
            local_path: None,
            ref_spec: None,
            resolved_commit: None,
            fetched_at: None,
        }),
        "local" => Ok(SourceSummary {
            id: "local".to_string(),
            kind: "implicit".to_string(),
            url: None,
            allow: Some("*".to_string()),
            local_path: Some(ctx.local_root.display().to_string()),
            ref_spec: None,
            resolved_commit: None,
            fetched_at: None,
        }),
        other => {
            let sources = load_sources_optional(ctx)?;
            let entry = sources
                .sources
                .iter()
                .find(|e| e.id == other)
                .ok_or_else(|| AppError::SourceNotFound {
                    id: other.to_string(),
                })?;
            let lock = load_lock_optional(ctx);
            Ok(build_external_source_summary(ctx, entry, &lock))
        }
    }
}

// ---------------------------------------------------------------------------
// list_configs
// ---------------------------------------------------------------------------

/// List all config files under `{config_home}/configs/`.
///
/// `active` should be the current context name (from `{config_home}/current`),
/// or `None` if no context is set.  The matching entry will have `active: true`.
/// Results are sorted by config name.
pub fn list_configs(ctx: &AppContext, active: Option<&str>) -> Vec<ConfigSummary> {
    let configs_dir = ctx.dirs.config_home.join("configs");
    if !configs_dir.exists() {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(&configs_dir) else {
        return Vec::new();
    };

    let mut result: Vec<ConfigSummary> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let ext = path.extension()?.to_str()?;
            if ext != "yaml" && ext != "yml" {
                return None;
            }
            let name = path.file_stem()?.to_str()?.to_string();
            Some(ConfigSummary {
                active: active == Some(name.as_str()),
                path: path.display().to_string(),
                name,
            })
        })
        .collect();

    result.sort_by(|a, b| a.name.cmp(&b.name));
    result
}

// ---------------------------------------------------------------------------
// show_config
// ---------------------------------------------------------------------------

/// Load and return detail for the config at `path`.
///
/// Returns [`AppError::ConfigNotFound`] if the file does not exist.
/// The config name is derived from the file stem of `path`.
pub fn show_config(ctx: &AppContext, path: &Path) -> Result<ConfigDetail, AppError> {
    let _ = ctx; // ctx reserved for future use (e.g. schema validation against platform)
    if !path.exists() {
        return Err(AppError::ConfigNotFound {
            path: path.to_path_buf(),
        });
    }
    let (profile, _) = config::load_config(path)?;
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    Ok(ConfigDetail {
        name,
        path: path.to_path_buf(),
        profile,
    })
}

// ---------------------------------------------------------------------------
// show_state
// ---------------------------------------------------------------------------

/// Load and return the current authoritative state.
///
/// Returns an empty state if `state.json` does not exist yet.
pub fn show_state(ctx: &AppContext) -> Result<state::State, AppError> {
    Ok(state::load(&ctx.state_path())?)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Enumerate all script-backend directories across all source roots.
fn scan_backend_dirs_impl(
    ctx: &AppContext,
    sources: &config::SourcesSpec,
) -> Vec<(String, PathBuf)> {
    let mut result = Vec::new();
    scan_one_backend_source(&mut result, "local", &ctx.local_root.join("backends"));
    for entry in &sources.sources {
        let backends_dir = match entry.source_type {
            config::SourceType::Git => ctx
                .dirs
                .data_home
                .join("sources")
                .join(&entry.id)
                .join("backends"),
            config::SourceType::Path => {
                // path is pre-resolved to absolute by config::load_sources.
                let Some(ref p) = entry.path else { continue };
                std::path::Path::new(p).join("backends")
            }
        };
        scan_one_backend_source(&mut result, &entry.id, &backends_dir);
    }
    result.sort_by(|a, b| a.0.cmp(&b.0));
    result
}

fn scan_one_backend_source(
    result: &mut Vec<(String, PathBuf)>,
    source_id: &str,
    backends_dir: &Path,
) {
    if !backends_dir.exists() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(backends_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if !path.join("backend.yaml").exists() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        result.push((format!("{source_id}/{name}"), path));
    }
}

/// Read `api_version` from `backend.yaml`, returning 0 if absent or unreadable.
fn read_api_version(dir: &Path) -> u32 {
    let yaml_path = dir.join("backend.yaml");
    let Ok(content) = std::fs::read_to_string(&yaml_path) else {
        return 0;
    };
    for line in content.lines() {
        if let Some(rest) = line.trim().strip_prefix("api_version:") {
            if let Ok(n) = rest.trim().parse::<u32>() {
                return n;
            }
        }
    }
    0
}

fn has_script(dir: &Path, name: &str, ext: &str) -> bool {
    dir.join(format!("{name}.{ext}")).exists()
}

fn platform_script_ext(platform: &platform::Platform) -> &'static str {
    match platform {
        platform::Platform::Windows => "ps1",
        _ => "sh",
    }
}

/// Format an `AllowSpec` into a human-readable string.
fn format_allow(allow: &Option<model::sources::AllowSpec>) -> Option<String> {
    match allow {
        None => None,
        Some(model::sources::AllowSpec::All(_)) => Some("*".to_string()),
        Some(model::sources::AllowSpec::Detailed(d)) => {
            let features = d.components.as_ref().map(|l| match l {
                model::sources::AllowList::All(_) => "components:*".to_string(),
                model::sources::AllowList::Names(v) => format!("components:[{}]", v.join(",")),
            });
            let backends = d.backends.as_ref().map(|l| match l {
                model::sources::AllowList::All(_) => "backends:*".to_string(),
                model::sources::AllowList::Names(v) => format!("backends:[{}]", v.join(",")),
            });
            Some(
                [features, backends]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        }
    }
}

/// Return the `kind` label and resolved `local_path` string for an external source entry.
fn source_kind_and_path(ctx: &AppContext, entry: &config::SourceEntry) -> (String, String) {
    match entry.source_type {
        config::SourceType::Git => {
            let kind = "git".to_string();
            let local_path = ctx
                .dirs
                .data_home
                .join("sources")
                .join(&entry.id)
                .display()
                .to_string();
            (kind, local_path)
        }
        config::SourceType::Path => {
            let kind = "path".to_string();
            // path is pre-resolved to absolute by config::load_sources.
            // Canonicalize for display so that paths with `..` components
            // (e.g. `/root/../tmp/repo`) are shown as their clean real path.
            let raw = entry.path.clone().unwrap_or_else(|| entry.id.clone());
            let local_path = std::path::Path::new(&raw)
                .canonicalize()
                .map(|p| p.display().to_string())
                .unwrap_or(raw);
            (kind, local_path)
        }
    }
}

/// Build a `SourceSummary` for an external (non-implicit) source entry.
///
/// Looks up the lock entry for `type: git` sources to populate
/// `resolved_commit` and `fetched_at`.
fn build_external_source_summary(
    ctx: &AppContext,
    entry: &config::SourceEntry,
    lock: &config::SourcesLock,
) -> SourceSummary {
    let (kind, local_path) = source_kind_and_path(ctx, entry);
    let ref_spec = ref_spec_label(&entry.source_ref);
    let lock_entry = lock.sources.get(&entry.id);
    SourceSummary {
        id: entry.id.clone(),
        kind,
        url: entry.url.clone(),
        allow: format_allow(&entry.allow),
        local_path: Some(local_path),
        ref_spec,
        resolved_commit: lock_entry.map(|l| l.resolved_commit.clone()),
        fetched_at: lock_entry.map(|l| l.fetched_at.clone()),
    }
}

/// Format a `SourceRef` into a human-readable label.
///
/// Returns `"branch:<name>"`, `"tag:<name>"`, or `"commit:<hash>"`,
/// depending on which field is set.
/// Returns `None` if the ref is absent or all fields are unset.
fn ref_spec_label(source_ref: &Option<config::SourceRef>) -> Option<String> {
    source_ref.as_ref().and_then(|r| {
        r.branch
            .as_ref()
            .map(|b| format!("branch:{b}"))
            .or_else(|| r.tag.as_ref().map(|t| format!("tag:{t}")))
            .or_else(|| r.commit.as_ref().map(|c| format!("commit:{c}")))
    })
}

/// Load the sources lock file; return an empty lock on absence or error.
///
/// The lock file is advisory for display purposes; absence is not an error.
fn load_lock_optional(ctx: &AppContext) -> config::SourcesLock {
    config::load_sources_lock(&ctx.sources_lock_path()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_spec_label_branch() {
        let r = Some(config::SourceRef {
            branch: Some("main".into()),
            ..Default::default()
        });
        assert_eq!(ref_spec_label(&r), Some("branch:main".to_string()));
    }

    #[test]
    fn ref_spec_label_tag() {
        let r = Some(config::SourceRef {
            tag: Some("v1.0".into()),
            ..Default::default()
        });
        assert_eq!(ref_spec_label(&r), Some("tag:v1.0".to_string()));
    }

    #[test]
    fn ref_spec_label_commit() {
        let r = Some(config::SourceRef {
            commit: Some("abc123def456".into()),
            ..Default::default()
        });
        assert_eq!(ref_spec_label(&r), Some("commit:abc123def456".to_string()));
    }

    #[test]
    fn ref_spec_label_none() {
        assert_eq!(ref_spec_label(&None), None);
    }

    #[test]
    fn ref_spec_label_empty_struct() {
        assert_eq!(ref_spec_label(&Some(config::SourceRef::default())), None);
    }
}
