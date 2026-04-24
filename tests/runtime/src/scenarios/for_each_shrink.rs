//! for_each shrink scenario — removing an element from the array removes that resource from state.
//!
//! Uses dummy backends and components; no network access required.

use crate::assert::{assert_resource_count, assert_state_valid, load_state};
use crate::context::Context;
use crate::runner::loadout_apply_yes;

pub fn run(ctx: &Context) -> Result<(), String> {
    println!("==> for_each shrink scenario");

    let config_expand = ctx.config("config-for-each-v18-v22.yaml");
    let config_shrink = ctx.config("config-for-each-v22.yaml");

    // ── Phase 1: install both versions ───────────────────────────────────────
    println!("==> First apply (versions=[\"18\",\"22\"])");
    loadout_apply_yes(ctx, &config_expand)?;

    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;
    assert_resource_count(&state, "local/dummy-rt-multi", 2)?;

    // ── Phase 2: shrink to one version ───────────────────────────────────────
    println!("==> Second apply (versions=[\"22\"] — should remove rt:dummy-rt@18)");
    loadout_apply_yes(ctx, &config_shrink)?;

    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;

    println!("==> Verifying only one runtime resource remains in state");
    assert_resource_count(&state, "local/dummy-rt-multi", 1)?;

    let component = state
        .components
        .get("local/dummy-rt-multi")
        .ok_or_else(|| "local/dummy-rt-multi not found in state".to_string())?;

    let remaining_versions: Vec<&str> = component
        .resources
        .iter()
        .filter_map(|r| {
            if let model::state::ResourceKind::Runtime { runtime, .. } = &r.kind {
                Some(runtime.version.as_str())
            } else {
                None
            }
        })
        .collect();

    if !remaining_versions.contains(&"22") {
        return Err(format!(
            "expected runtime version '22' to remain in state but got: {:?}",
            remaining_versions
        ));
    }

    if remaining_versions.contains(&"18") {
        return Err(format!(
            "runtime version '18' should have been removed from state but still present: {:?}",
            remaining_versions
        ));
    }

    println!("==> for_each shrink scenario PASSED");
    Ok(())
}
