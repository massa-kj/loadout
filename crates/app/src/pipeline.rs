// Shared read-only pipeline stages used by plan(), prepare_execution(), and read helpers.

use std::collections::HashMap;
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
    /// Desired-only topological order (used by compiler, executor, and public API).
    pub(crate) order: model::ResolvedComponentOrder,
    /// Full order: desired + resolvable state-only components (used by planner for
    /// correct reverse destroy ordering when both a component and its dependency are removed).
    pub(crate) full_order: model::ResolvedComponentOrder,
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
///
/// Note: state is loaded at step 5.5 (before resolver) so that state component IDs can be
/// included in the resolver's extended order. This enables correct reverse destroy ordering
/// when both a component and its dependency are removed from the profile simultaneously.
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

    // Step 5.5: load state early so that state-only component IDs can participate in
    // dependency-order resolution. This ensures destroy ordering is correct when both
    // a component and its dependency are removed from the profile simultaneously.
    let state = state::load(&ctx.state_path())?;

    // Step 6: resolve dependency order (topological sort) for a combined set of desired
    // and state-only components. State-only components whose yaml is gone are silently
    // excluded; their missing dependencies are treated as soft (skipped).
    //
    // full_order = desired + resolvable state-only components (used by planner for destroy order)
    // order      = desired-only subset (used by compiler, validate_and_materialize_params, executor)
    let desired_id_set: std::collections::HashSet<&str> =
        desired_ids.iter().map(|id| id.as_str()).collect();
    let state_extras: Vec<model::CanonicalComponentId> = state
        .components
        .keys()
        .filter(|k| !desired_id_set.contains(k.as_str()))
        .filter_map(|k| model::CanonicalComponentId::new(k).ok())
        .collect();
    let full_order = resolver::resolve_extended(&index, &desired_ids, &state_extras)?;
    // Derive the desired-only order by filtering full_order, preserving install sequence.
    let order: model::ResolvedComponentOrder = full_order
        .iter()
        .filter(|id| desired_id_set.contains(id.as_str()))
        .cloned()
        .collect();

    // Step 7: materialize fs sources (impure: resolves defaults, validates paths,
    // computes fingerprints according to fingerprint_policy).
    let fingerprint_policy = strategy
        .fs
        .as_ref()
        .and_then(|fs| fs.fingerprint_policy)
        .unwrap_or_default();
    let materialized = materializer::materialize_fs_sources(&index, fingerprint_policy)?;

    // Step 7.5: validate params and materialize component specs.
    // For each desired component with a params_schema, validate profile params
    // against the schema, resolve defaults, then replace ${params.*} references
    // in the component's resource templates.
    // Uses desired-only order so that state-only components are not processed.
    let materialized_specs = validate_and_materialize_params(&profile, &index, &order)?;

    // Step 8: compile desired resource graph (pure: uses materialized sources).
    // Uses desired-only order to avoid adding state-only components to the desired graph.
    let compiler_ms = to_compiler_materialized(&materialized);
    let graph = compiler::compile(&index, &materialized_specs, &strategy, &order, &compiler_ms)?;

    Ok(PipelineOutput {
        profile,
        strategy,
        index,
        order,
        full_order,
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

/// Validate profile params against component schemas and materialize specs.
///
/// For each component in `resolved_order`:
/// 1. Look up the component's `params_schema` from the index.
/// 2. Look up the profile's `params` for that component.
/// 3. Validate and resolve defaults via `params_validator::validate_and_resolve`.
/// 4. Materialize `${params.*}` references via `params_materializer::materialize`.
///
/// Components without `params_schema` and without profile params are skipped.
/// The result maps component IDs to their param-resolved specs.
fn validate_and_materialize_params(
    profile: &config::Profile,
    index: &model::ComponentIndex,
    order: &model::ResolvedComponentOrder,
) -> Result<HashMap<String, model::params::MaterializedComponentSpec>, AppError> {
    let mut result = HashMap::new();

    for component_id in order {
        let id_str = component_id.as_str();

        let meta = match index.components.get(id_str) {
            Some(m) => m,
            None => continue, // handled by compiler
        };

        let profile_params = profile
            .components
            .get(id_str)
            .and_then(|c| c.params.as_ref());

        let resolved = params_validator::validate_and_resolve(
            id_str,
            meta.params_schema.as_ref(),
            profile_params,
        )?;

        // Only materialize if the component has a spec with resources.
        if resolved.values.is_empty() {
            continue;
        }

        let spec = match &meta.spec {
            Some(s) => s,
            None => continue,
        };

        let materialized = params_materializer::materialize(id_str, &spec.resources, &resolved)?;
        result.insert(id_str.to_string(), materialized);
    }

    Ok(result)
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

#[cfg(test)]
mod tests {
    use super::*;
    use model::{
        component_index::{
            ComponentIndex, ComponentMeta, ComponentMode, ComponentSpec, DepSpec, SpecResource,
            SpecResourceKind,
        },
        params::{ParamProperty, ParamType, ParamValue, ParamsSchema},
        profile::{Profile, ProfileComponentConfig},
        CanonicalComponentId,
    };
    use std::collections::HashMap;

    // --- Helpers ---

    fn make_rt_meta(version_template: &str, schema: Option<ParamsSchema>) -> ComponentMeta {
        ComponentMeta {
            spec_version: 1,
            mode: ComponentMode::Declarative,
            description: None,
            source_dir: "/tmp".to_string(),
            dep: DepSpec::default(),
            params_schema: schema,
            spec: Some(ComponentSpec {
                resources: vec![SpecResource {
                    id: "rt:test".to_string(),
                    kind: SpecResourceKind::Runtime {
                        name: "test".to_string(),
                        version: version_template.to_string(),
                    },
                }],
            }),
            scripts: None,
        }
    }

    fn make_index(id: &str, meta: ComponentMeta) -> ComponentIndex {
        let mut components = HashMap::new();
        components.insert(id.to_string(), meta);
        ComponentIndex {
            schema_version: 1,
            components,
        }
    }

    fn make_profile(id: &str, params: Option<HashMap<String, ParamValue>>) -> Profile {
        let mut components = HashMap::new();
        components.insert(id.to_string(), ProfileComponentConfig { params });
        Profile { components }
    }

    fn make_order(id: &str) -> model::ResolvedComponentOrder {
        vec![CanonicalComponentId::new(id).unwrap()]
    }

    fn make_schema_with_default(default_ver: &str) -> ParamsSchema {
        let mut props = HashMap::new();
        props.insert(
            "version".to_string(),
            ParamProperty {
                param_type: ParamType::String,
                default: Some(ParamValue::String(default_ver.to_string())),
            },
        );
        ParamsSchema {
            properties: props,
            required: vec![],
            additional_properties: false,
        }
    }

    // --- Tests ---

    /// Profile params override the template reference in the spec.
    #[test]
    fn profile_params_are_materialized_into_spec() {
        let schema = make_schema_with_default("1.0");
        let meta = make_rt_meta("${params.version}", Some(schema));
        let index = make_index("local/test-comp", meta);

        let mut params_map = HashMap::new();
        params_map.insert("version".to_string(), ParamValue::String("2.0".to_string()));
        let profile = make_profile("local/test-comp", Some(params_map));
        let order = make_order("local/test-comp");

        let result = validate_and_materialize_params(&profile, &index, &order).unwrap();

        let mcs = result
            .get("local/test-comp")
            .expect("materialized spec must be present");
        match &mcs.resources[0].kind {
            SpecResourceKind::Runtime { version, .. } => {
                assert_eq!(version, "2.0", "profile param must override template");
            }
            _ => panic!("expected Runtime resource"),
        }
    }

    /// Default value is applied when the profile omits params entirely.
    #[test]
    fn default_param_used_when_profile_omits_params() {
        let schema = make_schema_with_default("1.0");
        let meta = make_rt_meta("${params.version}", Some(schema));
        let index = make_index("local/test-comp", meta);
        let profile = make_profile("local/test-comp", None);
        let order = make_order("local/test-comp");

        let result = validate_and_materialize_params(&profile, &index, &order).unwrap();

        let mcs = result
            .get("local/test-comp")
            .expect("materialized spec must be present when default is available");
        match &mcs.resources[0].kind {
            SpecResourceKind::Runtime { version, .. } => {
                assert_eq!(version, "1.0", "schema default must be applied");
            }
            _ => panic!("expected Runtime resource"),
        }
    }

    /// Providing params to a component that has no schema is an error.
    #[test]
    fn params_without_schema_returns_validation_error() {
        let meta = make_rt_meta("1.0", None);
        let index = make_index("local/test-comp", meta);

        let mut params_map = HashMap::new();
        params_map.insert("version".to_string(), ParamValue::String("2.0".to_string()));
        let profile = make_profile("local/test-comp", Some(params_map));
        let order = make_order("local/test-comp");

        let err = validate_and_materialize_params(&profile, &index, &order).unwrap_err();
        assert!(
            matches!(err, AppError::ParamsValidation(_)),
            "expected ParamsValidation error, got {err:?}"
        );
    }

    /// Component without schema and without profile params produces no materialized spec.
    #[test]
    fn component_without_schema_and_without_params_is_skipped() {
        let meta = make_rt_meta("1.0", None);
        let index = make_index("local/test-comp", meta);
        let profile = make_profile("local/test-comp", None);
        let order = make_order("local/test-comp");

        let result = validate_and_materialize_params(&profile, &index, &order).unwrap();
        assert!(
            result.is_empty(),
            "no materialized specs expected for component without schema and without params"
        );
    }
}
