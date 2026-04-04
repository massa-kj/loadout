// crates/cli/src/cmd/backend/mod.rs — `loadout backend` subcommand dispatch

mod list;
mod show;

use crate::args::BackendCommand;

pub fn run(cmd: BackendCommand) {
    match cmd {
        BackendCommand::List(args) => list::run(args),
        BackendCommand::Show(args) => show::run(args),
    }
}
