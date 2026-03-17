// crates/cli/src/main.rs — loadout entry point
//
// This binary is a pure dispatcher: it locates the appropriate cmd script
// relative to its own location and spawns it, forwarding all arguments and
// propagating the exit code transparently.
//
// No business logic lives here. All logic is in cmd/*.sh (unix) or
// cmd/*.ps1 (windows).

use std::path::{Path, PathBuf};
use std::{env, process};

// ── Commands ──────────────────────────────────────────────────────────────────

const COMMANDS: &[&str] = &["apply", "plan", "migrate"];

const USAGE: &str = "\
Usage: loadout <command> [options]

A declarative environment management system.

Available commands:
  apply <profile>    Apply a loadout profile
  plan  <profile>    Show what apply would do (no changes made)
  migrate            Migrate state/profile keys to current schema

Options:
  plan --verbose     Also list noop (already up-to-date) features

Examples:
  loadout apply profiles/linux.yaml
  loadout plan  profiles/linux.yaml
  loadout plan  profiles/linux.yaml --verbose
  loadout migrate --dry-run\
";

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = env::args().collect();

    let subcommand = match args.get(1).map(String::as_str) {
        None | Some("help") | Some("--help") | Some("-h") => {
            println!("{USAGE}");
            process::exit(0);
        }
        Some(cmd) => cmd,
    };

    let rest = &args[2..];

    if !COMMANDS.contains(&subcommand) {
        eprintln!("error: unknown command: {subcommand}");
        eprintln!();
        eprintln!("{USAGE}");
        process::exit(1);
    }

    run_cmd(subcommand, rest);
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

fn run_cmd(subcommand: &str, args: &[String]) {
    let script = exe_dir().join("cmd").join(script_name(subcommand));

    if !script.exists() {
        eprintln!("error: cmd script not found: {}", script.display());
        process::exit(1);
    }

    let status = spawn_shell(&script, args).unwrap_or_else(|e| {
        eprintln!("error: failed to spawn shell: {e}");
        process::exit(1);
    });

    // Propagate exit code transparently; treat signal termination as 1.
    process::exit(status.code().unwrap_or(1));
}

/// Returns the directory containing this binary, following symlinks.
fn exe_dir() -> PathBuf {
    env::current_exe()
        .expect("failed to locate current executable")
        .canonicalize()
        .expect("failed to canonicalize executable path")
        .parent()
        .expect("executable has no parent directory")
        .to_path_buf()
}

// ── Platform-specific dispatch ────────────────────────────────────────────────

#[cfg(unix)]
fn script_name(subcommand: &str) -> String {
    format!("{subcommand}.sh")
}

#[cfg(windows)]
fn script_name(subcommand: &str) -> String {
    format!("{subcommand}.ps1")
}

#[cfg(unix)]
fn spawn_shell(script: &Path, args: &[String]) -> std::io::Result<process::ExitStatus> {
    process::Command::new("bash")
        .arg(script)
        .args(args)
        .status()
}

#[cfg(windows)]
fn spawn_shell(script: &Path, args: &[String]) -> std::io::Result<process::ExitStatus> {
    process::Command::new(find_powershell())
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(script)
        .args(args)
        .status()
}

/// Returns `"pwsh"` (PowerShell 7+) if available, otherwise `"powershell"` (5.1).
#[cfg(windows)]
fn find_powershell() -> &'static str {
    let available = process::Command::new("where")
        .arg("pwsh")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if available { "pwsh" } else { "powershell" }
}
