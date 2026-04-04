// crates/cli/src/cmd/context/set.rs — `loadout context set <name>` implementation

use super::current_context_path;
use crate::args::ContextSetArgs;
use crate::context::build_app_context;
use std::process;

pub fn run(args: ContextSetArgs) {
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
