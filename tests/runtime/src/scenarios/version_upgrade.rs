//! Version upgrade scenario — version mismatch triggers reinstall and state is updated.
//!
//! Mirrors `tests/e2e/linux/docker/scenarios/version_upgrade.sh`.

use model::state::ResourceKind;

use crate::assert::{assert_state_valid, get_runtime_version, load_state};
use crate::context::Context;
use crate::runner::loadout_apply;

pub fn run(ctx: &Context) -> Result<(), String> {
    println!("==> Version upgrade scenario");

    let config_v20 = ctx.config("config-version-v20.yaml");
    let config_v22 = ctx.config("config-version-v22.yaml");

    // ── Phase 1: install Node 20 ──────────────────────────────────────────────
    println!("==> First apply (Node 20)");
    loadout_apply(ctx, &config_v20)?;

    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;

    let version_1 = get_runtime_version(&state, "core/node")?;
    if version_1 != "20" {
        return Err(format!(
            "expected Node version '20' after first apply, got '{}'",
            version_1
        ));
    }

    let has_package_v20 = state
        .features
        .get("core/node")
        .map(|f| {
            f.resources.iter().any(|r| {
                matches!(&r.kind, ResourceKind::Package { package, .. }
                    if package.name.starts_with("node@20"))
            })
        })
        .unwrap_or(false);
    if !has_package_v20 {
        return Err("node@20 package not registered in state after first apply".to_owned());
    }

    // ── Phase 2: upgrade to Node 22 ───────────────────────────────────────────
    println!("==> Second apply (Node 22 — should trigger reinstall)");
    loadout_apply(ctx, &config_v22)?;

    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;

    let version_2 = get_runtime_version(&state, "core/node")?;
    if version_2 != "22" {
        return Err(format!(
            "expected Node version '22' after upgrade apply, got '{}'",
            version_2
        ));
    }

    let has_package_v22 = state
        .features
        .get("core/node")
        .map(|f| {
            f.resources.iter().any(|r| {
                matches!(&r.kind, ResourceKind::Package { package, .. }
                    if package.name.starts_with("node@22"))
            })
        })
        .unwrap_or(false);
    if !has_package_v22 {
        return Err("node@22 package not registered in state after upgrade".to_owned());
    }

    if version_1 == version_2 {
        return Err(format!("version did not change: both are '{}'", version_1));
    }

    println!("==> Version upgrade scenario PASSED");
    Ok(())
}
