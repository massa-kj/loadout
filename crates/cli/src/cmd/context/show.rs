// crates/cli/src/cmd/context/show.rs — `loadout context show` implementation

use super::read_current_context;
use crate::context::build_app_context;

pub fn run() {
    let ctx = build_app_context();
    match read_current_context(&ctx.dirs) {
        Some(name) => println!("{name}"),
        None => println!("(no context set — use 'loadout context set <name>')"),
    }
}
