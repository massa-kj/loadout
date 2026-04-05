// crates/cli/src/cmd/editor.rs — $EDITOR launch utility

use std::path::Path;
use std::process;

/// Open `path` in the user's preferred editor.
///
/// Editor resolution order:
///   1. `$EDITOR`
///   2. `$VISUAL`
///   3. Platform default (`vi` → `nano` on Unix, `notepad` on Windows)
///
/// Exits the process with a non-zero code on failure.
pub fn open(path: &Path) {
    if !path.exists() {
        eprintln!("error: file not found: {}", path.display());
        process::exit(1);
    }

    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| default_editor());

    let status = std::process::Command::new(&editor)
        .arg(path)
        .status()
        .unwrap_or_else(|e| {
            eprintln!("error: failed to launch editor '{editor}': {e}");
            eprintln!("hint: set the EDITOR environment variable to your preferred editor");
            process::exit(1);
        });

    if !status.success() {
        let code = status.code().unwrap_or(1);
        eprintln!("error: editor '{editor}' exited with code {code}");
        process::exit(code);
    }
}

fn default_editor() -> String {
    if cfg!(windows) {
        return "notepad".to_string();
    }
    // Try vi, then nano, in order.
    for candidate in ["vi", "nano"] {
        if is_in_path(candidate) {
            return candidate.to_string();
        }
    }
    "vi".to_string()
}

fn is_in_path(cmd: &str) -> bool {
    let sep = if cfg!(windows) { ';' } else { ':' };
    std::env::var("PATH")
        .unwrap_or_default()
        .split(sep)
        .any(|dir| std::path::Path::new(dir).join(cmd).is_file())
}
