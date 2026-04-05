// crates/cli/src/args.rs — CLI argument definitions (clap derive API)
//
// All subcommand argument structs live here. No application logic.
//
// Phase 1: apply, plan, activate, completions
// Phase 2: state (migrate), context (set/show/unset), doctor
// Phase 3: state show, config, feature, backend, source (read-only commands)
// Phase 4: config edit/init/feature/raw, feature/backend/source edit (mutation commands)
// Phase 5: feature/backend new, feature/backend validate (scaffold/validation)

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

    /// Read and list available features
    Feature {
        #[command(subcommand)]
        command: FeatureCommand,
    },

    /// Read and list available backends
    Backend {
        #[command(subcommand)]
        command: BackendCommand,
    },

    /// Read and list sources
    Source {
        #[command(subcommand)]
        command: SourceCommand,
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
    /// Show the current loadout state
    Show(StateShowArgs),

    /// Migrate state file to the latest schema version
    Migrate(MigrateArgs),
}

#[derive(Debug, clap::Args)]
pub struct StateShowArgs {
    #[command(flatten)]
    pub output: OutputArgs,
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

    /// Open the active config file in $EDITOR
    Edit,

    /// Create a new config file from the built-in template
    Init(ConfigInitArgs),

    /// Manage features declared in a config file
    Feature {
        #[command(subcommand)]
        command: ConfigFeatureCommand,
    },

    /// Low-level YAML access (escape hatch; prefer typed commands)
    Raw {
        #[command(subcommand)]
        command: ConfigRawCommand,
    },
}

#[derive(Debug, clap::Args)]
pub struct ConfigInitArgs {
    /// Config name to create (e.g. `linux` → `configs/linux.yaml`)
    pub name: String,
}

// ── config feature ───────────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
pub enum ConfigFeatureCommand {
    /// Add a feature to the config file
    Add(ConfigFeatureAddArgs),

    /// Remove a feature from the config file
    Remove(ConfigFeatureRemoveArgs),
}

#[derive(Debug, clap::Args)]
pub struct ConfigFeatureAddArgs {
    /// Feature ID (`source/name`) or bare name (resolves to `local/<name>`)
    pub id: String,

    /// Config name or path. Defaults to the active context.
    #[arg(short, long, value_name = "NAME|PATH")]
    pub config: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct ConfigFeatureRemoveArgs {
    /// Feature ID (`source/name`) or bare name (resolves to `local/<name>`)
    pub id: String,

    /// Config name or path. Defaults to the active context.
    #[arg(short, long, value_name = "NAME|PATH")]
    pub config: Option<String>,
}

// ── config raw ───────────────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
pub enum ConfigRawCommand {
    /// Show the raw YAML content of a config file
    Show(ConfigRawShowArgs),

    /// Set a value at a dot-separated YAML path
    Set(ConfigRawSetArgs),

