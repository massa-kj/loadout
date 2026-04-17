// Shared read-only pipeline stages used by plan(), prepare_execution(), and read helpers.

use std::path::Path;

use crate::context::{AppContext, AppError};
use crate::materializer;

/// Outputs from the common read-only pipeline stages.
pub(crate) struct PipelineOutput {
    #[allow(dead_code)]
    pub(crate) profile: config::Profile,
    #[allow(dead_code)]
    pub(crate) strategy: config::Strategy,
    pub(crate) index: model::ComponentIndex,
    pub(crate) order: model::ResolvedComponentOrder,
    pub(crate) graph: model::desired_resource_graph::DesiredResourceGraph,
    pub(crate) state: state::State,
}

/// Run the read-only stages common to both `plan()` and `apply()`.
///
/// Steps:
///   1. Validate config file exists.
///   2. Load config → `Profile` + `Strategy` via `config::load_config`.
///   3. Load sources (optional) → `SourcesSpec`.
///   4. Build `ComponentIndex` from source roots.
///   5. Map profile components to `CanonicalComponentId`s (skip unknown).
///   6. Resolve dependency order.
///   7. Materialize fs sources (resolve defaults, validate, compute fingerprints).
///   8. Compile: `ComponentIndex + Strategy + order + materialized` → `DesiredResourceGraph`.
///   9. Load (or initialise) state.
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

    // Step 4: build component index from all source roots.
    let source_roots = build_source_roots(ctx, &sources);
    let fi_platform = to_ci_platform(&ctx.platform);
    let index = component_index::build(&source_roots, &fi_platform)?;

    // Step 5: convert profile keys to CanonicalComponentIds; skip those absent from index.
    // An empty desired list is valid: it means "uninstall everything in state".
    let desired_ids = profile_to_desired_ids(&profile, &index);

    // Step 6: resolve dependency order (topological sort).
    let order = resolver::resolve(&index, &desired_ids)?;

    // Step 7: materialize fs sources (impure: resolves defaults, validates paths,
    // computes fingerprints for eligible sources).
    let materialized = materializer::materialize_fs_sources(&index)?;

    // Step 8: compile desired resource graph (pure: uses materialized sources).
    let compiler_ms = to_compiler_materialized(&materialized);
    let graph = compiler::compile(&index, &strategy, &order, &compiler_ms)?;

    // Step 9: load state (state::load returns empty state if file absent).
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

/// Convert app-level materialized sources into compiler-level materialized sources.
fn to_compiler_materialized(
    app_ms: &materializer::MaterializedSources,
) -> compiler::MaterializedSources {
    app_ms
        .iter()
        .map(|(key, val)| {
            (
                key.clone(),
                compiler::MaterializedFsResource {
                    source: val.source.clone(),
                    source_fingerprint: val.source_fingerprint.clone(),
                    expanded_path: val.expanded_path.clone(),
                },
            )
        })
        .collect()
}

/// Map `platform::Platform` → `component_index::Platform`.
pub(crate) fn to_ci_platform(p: &platform::Platform) -> component_index::Platform {
    match p {
        platform::Platform::Linux => component_index::Platform::Linux,
        platform::Platform::Windows => component_index::Platform::Windows,
        platform::Platform::Wsl => component_index::Platform::Wsl,
    }
}

/// Build the list of component source roots for the component-index scanner.
///
/// Implicit sources:
/// - `local` → `{local_root}/components/`
///
/// (The `core` source is embedded in the binary; no filesystem path applies.)
///
/// External sources (from `sources.yaml`):
/// - `{id}` → `{data_home}/sources/{id}/components/`
pub(crate) fn build_source_roots(
    ctx: &AppContext,
    sources: &config::SourcesSpec,
) -> Vec<component_index::SourceRoot> {
    let mut roots = vec![component_index::SourceRoot {
        source_id: "local".into(),
        components_dir: ctx.local_root.join("components"),
    }];

    for entry in &sources.sources {
        let components_dir = match entry.source_type {
            config::SourceType::Git => ctx
                .dirs
                .data_home
                .join("sources")
                .join(&entry.id)
                .join("components"),
            config::SourceType::Path => {
                // path is pre-resolved to absolute by config::load_sources.
                let Some(ref p) = entry.path else { continue };
                std::path::Path::new(p).join("components")
            }
        };
        roots.push(component_index::SourceRoot {
            source_id: entry.id.clone(),
            components_dir,
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

/// Map profile component keys (already normalised by `config::load_profile`) to
/// `CanonicalComponentId`s, keeping only those present in the component index.
///
/// Components absent from the index may belong to a source that has not been
/// cloned yet; they are silently skipped rather than returning an error,
/// so that a machine with a partial set of sources can still make progress.
pub(crate) fn profile_to_desired_ids(
    profile: &config::Profile,
    index: &model::ComponentIndex,
) -> Vec<model::CanonicalComponentId> {
    let mut ids: Vec<model::CanonicalComponentId> = profile
        .components
        .keys()
        .filter(|k| index.components.contains_key(k.as_str()))
        .filter_map(|k| model::CanonicalComponentId::new(k).ok())
        .collect();

    // Sort for deterministic order (profile is a HashMap, iteration order varies).
    ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    ids
}
