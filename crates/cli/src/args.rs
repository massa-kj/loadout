// crates/cli/src/args.rs — CLI argument definitions (clap derive API)
//
// All subcommand argument structs live here. No application logic.
//
// Phase 1: apply, plan, activate, migrate, completions
// Phase 2: state (migrate), context (set/show/unset), doctor
// Phase 3+: config, feature, backend, source

use clap::{Parser, Subcommand};
use clap_complete::Shell;
use std::path::PathBuf;

/// A declarative environment management system.
#[derive(Debug, Parser)]
#[command(
    name = "loadout",
    version,
    about = "A declarative environment management system",
    long_about = None,
    disable_help_subcommand = true,
    arg_required_else_help = true,
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Apply a loadout config to the system
    Apply(ApplyArgs),

    /// Show what apply would do (no changes made)
    Plan(PlanArgs),

    /// Export env variables from last apply
    ///
    /// Evaluate the output in your shell:
    ///   bash/zsh:  eval "$(loadout activate)"
    ///   fish:      loadout activate --shell fish | source
    ///   pwsh:      Invoke-Expression (loadout activate --shell pwsh)
    Activate(ActivateArgs),

    /// Manage the current context (active config)
    Context {
        #[command(subcommand)]
        command: ContextCommand,
    },

    /// Diagnose the loadout environment
    Doctor(DoctorArgs),

    /// Migrate state file to the latest schema version
    ///
    /// Note: this command will move to `loadout state migrate` in a future release.
    Migrate(MigrateArgs),

    /// Generate shell completion scripts
    ///
    /// Example: loadout completions bash >> ~/.bashrc
    Completions(CompletionsArgs),
}

/// Arguments shared by commands that accept a config file.
///
/// Flattened into `ApplyArgs` and `PlanArgs`. Kept as a separate struct so
/// Phase 2 can add a context fallback in one place.
#[derive(Debug, clap::Args)]
pub struct ConfigArgs {
    /// Config name (e.g. `linux`) or path (e.g. `./work.yaml`)
    ///
    /// A bare name resolves to $XDG_CONFIG_HOME/loadout/configs/<name>.yaml.
    /// A value containing `.yaml` or `.yml` is treated as a literal path.
    #[arg(short, long, value_name = "NAME|PATH")]
    pub config: Option<String>,

    /// Sources spec override (CI / verification use only)
    #[arg(long, value_name = "PATH", hide = true)]
    pub sources: Option<PathBuf>,
}

#[derive(Debug, clap::Args)]
pub struct ApplyArgs {
    #[command(flatten)]
    pub config: ConfigArgs,

    /// Show per-feature detail
    #[arg(long)]
    pub verbose: bool,

    /// Skip confirmation prompt
    #[arg(short = 'y', long = "yes")]
    pub yes: bool,
}

#[derive(Debug, clap::Args)]
pub struct PlanArgs {
    #[command(flatten)]
    pub config: ConfigArgs,

    /// Show per-feature detail
    #[arg(long)]
    pub verbose: bool,
}

#[derive(Debug, clap::Args)]
pub struct ActivateArgs {
    /// Target shell (default: auto-detected from $SHELL)
    #[arg(long, value_name = "SHELL")]
    pub shell: Option<ShellChoice>,
}

/// Target shell for the `activate` command.
///
/// Intentionally separate from `clap_complete::Shell` because `activate`
/// only supports the shells that loadout's env scripts target.
#[derive(Debug, Clone, clap::ValueEnum)]
pub enum ShellChoice {
    Bash,
    Zsh,
    Fish,
    /// PowerShell (also accepted as `powershell`)
    #[value(name = "pwsh", alias = "powershell")]
    Pwsh,
}

#[derive(Debug, clap::Args)]
pub struct MigrateArgs {
    /// Show what would change without writing
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, clap::Args)]
pub struct CompletionsArgs {
    /// Target shell
    pub shell: Shell,
}
// ── context ──────────────────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
pub enum ContextCommand {
    /// Show the currently active context (config name)
    Show,

    /// Set the active context to the given config name
    Set(ContextSetArgs),

    /// Clear the active context
    Unset,
}

#[derive(Debug, clap::Args)]
pub struct ContextSetArgs {
    /// Config name to set as the active context (e.g. `linux`)
    pub name: String,
}

// ── doctor ───────────────────────────────────────────────────────────────────

#[derive(Debug, clap::Args)]
pub struct DoctorArgs {
    /// Also check a specific config file for readability
    #[arg(short, long, value_name = "NAME|PATH")]
    pub config: Option<String>,
}