    /// Remove a value at a dot-separated YAML path
    Unset(ConfigRawUnsetArgs),
}

#[derive(Debug, clap::Args)]
pub struct ConfigRawShowArgs {
    /// Config name or path. Defaults to the active context.
    pub name: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct ConfigRawSetArgs {
    /// Dot-separated YAML path (e.g. `profile.features.local.git`)
    pub path: String,

    /// YAML value to set (e.g. `{}`, `true`, `v1`)
    pub value: String,

    /// Config name or path. Defaults to the active context.
    #[arg(short, long, value_name = "NAME|PATH")]
    pub config: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct ConfigRawUnsetArgs {
    /// Dot-separated YAML path to remove
    pub path: String,

    /// Config name or path. Defaults to the active context.
    #[arg(short, long, value_name = "NAME|PATH")]
    pub config: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct ConfigShowArgs {
    /// Config name (e.g. `linux`) or path. Defaults to the current context.
    pub name: Option<String>,

    #[command(flatten)]
    pub output: OutputArgs,
}

// ── feature ──────────────────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
pub enum FeatureCommand {
    /// List all available features
    List(FeatureListArgs),

    /// Show details for a specific feature
    Show(FeatureShowArgs),

    /// Open a local feature's `feature.yaml` in $EDITOR
    Edit(FeatureEditArgs),

    /// Scaffold a new local feature directory from a template
    New(FeatureNewArgs),

    /// Validate a feature's `feature.yaml` and directory structure
    Validate(FeatureValidateArgs),
}

#[derive(Debug, clap::Args)]
pub struct FeatureEditArgs {
    /// Feature name or canonical ID. Bare name resolves to `local/<name>`.
    pub name: String,
}

#[derive(Debug, clap::Args)]
pub struct FeatureNewArgs {
    /// Feature name (e.g. `myfeature` → creates `features/myfeature/`)
    pub name: String,

    /// Template to use: `declarative` (default) or `script`
    #[arg(long, value_name = "TEMPLATE", default_value = "declarative")]
    pub template: FeatureTemplate,
}

/// Template choice for `feature new`.
#[derive(Debug, Clone, Default, clap::ValueEnum)]
pub enum FeatureTemplate {
    /// Declarative feature: `feature.yaml` with a `resources:` skeleton
    #[default]
    Declarative,
    /// Script feature: `feature.yaml` + stub `install.sh` / `uninstall.sh`
    Script,
}

#[derive(Debug, clap::Args)]
pub struct FeatureValidateArgs {
    /// Feature canonical ID (e.g. `local/git`) or bare name (resolves to `local/<name>`)
    pub id: String,
}

#[derive(Debug, clap::Args)]
pub struct FeatureListArgs {
    /// Filter by source ID (e.g. `local`, `core`)
    #[arg(long, value_name = "SOURCE")]
    pub source: Option<String>,

    #[command(flatten)]
    pub output: OutputArgs,
}

#[derive(Debug, clap::Args)]
pub struct FeatureShowArgs {
    /// Canonical feature ID (e.g. `core/git` or `local/nvim`)
    pub id: String,

    #[command(flatten)]
    pub output: OutputArgs,
}

// ── backend ──────────────────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
pub enum BackendCommand {
    /// List all available backends
    List(BackendListArgs),

    /// Show details for a specific backend
    Show(BackendShowArgs),

    /// Open a local backend's `backend.yaml` in $EDITOR
    Edit(BackendEditArgs),

    /// Scaffold a new local backend directory from a template
    New(BackendNewArgs),

    /// Validate a backend's `backend.yaml` and directory structure
    Validate(BackendValidateArgs),
}

#[derive(Debug, clap::Args)]
pub struct BackendEditArgs {
    /// Backend name or canonical ID. Bare name resolves to `local/<name>`.
    pub name: String,
}

#[derive(Debug, clap::Args)]
pub struct BackendNewArgs {
    /// Backend name (e.g. `mypkg` → creates `backends/mypkg/`)
    pub name: String,

    /// Target platform(s) for generated scripts
    #[arg(long, value_name = "PLATFORM", default_value = "unix")]
    pub platform: BackendPlatform,
}

/// Platform choice for `backend new`.
#[derive(Debug, Clone, Default, clap::ValueEnum)]
pub enum BackendPlatform {
    /// Generate `.sh` scripts only (Linux / macOS / WSL)
    #[default]
    Unix,
    /// Generate both `.sh` and `.ps1` scripts
    #[value(name = "unix-windows")]
    UnixWindows,
}

#[derive(Debug, clap::Args)]
pub struct BackendValidateArgs {
    /// Backend canonical ID (e.g. `local/mise`) or bare name (resolves to `local/<name>`)
    pub id: String,
}

#[derive(Debug, clap::Args)]
pub struct BackendListArgs {
    /// Filter by source ID (e.g. `local`)
    #[arg(long, value_name = "SOURCE")]
    pub source: Option<String>,

    #[command(flatten)]
    pub output: OutputArgs,
}

#[derive(Debug, clap::Args)]
pub struct BackendShowArgs {
    /// Canonical backend ID (e.g. `local/mise`)
    pub id: String,

    #[command(flatten)]
    pub output: OutputArgs,
}

// ── source ───────────────────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
pub enum SourceCommand {
    /// List all sources (implicit and declared)
    List(OutputArgs),

    /// Show details for a specific source
    Show(SourceShowArgs),

    /// Open `sources.yaml` in $EDITOR (creates a template if absent)
    Edit,
}

#[derive(Debug, clap::Args)]
pub struct SourceShowArgs {
    /// Source ID (`core`, `local`, or an external source ID)
    pub id: String,

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
