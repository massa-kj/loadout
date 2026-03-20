// crates/cli/src/main.rs — loadout CLI entry point (Phase 4)
//
// This binary implements all subcommands directly in Rust by calling the `app`
// crate. No shell scripts are spawned for core commands.
//
// Subcommands:
//   apply   <profile>             Apply a profile (install/update/remove features)
//   plan    <profile> [--verbose] Show what apply would do without changes
//   migrate [--dry-run]           Migrate state file to the current schema version
//
// Repository root resolution (for features/, backends/, policies/):
//   1. LOADOUT_ROOT environment variable (explicit override)
//   2. Parent of the binary's directory (tarball layout: bin/loadout → ../)
//
// See: docs/architecture/layers.md (cmd / app boundary)

use std::path::{Path, PathBuf};
use std::{env, process};

// ── Usage text ────────────────────────────────────────────────────────────────

const USAGE: &str = "\
Usage: loadout <command> [options]

A declarative environment management system.

Available commands:
  apply <profile>              Apply a loadout profile to the system
  plan  <profile> [--verbose]  Show what apply would do (no changes made)
  migrate [--dry-run]          Migrate state file to the latest schema version

Examples:
  loadout apply profiles/linux.yaml
  loadout plan  profiles/linux.yaml
  loadout plan  profiles/linux.yaml --verbose
  loadout migrate --dry-run\
";

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = env::args().collect();

    let subcommand = match args.get(1).map(String::as_str) {
        None | Some("help") | Some("--help") | Some("-h") => {
            println!("{USAGE}");
            process::exit(0);
        }
        Some("--version") | Some("-V") => {
            println!("loadout {}", env!("CARGO_PKG_VERSION"));
            process::exit(0);
        }
        Some(cmd) => cmd.to_string(),
    };

    let rest = args[2..].to_vec();

    match subcommand.as_str() {
        "apply" => cmd_apply(&rest),
        "plan" => cmd_plan(&rest),
        "migrate" => cmd_migrate(&rest),
        other => {
            eprintln!("error: unknown command: {other}");
            eprintln!();
            eprintln!("{USAGE}");
            process::exit(1);
        }
    }
}

// ── apply ─────────────────────────────────────────────────────────────────────

fn cmd_apply(args: &[String]) {
    let profile_path = require_profile_arg(args, "apply");
    let verbose = args.contains(&"--verbose".to_string());

    let ctx = build_app_context();

    println!("Applying profile: {}", profile_path.display());

    let result = app::apply(&ctx, &profile_path, &mut |event| {
        use app::Event;
        match event {
            Event::FeatureStart { id } => {
                if verbose {
                    println!("  → {id}");
                }
            }
            Event::FeatureDone { id } => {
                println!("  ✓ {id}");
            }
            Event::ResourceFailed {
                feature_id,
                resource_id,
                error,
            } => {
                eprintln!("  ✗ [{feature_id}] resource '{resource_id}': {error}");
            }
            Event::FeatureFailed { id, error } => {
                eprintln!("  ✗ {id}: {error}");
            }
        }
    });

    match result {
        Ok(report) => {
            println!();
            if report.failed.is_empty() {
                println!("Profile applied successfully.");
            } else {
                println!("Profile applied with errors.");
            }
            if !report.executed.is_empty() {
                println!();
                println!("Executed ({}):", report.executed.len());
                for f in &report.executed {
                    println!("  {} [{}]", f.id, f.operation);
                }
            }
            if !report.failed.is_empty() {
                println!();
                println!("Failed ({}):", report.failed.len());
                for f in &report.failed {
                    println!("  {} [{}]: {}", f.id, f.operation, f.error);
                }
                println!();
                process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    }
}

// ── plan ──────────────────────────────────────────────────────────────────────

fn cmd_plan(args: &[String]) {
    let profile_path = require_profile_arg(args, "plan");
    let verbose = args.contains(&"--verbose".to_string());

    let ctx = build_app_context();

    match app::plan(&ctx, &profile_path) {
        Ok(plan) => {
            print_plan(&plan, verbose);
        }
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    }
}

fn print_plan(plan: &model::Plan, verbose: bool) {
    use model::plan::Operation;

    let has_anything =
        !plan.actions.is_empty() || !plan.blocked.is_empty() || (verbose && !plan.noops.is_empty());

    if !has_anything {
        println!("Nothing to do.");
        return;
    }

    if !plan.actions.is_empty() {
        println!("Actions:");
        for action in &plan.actions {
            let op_label = match action.operation {
                Operation::Create => "create",
                Operation::Destroy => "destroy",
                Operation::Replace => "replace",
                Operation::ReplaceBackend => "replace-backend",
                Operation::Strengthen => "strengthen",
            };
            println!("  [{op_label}] {}", action.feature.as_str());
        }
        println!();
    }

    if !plan.blocked.is_empty() {
        println!("Blocked:");
        for entry in &plan.blocked {
            println!("  [blocked] {}: {}", entry.feature.as_str(), entry.reason);
        }
        println!();
    }

    if verbose && !plan.noops.is_empty() {
        println!("No-op (already up to date):");
        for entry in &plan.noops {
            println!("  [noop] {}", entry.feature.as_str());
        }
        println!();
    }

    let s = &plan.summary;
    let total_action = s.create + s.destroy + s.replace + s.replace_backend + s.strengthen;
    print!("Summary: {total_action} action(s)");
    if s.blocked > 0 {
        print!(", {} blocked", s.blocked);
    }
    if verbose {
        print!(", {} noop", plan.noops.len());
    }
    println!();
}

// ── migrate ───────────────────────────────────────────────────────────────────

fn cmd_migrate(args: &[String]) {
    let dry_run = args.contains(&"--dry-run".to_string());

    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("Usage: loadout migrate [--dry-run]");
        println!();
        println!("Migrate the state file to the current schema version.");
        println!("  --dry-run  Show what would change without writing");
        process::exit(0);
    }

    let ctx = build_app_context();
    let state_path = ctx.state_path();

    if !state_path.exists() {
        println!(
            "State file not found: {} — nothing to migrate.",
            state_path.display()
        );
        process::exit(0);
    }

    // Load raw JSON to inspect version without triggering version guards.
    let raw = match state::load_raw(&state_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error reading state: {e}");
            process::exit(1);
        }
    };

    let version = raw.get("version").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

    match version {
        3 => {
            println!("State is already at version 3 — nothing to migrate.");
        }
        2 => {
            println!("Migrating state v2 → v3 ...");
            match state::migrate_v2_to_v3(&raw) {
                Ok(migrated) => {
                    let feature_count = migrated.features.len();
                    if dry_run {
                        println!(
                            "[dry-run] Would migrate {feature_count} feature(s). \
                             No changes written."
                        );
                    } else {
                        if let Err(e) = state::commit(&state_path, &migrated) {
                            eprintln!("error: failed to commit migrated state: {e}");
                            process::exit(1);
                        }
                        println!("Migration complete. {feature_count} feature(s) migrated.");
                    }
                }
                Err(e) => {
                    eprintln!("error: migration failed: {e}");
                    process::exit(1);
                }
            }
        }
        other => {
            eprintln!("error: unknown state version {other}; cannot migrate");
            process::exit(1);
        }
    }
}

