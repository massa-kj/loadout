//! managed-script scenario — create, idempotent, and destroy flows for a
//! `managed_script` component with a `tool` resource.
//!
//! Uses the `dummy-tool` fixture component, which creates a marker file at
//! `/tmp/loadout-dummy/tools/dummy-tool` on install and removes it on uninstall.
//!
//! ## Phases
//!
//! 1. **create** — apply `config-tool.yaml`; the tool resource must appear in
//!    state and the marker file must exist on disk.
//! 2. **idempotent** — re-apply the same config; state must not change.
//! 3. **destroy** — apply `config-empty.yaml` (prune); state must be empty and
//!    the marker file must be gone.

use std::path::Path;

use crate::assert::{
    assert_component_absent, assert_component_present, assert_state_unchanged, assert_state_valid,
    assert_tool_resource_present, load_state, load_state_raw,
};
use crate::context::Context;
use crate::runner::loadout_apply_yes;

/// Path of the marker file created by the dummy-tool install script.
const MARKER: &str = "/tmp/loadout-dummy/tools/dummy-tool";

/// Component identifier as declared in `config-tool.yaml`.
const COMPONENT_ID: &str = "local/dummy-tool";

pub fn run(ctx: &Context) -> Result<(), String> {
    println!("==> managed-script scenario");

    let config_tool = ctx.config("config-tool.yaml");
    let config_empty = ctx.config("config-empty.yaml");

    // ── Phase 1: create ────────────────────────────────────────────────────
    println!("==> Phase 1: create — apply managed_script component");
    loadout_apply_yes(ctx, &config_tool)?;

    println!("==> Validating state after create");
    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;

    println!("==> Checking component present in state");
    assert_component_present(&state, COMPONENT_ID)?;

    println!("==> Checking tool resource recorded in state");
    assert_tool_resource_present(&state, COMPONENT_ID, "dummy-tool")?;

    println!("==> Checking marker file exists on disk");
    if !Path::new(MARKER).exists() {
        return Err(format!(
            "marker file '{}' does not exist after install",
            MARKER
        ));
    }

    // ── Phase 2: idempotent ────────────────────────────────────────────────
    println!("==> Phase 2: idempotent — re-apply same config");
    let snapshot_before = load_state_raw(&ctx.state_file)?;
    loadout_apply_yes(ctx, &config_tool)?;
    let snapshot_after = load_state_raw(&ctx.state_file)?;

    assert_state_unchanged(
        &snapshot_before,
        &snapshot_after,
        "managed-script idempotent apply",
    )?;

    println!("==> Checking marker file still present after idempotent apply");
    if !Path::new(MARKER).exists() {
        return Err(format!(
            "marker file '{}' was unexpectedly removed during idempotent apply",
            MARKER
        ));
    }

    // ── Phase 3: destroy ────────────────────────────────────────────────────
    println!("==> Phase 3: destroy — apply empty config (prune)");
    loadout_apply_yes(ctx, &config_empty)?;

    println!("==> Validating state after destroy");
    let state = load_state(&ctx.state_file)?;
    assert_state_valid(&state)?;

    println!("==> Checking component absent from state");
    assert_component_absent(&state, COMPONENT_ID)?;

    println!("==> Checking marker file removed from disk");
    if Path::new(MARKER).exists() {
        return Err(format!(
            "marker file '{}' still exists after uninstall",
            MARKER
        ));
    }

    println!("==> managed-script scenario PASSED");
    Ok(())
}
