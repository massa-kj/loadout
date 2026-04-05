// activate() use case — generate shell activation script from last apply's env plan.

use crate::context::{AppContext, AppError};

pub use executor::activate::ShellKind;

/// Generate a shell activation script from the last apply's env plan.
///
/// Reads the env plan cache written by `execute()` and returns a shell script
/// suitable for evaluation in the target shell.
///
/// # Usage
///
/// ```text
/// eval "$(loadout activate)"               # bash / zsh
/// loadout activate --shell fish | source   # fish
/// Invoke-Expression (loadout activate --shell pwsh)  # PowerShell
/// ```
///
/// # Errors
///
/// Returns [`AppError::EnvPlanNotFound`] if no cache exists — the user must
/// run `loadout apply` first.
pub fn activate(ctx: &AppContext, shell: ShellKind) -> Result<String, AppError> {
    let cache_path = ctx.env_plan_cache_path();
    if !cache_path.exists() {
        return Err(AppError::EnvPlanNotFound);
    }
    let json = std::fs::read_to_string(&cache_path).map_err(AppError::EnvPlanIo)?;
    let plan: model::env::ExecutionEnvPlan =
        serde_json::from_str(&json).map_err(AppError::EnvPlanDeserialize)?;
    Ok(executor::activate::generate_activation(&plan, shell))
}
