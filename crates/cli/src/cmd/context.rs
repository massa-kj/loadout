// crates/cli/src/cmd/context.rs — `loadout context` subcommand dispatch and implementations
//
// The current context is stored as a bare config name (e.g. `linux`) in the
// file `{config_home}/current`. Bare names resolve via the same rules as
// `--config`: `{config_home}/configs/<name>.yaml`.

use std::process;

use crate::args::{ContextCommand, ContextSetArgs};
use crate::context::build_app_context;

pub fn run(cmd: ContextCommand) {
    match cmd {
        ContextCommand::Show => show(),
        ContextCommand::Set(args) => set(args),
        ContextCommand::Unset => unset(),
    }
}

/// Path to the current-context file.
pub fn current_context_path(dirs: &platform::Dirs) -> std::path::PathBuf {
    dirs.config_home.join("current")
}

/// Read the current context name from disk. Returns `None` if not set.
pub fn read_current_context(dirs: &platform::Dirs) -> Option<String> {
    let path = current_context_path(dirs);
    std::fs::read_to_string(&path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn show() {
    let ctx = build_app_context();
    match read_current_context(&ctx.dirs) {
        Some(name) => println!("{name}"),
        None => println!("(no context set — use 'loadout context set <name>')"),
    }
}

fn set(args: ContextSetArgs) {
    let ctx = build_app_context();
    let path = current_context_path(&ctx.dirs);

    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("error: failed to create config directory: {e}");
            process::exit(1);
        }
    }

    if let Err(e) = std::fs::write(&path, format!("{}\n", args.name)) {
        eprintln!("error: failed to write context file: {e}");
        process::exit(1);
    }

    println!("Context set to '{}'.", args.name);
    println!("Run 'loadout apply' (without -c) to apply this config.");
}

fn unset() {
    let ctx = build_app_context();
    let path = current_context_path(&ctx.dirs);

    if !path.exists() {
        println!("No context is set.");
        return;
    }

    if let Err(e) = std::fs::remove_file(&path) {
        eprintln!("error: failed to remove context file: {e}");
        process::exit(1);
    }

    println!("Context cleared.");
}
