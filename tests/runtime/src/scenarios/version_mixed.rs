//! Version mixed scenario — versioned and unversioned features coexist correctly.
//!
//! Mirrors `tests/e2e/linux/docker/scenarios/version_mixed.sh`.

use crate::assert::{
    assert_feature_count_at_least, assert_no_runtime, assert_state_valid, get_runtime_version,
    load_state,
};
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

    println!("==> Verifying node has version recorded");
    let node_version = get_runtime_version(&state, "core/node")?;
    if node_version != "20" {
        return Err(format!(
            "node version not recorded correctly: expected '20', got '{}'",
            node_version
        ));
    }

    println!("==> Verifying git has no runtime recorded");
    assert_no_runtime(&state, "core/git")?;

    println!("==> Verifying bash has no runtime recorded");
    assert_no_runtime(&state, "core/bash")?;

    println!("==> Verifying all expected features are installed");
    // The mixed config should install at least 5 features.
    assert_feature_count_at_least(&state, 5)?;

    println!("==> Version mixed scenario PASSED");
    Ok(())
}