// ── Context ───────────────────────────────────────────────────────────────────

/// Build an `AppContext` from the current environment.
///
/// See [`resolve_repo_root`] for repository root resolution order.
/// Platform and XDG/AppData dirs are detected automatically.
fn build_app_context() -> app::AppContext {
    let repo_root = resolve_repo_root();
    let platform = platform::detect_platform();
    let dirs = platform::resolve_dirs(&platform).unwrap_or_else(|e| {
        eprintln!("error: failed to resolve directories: {e}");
        process::exit(1);
    });
    app::AppContext::new(repo_root, platform, dirs)
}

/// Resolve the loadout repository root directory.
///
/// Resolution order:
///   1. `LOADOUT_ROOT` environment variable (explicit override)
///   2. Parent of the binary's directory, if it contains `features/`
///      (tarball layout: `<install_root>/bin/loadout → ../`)
///   3. Parent of the binary's directory (unconditional fallback)
fn resolve_repo_root() -> PathBuf {
    // 1. Explicit override via environment variable.
    if let Ok(root) = env::var("LOADOUT_ROOT") {
        let p = PathBuf::from(&root);
        if p.is_dir() {
            return p;
        }
        eprintln!("warning: LOADOUT_ROOT={root} is not a directory; falling back");
    }

    // 2. Tarball layout: binary is at `<install_root>/bin/loadout`.
    if let Some(candidate) = exe_dir().parent().map(Path::to_path_buf) {
        if looks_like_repo_root(&candidate) {
            return candidate;
        }
    }

    // 3. Last resort: exe parent even without a features/ directory.
    exe_dir()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(exe_dir)
}

/// Returns `true` if `path` looks like the loadout repository root.
///
/// The presence of a `features/` directory is used as the heuristic.
fn looks_like_repo_root(path: &Path) -> bool {
    path.join("features").is_dir()
}

/// Returns the directory containing this binary, following symlinks.
fn exe_dir() -> PathBuf {
    env::current_exe()
        .expect("failed to locate current executable")
        .canonicalize()
        .unwrap_or_else(|_| env::current_exe().expect("failed to locate current executable"))
        .parent()
        .expect("executable has no parent directory")
        .to_path_buf()
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Require the first non-flag positional argument as a profile path.
fn require_profile_arg(args: &[String], subcommand: &str) -> PathBuf {
    args.iter()
        .find(|a| !a.starts_with('-'))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            eprintln!("error: {subcommand} requires a <profile> argument");
            eprintln!("Usage: loadout {subcommand} <profile.yaml>");
            process::exit(1);
        })
}
