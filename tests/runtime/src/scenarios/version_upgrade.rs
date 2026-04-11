//! Version upgrade scenario — version mismatch triggers reinstall and state is updated.
//!
//! Uses dummy backends and components; no network access required.

use crate::assert::{assert_state_valid, get_runtime_version, load_state};
use crate::context::Context;
use crate::runner::loadout_apply;

pub fn run(ctx: &Context) -> Result<(), String> {
    println!("==> Version upgrade scenario");

    let config_v20 = ctx.config("config-version-v20.yaml");
    let config_v22 = ctx.config("config-version-v22.yaml");

    // ── Phase 1: install dummy-rt@20 ─────────────────────────────────────────
    println!("==> First apply (dummy-rt@20)");
    loadout_apply(ctx, &config_v20)?;

    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;

    let version_1 = get_runtime_version(&state, "local/dummy-rt")?;
    if version_1 != "20" {
        return Err(format!(
            "expected version '20' after first apply, got '{}'",
            version_1
        ));
    }

    // ── Phase 2: upgrade to dummy-rt@22 ──────────────────────────────────────
    println!("==> Second apply (dummy-rt@22 — should trigger reinstall)");
    loadout_apply(ctx, &config_v22)?;

    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;

    let version_2 = get_runtime_version(&state, "local/dummy-rt")?;
    if version_2 != "22" {
        return Err(format!(
            "expected version '22' after upgrade apply, got '{}'",
            version_2
        ));
    }

    if version_1 == version_2 {
        return Err(format!("version did not change: both are '{}'", version_1));
    }

    println!("==> Version upgrade scenario PASSED");
    Ok(())
}
