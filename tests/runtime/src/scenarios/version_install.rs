//! Version install scenario — version is recorded in state after install.
//!
//! Uses dummy backends and features; no network access required.

use crate::assert::{assert_feature_present, assert_state_valid, get_runtime_version, load_state};
use crate::context::Context;
use crate::runner::loadout_apply;

pub fn run(ctx: &Context) -> Result<(), String> {
    println!("==> Version install scenario");

    let config = ctx.config("config-version-v20.yaml");

    println!("==> Running apply with version-specified features");
    loadout_apply(ctx, &config)?;

    println!("==> Checking state file existence");
    if !ctx.state_file.exists() {
        return Err(format!(
            "state file not created: {}",
            ctx.state_file.display()
        ));
    }

    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;

    println!("==> Verifying dummy-rt is installed");
    assert_feature_present(&state, "local/dummy-rt")?;

    println!("==> Verifying runtime version recorded in state");
    let version = get_runtime_version(&state, "local/dummy-rt")?;
    if version != "20" {
        return Err(format!(
            "runtime version not recorded correctly: expected '20', got '{}'",
            version
        ));
    }

    println!("==> Version install scenario PASSED");
    Ok(())
}
