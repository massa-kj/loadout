// crates/cli/src/cmd/doctor.rs — `loadout doctor` implementation
//
// doctor = loadout 実行前提・設定整合性・実行経路の診断
//
// Checks:
//   1. Platform & arch detection
//   2. Required directory existence (config_home, state_home, cache_home)
//   3. state.json — readable + version
//   4. sources.yaml — readable (optional)
//   5. env plan cache — presence
//   6. Shell env var ($SHELL on Unix)
//   7. LOADOUT_ROOT override validity
//   8. Config file readability (if -c is provided)
//
// doctor does NOT:
//   - infer ownership from the filesystem
//   - run planner or executor
//   - treat package manager output as source of truth

use crate::args::DoctorArgs;
use crate::context::{build_app_context, resolve_config_path};
use std::fmt;

// ── Result types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok,
    Warn,
    Error,
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Status::Ok => write!(f, "ok   "),
            Status::Warn => write!(f, "warn "),
            Status::Error => write!(f, "error"),
        }
    }
}

struct Check {
    label: String,
    status: Status,
    detail: String,
}

impl Check {
    fn ok(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            status: Status::Ok,
            detail: detail.into(),
        }
    }

    fn warn(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            status: Status::Warn,
            detail: detail.into(),
        }
    }

    fn error(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            status: Status::Error,
            detail: detail.into(),
        }
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub fn run(args: DoctorArgs) {
    let ctx = build_app_context();
    let mut checks: Vec<Check> = Vec::new();

    // 1. Platform & arch
    checks.push(Check::ok(
        "platform",
        format!("{} ({})", ctx.platform.as_str(), platform::detect_arch()),
    ));

    // 2. Directory existence
    for (label, path) in [
        ("config_home", &ctx.dirs.config_home),
        ("state_home", &ctx.dirs.state_home),
        ("data_home", &ctx.dirs.data_home),
        ("cache_home", &ctx.dirs.cache_home),
    ] {
        if path.exists() {
            checks.push(Check::ok(label, path.display().to_string()));
        } else {
            // Directories are created on first use — warn, not error.
            checks.push(Check::warn(
                label,
                format!("{} (not yet created)", path.display()),
            ));
        }
    }

    // 3. state.json
    {
        let state_path = ctx.state_path();
        if !state_path.exists() {
            checks.push(Check::warn(
                "state.json",
                format!(
                    "{} (not found — run 'loadout apply' first)",
                    state_path.display()
                ),
            ));
        } else {
            match state::load_raw(&state_path) {
                Ok(raw) => {
                    let version = raw.get("version").and_then(|v| v.as_u64()).unwrap_or(0);
                    checks.push(Check::ok(
                        "state.json",
                        format!("{} (version: {version})", state_path.display()),
                    ));
                }
                Err(e) => {
                    checks.push(Check::error(
                        "state.json",
                        format!("{}: {e}", state_path.display()),
                    ));
                }
            }
        }
    }

    // 4. sources.yaml (optional)
    {
        let sources_path = ctx.sources_path();
        if !sources_path.exists() {
            checks.push(Check::ok(
                "sources.yaml",
                format!("{} (not present — using defaults)", sources_path.display()),
            ));
        } else {
            match config::load_sources(&sources_path) {
                Ok(_) => checks.push(Check::ok(
                    "sources.yaml",
                    sources_path.display().to_string(),
                )),
                Err(e) => checks.push(Check::error(
                    "sources.yaml",
                    format!("{}: {e}", sources_path.display()),
                )),
            }
        }
    }

    // 5. env plan cache (activate prerequisite)
    {
        let cache_path = ctx.env_plan_cache_path();
        if cache_path.exists() {
            checks.push(Check::ok(
                "env_plan cache",
                cache_path.display().to_string(),
            ));
        } else {
            checks.push(Check::warn(
                "env_plan cache",
                format!(
                    "{} (not found — run 'loadout apply' to enable 'activate')",
                    cache_path.display()
                ),
            ));
        }
    }

    // 6. SHELL env var (Unix only — skip on Windows)
    #[cfg(not(target_os = "windows"))]
    {
        match std::env::var("SHELL") {
            Ok(shell) => checks.push(Check::ok("$SHELL", shell)),
            Err(_) => checks.push(Check::warn(
                "$SHELL",
                "not set — 'loadout activate' will default to bash",
            )),
        }
    }

    // 7. LOADOUT_ROOT override
    if let Ok(root) = std::env::var("LOADOUT_ROOT") {
        let p = std::path::Path::new(&root);
        if p.is_dir() {
            checks.push(Check::ok(
                "LOADOUT_ROOT",
                format!("{root} (override active)"),
            ));
        } else {
            checks.push(Check::warn(
                "LOADOUT_ROOT",
                format!("{root} (set but not a directory — ignored)"),
            ));
        }
    }

    // 8. Config file (if -c provided)
    if let Some(config_value) = args.config {
        let config_path = resolve_config_path(&config_value, &ctx.dirs);
        if config_path.exists() {
            match config::load_config(&config_path) {
                Ok(_) => checks.push(Check::ok("config file", config_path.display().to_string())),
                Err(e) => checks.push(Check::error(
                    "config file",
                    format!("{}: {e}", config_path.display()),
                )),
            }
        } else {
            checks.push(Check::error(
                "config file",
                format!("{} (not found)", config_path.display()),
            ));
        }
    }

    // ── Print results ─────────────────────────────────────────────────────────

    let label_width = checks.iter().map(|c| c.label.len()).max().unwrap_or(0);

    println!("loadout doctor");
    println!("{}", "─".repeat(50));
    for check in &checks {
        println!(
            "  [{status}]  {label:<width$}  {detail}",
            status = check.status,
            label = check.label,
            width = label_width,
            detail = check.detail,
        );
    }
    println!("{}", "─".repeat(50));

    let errors = checks.iter().filter(|c| c.status == Status::Error).count();
    let warns = checks.iter().filter(|c| c.status == Status::Warn).count();

    if errors > 0 {
        println!("{errors} error(s), {warns} warning(s)");
        std::process::exit(1);
    } else if warns > 0 {
        println!("0 errors, {warns} warning(s)");
    } else {
        println!("All checks passed.");
    }
}
