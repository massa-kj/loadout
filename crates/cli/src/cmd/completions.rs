// crates/cli/src/cmd/completions.rs — `loadout completions` implementation

use crate::args::{Cli, CompletionsArgs};
use clap::CommandFactory;
use clap_complete::generate;
use std::io;

pub fn run(args: CompletionsArgs) {
    let mut cmd = Cli::command();
    generate(args.shell, &mut cmd, "loadout", &mut io::stdout());
}
