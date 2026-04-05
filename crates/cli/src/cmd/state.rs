// crates/cli/src/cmd/state.rs — `loadout state` subcommand dispatch and implementations

use std::process;

use crate::args::{MigrateArgs, OutputFormat, StateCommand, StateShowArgs};
use crate::context::build_app_context;

pub fn run(cmd: StateCommand) {
    match cmd {
        StateCommand::Show(args) => show_state(args),
        StateCommand::Migrate(args) => migrate(args),
    }
}

fn show_state(args: StateShowArgs) {
    let ctx = build_app_context();
    let st = app::show_state(&ctx).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });

    match args.output.output {
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&st).unwrap_or_else(|e| {
                eprintln!("error: failed to serialize state: {e}");
                process::exit(1);
            });
            println!("{json}");
        }
        OutputFormat::Text => print_state_text(&st),
    }
}

fn print_state_text(st: &state::State) {
    let feature_count = st.features.len();
    let resource_count: usize = st.features.values().map(|f| f.resources.len()).sum();

    println!("version:   {}", st.version);
    println!("features:  {feature_count}");
    println!("resources: {resource_count}");

    if feature_count == 0 {
        println!("\n(no features installed)");
        return;
    }

    println!();
    let mut features: Vec<(&String, &state::FeatureState)> = st.features.iter().collect();
    features.sort_by_key(|(id, _)| id.as_str());

    for (id, feature) in &features {
        let n = feature.resources.len();
        println!(
            "  {id:<40}  {} resource{}",
            n,
            if n == 1 { "" } else { "s" }
        );
    }
}

fn migrate(args: MigrateArgs) {
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
