// crates/cli/src/cmd/config.rs — `loadout config` subcommand dispatch and implementations

use std::process;

use serde::Serialize;

use crate::args::{
    ConfigCommand, ConfigComponentAddArgs, ConfigComponentCommand, ConfigComponentRemoveArgs,
    ConfigInitArgs, ConfigRawCommand, ConfigRawSetArgs, ConfigRawShowArgs, ConfigRawUnsetArgs,
    ConfigShowArgs, OutputArgs, OutputFormat,
};
use crate::context::{build_app_context, resolve_config_path};

pub fn run(cmd: ConfigCommand) {
    match cmd {
        ConfigCommand::Show(args) => show(args),
        ConfigCommand::List(args) => list(args),
        ConfigCommand::Edit => edit(),
        ConfigCommand::Init(args) => init(args),
        ConfigCommand::Component { command } => match command {
            ConfigComponentCommand::Add(args) => component_add(args),
            ConfigComponentCommand::Remove(args) => component_remove(args),
        },
        ConfigCommand::Raw { command } => match command {
            ConfigRawCommand::Show(args) => raw_show(args),
            ConfigRawCommand::Set(args) => raw_set(args),
            ConfigRawCommand::Unset(args) => raw_unset(args),
        },
    }
}

fn list(args: OutputArgs) {
    let ctx = build_app_context();
    let active = crate::cmd::context::read_current_context(&ctx.dirs);
    let entries = app::list_configs(&ctx, active.as_deref());

    match args.output {
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&entries).unwrap_or_else(|e| {
                eprintln!("error: {e}");
                process::exit(1);
            });
            println!("{json}");
        }
        OutputFormat::Text => {
            if entries.is_empty() {
                let configs_dir = ctx.dirs.config_home.join("configs");
                println!("(no configs found in {})", configs_dir.display());
                return;
            }
            for e in &entries {
                let marker = if e.active { " *" } else { "  " };
                println!("{marker} {:<24}  {}", e.name, e.path);
            }
        }
    }
}

#[derive(Serialize)]
struct ConfigShowOutput {
    name: String,
    path: String,
    components: Vec<ComponentEntry>,
}

#[derive(Serialize)]
struct ComponentEntry {
    id: String,
    version: Option<String>,
}

fn show(args: ConfigShowArgs) {
    let ctx = build_app_context();

    // Resolve which config to show: explicit arg > current context > error.
    let config_name = args
        .name
        .or_else(|| crate::cmd::context::read_current_context(&ctx.dirs));

    let Some(name_or_path) = config_name else {
        eprintln!("error: no config specified and no context is set");
        eprintln!("hint: run 'loadout context set <name>' or pass a config name");
        process::exit(1);
    };

    let config_path = resolve_config_path(&name_or_path, &ctx.dirs);
    let detail = app::show_config(&ctx, &config_path).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });

    match args.output.output {
        OutputFormat::Json => {
            let mut components: Vec<ComponentEntry> = detail
                .profile
                .components
                .iter()
                .map(|(id, cfg)| ComponentEntry {
                    id: id.clone(),
                    version: cfg.version.clone(),
                })
                .collect();
            components.sort_by(|a, b| a.id.cmp(&b.id));
            let out = ConfigShowOutput {
                name: detail.name.clone(),
                path: detail.path.display().to_string(),
                components,
            };
            let json = serde_json::to_string_pretty(&out).unwrap_or_else(|e| {
                eprintln!("error: {e}");
                process::exit(1);
            });
            println!("{json}");
        }
        OutputFormat::Text => {
            println!("name:  {}", detail.name);
            println!("path:  {}", detail.path.display());
            println!("components ({}):", detail.profile.components.len());

            let mut components: Vec<(&String, &config::ProfileComponentConfig)> =
                detail.profile.components.iter().collect();
            components.sort_by_key(|(id, _)| id.as_str());

            for (id, cfg) in &components {
                if let Some(ver) = &cfg.version {
                    println!("  {id}  (version: {ver})");
                } else {
                    println!("  {id}");
                }
            }
        }
    }
}

// ── edit ─────────────────────────────────────────────────────────────────────

fn edit() {
    let ctx = build_app_context();
    let name = crate::cmd::context::read_current_context(&ctx.dirs).unwrap_or_else(|| {
        eprintln!("error: no context is set");
        eprintln!(
            "hint: run 'loadout context set <name>' first, or use 'loadout config show <name>'"
        );
        process::exit(1);
    });
    let path = resolve_config_path(&name, &ctx.dirs);
    if !path.exists() {
        eprintln!("error: config file not found: {}", path.display());
        eprintln!("hint: run 'loadout config init {}' to create it", name);
        process::exit(1);
    }
    super::editor::open(&path);
}

// ── init ─────────────────────────────────────────────────────────────────────

fn init(args: ConfigInitArgs) {
    let ctx = build_app_context();
    let path = app::config_init(&ctx, &args.name).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });
    println!("Created: {}", path.display());
    println!(
        "Run 'loadout context set {}' to make it the active config.",
        args.name
    );
}

// ── component add / remove ─────────────────────────────────────────────

fn component_add(args: ConfigComponentAddArgs) {
    let ctx = build_app_context();
    let path =
        app::config_component_add(&ctx, args.config.as_deref(), &args.id).unwrap_or_else(|e| {
            eprintln!("error: {e}");
            process::exit(1);
        });
    println!("Added '{}' to {}.", args.id, path.display());
}

fn component_remove(args: ConfigComponentRemoveArgs) {
    let ctx = build_app_context();
    let (path, found) = app::config_component_remove(&ctx, args.config.as_deref(), &args.id)
        .unwrap_or_else(|e| {
            eprintln!("error: {e}");
            process::exit(1);
        });
    if found {
        println!("Removed '{}' from {}.", args.id, path.display());
    } else {
        println!(
            "'{}' was not found in {} — no change.",
            args.id,
            path.display()
        );
    }
}

// ── raw show / set / unset ───────────────────────────────────────────────────

fn raw_show(args: ConfigRawShowArgs) {
    let ctx = build_app_context();
    let content = app::config_raw_show(&ctx, args.name.as_deref()).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });
    print!("{content}");
}

fn raw_set(args: ConfigRawSetArgs) {
    let ctx = build_app_context();
    let path = app::config_raw_set(&ctx, args.config.as_deref(), &args.path, &args.value)
        .unwrap_or_else(|e| {
            eprintln!("error: {e}");
            process::exit(1);
        });
    println!("Set '{}' in {}.", args.path, path.display());
}

fn raw_unset(args: ConfigRawUnsetArgs) {
    let ctx = build_app_context();
    let (path, found) = app::config_raw_unset(&ctx, args.config.as_deref(), &args.path)
        .unwrap_or_else(|e| {
            eprintln!("error: {e}");
            process::exit(1);
        });
    if found {
        println!("Removed '{}' from {}.", args.path, path.display());
    } else {
        println!(
            "'{}' was not found in {} — no change.",
            args.path,
            path.display()
        );
    }
}
