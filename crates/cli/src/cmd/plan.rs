// crates/cli/src/cmd/plan.rs — `loadout plan` implementation

use crate::args::PlanArgs;
use crate::context::{build_app_context, config_id_from_path, resolve_config_path};
use crate::output;
use std::process;

pub fn run(args: PlanArgs) {
    let mut ctx = build_app_context();

    let config_value = resolve_config_arg(args.config.config, "plan", &ctx.dirs);
    let config_path = resolve_config_path(&config_value, &ctx.dirs);
    ctx.sources_override = args.config.sources;

    let config_id = config_id_from_path(&config_path);
    println!("Using config: {config_id}");

    match app::plan(&ctx, &config_path) {
        Ok(plan) => {
            output::plan::print_plan(&plan, args.verbose);
        }
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
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
