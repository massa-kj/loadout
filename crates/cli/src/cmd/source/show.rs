// crates/cli/src/cmd/source/show.rs — `loadout source show` implementation

use std::process;

use crate::args::{OutputFormat, SourceShowArgs};
use crate::context::build_app_context;

pub fn run(args: SourceShowArgs) {
    let ctx = build_app_context();
    let detail = app::show_source(&ctx, &args.id).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });

    match args.output.output {
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&detail).unwrap_or_else(|e| {
                eprintln!("error: {e}");
                process::exit(1);
            });
            println!("{json}");
        }
        OutputFormat::Text => {
            println!("id:         {}", detail.id);
            println!("kind:       {}", detail.kind);
            if let Some(url) = &detail.url {
                println!("url:        {url}");
            }
            if let Some(commit) = &detail.commit {
                println!("commit:     {commit}");
            }
            if let Some(allow) = &detail.allow {
                println!("allow:      {allow}");
            }
            if let Some(local_path) = &detail.local_path {
                println!("local_path: {local_path}");
            }
        }
    }
}
