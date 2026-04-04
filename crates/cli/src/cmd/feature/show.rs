// crates/cli/src/cmd/feature/show.rs — `loadout feature show` implementation

use std::process;

use crate::args::{FeatureShowArgs, OutputFormat};
use crate::context::build_app_context;

pub fn run(args: FeatureShowArgs) {
    let ctx = build_app_context();

    let sources = app::load_sources(&ctx).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });

    let index = app::build_feature_index(&ctx, &sources).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });

    let meta = index.features.get(&args.id).unwrap_or_else(|| {
        eprintln!("error: feature '{}' not found", args.id);
        process::exit(1);
    });

    match args.output.output {
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(meta).unwrap_or_else(|e| {
                eprintln!("error: {e}");
                process::exit(1);
            });
            println!("{json}");
        }
        OutputFormat::Text => print_feature_text(&args.id, meta),
    }
}

fn print_feature_text(id: &str, meta: &model::feature_index::FeatureMeta) {
    println!("feature:    {id}");
    println!(
        "mode:       {}",
        match meta.mode {
            model::feature_index::FeatureMode::Script => "script",
            model::feature_index::FeatureMode::Declarative => "declarative",
        }
    );
    if let Some(desc) = &meta.description {
        println!("description: {desc}");
    }
    println!("source_dir: {}", meta.source_dir);

    if !meta.dep.depends.is_empty() {
        println!("depends:");
        for dep in &meta.dep.depends {
            println!("  - {dep}");
        }
    }

    if let Some(spec) = &meta.spec {
        println!("resources ({}):", spec.resources.len());
        for res in &spec.resources {
            println!("  - id: {}", res.id);
            match &res.kind {
                model::feature_index::SpecResourceKind::Package { name } => {
                    println!("    kind: package  name: {name}");
                }
                model::feature_index::SpecResourceKind::Runtime { name, version } => {
                    println!("    kind: runtime  name: {name}  version: {version}");
                }
                model::feature_index::SpecResourceKind::Fs {
                    path,
                    entry_type,
                    op,
                    ..
                } => {
                    println!("    kind: fs  path: {path}  type: {entry_type:?}  op: {op:?}",);
                }
            }
        }
    }
}
