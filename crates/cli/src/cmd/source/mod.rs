// crates/cli/src/cmd/source/mod.rs — `loadout source` subcommand dispatch

mod list;
mod show;

use crate::args::SourceCommand;

pub fn run(cmd: SourceCommand) {
    match cmd {
        SourceCommand::List(args) => list::run(args),
        SourceCommand::Show(args) => show::run(args),
    }
}
