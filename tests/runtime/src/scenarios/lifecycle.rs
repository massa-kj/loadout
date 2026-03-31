//! Lifecycle scenario — full multi-phase cycle: base → full → reapply → shrink → uninstall.
//!
//! Mirrors `tests/e2e/linux/docker/scenarios/lifecycle.sh`.

use std::path::Path;

use crate::assert::{
    assert_feature_present, assert_features_empty, assert_no_packages_in_state, assert_path_exists,
    assert_paths_removed, assert_state_unchanged, assert_state_valid, collect_fs_paths, load_state,
    load_state_raw,
};
use crate::context::Context;
use crate::runner::loadout_apply;

pub fn run(ctx: &Context) -> Result<(), String> {
    println!("==> Lifecycle scenario");

    let config_base = ctx.config("config-base.yaml");
    let config_full = ctx.config("config-full.yaml");
    let config_empty = ctx.config("config-empty.yaml");

    // ── Phase 1: base apply ───────────────────────────────────────────────────
    println!("==> Phase 1: base apply");
    loadout_apply(ctx, &config_base)?;

    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;

    // ── Phase 2: expand to full profile ──────────────────────────────────────
    println!("==> Phase 2: expand to full profile");
    loadout_apply(ctx, &config_full)?;

    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;
    assert_feature_present(&state, "core/ripgrep")?;

    println!("==> Snapshotting full state");
    let snapshot_full = load_state_raw(&ctx.state_file)?;

    // ── Phase 3: re-apply full profile (idempotency) ─────────────────────────
    println!("==> Phase 3: re-apply full profile (idempotency check)");
    loadout_apply(ctx, &config_full)?;

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
    loadout_apply(ctx, &config_base)?;

    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;
    assert_feature_present(&state, "core/bash")?;
    assert_feature_present(&state, "core/git")?;

    let unexpected: Vec<&str> = state
        .features
        .keys()
        .map(String::as_str)
        .filter(|k| *k != "core/bash" && *k != "core/git")
        .collect();
    if !unexpected.is_empty() {
        return Err(format!(
            "unexpected features remain after profile shrink: {:?}",
            unexpected
        ));
    }

    // ── Phase 5: full uninstall ───────────────────────────────────────────────
    println!("==> Phase 5: full uninstall");
    loadout_apply(ctx, &config_empty)?;

    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;
    assert_features_empty(&state)?;
    assert_paths_removed(&tracked_files)?;
    assert_path_exists(sentinel)?;
    assert_no_packages_in_state(&state)?;

    println!("==> Lifecycle scenario PASSED");
    Ok(())
}
