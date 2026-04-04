// crates/cli/src/cmd/state/mod.rs — `loadout state` subcommand dispatch

pub mod migrate;
pub mod show;

use crate::args::StateCommand;

pub fn run(cmd: StateCommand) {
    match cmd {
        StateCommand::Show(args) => show::run(args),
        StateCommand::Migrate(args) => migrate::run(args),
    }
}
