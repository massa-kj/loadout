// crates/cli/src/cmd/context/mod.rs — `loadout context` subcommand dispatch
//
// The current context is stored as a bare config name (e.g. `linux`) in the
// file `{config_home}/current`. Bare names resolve via the same rules as
// `--config`: `{config_home}/configs/<name>.yaml`.

mod set;
mod show;
mod unset;

use crate::args::ContextCommand;

pub fn run(cmd: ContextCommand) {
    match cmd {
        ContextCommand::Show => show::run(),
        ContextCommand::Set(args) => set::run(args),
        ContextCommand::Unset => unset::run(),
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
