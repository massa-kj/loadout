// crates/cli/src/cmd/backend.rs — `loadout backend` subcommand dispatch and implementations

use std::path::PathBuf;
use std::process;

use crate::args::{
    BackendCommand, BackendEditArgs, BackendListArgs, BackendShowArgs, OutputFormat,
};
use crate::context::build_app_context;

pub fn run(cmd: BackendCommand) {
    match cmd {
        BackendCommand::List(args) => list(args),
        BackendCommand::Show(args) => show(args),
        BackendCommand::Edit(args) => edit(args),
    }
}

fn list(args: BackendListArgs) {
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

fn show(args: BackendShowArgs) {
    let ctx = build_app_context();
    let detail = app::show_backend(&ctx, &args.id).unwrap_or_else(|e| {
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
        OutputFormat::Text => print_backend_text(&detail),
    }
}

fn print_backend_text(d: &app::BackendDetail) {
    println!("id:          {}", d.id);
    println!("source:      {}", d.source);
    println!("dir:         {}", d.dir);
    println!("api_version: {}", d.api_version);
    println!("scripts:");
    println!("  apply:    {}", present(d.scripts.apply));
    println!("  remove:   {}", present(d.scripts.remove));
    println!("  status:   {}", present(d.scripts.status));
    println!("  env_pre:  {}", present(d.scripts.env_pre));
    println!("  env_post: {}", present(d.scripts.env_post));
}

fn present(exists: bool) -> &'static str {
    if exists {
        "present"
    } else {
        "absent"
    }
}

// ── edit ─────────────────────────────────────────────────────────────────────

fn edit(args: BackendEditArgs) {
    let ctx = build_app_context();

    let id = if args.name.contains('/') {
        if !args.name.starts_with("local/") {
            eprintln!(
                "error: only 'local' source backends are editable (got '{}')",
                args.name
            );
            process::exit(1);
        }
        args.name.clone()
    } else {
        format!("local/{}", args.name)
    };

    let detail = app::show_backend(&ctx, &id).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });

    let backend_yaml = PathBuf::from(&detail.dir).join("backend.yaml");
    super::editor::open(&backend_yaml);
}
