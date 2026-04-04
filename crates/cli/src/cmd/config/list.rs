// crates/cli/src/cmd/config/list.rs — `loadout config list` implementation

use std::path::{Path, PathBuf};
use std::process;

use serde::Serialize;

use crate::args::{OutputArgs, OutputFormat};
use crate::context::build_app_context;

#[derive(Serialize)]
struct ConfigEntry {
    name: String,
    path: String,
    active: bool,
}

pub fn run(args: OutputArgs) {
    let ctx = build_app_context();
    let configs_dir = ctx.dirs.config_home.join("configs");
    let active = crate::cmd::context::read_current_context(&ctx.dirs);
    let entries = collect_configs(&configs_dir);

    match args.output {
        OutputFormat::Json => {
            let json_entries: Vec<ConfigEntry> = entries
                .iter()
                .map(|(name, path)| ConfigEntry {
                    name: name.clone(),
                    path: path.display().to_string(),
                    active: active.as_deref() == Some(name.as_str()),
                })
                .collect();
            let json = serde_json::to_string_pretty(&json_entries).unwrap_or_else(|e| {
                eprintln!("error: {e}");
                process::exit(1);
            });
            println!("{json}");
        }
        OutputFormat::Text => {
            if entries.is_empty() {
                println!("(no configs found in {})", configs_dir.display());
                return;
            }
            for (name, path) in &entries {
                let marker = if active.as_deref() == Some(name.as_str()) {
                    " *"
                } else {
                    "  "
                };
                println!("{marker} {name:<24}  {}", path.display());
            }
        }
    }
}

/// Collect all `.yaml` / `.yml` files under `dir`, sorted by name.
fn collect_configs(dir: &Path) -> Vec<(String, PathBuf)> {
    if !dir.exists() {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut result: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let ext = path.extension()?.to_str()?;
            if ext != "yaml" && ext != "yml" {
                return None;
            }
            let name = path.file_stem()?.to_str()?.to_string();
            Some((name, path))
        })
        .collect();
    result.sort_by(|a, b| a.0.cmp(&b.0));
    result
}
