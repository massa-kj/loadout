// crates/cli/src/context.rs — AppContext construction and config path resolution
//
// These helpers are shared across multiple cmd/ modules and contain no
// output or argument parsing logic.

use std::path::{Path, PathBuf};
use std::process;

/// Build an `AppContext` from the current environment.
///
/// Platform and XDG/AppData dirs are detected automatically.
/// Set `LOADOUT_ROOT` to redirect the `local` source root during development
/// (must point to a directory containing `features/` and `backends/`).
pub fn build_app_context() -> app::AppContext {
    let platform = platform::detect_platform();
    let dirs = platform::resolve_dirs(&platform).unwrap_or_else(|e| {
        eprintln!("error: failed to resolve directories: {e}");
        process::exit(1);
    });
    let mut ctx = app::AppContext::new(platform, dirs);
    if let Ok(root) = std::env::var("LOADOUT_ROOT") {
        let p = PathBuf::from(&root);
        if p.is_dir() {
            ctx = ctx.with_local_root(p);
        } else {
            eprintln!("warning: LOADOUT_ROOT={root} is not a directory; ignored");
        }
    }
    ctx
}

/// Resolve a `--config` value to a `PathBuf`.
///
/// - Value contains `.yaml` or `.yml` → literal path (relative or absolute).
/// - Otherwise → `{config_home}/configs/{value}.yaml`.
pub fn resolve_config_path(value: &str, dirs: &platform::Dirs) -> PathBuf {
    if value.contains(".yaml") || value.contains(".yml") {
        PathBuf::from(value)
    } else {
        dirs.config_home
            .join("configs")
            .join(format!("{value}.yaml"))
    }
}

/// Derive a human-readable config id from a path.
///
/// Returns the file stem; e.g. `linux` from `.../configs/linux.yaml`.
pub fn config_id_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| path.display().to_string())
}
