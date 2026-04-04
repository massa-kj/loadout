// crates/cli/src/cmd/context/unset.rs — `loadout context unset` implementation

use super::current_context_path;
use crate::context::build_app_context;
use std::process;

pub fn run() {
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
