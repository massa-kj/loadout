//! E2E scenario modules.
//!
//! Each module exposes a single `run(ctx: &Context) -> Result<(), String>` function.
//! Add new scenarios here and register them in [`crate::main`].

pub mod for_each_expand;
pub mod for_each_shrink;
pub mod idempotent;
pub mod import_cycle;
pub mod import_merge_order;
pub mod import_single;
pub mod lifecycle;
pub mod managed_script;
pub mod minimal;
pub mod params_default;
pub mod params_validation_error;
pub mod pkg_version_install;
pub mod pkg_version_upgrade;
pub mod uninstall;
pub mod version_install;
pub mod version_mixed;
pub mod version_upgrade;
