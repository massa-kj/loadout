//! Lifecycle scenario — full multi-phase cycle: base → full → reapply → shrink → uninstall.
//!
//! Uses dummy backends and components; no network access required.

use std::path::Path;

use crate::assert::{
    assert_component_present, assert_components_empty, assert_no_packages_in_state,
    assert_path_exists, assert_paths_removed, assert_state_unchanged, assert_state_valid,
    collect_fs_paths, load_state, load_state_raw,
};
use crate::context::Context;
use crate::runner::loadout_apply_yes;

pub fn run(ctx: &Context) -> Result<(), String> {
    println!("==> Lifecycle scenario");

    let config_base = ctx.config("config-base.yaml");
    let config_full = ctx.config("config-full.yaml");
    let config_empty = ctx.config("config-empty.yaml");

    // ── Phase 1: base apply ───────────────────────────────────────────────────
    println!("==> Phase 1: base apply");
    loadout_apply_yes(ctx, &config_base)?;

    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;
    assert_component_present(&state, "local/dummy-pkg")?;
    assert_component_present(&state, "local/dummy-fs")?;

    // ── Phase 2: expand to full profile ──────────────────────────────────────
    println!("==> Phase 2: expand to full profile");
    loadout_apply_yes(ctx, &config_full)?;

    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;
    // full profile adds local/dummy-rt on top of base components
    assert_component_present(&state, "local/dummy-rt")?;

    println!("==> Snapshotting full state");
    let snapshot_full = load_state_raw(&ctx.state_file)?;

    // ── Phase 3: re-apply full profile (idempotency) ─────────────────────────
    println!("==> Phase 3: re-apply full profile (idempotency check)");
    loadout_apply_yes(ctx, &config_full)?;

    let after_reapply = load_state_raw(&ctx.state_file)?;
    assert_state_unchanged(&snapshot_full, &after_reapply, "full-profile second apply")?;

    println!("==> Collecting tracked resources before shrink");
    let state = load_state(&ctx.state_file)?;
    let tracked_files = collect_fs_paths(&state);

    println!("==> Creating sentinel file (must NOT be removed)");
    let sentinel = Path::new("/tmp/loadout_sentinel");
    std::fs::write(sentinel, "do not delete")
        .map_err(|e| format!("failed to create sentinel: {}", e))?;

    // ── Phase 4: shrink back to base ─────────────────────────────────────────
    println!("==> Phase 4: shrink back to base profile");
    loadout_apply_yes(ctx, &config_base)?;

    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;
    // base components must be present
    assert_component_present(&state, "local/dummy-pkg")?;
    assert_component_present(&state, "local/dummy-fs")?;

    // full-only component must have been removed
    let unexpected: Vec<&str> = state
        .components
        .keys()
        .map(String::as_str)
        .filter(|k| *k != "local/dummy-pkg" && *k != "local/dummy-fs")
        .collect();
    if !unexpected.is_empty() {
        return Err(format!(
            "unexpected components remain after profile shrink: {:?}",
            unexpected
        ));
    }

    // ── Phase 5: full uninstall ───────────────────────────────────────────────
    println!("==> Phase 5: full uninstall");
    loadout_apply_yes(ctx, &config_empty)?;

    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;
    assert_components_empty(&state)?;
    assert_paths_removed(&tracked_files)?;
    assert_path_exists(sentinel)?;
    assert_no_packages_in_state(&state)?;

    println!("==> Lifecycle scenario PASSED");
    Ok(())
}
