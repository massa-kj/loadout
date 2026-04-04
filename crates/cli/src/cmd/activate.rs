// crates/cli/src/cmd/activate.rs — `loadout activate` implementation

use crate::args::{ActivateArgs, ShellChoice};
use crate::context::build_app_context;
use std::process;

pub fn run(args: ActivateArgs) {
    let shell = match args.shell {
        Some(ShellChoice::Bash) => app::ShellKind::Bash,
        Some(ShellChoice::Zsh) => app::ShellKind::Zsh,
        Some(ShellChoice::Fish) => app::ShellKind::Fish,
        Some(ShellChoice::Pwsh) => app::ShellKind::PowerShell,
        None => detect_shell(),
    };

    let ctx = build_app_context();
    match app::activate(&ctx, shell) {
        Ok(script) => {
            // Print without trailing newline so shells can eval cleanly.
            print!("{script}");
        }
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    }
}

/// Detect the target shell from the `$SHELL` environment variable.
///
/// Falls back to Bash on Unix and PowerShell on Windows.
fn detect_shell() -> app::ShellKind {
    #[cfg(target_os = "windows")]
    {
        app::ShellKind::PowerShell
    }
    #[cfg(not(target_os = "windows"))]
    {
        match std::env::var("SHELL").as_deref() {
            Ok(s) if s.ends_with("zsh") => app::ShellKind::Zsh,
            Ok(s) if s.ends_with("fish") => app::ShellKind::Fish,
            _ => app::ShellKind::Bash,
        }
    }
}
