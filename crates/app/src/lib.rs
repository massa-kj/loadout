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
mod pipeline;
mod plan;
mod read;

pub use activate::{activate, ShellKind};
pub use apply::{apply, execute, prepare_execution, ExecutionPlan};
pub use context::{AppContext, AppError};
pub use executor::{Event, ExecutorReport};
pub use model::plan::Plan;
pub use plan::plan;
pub use read::{build_feature_index, load_sources, scan_backend_dirs};

#[cfg(test)]
mod tests;
