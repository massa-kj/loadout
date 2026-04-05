// Shared read-only pipeline stages used by plan(), prepare_execution(), and read helpers.

use std::path::Path;

use crate::context::{AppContext, AppError};

/// Outputs from the common read-only pipeline stages.
pub(crate) struct PipelineOutput {
    #[allow(dead_code)]
    pub(crate) profile: config::Profile,
    #[allow(dead_code)]
    pub(crate) strategy: config::Strategy,
    pub(crate) index: model::FeatureIndex,
    pub(crate) order: model::ResolvedFeatureOrder,
    pub(crate) graph: model::desired_resource_graph::DesiredResourceGraph,
    pub(crate) state: state::State,
}

/// Run the read-only stages common to both `plan()` and `apply()`.
///
/// Steps:
///   1. Validate config file exists.
///   2. Load config → `Profile` + `Strategy` via `config::load_config`.
///   3. Load sources (optional) → `SourcesSpec`.
///   4. Build `FeatureIndex` from source roots.
///   5. Map profile features to `CanonicalFeatureId`s (skip unknown).
///   6. Resolve dependency order.
///   7. Compile: `FeatureIndex + Strategy + order` → `DesiredResourceGraph`.
///   8. Load (or initialise) state.
pub(crate) fn run_pipeline(
    ctx: &AppContext,
    config_path: &Path,
) -> Result<PipelineOutput, AppError> {
    // Step 1: config file must exist.
    if !config_path.exists() {
        return Err(AppError::ConfigNotFound {
            path: config_path.to_path_buf(),
        });
    }

    // Step 2: load config — profile is required, strategy is optional (defaults to
    // Strategy::default() if the 'strategy' section is absent from the file).
    let (profile, strategy) = config::load_config(config_path)?;

    // Step 3: load sources if present; default to empty (core + local only).
    let sources = load_sources_optional(ctx)?;

    // Step 4: build feature index from all source roots.
    let source_roots = build_source_roots(ctx, &sources);
    let fi_platform = to_fi_platform(&ctx.platform);
    let index = feature_index::build(&source_roots, &fi_platform)?;

    // Step 5: convert profile keys to CanonicalFeatureIds; skip those absent from index.
    // An empty desired list is valid: it means "uninstall everything in state".
    let desired_ids = profile_to_desired_ids(&profile, &index);

    // Step 6: resolve dependency order (topological sort).
    let order = resolver::resolve(&index, &desired_ids)?;

    // Step 7: compile desired resource graph.
    let graph = compiler::compile(&index, &strategy, &order)?;

    // Step 8: load state (state::load returns empty state if file absent).
    let state = state::load(&ctx.state_path())?;

    Ok(PipelineOutput {
        profile,
        strategy,
        index,
        order,
        graph,
        state,
    })
}

/// Map `platform::Platform` → `feature_index::Platform`.
pub(crate) fn to_fi_platform(p: &platform::Platform) -> feature_index::Platform {
    match p {
        platform::Platform::Linux => feature_index::Platform::Linux,
        platform::Platform::Windows => feature_index::Platform::Windows,
        platform::Platform::Wsl => feature_index::Platform::Wsl,
    }
}

/// Build the list of feature source roots for the feature-index scanner.
///
/// Implicit sources:
/// - `local` → `{local_root}/features/`
///
/// (The `core` source is embedded in the binary; no filesystem path applies.)
///
/// External sources (from `sources.yaml`):
/// - `{id}` → `{data_home}/sources/{id}/features/`
pub(crate) fn build_source_roots(
    ctx: &AppContext,
    sources: &config::SourcesSpec,
) -> Vec<feature_index::SourceRoot> {
    let mut roots = vec![feature_index::SourceRoot {
        source_id: "local".into(),
        features_dir: ctx.local_root.join("features"),
    }];

    for entry in &sources.sources {
        roots.push(feature_index::SourceRoot {
            source_id: entry.id.clone(),
            features_dir: ctx
                .dirs
                .data_home
                .join("sources")
                .join(&entry.id)
                .join("features"),
        });
    }

    roots
}

/// Load the sources spec; return an empty `SourcesSpec` if the file is absent.
///
/// If `ctx.sources_override` is set, that path is used exclusively (no fallback).
/// This mirrors the `--sources` CLI flag, intended for CI / verification use only.
pub(crate) fn load_sources_optional(ctx: &AppContext) -> Result<config::SourcesSpec, AppError> {
    if let Some(ref path) = ctx.sources_override {
        return Ok(config::load_sources(path)?);
    }
    let path = ctx.sources_path();
    if path.exists() {
        Ok(config::load_sources(&path)?)
    } else {
        Ok(config::SourcesSpec::default())
    }
}

/// Map profile feature keys (already normalised by `config::load_profile`) to
/// `CanonicalFeatureId`s, keeping only those present in the feature index.
///
/// Features absent from the index may belong to a source that has not been
/// cloned yet; they are silently skipped rather than returning an error,
/// so that a machine with a partial set of sources can still make progress.
pub(crate) fn profile_to_desired_ids(
    profile: &config::Profile,
    index: &model::FeatureIndex,
) -> Vec<model::CanonicalFeatureId> {
    let mut ids: Vec<model::CanonicalFeatureId> = profile
        .features
        .keys()
        .filter(|k| index.features.contains_key(k.as_str()))
        .filter_map(|k| model::CanonicalFeatureId::new(k).ok())
        .collect();

    // Sort for deterministic order (profile is a HashMap, iteration order varies).
    ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    ids
}
