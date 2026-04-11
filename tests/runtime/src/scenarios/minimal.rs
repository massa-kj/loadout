//! Minimal scenario — state is created, version is correct, no duplicates.
//!
//! Mirrors `tests/e2e/linux/docker/scenarios/minimal.sh`.

use crate::assert::{
    assert_all_fs_paths_absolute, assert_components_present, assert_no_duplicate_fs_paths,
    assert_no_duplicate_resource_ids, assert_state_version, load_state,
};
use crate::context::Context;
use crate::runner::loadout_apply;

pub fn run(ctx: &Context) -> Result<(), String> {
    println!("==> Minimal scenario");

    let config = ctx.config("config-base.yaml");

    println!("==> Running apply");
    loadout_apply(ctx, &config)?;

    println!("==> Checking state file existence");
    if !ctx.state_file.exists() {
        return Err(format!(
            "state file not created: {}",
            ctx.state_file.display()
        ));
    }

    println!("==> Loading and validating state");
    let state = load_state(&ctx.state_file)?;

    println!("==> Checking version field");
    assert_state_version(&state)?;

    println!("==> Checking features object exists");
    assert_components_present(&state)?;

    println!("==> Checking no duplicate resource ids per feature");
    assert_no_duplicate_resource_ids(&state)?;

    println!("==> Checking no duplicate fs.path across features");
    assert_no_duplicate_fs_paths(&state)?;

    println!("==> Checking all fs paths are absolute");
    assert_all_fs_paths_absolute(&state)?;

    println!("==> Minimal scenario PASSED");
    Ok(())
}
