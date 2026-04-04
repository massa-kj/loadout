// crates/cli/src/cmd/migrate.rs — `loadout migrate` implementation
//
// NOTE: This command will move to `loadout state migrate` in Phase 2.
// At that point this module is relocated to cmd/state/migrate.rs and the
// top-level `migrate` entry in args.rs becomes a deprecated alias.

use crate::args::MigrateArgs;
use crate::context::build_app_context;
use std::process;

pub fn run(args: MigrateArgs) {
    let ctx = build_app_context();
    let state_path = ctx.state_path();

    if !state_path.exists() {
        println!(
            "State file not found: {} — nothing to migrate.",
            state_path.display()
        );
        process::exit(0);
    }

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
                    if args.dry_run {
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
