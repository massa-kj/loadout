//! Params validation error scenario — unknown param key causes loadout to abort.
//!
//! dummy-rt declares `additionalProperties: false`, so any param key not in its
//! `params_schema.properties` must cause validation to fail. This scenario verifies
//! that `loadout apply` exits with a non-zero status code when such a key is provided.

use crate::context::Context;
use crate::runner::loadout_apply_yes_expect_fail;

pub fn run(ctx: &Context) -> Result<(), String> {
    println!("==> Params validation error scenario");

    let config = ctx.config("config-params-invalid.yaml");

    println!("==> Running apply with unknown param 'build_from_source' (must fail)");
    loadout_apply_yes_expect_fail(ctx, &config)?;

    println!("==> Params validation error scenario PASSED");
    Ok(())
}
