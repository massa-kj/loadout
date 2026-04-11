//! Platform detection and base directory resolution.
//!
//! Provides the runtime Platform / Arch detection and resolves the
//! XDG (Linux/WSL) or AppData (Windows) base directories for loadout.
//!
//! See: `docs/architecture/layers.md` (platform layer)

use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Host platform, detected at runtime.
///
/// WSL is Linux with `WSL_DISTRO_NAME` set; it is treated separately because
/// path conventions (Windows mounts, interop) differ from plain Linux.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    Windows,
    Wsl,
}

impl Platform {
    /// Lowercase string representation used in file name suffixes (e.g. `component.linux.yaml`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Platform::Linux => "linux",
            Platform::Windows => "windows",
            Platform::Wsl => "wsl",
        }
    }
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// CPU architecture, detected at runtime via compile-time constants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    Aarch64,
    /// Any architecture not explicitly enumerated.
    Other(String),
}

impl std::fmt::Display for Arch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Arch::X86_64 => write!(f, "x86_64"),
            Arch::Aarch64 => write!(f, "aarch64"),
            Arch::Other(s) => write!(f, "{s}"),
        }
    }
}

/// Resolved base directories for a loadout installation.
///
/// All paths are absolute and include the `loadout` namespace suffix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirs {
    /// Configuration home: profile, strategy, sources.yaml.
    /// Linux/WSL: `$XDG_CONFIG_HOME/loadout`  (default `~/.config/loadout`)
    /// Windows:   `%APPDATA%\loadout`
    pub config_home: PathBuf,

    /// Data home: external source caches, backend data.
    /// Linux/WSL: `$XDG_DATA_HOME/loadout`    (default `~/.local/share/loadout`)
    /// Windows:   `%APPDATA%\loadout`
    pub data_home: PathBuf,

    /// State home: authoritative state.json.
    /// Linux/WSL: `$XDG_STATE_HOME/loadout`   (default `~/.local/state/loadout`)
    /// Windows:   `%APPDATA%\loadout`
    pub state_home: PathBuf,

    /// Cache home: ephemeral execution artifacts (env plan cache, etc.).
    /// Linux/WSL: `$XDG_CACHE_HOME/loadout`   (default `~/.cache/loadout`)
    /// Windows:   `%APPDATA%\loadout`
    pub cache_home: PathBuf,
}

/// Errors from platform detection or directory resolution.
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    /// The user home directory could not be determined.
    /// On Linux/WSL: `HOME` env var is absent.
    #[error("home directory not found (HOME is not set)")]
    HomeDirNotFound,

    /// The Windows `APPDATA` environment variable is absent.
    #[error("APPDATA directory not found (APPDATA is not set)")]
    AppDataNotFound,
}

// ---------------------------------------------------------------------------
// Detection functions
// ---------------------------------------------------------------------------

/// Detect the current platform at runtime.
///
/// - Checks `WSL_DISTRO_NAME` first (WSL is Linux with this env var set).
/// - Falls back to compile-time `cfg!(target_os = ...)`.
pub fn detect_platform() -> Platform {
    detect_platform_from_env(|k| std::env::var(k).ok())
}

/// Detect the CPU architecture at compile time.
pub fn detect_arch() -> Arch {
    if cfg!(target_arch = "x86_64") {
        Arch::X86_64
    } else if cfg!(target_arch = "aarch64") {
        Arch::Aarch64
    } else {
        Arch::Other(std::env::consts::ARCH.to_string())
    }
}

/// Resolve the canonical loadout base directories for the given platform.
///
/// Reads `HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME` on
/// Linux/WSL, and `APPDATA` on Windows.
pub fn resolve_dirs(platform: &Platform) -> Result<Dirs, PlatformError> {
    resolve_dirs_from_env(platform, |k| std::env::var(k).ok())
}

// ---------------------------------------------------------------------------
// Testable inner implementations (env lookup injected)
// ---------------------------------------------------------------------------

fn detect_platform_from_env(get_env: impl Fn(&str) -> Option<String>) -> Platform {
    if get_env("WSL_DISTRO_NAME").is_some() {
        return Platform::Wsl;
    }
    if cfg!(target_os = "windows") {
        return Platform::Windows;
    }
    Platform::Linux
}

