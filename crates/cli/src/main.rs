// crates/cli/src/main.rs — loadout CLI entry point
//
// Argument parsing and subcommand dispatch only. No application logic.
//
// Module layout:
//   args.rs          — clap argument structs (Cli, Command, *Args)
//   context.rs       — AppContext construction, config path helpers
//   output/          — display formatting (plan, report)
//   cmd/             — one module per subcommand
//
// See: docs/architecture/layers.md

mod args;
mod cmd;
mod context;
mod output;

use clap::Parser;

fn main() {
    let cli = args::Cli::parse();

    match cli.command {
        args::Command::Apply(args) => cmd::apply::run(args),
        args::Command::Plan(args) => cmd::plan::run(args),
        args::Command::Activate(args) => cmd::activate::run(args),
        args::Command::Migrate(args) => cmd::migrate::run(args),
        args::Command::Context { command } => cmd::context::run(command),
        args::Command::Completions(args) => cmd::completions::run(args),
    }
}
