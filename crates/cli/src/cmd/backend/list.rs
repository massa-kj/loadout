// crates/cli/src/cmd/backend/list.rs — `loadout backend list` implementation

use std::process;

use crate::args::{BackendListArgs, OutputFormat};
use crate::context::build_app_context;

pub fn run(args: BackendListArgs) {
    let ctx = build_app_context();
    let items = app::list_backends(&ctx, args.source.as_deref()).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });

    match args.output.output {
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&items).unwrap_or_else(|e| {
                eprintln!("error: {e}");
                process::exit(1);
            });
            println!("{json}");
        }
        OutputFormat::Text => {
            if items.is_empty() {
                println!("(no backends found)");
                return;
            }
            for b in &items {
                println!("  {:<32}  {}", b.id, b.dir);
            }
        }
    }
}
