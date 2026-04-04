// crates/cli/src/args.rs — CLI argument definitions (clap derive API)
//
// All subcommand argument structs live here. No application logic.
//
// Phase 1: apply, plan, activate, completions
// Phase 2: state (migrate), context (set/show/unset), doctor
// Phase 3: state show, config, feature, backend, source (read-only commands)

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

    /// Manage loadout state
    State {
        #[command(subcommand)]
        command: StateCommand,
    },

    /// Manage the current context (active config)
    Context {
        #[command(subcommand)]
        command: ContextCommand,
    },

    /// Read and list config files
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },

    /// Diagnose the loadout environment
    Doctor(DoctorArgs),

    /// Generate shell completion scripts
    ///
    /// Example: loadout completions bash >> ~/.bashrc
    Completions(CompletionsArgs),
}

/// Output format for `list` and `show` commands.
#[derive(Debug, Clone, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    /// Human-readable text (default)
    #[default]
    Text,
    /// Machine-readable JSON
    Json,
}

/// Arguments for `--output` shared by all `list` and `show` subcommands.
#[derive(Debug, clap::Args)]
pub struct OutputArgs {
    /// Output format
    #[arg(long, value_name = "FORMAT", default_value = "text")]
    pub output: OutputFormat,
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

// ── state ────────────────────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
pub enum StateCommand {
    /// Migrate state file to the latest schema version
    Migrate(MigrateArgs),
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

// ── config ───────────────────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Show the resolved profile for a config file
    Show(ConfigShowArgs),

    /// List all config files in the config directory
    List(OutputArgs),
}

#[derive(Debug, clap::Args)]
pub struct ConfigShowArgs {
    /// Config name (e.g. `linux`) or path. Defaults to the current context.
    pub name: Option<String>,

    #[command(flatten)]
    pub output: OutputArgs,
}

// ── doctor ───────────────────────────────────────────────────────────────────

#[derive(Debug, clap::Args)]
pub struct DoctorArgs {
    /// Also check a specific config file for readability
    #[arg(short, long, value_name = "NAME|PATH")]
    pub config: Option<String>,
}
