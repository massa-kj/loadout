// crates/cli/src/cmd/backend/list.rs — `loadout backend list` implementation

use std::process;

use serde::Serialize;

use crate::args::{BackendListArgs, OutputFormat};
use crate::context::build_app_context;

#[derive(Serialize)]
struct BackendEntry {
    id: String,
    source: String,
    dir: String,
    api_version: u32,
}

pub fn run(args: BackendListArgs) {
    let ctx = build_app_context();

    let sources = app::load_sources(&ctx).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });

    let dirs = app::scan_backend_dirs(&ctx, &sources);

    // Filter by source if requested.
    let filtered: Vec<_> = dirs
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
            let entries: Vec<BackendEntry> = filtered
                .iter()
                .map(|(id, path)| {
                    let source = id.split('/').next().unwrap_or("unknown").to_string();
                    BackendEntry {
                        id: id.clone(),
                        source,
                        dir: path.display().to_string(),
                        api_version: read_api_version(path),
                    }
                })
                .collect();
            let json = serde_json::to_string_pretty(&entries).unwrap_or_else(|e| {
                eprintln!("error: {e}");
                process::exit(1);
            });
            println!("{json}");
        }
        OutputFormat::Text => {
            if filtered.is_empty() {
                println!("(no backends found)");
                return;
            }
            for (id, path) in &filtered {
                println!("  {id:<32}  {}", path.display());
            }
        }
    }
}

/// Read `api_version` from `backend.yaml`, returning 0 if absent or unparseable.
fn read_api_version(dir: &std::path::Path) -> u32 {
    let yaml_path = dir.join("backend.yaml");
    let Ok(content) = std::fs::read_to_string(&yaml_path) else {
        return 0;
    };
    serde_yaml_value(&content)
        .and_then(|v: serde_json::Value| v.get("api_version")?.as_u64().map(|n| n as u32))
        .unwrap_or(0)
}

/// Quick YAML-to-JSON-value parse without pulling in a full YAML crate.
/// Only works for simple flat YAML; sufficient for `backend.yaml`.
fn serde_yaml_value(yaml: &str) -> Option<serde_json::Value> {
    let mut map = serde_json::Map::new();
    for line in yaml.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_string();
            let val = v.trim();
            if let Ok(n) = val.parse::<u64>() {
                map.insert(key, serde_json::Value::Number(n.into()));
            } else {
                map.insert(key, serde_json::Value::String(val.to_string()));
            }
        }
    }
    Some(serde_json::Value::Object(map))
}
