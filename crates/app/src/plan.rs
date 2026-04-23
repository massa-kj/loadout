// plan() use case — compute a plan without executing any actions.

use std::path::Path;

use crate::context::{AppContext, AppError};
use crate::pipeline::{run_pipeline, PipelineOutput};

/// Compute the plan for the given config without executing any actions.
///
/// `config_path` must point to a unified `config.yaml` containing both the
/// `profile` and (optionally) the `strategy` section.
///
/// Returns the [`model::plan::Plan`] that describes what `apply()` would do.
/// All stages are read-only; no state is modified.
pub fn plan(ctx: &AppContext, config_path: &Path) -> Result<model::plan::Plan, AppError> {
    let PipelineOutput {
        full_order,
        graph,
        state,
        ..
    } = run_pipeline(ctx, config_path)?;
    // Use full_order (desired + state-only components) so the planner can compute
    // correct reverse destroy ordering when both a component and its dependency are removed.
    let p = planner::plan(&graph, &state, &full_order)?;
    Ok(p)
}
