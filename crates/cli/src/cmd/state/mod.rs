// crates/cli/src/cmd/state/mod.rs — `loadout state` subcommand dispatch

pub mod migrate;

use crate::args::StateCommand;

pub fn run(cmd: StateCommand) {
    match cmd {
        StateCommand::Migrate(args) => migrate::run(args),
    }
}
