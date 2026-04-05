// crates/cli/src/cmd/source/list.rs — `loadout source list` implementation

use std::process;

use crate::args::{OutputArgs, OutputFormat};
use crate::context::build_app_context;

pub fn run(args: OutputArgs) {
    let ctx = build_app_context();
    let entries = app::list_sources(&ctx).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });

    match args.output {
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&entries).unwrap_or_else(|e| {
                eprintln!("error: {e}");
                process::exit(1);
            });
            println!("{json}");
        }
        OutputFormat::Text => {
            for e in &entries {
                let url_part = e.url.as_deref().unwrap_or("-");
                println!("  {:<16}  {:<8}  {url_part}", e.id, e.kind);
            }
        }
    }
}
