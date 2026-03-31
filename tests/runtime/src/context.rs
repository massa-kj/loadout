//! E2E test context derived from environment variables.
//!
//! All paths follow the same conventions as the shell scenario scripts.
//! Override any value by setting the corresponding environment variable before
//! invoking `loadout-e2e`.

use std::path::PathBuf;

/// Runtime context shared by all scenarios.
///
/// # Environment variables
///
/// | Variable            | Description                              | Default                          |
/// |---------------------|------------------------------------------|----------------------------------|
/// | `LOADOUT_BIN`       | Path or name of the loadout binary       | `loadout`                        |
/// | `LOADOUT_REPO`      | Repository root inside the container     | `/tmp/loadout-repo`              |
/// | `XDG_CONFIG_HOME`   | XDG config root (set by scenario scripts)| `$HOME/.config`                  |
/// | `XDG_STATE_HOME`    | XDG state root (set by scenario scripts) | `$HOME/.local/state`             |
#[derive(Debug, Clone)]
pub struct Context {
    /// Loadout binary name or absolute path.
    pub loadout_bin: String,
    /// Repository root (mounted inside the container).
    pub repo_root: PathBuf,
    /// Config directory: `$XDG_CONFIG_HOME/loadout/configs/`.
    pub config_dir: PathBuf,
    /// State file path: `$XDG_STATE_HOME/loadout/state.json`.
    pub state_file: PathBuf,
}

impl Context {
    /// Build a context from the current process environment.
    pub fn from_env() -> Self {
        let loadout_bin = std::env::var("LOADOUT_BIN").unwrap_or_else(|_| "loadout".to_owned());

        let repo_root = std::env::var("LOADOUT_REPO")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp/loadout-repo"));

        // Resolve XDG_CONFIG_HOME → config_dir
        let xdg_config = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| dirs_home().join(".config"));
        let config_dir = xdg_config.join("loadout").join("configs");

        // Resolve XDG_STATE_HOME → state_file
        let xdg_state = std::env::var("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| dirs_home().join(".local").join("state"));
        let state_file = xdg_state.join("loadout").join("state.json");

        Self {
            loadout_bin,
            repo_root,
            config_dir,
            state_file,
        }
    }

    /// Return the absolute path to a named config file in `config_dir`.
    pub fn config(&self, filename: &str) -> PathBuf {
        self.config_dir.join(filename)
    }
}

/// Portable home directory fallback.
fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/root"))
}
