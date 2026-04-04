// crates/cli/src/cmd/source/list.rs — `loadout source list` implementation

use std::process;

use serde::Serialize;

use crate::args::{OutputArgs, OutputFormat};
use crate::context::build_app_context;

#[derive(Serialize)]
struct SourceEntry {
    id: String,
    kind: &'static str,
    url: Option<String>,
    commit: Option<String>,
    allow: Option<String>,
    local_path: Option<String>,
}

pub fn run(args: OutputArgs) {
    let ctx = build_app_context();

    let sources = app::load_sources(&ctx).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });

    // Build a combined list: implicit sources first, then declared external sources.
    let mut entries: Vec<SourceEntry> = vec![
        SourceEntry {
            id: "core".to_string(),
            kind: "implicit",
            url: None,
            commit: None,
            allow: Some("*".to_string()),
            local_path: None,
        },
        SourceEntry {
            id: "local".to_string(),
            kind: "implicit",
            url: None,
            commit: None,
            allow: Some("*".to_string()),
            local_path: Some(ctx.local_root.display().to_string()),
        },
    ];

    for entry in &sources.sources {
        let allow = match &entry.allow {
            None => None,
            Some(model::sources::AllowSpec::All(_)) => Some("*".to_string()),
            Some(model::sources::AllowSpec::Detailed(d)) => {
                let features = d.features.as_ref().map(|l| match l {
                    model::sources::AllowList::All(_) => "features:*".to_string(),
                    model::sources::AllowList::Names(v) => format!("features:[{}]", v.join(",")),
                });
                let backends = d.backends.as_ref().map(|l| match l {
                    model::sources::AllowList::All(_) => "backends:*".to_string(),
                    model::sources::AllowList::Names(v) => format!("backends:[{}]", v.join(",")),
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
        entries.push(SourceEntry {
            id: entry.id.clone(),
            kind: "git",
            url: Some(entry.url.clone()),
            commit: entry.commit.clone(),
            allow,
            local_path: Some(local_path),
        });
    }

    match args.output {
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&entries).unwrap_or_else(|e| {
                eprintln!("error: {e}");
                process::exit(1);
            });
            println!("{json}");
        }
        OutputFormat::Text => {
            for e in &entries {
                let url_part = e.url.as_deref().unwrap_or("-");
                println!("  {:<16}  {:<8}  {url_part}", e.id, e.kind);
            }
        }
    }
}
