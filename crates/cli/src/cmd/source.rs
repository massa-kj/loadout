// crates/cli/src/cmd/source.rs — `loadout source` subcommand dispatch and implementations

use std::process;

use crate::args::{
    OutputArgs, OutputFormat, SourceAddCommand, SourceCommand, SourceRemoveArgs, SourceShowArgs,
    SourceTrustArgs, SourceUntrustArgs,
};
use crate::context::build_app_context;

pub fn run(cmd: SourceCommand) {
    match cmd {
        SourceCommand::List(args) => list(args),
        SourceCommand::Show(args) => show(args),
        SourceCommand::Edit => edit(),
        SourceCommand::Add { command } => add(command),
        SourceCommand::Remove(args) => remove(args),
        SourceCommand::Trust(args) => trust(args),
        SourceCommand::Untrust(args) => untrust(args),
    }
}

fn list(args: OutputArgs) {
    let ctx = build_app_context();
    let entries = app::list_sources(&ctx).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });

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

fn show(args: SourceShowArgs) {
    let ctx = build_app_context();
    let detail = app::show_source(&ctx, &args.id).unwrap_or_else(|e| {
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
        OutputFormat::Text => {
            println!("id:         {}", detail.id);
            println!("kind:       {}", detail.kind);
            if let Some(url) = &detail.url {
                println!("url:        {url}");
            }
            if let Some(ref_spec) = &detail.ref_spec {
                println!("ref:        {ref_spec}");
            }
            if let Some(resolved_commit) = &detail.resolved_commit {
                println!("commit:     {resolved_commit}");
            }
            if let Some(fetched_at) = &detail.fetched_at {
                println!("fetched_at: {fetched_at}");
            }
            if let Some(allow) = &detail.allow {
                println!("allow:      {allow}");
            }
            if let Some(local_path) = &detail.local_path {
                println!("local_path: {local_path}");
            }
        }
    }
}

// ── edit ─────────────────────────────────────────────────────────────────────

/// Template written when `sources.yaml` does not exist yet.
const SOURCES_TEMPLATE: &str = "\
# loadout sources
#
# Declare external plugin sources here.
# Each source provides features and/or backends.
#
# type: git — clone a git repository as a source
# type: path — use a local directory as a source
#
# Examples:
#
#   - id: community
#     type: git
#     url: https://github.com/example/loadout-community
#     ref:
#       branch: main
#     allow:
#       features: \"*\"
#       backends: \"*\"
#
#   - id: mylab
#     type: path
#     path: ../loadout-mylab
#     allow:
#       features: \"*\"

sources: []
";

fn edit() {
    let ctx = build_app_context();
    let sources_path = ctx.dirs.config_home.join("sources.yaml");

    // Create a template if the file does not exist yet.
    if !sources_path.exists() {
        if let Some(parent) = sources_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("error: failed to create directory: {e}");
                process::exit(1);
            }
        }
        if let Err(e) = std::fs::write(&sources_path, SOURCES_TEMPLATE) {
            eprintln!("error: failed to create sources.yaml: {e}");
            process::exit(1);
        }
    }

    super::editor::open(&sources_path);
}

// ── add ───────────────────────────────────────────────────────────────────────

fn add(cmd: SourceAddCommand) {
    match cmd {
        SourceAddCommand::Git(args) => add_git(args),
        SourceAddCommand::Path(args) => add_path(args),
    }
}

fn add_git(args: crate::args::SourceAddGitArgs) {
    let ctx = build_app_context();

    // Build SourceRef from the individual --branch / --tag / --commit flags.
    let source_ref = if args.branch.is_some() || args.tag.is_some() || args.commit.is_some() {
        Some(config::SourceRef {
            branch: args.branch,
            tag: args.tag,
            commit: args.commit,
        })
    } else {
        None
    };

    let path = app::source_add_git(
        &ctx,
        &args.url,
        args.id.as_deref(),
        source_ref,
        args.path.as_deref(),
    )
    .unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });

    println!("added source to {}", path.display());
}

fn add_path(args: crate::args::SourceAddPathArgs) {
    let ctx = build_app_context();

    let path = app::source_add_path(&ctx, &args.path, args.id.as_deref()).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });

    println!("added source to {}", path.display());
}

// ── remove ───────────────────────────────────────────────────────────────────

fn remove(args: SourceRemoveArgs) {
    let ctx = build_app_context();

    let path = app::source_remove(&ctx, &args.id, args.force).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });

    println!("removed '{}' from {}", args.id, path.display());
}

// ── trust ─────────────────────────────────────────────────────────────────────

fn trust(args: SourceTrustArgs) {
    let ctx = build_app_context();

    if args.features.is_none() && args.backends.is_none() {
        eprintln!("error: at least one of --features or --backends must be specified");
        process::exit(1);
    }

    let features = args.features.as_deref().map(parse_allow_list);
    let backends = args.backends.as_deref().map(parse_allow_list);

    let path = app::source_trust(&ctx, &args.id, features, backends).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });

    println!("updated allow-list in {}", path.display());
}

// ── untrust ───────────────────────────────────────────────────────────────────

fn untrust(args: SourceUntrustArgs) {
    let ctx = build_app_context();

    if args.features.is_none() && args.backends.is_none() {
        eprintln!("error: at least one of --features or --backends must be specified");
        process::exit(1);
    }

    let features = args.features.as_deref().map(parse_allow_list);
    let backends = args.backends.as_deref().map(parse_allow_list);

    let path =
        app::source_untrust(&ctx, &args.id, features, backends, args.force).unwrap_or_else(|e| {
            eprintln!("error: {e}");
            process::exit(1);
        });

    println!("updated allow-list in {}", path.display());
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Parse a `--features` / `--backends` string into an `AllowList`.
///
/// `"*"` → `AllowList::All`; anything else is treated as comma-separated names.
fn parse_allow_list(s: &str) -> config::AllowList {
    if s.trim() == "*" {
        config::AllowList::All(config::WildcardAll)
    } else {
        config::AllowList::Names(
            s.split(',')
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_string)
                .collect(),
        )
    }
}
