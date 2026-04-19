//! Import single scenario — bundle defined in an imported file is applied.
//!
//! Verifies:
//! - `imports:` causes the referenced file's `bundles:` section to be available
//!   in the main config's `bundle.use`.
//! - Components declared through an imported bundle are installed and recorded
//!   in state exactly as if they had been defined inline.
//! - `profile.components` of the main file is merged on top.

use crate::assert::{assert_component_present, assert_state_valid, load_state};
use crate::context::Context;
use crate::runner::loadout_apply_yes;

pub fn run(ctx: &Context) -> Result<(), String> {
    println!("==> Import single scenario");

    let config = ctx.config("config-import-single.yaml");

    println!("==> Running apply with imported bundle definition");
    loadout_apply_yes(ctx, &config)?;

    if !ctx.state_file.exists() {
        return Err(format!(
            "state file not created: {}",
            ctx.state_file.display()
        ));
    }

    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;

    println!("==> Verifying component from imported bundle (local/dummy-pkg)");
    assert_component_present(&state, "local/dummy-pkg")?;

    println!("==> Verifying component from main profile (local/dummy-fs-copy)");
    assert_component_present(&state, "local/dummy-fs-copy")?;

    println!("==> Import single scenario PASSED");
    Ok(())
}
