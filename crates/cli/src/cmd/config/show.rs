// crates/cli/src/cmd/config/show.rs — `loadout config show` implementation

use std::process;

use serde::Serialize;

use crate::args::{ConfigShowArgs, OutputFormat};
use crate::context::{build_app_context, resolve_config_path};

#[derive(Serialize)]
struct ConfigShowOutput {
    name: String,
    path: String,
    features: Vec<FeatureEntry>,
}

#[derive(Serialize)]
struct FeatureEntry {
    id: String,
    version: Option<String>,
}

pub fn run(args: ConfigShowArgs) {
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
            let mut features: Vec<FeatureEntry> = detail
                .profile
                .features
                .iter()
                .map(|(id, cfg)| FeatureEntry {
                    id: id.clone(),
                    version: cfg.version.clone(),
                })
                .collect();
            features.sort_by(|a, b| a.id.cmp(&b.id));
            let out = ConfigShowOutput {
                name: detail.name.clone(),
                path: detail.path.display().to_string(),
                features,
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
            println!("features ({}):", detail.profile.features.len());

            let mut features: Vec<(&String, &config::ProfileFeatureConfig)> =
                detail.profile.features.iter().collect();
            features.sort_by_key(|(id, _)| id.as_str());

            for (id, cfg) in &features {
                if let Some(ver) = &cfg.version {
                    println!("  {id}  (version: {ver})");
                } else {
                    println!("  {id}");
                }
            }
        }
    }
}
