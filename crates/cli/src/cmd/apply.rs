// crates/cli/src/cmd/apply.rs — `loadout apply` implementation

use crate::args::ApplyArgs;
use crate::context::{build_app_context, config_id_from_path, resolve_config_path};
use crate::output;
use std::io::{self, Write};
use std::process;

pub fn run(args: ApplyArgs) {
    let mut ctx = build_app_context();

    let config_value = resolve_config_arg(args.config.config, "apply", &ctx.dirs);
    let config_path = resolve_config_path(&config_value, &ctx.dirs);
    ctx.sources_override = args.config.sources;

    let config_id = config_id_from_path(&config_path);
    println!("Using config: {config_id}");

    // Phase 1: Prepare execution (plan generation)
    let execution_plan = match app::prepare_execution(&ctx, &config_path) {
        Ok(plan) => plan,
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    };

    // Phase 2: Display plan and confirm (unless -y is specified or plan is empty)
    if !execution_plan.plan.actions.is_empty() && !args.yes {
        output::plan::print_plan(&execution_plan.plan, args.verbose);
        println!();

        match confirm_apply() {
            Ok(()) => {}
            Err(e) => {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
    }

    // Phase 3: Execute the plan
    let result = app::execute(&ctx, execution_plan, &mut |event| {
        use app::Event;
        match event {
            Event::ComponentStart { id } => {
                if args.verbose {
                    println!("  → {id}");
                }
            }
            Event::ComponentDone { id } => {
                println!("  ✓ {id}");
            }
            Event::ResourceFailed {
                component_id,
                resource_id,
                error,
            } => {
                eprintln!("  ✗ [{component_id}] resource '{resource_id}': {error}");
            }
            Event::ComponentFailed { id, error } => {
                eprintln!("  ✗ {id}: {error}");
            }
            Event::ContributorWarning { backend_id, reason } => {
                eprintln!("  ⚠ contributor '{backend_id}': {reason}");
            }
        }
    });

    match result {
        Ok(report) => {
            if !output::report::print_apply_report(&report) {
                process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    }
}

/// Ask the user to confirm before applying.
///
/// Returns `Ok(())` if confirmed, `Err` if aborted or I/O fails.
fn confirm_apply() -> Result<(), String> {
    print!("Apply this plan? [y/N]: ");
    io::stdout().flush().ok();

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|e| format!("Failed to read input: {e}"))?;

    match input.trim().to_lowercase().as_str() {
        "y" | "yes" => Ok(()),
        _ => Err("Aborted by user".to_string()),
    }
}

/// Resolve the config name/path, falling back to the current context if `-c` is omitted.
fn resolve_config_arg(
    config_flag: Option<String>,
    subcommand: &str,
    dirs: &platform::Dirs,
) -> String {
    if let Some(v) = config_flag {
        return v;
    }
    if let Some(name) = crate::cmd::context::read_current_context(dirs) {
        eprintln!("Using context: {name}");
        return name;
    }
    eprintln!("error: {subcommand} requires -c / --config <name|path>");
    eprintln!("  name example:  loadout {subcommand} -c linux");
    eprintln!("  path example:  loadout {subcommand} -c ./configs/work.yaml");
    eprintln!("  or set a context: loadout context set <name>");
    std::process::exit(1);
}