fn resolve_dirs_from_env(
    platform: &Platform,
    get_env: impl Fn(&str) -> Option<String>,
) -> Result<Dirs, PlatformError> {
    match platform {
        Platform::Linux | Platform::Wsl => resolve_xdg_dirs(get_env),
        Platform::Windows => resolve_appdata_dirs(get_env),
    }
}

/// Resolve XDG-based directories (Linux / WSL).
///
/// Falls back to `~/.config`, `~/.local/share`, `~/.local/state` when the
/// corresponding `XDG_*` variable is absent.
fn resolve_xdg_dirs(get_env: impl Fn(&str) -> Option<String>) -> Result<Dirs, PlatformError> {
    let home = get_env("HOME").ok_or(PlatformError::HomeDirNotFound)?;
    let home = PathBuf::from(home);

    let config_base = get_env("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));

    let data_base = get_env("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local").join("share"));

    let state_base = get_env("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local").join("state"));

    let cache_base = get_env("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".cache"));

    Ok(Dirs {
        config_home: config_base.join("loadout"),
        data_home: data_base.join("loadout"),
        state_home: state_base.join("loadout"),
        cache_home: cache_base.join("loadout"),
    })
}

/// Resolve AppData-based directories (Windows).
///
/// All four dirs map to `%APPDATA%\loadout` on Windows.
fn resolve_appdata_dirs(get_env: impl Fn(&str) -> Option<String>) -> Result<Dirs, PlatformError> {
    let appdata = get_env("APPDATA").ok_or(PlatformError::AppDataNotFound)?;
    let base = PathBuf::from(appdata).join("loadout");
    Ok(Dirs {
        config_home: base.clone(),
        data_home: base.clone(),
        state_home: base.clone(),
        cache_home: base,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_map<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        let map: HashMap<&str, &str> = pairs.iter().copied().collect();
        move |k| map.get(k).map(|v| v.to_string())
    }

    // --- Platform detection ------------------------------------------------

    #[test]
    fn detect_wsl_when_distro_name_set() {
        let p = detect_platform_from_env(env_map(&[("WSL_DISTRO_NAME", "Ubuntu")]));
        assert_eq!(p, Platform::Wsl);
    }

    #[test]
    fn detect_linux_when_no_wsl_var() {
        // On a Linux build host without WSL_DISTRO_NAME → Linux.
        // (On Windows build host this would return Windows, which is correct.)
        let p = detect_platform_from_env(env_map(&[]));
        // On the CI Linux runner this must be Linux.
        if cfg!(target_os = "windows") {
            assert_eq!(p, Platform::Windows);
        } else {
            assert_eq!(p, Platform::Linux);
        }
    }

    #[test]
    fn wsl_takes_priority_over_linux_cfg() {
        // Even on a non-WSL Linux binary, if WSL_DISTRO_NAME is set, return Wsl.
        let p = detect_platform_from_env(env_map(&[("WSL_DISTRO_NAME", "Debian")]));
        assert_eq!(p, Platform::Wsl);
    }

    // --- Platform helpers --------------------------------------------------

    #[test]
    fn platform_as_str() {
        assert_eq!(Platform::Linux.as_str(), "linux");
        assert_eq!(Platform::Windows.as_str(), "windows");
        assert_eq!(Platform::Wsl.as_str(), "wsl");
    }

    #[test]
    fn platform_display() {
        assert_eq!(Platform::Linux.to_string(), "linux");
        assert_eq!(Platform::Wsl.to_string(), "wsl");
    }

    // --- Arch detection ----------------------------------------------------

    #[test]
    fn detect_arch_returns_known_variant() {
        let arch = detect_arch();
        // On CI: x86_64 or aarch64 — either way not panics and formats cleanly.
        let s = arch.to_string();
        assert!(!s.is_empty());
    }

    #[test]
    fn arch_display() {
        assert_eq!(Arch::X86_64.to_string(), "x86_64");
        assert_eq!(Arch::Aarch64.to_string(), "aarch64");
        assert_eq!(Arch::Other("riscv64".to_string()).to_string(), "riscv64");
    }

    // --- XDG directory resolution -----------------------------------------

    #[test]
    fn xdg_uses_defaults_when_vars_absent() {
        let dirs =
            resolve_dirs_from_env(&Platform::Linux, env_map(&[("HOME", "/home/user")])).unwrap();

        assert_eq!(
            dirs.config_home,
            PathBuf::from("/home/user/.config/loadout")
        );
        assert_eq!(
            dirs.data_home,
            PathBuf::from("/home/user/.local/share/loadout")
        );
        assert_eq!(
            dirs.state_home,
            PathBuf::from("/home/user/.local/state/loadout")
        );
        assert_eq!(dirs.cache_home, PathBuf::from("/home/user/.cache/loadout"));
    }

    #[test]
    fn xdg_uses_override_vars_when_set() {
        let dirs = resolve_dirs_from_env(
            &Platform::Linux,
            env_map(&[
                ("HOME", "/home/user"),
                ("XDG_CONFIG_HOME", "/custom/cfg"),
                ("XDG_DATA_HOME", "/custom/data"),
                ("XDG_STATE_HOME", "/custom/state"),
                ("XDG_CACHE_HOME", "/custom/cache"),
            ]),
        )
        .unwrap();

        assert_eq!(dirs.config_home, PathBuf::from("/custom/cfg/loadout"));
        assert_eq!(dirs.data_home, PathBuf::from("/custom/data/loadout"));
        assert_eq!(dirs.state_home, PathBuf::from("/custom/state/loadout"));
        assert_eq!(dirs.cache_home, PathBuf::from("/custom/cache/loadout"));
    }

    #[test]
    fn xdg_partial_override_falls_back_for_missing() {
        // Only XDG_CONFIG_HOME set; others fall back to HOME defaults.
        let dirs = resolve_dirs_from_env(
            &Platform::Linux,
            env_map(&[("HOME", "/home/user"), ("XDG_CONFIG_HOME", "/custom/cfg")]),
        )
        .unwrap();

        assert_eq!(dirs.config_home, PathBuf::from("/custom/cfg/loadout"));
        assert_eq!(
            dirs.data_home,
            PathBuf::from("/home/user/.local/share/loadout")
        );
        assert_eq!(
            dirs.state_home,
            PathBuf::from("/home/user/.local/state/loadout")
        );
        assert_eq!(dirs.cache_home, PathBuf::from("/home/user/.cache/loadout"));
    }

    #[test]
    fn xdg_missing_home_returns_error() {
        let err = resolve_dirs_from_env(&Platform::Linux, env_map(&[])).unwrap_err();
        assert!(matches!(err, PlatformError::HomeDirNotFound));
    }

    #[test]
    fn wsl_uses_same_xdg_logic() {
        let dirs =
            resolve_dirs_from_env(&Platform::Wsl, env_map(&[("HOME", "/home/wsluser")])).unwrap();
        assert_eq!(
            dirs.config_home,
            PathBuf::from("/home/wsluser/.config/loadout")
        );
        assert_eq!(
            dirs.cache_home,
            PathBuf::from("/home/wsluser/.cache/loadout")
        );
    }

    // --- AppData directory resolution (Windows) ---------------------------

    #[test]
    fn appdata_sets_all_four_to_same_base() {
        let dirs = resolve_dirs_from_env(
            &Platform::Windows,
            env_map(&[("APPDATA", r"C:\Users\user\AppData\Roaming")]),
        )
        .unwrap();

        // Use PathBuf::join so the separator matches the host OS (Linux CI uses `/`).
        let expected = PathBuf::from(r"C:\Users\user\AppData\Roaming").join("loadout");
        assert_eq!(dirs.config_home, expected);
        assert_eq!(dirs.data_home, expected);
        assert_eq!(dirs.state_home, expected);
        assert_eq!(dirs.cache_home, expected);
    }

    #[test]
    fn appdata_missing_returns_error() {
        let err = resolve_dirs_from_env(&Platform::Windows, env_map(&[])).unwrap_err();
        assert!(matches!(err, PlatformError::AppDataNotFound));
    }

    // --- Dirs all paths include loadout suffix ----------------------------

    #[test]
    fn all_dirs_end_with_loadout() {
        let dirs =
            resolve_dirs_from_env(&Platform::Linux, env_map(&[("HOME", "/home/u")])).unwrap();

        for path in [
            &dirs.config_home,
            &dirs.data_home,
            &dirs.state_home,
            &dirs.cache_home,
        ] {
            assert_eq!(
                path.file_name().and_then(|n| n.to_str()),
                Some("loadout"),
                "{path:?} should end with 'loadout'"
            );
        }
    }
}
