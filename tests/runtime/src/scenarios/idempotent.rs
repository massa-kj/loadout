//! Idempotent scenario — a second apply produces an identical state.
//!
//! Mirrors `tests/e2e/linux/docker/scenarios/idempotent.sh`.

use crate::assert::{
    assert_no_duplicate_fs_paths, assert_no_duplicate_resource_ids, assert_state_unchanged,
    assert_state_valid, load_state, load_state_raw,
};
use crate::context::Context;
use crate::runner::loadout_apply_yes;

pub fn run(ctx: &Context) -> Result<(), String> {
    println!("==> Idempotent scenario");

    let config = ctx.config("config-base.yaml");

    println!("==> First apply");
    loadout_apply_yes(ctx, &config)?;
    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;

    println!("==> Snapshotting state after first apply");
    let snapshot = load_state_raw(&ctx.state_file)?;

    println!("==> Second apply");
    loadout_apply_yes(ctx, &config)?;

    println!("==> Comparing state");
    let after = load_state_raw(&ctx.state_file)?;
    assert_state_unchanged(&snapshot, &after, "second apply")?;

    println!("==> Verifying structural invariants after second apply");
    let state2 = load_state(&ctx.state_file)?;
    assert_no_duplicate_resource_ids(&state2)?;
    assert_no_duplicate_fs_paths(&state2)?;

    println!("==> Idempotent scenario PASSED");
    Ok(())
}
