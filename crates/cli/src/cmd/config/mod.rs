// crates/cli/src/cmd/config/mod.rs — `loadout config` subcommand dispatch

mod list;
mod show;

use crate::args::ConfigCommand;

pub fn run(cmd: ConfigCommand) {
    match cmd {
        ConfigCommand::Show(args) => show::run(args),
        ConfigCommand::List(args) => list::run(args),
    }
}
