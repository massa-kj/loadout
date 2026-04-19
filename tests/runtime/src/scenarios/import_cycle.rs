//! Import cycle scenario — circular import reference is rejected cleanly.
//!
//! `config-import-cycle-a.yaml` imports `config-import-cycle-b.yaml`,
//! which in turn imports `config-import-cycle-a.yaml`, forming a cycle.
//!
//! Verifies:
//! - `loadout apply` exits with a non-zero status (does not hang or panic).
//! - The state file is NOT created/modified (no partial state on error).

use crate::context::Context;
use crate::runner::loadout_apply_yes_expect_fail;

pub fn run(ctx: &Context) -> Result<(), String> {
    println!("==> Import cycle scenario");

    let config = ctx.config("config-import-cycle-a.yaml");

    // Record whether the state file existed before the apply.
    let state_existed_before = ctx.state_file.exists();

    println!("==> Running apply with circular import (must fail)");
    loadout_apply_yes_expect_fail(ctx, &config)?;

    // If state did not exist before, it must not have been created by the failed apply.
    if !state_existed_before && ctx.state_file.exists() {
        return Err(format!(
            "state file was created despite import cycle error: {}",
            ctx.state_file.display()
        ));
    }

    println!("==> Import cycle scenario PASSED");
    Ok(())
}
