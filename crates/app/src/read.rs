// Read-only helpers for CLI read commands (feature list, backend list, etc.).

use crate::context::{AppContext, AppError};
use crate::pipeline::{build_source_roots, load_sources_optional, to_fi_platform};

/// Load sources spec for read-only commands (`source list`, `feature list`, etc.).
///
/// Returns an empty `SourcesSpec` if `sources.yaml` does not exist.
pub fn load_sources(ctx: &AppContext) -> Result<config::SourcesSpec, AppError> {
    load_sources_optional(ctx)
}

/// Build a `FeatureIndex` from all available source roots.
///
/// Used by `loadout feature list/show`. For the full plan/apply pipeline,
/// use `plan()` or `prepare_execution()` instead.
pub fn build_feature_index(
    ctx: &AppContext,
    sources: &config::SourcesSpec,
) -> Result<model::FeatureIndex, AppError> {
    let roots = build_source_roots(ctx, sources);
    let fi_platform = to_fi_platform(&ctx.platform);
    Ok(feature_index::build(&roots, &fi_platform)?)
}

/// Enumerate all script-backend directories across all source roots.
///
/// Returns `(canonical_id, directory)` pairs sorted by canonical ID.
/// Only directories that contain a `backend.yaml` file are included.
/// The `core` built-in source has no filesystem directory and is not enumerated here.
pub fn scan_backend_dirs(
    ctx: &AppContext,
    sources: &config::SourcesSpec,
) -> Vec<(String, std::path::PathBuf)> {
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
    result: &mut Vec<(String, std::path::PathBuf)>,
    source_id: &str,
    backends_dir: &std::path::Path,
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
