//! E2E scenario modules.
//!
//! Each module exposes a single `run(ctx: &Context) -> Result<(), String>` function.
//! Add new scenarios here and register them in [`crate::main`].

pub mod idempotent;
pub mod lifecycle;
pub mod managed_script;
pub mod minimal;
pub mod params_default;
pub mod params_validation_error;
pub mod uninstall;
pub mod version_install;
pub mod version_mixed;
pub mod version_upgrade;
