//! loadout-e2e — standalone E2E test runner for loadout scenarios.
//!
//! This binary is built separately from the `loadout` product binary.
//! It is intended to be copied into Docker containers or Windows Sandbox
//! instances alongside the `loadout` binary and executed there.
//!
//! # Usage
//!
//! ```text
//! loadout-e2e [scenario]
//!
//! Scenarios:
//!   minimal          State created, version correct, no duplicates
//!   idempotent       Second apply produces identical state
//!   uninstall        Tracked files removed; untracked files preserved
//!   lifecycle        Full multi-phase cycle (base → full → shrink → empty)
//!   version-install  Version recorded in state after install
//!   version-upgrade  Version mismatch triggers reinstall; state updated
//!   version-mixed    Versioned and unversioned features coexist correctly
//!   all              Run all scenarios (default)
//! ```
//!
//! # Environment variables
//!
//! See [`context::Context`] for the full list of recognised variables.

mod assert;
mod context;
mod runner;
mod scenarios;

use context::Context;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let scenario = args.get(1).map(String::as_str).unwrap_or("all");

    let ctx = Context::from_env();

    println!("loadout-e2e: scenario = {scenario}");
    println!("  loadout binary : {}", ctx.loadout_bin);
    println!("  repo root      : {}", ctx.repo_root.display());
    println!("  config dir     : {}", ctx.config_dir.display());
    println!("  state file     : {}", ctx.state_file.display());
    println!();

    let result = dispatch(scenario, &ctx);

    match result {
        Ok(()) => {
            println!();
            println!("OK: '{scenario}' passed");
        }
        Err(e) => {
            eprintln!();
            eprintln!("FAILED: '{scenario}': {e}");
            std::process::exit(1);
        }
    }
}

/// Dispatch a scenario name to its `run` function.
fn dispatch(scenario: &str, ctx: &Context) -> Result<(), String> {
    match scenario {
        "all" => run_all(ctx),
        "minimal" => scenarios::minimal::run(ctx),
        "idempotent" => scenarios::idempotent::run(ctx),
        "uninstall" => scenarios::uninstall::run(ctx),
        "lifecycle" => scenarios::lifecycle::run(ctx),
        "version-install" => scenarios::version_install::run(ctx),
        "version-upgrade" => scenarios::version_upgrade::run(ctx),
        "version-mixed" => scenarios::version_mixed::run(ctx),
        other => Err(format!(
            "unknown scenario '{other}'. Valid: \
             minimal, idempotent, uninstall, lifecycle, \
             version-install, version-upgrade, version-mixed, all"
        )),
    }
}

/// Run every scenario in a fixed order and collect failures.
fn run_all(ctx: &Context) -> Result<(), String> {
    type ScenarioFn = fn(&Context) -> Result<(), String>;
    let all: &[(&str, ScenarioFn)] = &[
        ("minimal", scenarios::minimal::run),
        ("idempotent", scenarios::idempotent::run),
        ("uninstall", scenarios::uninstall::run),
        ("lifecycle", scenarios::lifecycle::run),
        ("version-install", scenarios::version_install::run),
        ("version-upgrade", scenarios::version_upgrade::run),
        ("version-mixed", scenarios::version_mixed::run),
    ];

    let mut failed: Vec<(&str, String)> = Vec::new();

    for (name, run) in all {
        println!("━━━ {name} ━━━");
        match run(ctx) {
            Ok(()) => println!("✓ {name} passed\n"),
            Err(e) => {
                eprintln!("✗ {name} FAILED: {e}\n");
                failed.push((name, e));
            }
        }
    }

    if failed.is_empty() {
        println!("All scenarios passed ({} total)", all.len());
        Ok(())
    } else {
        let summary: Vec<String> = failed.iter().map(|(n, e)| format!("  {n}: {e}")).collect();
        Err(format!(
            "{}/{} scenario(s) failed:\n{}",
            failed.len(),
            all.len(),
            summary.join("\n")
        ))
    }
}
