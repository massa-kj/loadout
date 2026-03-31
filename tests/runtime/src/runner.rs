//! Subprocess helpers for invoking the `loadout` binary inside scenarios.

use std::path::Path;
use std::process::Command;

use crate::context::Context;

/// Invoke `loadout apply --config <config>` in `ctx.repo_root` and wait for
/// completion.  Returns an error string if the process fails to spawn or exits
/// with a non-zero status.
pub fn loadout_apply(ctx: &Context, config: &Path) -> Result<(), String> {
    let status = Command::new(&ctx.loadout_bin)
        .arg("apply")
        .arg("--config")
        .arg(config)
        .current_dir(&ctx.repo_root)
        .status()
        .map_err(|e| {
            format!(
                "failed to spawn '{}': {} — is LOADOUT_BIN set correctly?",
                ctx.loadout_bin, e
            )
        })?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "'{}' apply --config {} exited with {}",
            ctx.loadout_bin,
            config.display(),
            status
        ))
    }
}
