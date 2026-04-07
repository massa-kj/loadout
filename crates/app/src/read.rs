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
use crate::pipeline::{build_source_roots, load_sources_optional, to_fi_platform};

// ---------------------------------------------------------------------------
// Feature types
// ---------------------------------------------------------------------------

/// Summary of a single feature for list output.
#[derive(Debug, Serialize)]
pub struct FeatureSummary {
    pub id: String,
    pub mode: model::feature_index::FeatureMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Full detail of a single feature for show output.
#[derive(Debug, Serialize)]
pub struct FeatureDetail {
    pub id: String,
    #[serde(flatten)]
    pub meta: model::feature_index::FeatureMeta,
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

/// List all features visible from the current source roots.
///
/// If `source_filter` is `Some("local")`, only features whose ID starts with
/// `"local/"` are returned.  Pass `None` to return all features.
/// Results are sorted by feature ID.
pub fn list_features(
    ctx: &AppContext,
    source_filter: Option<&str>,
) -> Result<Vec<FeatureSummary>, AppError> {
    let sources = load_sources_optional(ctx)?;
    let roots = build_source_roots(ctx, &sources);
    let fi_platform = to_fi_platform(&ctx.platform);
    let index = feature_index::build(&roots, &fi_platform)?;

    let mut summaries: Vec<FeatureSummary> = index
        .features
        .into_iter()
        .filter(|(id, _)| match source_filter {
            Some(filter) => id.starts_with(&format!("{filter}/")),
            None => true,
        })
        .map(|(id, meta)| FeatureSummary {
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

/// Load and return full detail for the feature with the given canonical ID.
///
/// Returns [`AppError::FeatureNotFound`] if the ID is not in the index.
pub fn show_feature(ctx: &AppContext, id: &str) -> Result<FeatureDetail, AppError> {
    let sources = load_sources_optional(ctx)?;
    let roots = build_source_roots(ctx, &sources);
    let fi_platform = to_fi_platform(&ctx.platform);
    let mut index = feature_index::build(&roots, &fi_platform)?;

    let meta = index
        .features
        .remove(id)
        .ok_or_else(|| AppError::FeatureNotFound { id: id.to_string() })?;

    Ok(FeatureDetail {
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

    let mut entries = vec![
        SourceSummary {
            id: "core".to_string(),
            kind: "implicit".to_string(),
            url: None,
            allow: Some("*".to_string()),
            local_path: None,
        },
        SourceSummary {
            id: "local".to_string(),
            kind: "implicit".to_string(),
            url: None,
            allow: Some("*".to_string()),
            local_path: Some(ctx.local_root.display().to_string()),
        },
    ];

    for entry in &sources.sources {
        let (kind, local_path) = source_kind_and_path(ctx, entry);
        entries.push(SourceSummary {
            id: entry.id.clone(),
            kind,
            url: entry.url.clone(),
            allow: format_allow(&entry.allow),
            local_path: Some(local_path),
        });
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
        }),
        "local" => Ok(SourceSummary {
            id: "local".to_string(),
            kind: "implicit".to_string(),
            url: None,
            allow: Some("*".to_string()),
            local_path: Some(ctx.local_root.display().to_string()),
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
            let (kind, local_path) = source_kind_and_path(ctx, entry);
            Ok(SourceSummary {
                id: entry.id.clone(),
                kind,
                url: entry.url.clone(),
                allow: format_allow(&entry.allow),
                local_path: Some(local_path),
            })
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
        let dir = ctx
            .dirs
            .data_home
            .join("sources")
            .join(&entry.id)
            .join("backends");
        scan_one_backend_source(&mut result, &entry.id, &dir);
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
            let features = d.features.as_ref().map(|l| match l {
                model::sources::AllowList::All(_) => "features:*".to_string(),
                model::sources::AllowList::Names(v) => format!("features:[{}]", v.join(",")),
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
            let local_path = entry.path.clone().unwrap_or_else(|| entry.id.clone());
            (kind, local_path)
        }
    }
}
