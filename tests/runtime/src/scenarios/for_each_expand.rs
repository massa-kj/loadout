//! for_each expand scenario — array params expand to multiple runtime resources in state.
//!
//! Uses dummy backends and components; no network access required.

use crate::assert::{
    assert_component_present, assert_resource_count, assert_state_valid, get_runtime_version,
    load_state,
};
use crate::context::Context;
use crate::runner::loadout_apply_yes;

pub fn run(ctx: &Context) -> Result<(), String> {
    println!("==> for_each expand scenario");

    let config = ctx.config("config-for-each-v18-v22.yaml");

    println!("==> Running apply with versions=[\"18\",\"22\"]");
    loadout_apply_yes(ctx, &config)?;

    println!("==> Checking state file existence");
    if !ctx.state_file.exists() {
        return Err(format!(
            "state file not created: {}",
            ctx.state_file.display()
        ));
    }

    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;

    println!("==> Verifying dummy-rt-multi is installed");
    assert_component_present(&state, "local/dummy-rt-multi")?;

    println!("==> Verifying two runtime resources in state");
    assert_resource_count(&state, "local/dummy-rt-multi", 2)?;

    println!("==> Verifying runtime versions recorded in state");
    // get_runtime_version returns the first runtime; check both by iterating state.
    let component = state
        .components
        .get("local/dummy-rt-multi")
        .ok_or_else(|| "local/dummy-rt-multi not found in state".to_string())?;

    let recorded_versions: Vec<&str> = component
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

    for expected in &["18", "22"] {
        if !recorded_versions.contains(expected) {
            return Err(format!(
                "expected runtime version '{}' in state but got: {:?}",
                expected, recorded_versions
            ));
        }
    }

    // Sanity-check get_runtime_version still works (returns one of the two).
    let _ = get_runtime_version(&state, "local/dummy-rt-multi")?;

    println!("==> for_each expand scenario PASSED");
    Ok(())
}
