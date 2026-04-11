//! Uninstall scenario — tracked files are removed; untracked files are preserved.
//!
//! Mirrors `tests/e2e/linux/docker/scenarios/uninstall.sh`.
//!
//! Three sub-tests:
//! 1. Partial uninstall (full → base profile)
//! 2. Full uninstall   (base →  empty profile)
//! 3. Idempotent uninstall (empty → empty)

use std::path::Path;

use crate::assert::{
    assert_component_absent, assert_component_present, assert_components_empty,
    assert_no_packages_in_state, assert_path_exists, assert_paths_removed, assert_state_valid,
    collect_fs_paths, load_state,
};
use crate::context::Context;
use crate::runner::loadout_apply;

pub fn run(ctx: &Context) -> Result<(), String> {
    println!("==> Uninstall scenario");

    let config_full = ctx.config("config-full.yaml");
    let config_partial = ctx.config("config-base.yaml");
    let config_empty = ctx.config("config-empty.yaml");

    // ── Phase 1: install full profile ────────────────────────────────────────
    println!("==> Phase 1: installing full profile");
    loadout_apply(ctx, &config_full)?;

    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;

    if state.components.is_empty() {
        return Err("no components installed — test invalid".to_owned());
    }

    let tracked_files = collect_fs_paths(&state);

    println!("==> Creating sentinel file (must NOT be removed by uninstall)");
    let sentinel = Path::new("/tmp/loadout_sentinel");
    std::fs::write(sentinel, "do not delete")
        .map_err(|e| format!("failed to create sentinel: {}", e))?;

    // ── Test 1: partial uninstall ─────────────────────────────────────────────
    println!("==> Test 1: partial uninstall (full → base)");
    loadout_apply(ctx, &config_partial)?;

    let state = load_state(&ctx.state_file)?;
    assert_component_present(&state, "core/bash")?;
    assert_component_present(&state, "core/git")?;

    let unexpected: Vec<&str> = state
        .components
        .keys()
        .map(String::as_str)
        .filter(|k| *k != "core/bash" && *k != "core/git")
        .collect();
    if !unexpected.is_empty() {
        return Err(format!(
            "unexpected components remain after partial uninstall: {:?}",
            unexpected
        ));
    }

    println!("==> Partial uninstall PASSED");

    // ── Test 2: full uninstall ────────────────────────────────────────────────
    println!("==> Test 2: full uninstall (base → empty)");
    loadout_apply(ctx, &config_empty)?;

    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;
    assert_components_empty(&state)?;
    assert_paths_removed(&tracked_files)?;
    assert_path_exists(sentinel)?;
    assert_no_packages_in_state(&state)?;

    // Confirm ripgrep was removed (it is a full-config-only component)
    assert_component_absent(&state, "core/ripgrep")?;

    println!("==> Full uninstall PASSED");

    // ── Test 3: idempotent uninstall ──────────────────────────────────────────
    println!("==> Test 3: idempotent uninstall (empty → empty)");
    loadout_apply(ctx, &config_empty)?;

    let state = load_state(&ctx.state_file)?;
    assert_components_empty(&state)?;

    println!("==> Idempotent uninstall PASSED");
    println!("==> Uninstall scenario PASSED");
    Ok(())
}
