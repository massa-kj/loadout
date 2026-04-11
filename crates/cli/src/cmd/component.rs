// crates/cli/src/cmd/component.rs — `loadout component` subcommand dispatch and implementations

use std::collections::BTreeMap;
use std::process;

use crate::args::{
    ComponentCommand, ComponentEditArgs, ComponentImportArgs, ComponentListArgs, ComponentNewArgs,
    ComponentShowArgs, ComponentTemplate, ComponentValidateArgs, OutputFormat,
};
use crate::context::build_app_context;

pub fn run(cmd: ComponentCommand) {
    match cmd {
        ComponentCommand::List(args) => list(args),
        ComponentCommand::Show(args) => show(args),
        ComponentCommand::Edit(args) => edit(args),
        ComponentCommand::New(args) => new(args),
        ComponentCommand::Validate(args) => validate(args),
        ComponentCommand::Import(args) => import(args),
    }
}

fn list(args: ComponentListArgs) {
    let ctx = build_app_context();
    let items = app::list_components(&ctx, args.source.as_deref()).unwrap_or_else(|e| {
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
                println!("(no components found)");
                return;
            }
            // Group by source for human-readable output.
            let mut by_source: BTreeMap<&str, Vec<&app::ComponentSummary>> = BTreeMap::new();
            for item in &items {
                let source = item.id.split('/').next().unwrap_or("unknown");
                by_source.entry(source).or_default().push(item);
            }
            for (source, group) in &by_source {
                println!("source: {source}");
                for s in group {
                    let mode = match s.mode {
                        model::component_index::ComponentMode::Script => "script     ",
                        model::component_index::ComponentMode::Declarative => "declarative",
                    };
                    let desc = s.description.as_deref().unwrap_or("");
                    println!("  {:<36}  {mode}  {desc}", s.id);
                }
                println!();
            }
        }
    }
}

fn show(args: ComponentShowArgs) {
    let ctx = build_app_context();
    let detail = app::show_component(&ctx, &args.id).unwrap_or_else(|e| {
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
        OutputFormat::Text => print_component_text(&detail),
    }
}

fn print_component_text(detail: &app::ComponentDetail) {
    let meta = &detail.meta;
    println!("component:    {}", detail.id);
    println!(
        "mode:       {}",
        match meta.mode {
            model::component_index::ComponentMode::Script => "script",
            model::component_index::ComponentMode::Declarative => "declarative",
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
                model::component_index::SpecResourceKind::Package { name } => {
                    println!("    kind: package  name: {name}");
                }
                model::component_index::SpecResourceKind::Runtime { name, version } => {
                    println!("    kind: runtime  name: {name}  version: {version}");
                }
                model::component_index::SpecResourceKind::Fs {
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

fn edit(args: ComponentEditArgs) {
    let ctx = build_app_context();

    // Resolve bare name to `local/<name>`; reject non-local sources.
    let id = resolve_local_id(&args.name, "component");

    // Derive the path directly from local_root — avoids building the full
    // component index (which would fail if any OTHER local component is broken).
    let name = id.trim_start_matches("local/");
    let component_yaml = ctx
        .local_root
        .join("components")
        .join(name)
        .join("component.yaml");
    if !component_yaml.exists() {
        eprintln!("error: component not found: {}", component_yaml.display());
        eprintln!("hint: run 'loadout component new {name}' to create it");
        process::exit(1);
    }
    super::editor::open(&component_yaml);
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

/// Resolve a bare name or canonical ID for validation.
/// Unlike `resolve_local_id`, this accepts any source prefix.
fn resolve_id_for_validate(name: &str) -> String {
    if name.contains('/') {
        name.to_string()
    } else {
        format!("local/{name}")
    }
}

// ── new ───────────────────────────────────────────────────────────────────────

fn new(args: ComponentNewArgs) {
    let ctx = build_app_context();
    let template = match args.template {
        ComponentTemplate::Declarative => app::ComponentTemplate::Declarative,
        ComponentTemplate::Script => app::ComponentTemplate::Script,
    };
    let dir = app::component_new(&ctx, &args.name, template).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });
    println!("Created: {}", dir.display());
    println!(
        "Edit component.yaml, then run 'loadout config component add {}' to include it.",
        args.name
    );
}

// ── validate ──────────────────────────────────────────────────────────────────

fn validate(args: ComponentValidateArgs) {
    let ctx = build_app_context();
    let id = resolve_id_for_validate(&args.id);

    let report = app::component_validate(&ctx, &id).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });

    print_validation_report(&report);

    if !report.is_ok() {
        process::exit(1);
    }
}

fn print_validation_report(report: &app::ValidationReport) {
    println!("component:  {}", report.id);
    println!("path:     {}", report.path.display());
    if report.issues.is_empty() {
        println!("result:   OK");
    } else {
        println!("result:   {} issue(s)", report.issues.len());
        for issue in &report.issues {
            let tag = match issue.level {
                app::IssueLevel::Error => "[error]",
                app::IssueLevel::Warning => "[warn] ",
            };
            println!("  {tag}  {}", issue.message);
        }
    }
}

// ── import ────────────────────────────────────────────────────────────────────

fn import(args: ComponentImportArgs) {
    let ctx = build_app_context();
    let report = app::component_import(&ctx, &args.id, args.move_config, args.dry_run)
        .unwrap_or_else(|e| {
            eprintln!("error: {e}");
            process::exit(1);
        });

    if args.dry_run {
        println!("(dry run — no changes will be made)");
    }

    println!("from:  {}", report.source_dir.display());
    println!("to:    {}", report.dest_dir.display());

    if !report.bare_depends_warnings.is_empty() {
        eprintln!("warning: component has bare depends (same-source references):");
        for dep in &report.bare_depends_warnings {
            eprintln!("  - {dep}");
        }
        eprintln!(
            "hint: bare depends may not resolve correctly after import; \
             consider converting them to canonical IDs (e.g. local/{dep})",
            dep = report.bare_depends_warnings[0]
        );
    }

    if args.move_config {
        if report.config_files_updated.is_empty() {
            println!("configs: (no files reference this component)");
        } else {
            println!("configs rewritten:");
            for p in &report.config_files_updated {
                println!("  {}", p.display());
            }
        }
    }

    if !args.dry_run {
        if args.move_config && !report.config_files_updated.is_empty() {
            println!("imported '{}' and rewrote config references", args.id);
        } else {
            println!("imported '{}'", args.id);
        }
    }
}
