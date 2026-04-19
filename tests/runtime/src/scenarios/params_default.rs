//! Params default scenario — schema default value is used when profile omits params.
//!
//! dummy-rt declares `default: "1.0"` in its params_schema. This scenario verifies
//! that omitting `params` in the profile causes the default to be substituted and
//! recorded in state as-is.

use crate::assert::{
    assert_component_present, assert_state_valid, get_runtime_version, load_state,
};
use crate::context::Context;
use crate::runner::loadout_apply_yes;

pub fn run(ctx: &Context) -> Result<(), String> {
    println!("==> Params default scenario");

    let config = ctx.config("config-params-default.yaml");

    println!("==> Running apply with no params override (should use default '1.0')");
    loadout_apply_yes(ctx, &config)?;

    if !ctx.state_file.exists() {
        return Err(format!(
            "state file not created: {}",
            ctx.state_file.display()
        ));
    }

    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;

    println!("==> Verifying dummy-rt is installed");
    assert_component_present(&state, "local/dummy-rt")?;

    println!("==> Verifying that the schema default version '1.0' is recorded in state");
    let version = get_runtime_version(&state, "local/dummy-rt")?;
    if version != "1.0" {
        return Err(format!(
            "schema default not applied: expected '1.0', got '{}'",
            version
        ));
    }

    println!("==> Params default scenario PASSED");
    Ok(())
}
