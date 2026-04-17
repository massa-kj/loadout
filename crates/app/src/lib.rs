//! Application service layer — orchestrates the loadout pipeline.
//!
//! This crate corresponds to `cmd/* + core/lib/orchestrator.sh` in the shell
//! implementation.  It assembles and sequences all pipeline stages:
//!
//! ```text
//! load_sources → SourcesSpec
//! load_profile → Profile
//! load_strategy  → Strategy
//! build_component_index → ComponentIndex
//! filter_desired_components → Vec<CanonicalComponentId>
//! resolver::resolve → ResolvedComponentOrder
//! compiler::compile → DesiredResourceGraph
//! state::load → State
//! planner::plan → Plan
//! (apply only) build_backend_registry + executor::execute
//! ```
//!
//! The only state mutation happens inside `executor::execute`, which atomically
//! commits state after each successful component.  Every other stage is read-only.
//!
//! See: `docs/architecture/layers.md` (cmd / app layer)

mod activate;
mod apply;
mod context;
mod materializer;
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
    backend_import, component_import, config_component_add, config_component_remove, config_init,
    config_raw_set, config_raw_show, config_raw_unset, source_add_git, source_add_path,
    source_remove, source_trust, source_untrust, source_update, ImportReport,
};
pub use plan::plan;
pub use read::{
    list_backends, list_components, list_configs, list_sources, show_backend, show_component,
    show_config, show_source, show_state, BackendDetail, BackendScripts, BackendSummary,
    ComponentDetail, ComponentSummary, ConfigDetail, ConfigSummary, SourceSummary,
};
pub use scaffold::{backend_new, component_new, BackendPlatform, ComponentTemplate};
pub use validate::{
    backend_validate, component_validate, IssueLevel, ValidationIssue, ValidationReport,
};

#[cfg(test)]
mod tests;
