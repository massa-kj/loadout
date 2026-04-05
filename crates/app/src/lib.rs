//! Application service layer — orchestrates the loadout pipeline.
//!
//! This crate corresponds to `cmd/* + core/lib/orchestrator.sh` in the shell
//! implementation.  It assembles and sequences all pipeline stages:
//!
//! ```text
//! load_sources → SourcesSpec
//! load_profile → Profile
//! load_strategy  → Strategy
//! build_feature_index → FeatureIndex
//! filter_desired_features → Vec<CanonicalFeatureId>
//! resolver::resolve → ResolvedFeatureOrder
//! compiler::compile → DesiredResourceGraph
//! state::load → State
//! planner::plan → Plan
//! (apply only) build_backend_registry + executor::execute
//! ```
//!
//! The only state mutation happens inside `executor::execute`, which atomically
//! commits state after each successful feature.  Every other stage is read-only.
//!
//! See: `docs/architecture/layers.md` (cmd / app layer)

mod activate;
mod apply;
mod context;
mod mutate;
mod pipeline;
mod plan;
mod read;
mod scaffold;
mod validate;

pub use activate::{activate, ShellKind};
pub use apply::{apply, execute, prepare_execution, ExecutionPlan};
pub use context::{AppContext, AppError};
pub use executor::{Event, ExecutorReport};
pub use model::plan::Plan;
pub use mutate::{
    config_feature_add, config_feature_remove, config_init, config_raw_set, config_raw_show,
    config_raw_unset,
};
pub use plan::plan;
pub use read::{
    list_backends, list_configs, list_features, list_sources, show_backend, show_config,
    show_feature, show_source, show_state, BackendDetail, BackendScripts, BackendSummary,
    ConfigDetail, ConfigSummary, FeatureDetail, FeatureSummary, SourceSummary,
};
pub use scaffold::{backend_new, feature_new, BackendPlatform, FeatureTemplate};
pub use validate::{
    backend_validate, feature_validate, IssueLevel, ValidationIssue, ValidationReport,
};

#[cfg(test)]
mod tests;
