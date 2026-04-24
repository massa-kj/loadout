//! Package version upgrade scenario — version change triggers Replace; state is updated.
//!
//! Uses dummy backends and components; no network access required.

use crate::assert::{assert_state_valid, get_package_version, load_state};
use crate::context::Context;
use crate::runner::loadout_apply_yes;

pub fn run(ctx: &Context) -> Result<(), String> {
    println!("==> Package version upgrade scenario");

    let config_v1 = ctx.config("config-pkg-version-v1.yaml");
    let config_v2 = ctx.config("config-pkg-version-v2.yaml");

    // ── Phase 1: install dummy-pkg-versioned@1.0 ─────────────────────────────
    println!("==> First apply (dummy-pkg-versioned@1.0)");
    loadout_apply_yes(ctx, &config_v1)?;

    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;

    let version_1 = get_package_version(&state, "local/dummy-pkg-versioned")?;
    if version_1 != Some("1.0") {
        return Err(format!(
            "expected version Some(\"1.0\") after first apply, got {:?}",
            version_1
        ));
    }

    // ── Phase 2: upgrade to dummy-pkg-versioned@2.0 ──────────────────────────
    println!("==> Second apply (dummy-pkg-versioned@2.0 — should trigger Replace)");
    loadout_apply_yes(ctx, &config_v2)?;

    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;

    let version_2 = get_package_version(&state, "local/dummy-pkg-versioned")?;
    if version_2 != Some("2.0") {
        return Err(format!(
            "expected version Some(\"2.0\") after upgrade apply, got {:?}",
            version_2
        ));
    }

    if version_1 == version_2 {
        return Err(format!("version did not change: both are {:?}", version_1));
    }

    println!("==> Package version upgrade scenario PASSED");
    Ok(())
}
