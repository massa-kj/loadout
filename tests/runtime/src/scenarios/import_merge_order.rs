//! Import merge order scenario — later import overrides earlier at bundle-name level.
//!
//! Import order in `config-import-merge-order.yaml`:
//!   1. `import-bundle-base.yaml`  → defines `bundles.base` with `local/dummy-pkg`
//!   2. `import-bundle-extra.yaml` → redefines `bundles.base` with `local/dummy-fs-copy`
//!
//! The second import wins at bundle-name level; `profile.components` adds
//! `local/dummy-fs-link` on top.
//!
//! Verifies:
//! - Bundle-name-level replace: the later import's full bundle definition wins.
//! - The component from the earlier (overridden) bundle is NOT installed.
//! - `profile.components` always takes priority over all imported bundles.

use crate::assert::{assert_component_present, assert_state_valid, load_state};
use crate::context::Context;
use crate::runner::loadout_apply_yes;

pub fn run(ctx: &Context) -> Result<(), String> {
    println!("==> Import merge order scenario");

    let config = ctx.config("config-import-merge-order.yaml");

    println!("==> Running apply with two imports that define the same bundle name");
    loadout_apply_yes(ctx, &config)?;

    if !ctx.state_file.exists() {
        return Err(format!(
            "state file not created: {}",
            ctx.state_file.display()
        ));
    }

    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;

    println!("==> Verifying component from later import's bundle (local/dummy-fs-copy)");
    assert_component_present(&state, "local/dummy-fs-copy")?;

    println!("==> Verifying component from main profile (local/dummy-fs-link)");
    assert_component_present(&state, "local/dummy-fs-link")?;

    println!("==> Verifying earlier bundle's component is NOT installed (local/dummy-pkg)");
    if state.components.contains_key("local/dummy-pkg") {
        return Err("local/dummy-pkg must NOT be installed: \
             earlier bundle was overridden by later import at bundle-name level"
            .to_string());
    }

    println!("==> Import merge order scenario PASSED");
    Ok(())
}
