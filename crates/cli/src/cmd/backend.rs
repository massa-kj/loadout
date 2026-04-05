// crates/cli/src/cmd/backend.rs — `loadout backend` subcommand dispatch and implementations

use std::process;

use crate::args::{
    BackendCommand, BackendEditArgs, BackendListArgs, BackendNewArgs, BackendPlatform,
    BackendShowArgs, BackendValidateArgs, OutputFormat,
};
use crate::context::build_app_context;

pub fn run(cmd: BackendCommand) {
    match cmd {
        BackendCommand::List(args) => list(args),
        BackendCommand::Show(args) => show(args),
        BackendCommand::Edit(args) => edit(args),
        BackendCommand::New(args) => new(args),
        BackendCommand::Validate(args) => validate(args),
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

    // Derive the path directly from local_root — avoids going through show_backend
    // which scans all backends and may encounter unrelated errors.
    let name = id.trim_start_matches("local/");
    let backend_yaml = ctx
        .local_root
        .join("backends")
        .join(name)
        .join("backend.yaml");
    if !backend_yaml.exists() {
        eprintln!("error: backend not found: {}", backend_yaml.display());
        eprintln!("hint: run 'loadout backend new {name}' to create it");
        process::exit(1);
    }
    super::editor::open(&backend_yaml);
}

// ── new ───────────────────────────────────────────────────────────────────────

fn new(args: BackendNewArgs) {
    let ctx = build_app_context();
    let platform = match args.platform {
        BackendPlatform::Unix => app::BackendPlatform::Unix,
        BackendPlatform::UnixWindows => app::BackendPlatform::UnixWindows,
    };
    let dir = app::backend_new(&ctx, &args.name, platform).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });
    println!("Created: {}", dir.display());
    println!(
        "Edit backend.yaml and implement the scripts, then run 'loadout backend validate {}' to check.",
        args.name
    );
}

// ── validate ──────────────────────────────────────────────────────────────────

fn validate(args: BackendValidateArgs) {
    let ctx = build_app_context();
    let id = if args.id.contains('/') {
        args.id.clone()
    } else {
        format!("local/{}", args.id)
    };

    let report = app::backend_validate(&ctx, &id).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });

    println!("backend:  {}", report.id);
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

    if !report.is_ok() {
        process::exit(1);
    }
}
