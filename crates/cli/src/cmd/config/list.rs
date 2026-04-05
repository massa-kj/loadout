// crates/cli/src/cmd/config/list.rs — `loadout config list` implementation

use std::process;

use crate::args::{OutputArgs, OutputFormat};
use crate::context::build_app_context;

pub fn run(args: OutputArgs) {
    let ctx = build_app_context();
    let active = crate::cmd::context::read_current_context(&ctx.dirs);
    let entries = app::list_configs(&ctx, active.as_deref());

    match args.output {
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&entries).unwrap_or_else(|e| {
                eprintln!("error: {e}");
                process::exit(1);
            });
            println!("{json}");
        }
        OutputFormat::Text => {
            if entries.is_empty() {
                let configs_dir = ctx.dirs.config_home.join("configs");
                println!("(no configs found in {})", configs_dir.display());
                return;
            }
            for e in &entries {
                let marker = if e.active { " *" } else { "  " };
                println!("{marker} {:<24}  {}", e.name, e.path);
            }
        }
    }
}
