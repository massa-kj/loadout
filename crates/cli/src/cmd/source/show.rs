// crates/cli/src/cmd/source/show.rs — `loadout source show` implementation

use std::process;

use serde::Serialize;

use crate::args::{OutputFormat, SourceShowArgs};
use crate::context::build_app_context;

#[derive(Serialize)]
struct SourceDetail {
    id: String,
    kind: &'static str,
    url: Option<String>,
    commit: Option<String>,
    allow: Option<String>,
    local_path: String,
}

pub fn run(args: SourceShowArgs) {
    let ctx = build_app_context();

    let sources = app::load_sources(&ctx).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });

    let detail = match args.id.as_str() {
        "core" => SourceDetail {
            id: "core".to_string(),
            kind: "implicit",
            url: None,
            commit: None,
            allow: Some("*".to_string()),
            local_path: "(built-in)".to_string(),
        },
        "local" => SourceDetail {
            id: "local".to_string(),
            kind: "implicit",
            url: None,
            commit: None,
            allow: Some("*".to_string()),
            local_path: ctx.local_root.display().to_string(),
        },
        id => {
            let entry = sources
                .sources
                .iter()
                .find(|e| e.id == id)
                .unwrap_or_else(|| {
                    eprintln!("error: source '{id}' not found in sources.yaml");
                    process::exit(1);
                });
            let allow = match &entry.allow {
                None => None,
                Some(model::sources::AllowSpec::All(_)) => Some("*".to_string()),
                Some(model::sources::AllowSpec::Detailed(d)) => {
                    let features = d.features.as_ref().map(|l| match l {
                        model::sources::AllowList::All(_) => "features:*".to_string(),
                        model::sources::AllowList::Names(v) => {
                            format!("features:[{}]", v.join(","))
                        }
                    });
                    let backends = d.backends.as_ref().map(|l| match l {
                        model::sources::AllowList::All(_) => "backends:*".to_string(),
                        model::sources::AllowList::Names(v) => {
                            format!("backends:[{}]", v.join(","))
                        }
                    });
                    Some(
                        [features, backends]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join(" "),
                    )
                }
            };
            let local_path = ctx
                .dirs
                .data_home
                .join("sources")
                .join(&entry.id)
                .display()
                .to_string();
            SourceDetail {
                id: entry.id.clone(),
                kind: "git",
                url: Some(entry.url.clone()),
                commit: entry.commit.clone(),
                allow,
                local_path,
            }
        }
    };

    match args.output.output {
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&detail).unwrap_or_else(|e| {
                eprintln!("error: {e}");
                process::exit(1);
            });
            println!("{json}");
        }
        OutputFormat::Text => {
            println!("id:         {}", detail.id);
            println!("kind:       {}", detail.kind);
            if let Some(url) = &detail.url {
                println!("url:        {url}");
            }
            if let Some(commit) = &detail.commit {
                println!("commit:     {commit}");
            }
            if let Some(allow) = &detail.allow {
                println!("allow:      {allow}");
            }
            println!("local_path: {}", detail.local_path);
        }
    }
}
