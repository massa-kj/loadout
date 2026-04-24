//! Package version install scenario — package version is recorded in state after install.
//!
//! Uses dummy backends and components; no network access required.

use crate::assert::{
    assert_component_present, assert_state_valid, get_package_version, load_state,
};
use crate::context::Context;
use crate::runner::loadout_apply_yes;

pub fn run(ctx: &Context) -> Result<(), String> {
    println!("==> Package version install scenario");

    let config = ctx.config("config-pkg-version-v1.yaml");

    println!("==> Running apply with versioned package component");
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

    println!("==> Verifying dummy-pkg-versioned is installed");
    assert_component_present(&state, "local/dummy-pkg-versioned")?;

    println!("==> Verifying package version recorded in state");
    let version = get_package_version(&state, "local/dummy-pkg-versioned")?;
    if version != Some("1.0") {
        return Err(format!(
            "package version not recorded correctly: expected Some(\"1.0\"), got {:?}",
            version
        ));
    }

    println!("==> Package version install scenario PASSED");
    Ok(())
}
