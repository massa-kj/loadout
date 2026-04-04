// crates/cli/src/cmd/plan.rs — `loadout plan` implementation

use crate::args::PlanArgs;
use crate::context::{build_app_context, config_id_from_path, resolve_config_path};
use crate::output;
use std::process;

pub fn run(args: PlanArgs) {
    let mut ctx = build_app_context();

    let config_value = match args.config.config {
        Some(v) => v,
        None => {
            eprintln!("error: plan requires -c / --config <name|path>");
            eprintln!("  name example:  loadout plan -c linux");
            eprintln!("  path example:  loadout plan -c ./configs/work.yaml");
            process::exit(1);
        }
    };
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
