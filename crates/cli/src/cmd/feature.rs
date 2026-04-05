// crates/cli/src/cmd/feature.rs — `loadout feature` subcommand dispatch and implementations

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process;

use crate::args::{
    FeatureCommand, FeatureEditArgs, FeatureListArgs, FeatureShowArgs, OutputFormat,
};
use crate::context::build_app_context;

pub fn run(cmd: FeatureCommand) {
    match cmd {
        FeatureCommand::List(args) => list(args),
        FeatureCommand::Show(args) => show(args),
        FeatureCommand::Edit(args) => edit(args),
    }
}

fn list(args: FeatureListArgs) {
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

fn show(args: FeatureShowArgs) {
    let ctx = build_app_context();
    let detail = app::show_feature(&ctx, &args.id).unwrap_or_else(|e| {
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
        OutputFormat::Text => print_feature_text(&detail),
    }
}

fn print_feature_text(detail: &app::FeatureDetail) {
    let meta = &detail.meta;
    println!("feature:    {}", detail.id);
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
                    println!("    kind: fs  path: {path}  type: {entry_type:?}  op: {op:?}");
                }
            }
        }
    }
}

// ── edit ─────────────────────────────────────────────────────────────────────

fn edit(args: FeatureEditArgs) {
    let ctx = build_app_context();

    // Resolve bare name to `local/<name>`; reject non-local sources.
    let id = resolve_local_id(&args.name, "feature");

    let detail = app::show_feature(&ctx, &id).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });

    let feature_yaml = PathBuf::from(&detail.meta.source_dir).join("feature.yaml");
    super::editor::open(&feature_yaml);
}

fn resolve_local_id(name: &str, kind: &str) -> String {
    if name.contains('/') {
        if !name.starts_with("local/") {
            eprintln!(
                "error: only 'local' source {}s are editable (got '{name}')",
                kind
            );
            process::exit(1);
        }
        name.to_string()
    } else {
        format!("local/{name}")
    }
}
