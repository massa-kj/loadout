// crates/cli/src/cmd/backend/show.rs — `loadout backend show` implementation

use std::path::Path;
use std::process;

use serde::Serialize;

use crate::args::{BackendShowArgs, OutputFormat};
use crate::context::build_app_context;

#[derive(Serialize)]
struct BackendDetail {
    id: String,
    source: String,
    dir: String,
    api_version: u32,
    scripts: Scripts,
}

#[derive(Serialize)]
struct Scripts {
    apply: bool,
    remove: bool,
    status: bool,
    env_pre: bool,
    env_post: bool,
}

pub fn run(args: BackendShowArgs) {
    let ctx = build_app_context();

    let id = args.id.clone();
    // Validate the id format: must be `source/name`.
    let parts: Vec<&str> = id.splitn(2, '/').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        eprintln!(
            "error: invalid backend ID '{}'; expected 'source/name' (e.g. 'local/mise')",
            args.id
        );
        process::exit(1);
    }
    let source_id = parts[0];

    let sources = app::load_sources(&ctx).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });

    let all_dirs = app::scan_backend_dirs(&ctx, &sources);
    let backend_dir = all_dirs
        .iter()
        .find(|(bid, _)| bid == &id)
        .map(|(_, path)| path.clone())
        .unwrap_or_else(|| {
            eprintln!("error: backend '{id}' not found");
            process::exit(1);
        });

    let api_version = read_api_version(&backend_dir);

    let platform_ext = platform_script_ext(&ctx.platform);
    let scripts = Scripts {
        apply: has_script(&backend_dir, "apply", platform_ext),
        remove: has_script(&backend_dir, "remove", platform_ext),
        status: has_script(&backend_dir, "status", platform_ext),
        env_pre: has_script(&backend_dir, "env_pre", platform_ext),
        env_post: has_script(&backend_dir, "env_post", platform_ext),
    };

    match args.output.output {
        OutputFormat::Json => {
            let detail = BackendDetail {
                id: id.clone(),
                source: source_id.to_string(),
                dir: backend_dir.display().to_string(),
                api_version,
                scripts,
            };
            let json = serde_json::to_string_pretty(&detail).unwrap_or_else(|e| {
                eprintln!("error: {e}");
                process::exit(1);
            });
            println!("{json}");
        }
        OutputFormat::Text => {
            println!("id:          {id}");
            println!("source:      {source_id}");
            println!("dir:         {}", backend_dir.display());
            println!("api_version: {api_version}");
            println!("scripts:");
            println!("  apply:    {}", present(scripts.apply));
            println!("  remove:   {}", present(scripts.remove));
            println!("  status:   {}", present(scripts.status));
            println!("  env_pre:  {}", present(scripts.env_pre));
            println!("  env_post: {}", present(scripts.env_post));
        }
    }
}

fn has_script(dir: &Path, name: &str, ext: &str) -> bool {
    dir.join(format!("{name}.{ext}")).exists()
}

fn present(exists: bool) -> &'static str {
    if exists {
        "present"
    } else {
        "absent"
    }
}

fn platform_script_ext(platform: &platform::Platform) -> &'static str {
    match platform {
        platform::Platform::Windows => "ps1",
        _ => "sh",
    }
}

fn read_api_version(dir: &Path) -> u32 {
    let yaml_path = dir.join("backend.yaml");
    let Ok(content) = std::fs::read_to_string(&yaml_path) else {
        return 0;
    };
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("api_version:") {
            if let Ok(n) = rest.trim().parse::<u32>() {
                return n;
            }
        }
    }
    0
}
