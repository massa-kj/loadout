// crates/cli/src/cmd/feature/list.rs — `loadout feature list` implementation

use std::collections::BTreeMap;
use std::process;

use crate::args::{FeatureListArgs, OutputFormat};
use crate::context::build_app_context;

pub fn run(args: FeatureListArgs) {
    let ctx = build_app_context();
    let items = app::list_features(&ctx, args.source.as_deref()).unwrap_or_else(|e| {
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
                println!("(no features found)");
                return;
            }
            // Group by source for human-readable output.
            let mut by_source: BTreeMap<&str, Vec<&app::FeatureSummary>> = BTreeMap::new();
            for item in &items {
                let source = item.id.split('/').next().unwrap_or("unknown");
                by_source.entry(source).or_default().push(item);
            }
            for (source, group) in &by_source {
                println!("source: {source}");
                for s in group {
                    let mode = match s.mode {
                        model::feature_index::FeatureMode::Script => "script     ",
                        model::feature_index::FeatureMode::Declarative => "declarative",
                    };
                    let desc = s.description.as_deref().unwrap_or("");
                    println!("  {:<36}  {mode}  {desc}", s.id);
                }
                println!();
            }
        }
    }
}
