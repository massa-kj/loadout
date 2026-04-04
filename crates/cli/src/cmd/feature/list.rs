// crates/cli/src/cmd/feature/list.rs — `loadout feature list` implementation

use std::collections::BTreeMap;
use std::process;

use crate::args::{FeatureListArgs, OutputFormat};
use crate::context::build_app_context;

pub fn run(args: FeatureListArgs) {
    let ctx = build_app_context();

    let sources = app::load_sources(&ctx).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });

    let index = app::build_feature_index(&ctx, &sources).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });

    // Filter by source if requested.
    let features: BTreeMap<&String, &model::feature_index::FeatureMeta> = index
        .features
        .iter()
        .filter(|(id, _)| {
            if let Some(ref filter) = args.source {
                id.starts_with(&format!("{filter}/"))
            } else {
                true
            }
        })
        .collect();

    match args.output.output {
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&index.features).unwrap_or_else(|e| {
                eprintln!("error: {e}");
                process::exit(1);
            });
            println!("{json}");
        }
        OutputFormat::Text => {
            if features.is_empty() {
                println!("(no features found)");
                return;
            }
            // Group by source for human-readable output.
            let mut by_source: BTreeMap<&str, Vec<(&String, &model::feature_index::FeatureMeta)>> =
                BTreeMap::new();
            for (id, meta) in &features {
                let source = id.split('/').next().unwrap_or("unknown");
                by_source.entry(source).or_default().push((id, meta));
            }
            for (source, items) in &by_source {
                println!("source: {source}");
                for (id, meta) in items {
                    let mode = match meta.mode {
                        model::feature_index::FeatureMode::Script => "script     ",
                        model::feature_index::FeatureMode::Declarative => "declarative",
                    };
                    let desc = meta.description.as_deref().unwrap_or("");
                    println!("  {id:<36}  {mode}  {desc}");
                }
                println!();
            }
        }
    }
}
