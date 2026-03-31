//! Version install scenario — version is recorded in state after install.
//!
//! Mirrors `tests/e2e/linux/docker/scenarios/version_install.sh`.

use model::state::ResourceKind;

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

    println!("==> Verifying node is installed");
    assert_feature_present(&state, "core/node")?;

    println!("==> Verifying node version recorded in state");
    let node_version = get_runtime_version(&state, "core/node")?;
    if node_version != "20" {
        return Err(format!(
            "node version not recorded correctly: expected '20', got '{}'",
            node_version
        ));
    }

    println!("==> Verifying node package registered in state");
    let node_feature = &state.features["core/node"];
    let has_node_package = node_feature.resources.iter().any(|r| {
        matches!(&r.kind, ResourceKind::Package { package, .. } if package.name.starts_with("node@20"))
    });
    if !has_node_package {
        return Err("node@20 package not registered in state".to_owned());
    }

    println!("==> Version install scenario PASSED");
    Ok(())
}
