// crates/cli/src/cmd/feature/mod.rs — `loadout feature` subcommand dispatch

mod list;
mod show;

use crate::args::FeatureCommand;

pub fn run(cmd: FeatureCommand) {
    match cmd {
        FeatureCommand::List(args) => list::run(args),
        FeatureCommand::Show(args) => show::run(args),
    }
}
