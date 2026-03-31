//! Version mixed scenario — versioned and unversioned features coexist correctly.
//!
//! Uses dummy backends and features; no network access required.
//! config-version-mixed.yaml has:
//!   - local/dummy-pkg  (no version — must have no Runtime resource)
//!   - local/dummy-rt   (version: "20" — must have a Runtime resource)

use crate::assert::{assert_no_runtime, assert_state_valid, get_runtime_version, load_state};
use crate::context::Context;
use crate::runner::loadout_apply;

pub fn run(ctx: &Context) -> Result<(), String> {
    println!("==> Version mixed scenario");

    let config = ctx.config("config-version-mixed.yaml");

    println!("==> Running apply with mixed features");
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

    println!("==> Verifying dummy-rt has runtime version recorded");
    let rt_version = get_runtime_version(&state, "local/dummy-rt")?;
    if rt_version != "20" {
        return Err(format!(
            "runtime version not recorded correctly: expected '20', got '{}'",
            rt_version
        ));
    }

    println!("==> Verifying dummy-pkg has no runtime recorded");
    assert_no_runtime(&state, "local/dummy-pkg")?;

    println!("==> Version mixed scenario PASSED");
    Ok(())
}
