//! Builtin backend registration — extension point for Rust-native backend adapters.
//!
//! # Current state
//!
//! All production backends are implemented as **script backends** (shell-script
//! directories under `backends/<name>/`). This crate previously held Rust-native
//! backends for `apt`, `brew`, `mise`, `npm`, `uv`, `scoop`, and `winget`, but
//! they have been removed in favour of the script-backend approach.
//!
//! # Extension points
//!
//! Two registration functions are retained as hooks for future Rust-native
//! implementations:
//!
//! - [`register_builtins`]  — register [`backend_host::Backend`] trait objects
//! - [`register_contributors`] — register [`executor::ExecutionEnvContributor`] objects
//!
//! Both are intentionally empty. Add implementations here only when shell
//! scripts are genuinely insufficient (e.g. Windows registry reads, OS API
//! probes, or unit-test mock injection).
//!
//! See: `docs/specs/api/backend.md`, `docs/guides/backends.md`

use backend_host::{BackendId, BackendRegistry};
use executor::ContributorRegistry;
use platform::Platform;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Register all builtin [`Backend`] implementations for `platform` into `registry`.
///
/// # Current state: intentionally empty
///
/// Builtin Rust backends (`core/brew`, `core/apt`, `core/mise`, …) are being
/// deprecated in favour of **script backends** — shell-script directories loaded
/// from `backends/<name>/` at runtime. All production backends are now script
/// backends; this function registers nothing by default.
///
/// # Extension point
///
/// The registration infrastructure is preserved so that a Rust-native backend
/// can be added here when shell scripts are genuinely insufficient (e.g. a
/// backend that requires deep OS integration). To add one, implement the
/// [`Backend`] trait and call `registry.register(id("core/name"), Box::new(…))`
/// inside the appropriate platform arm.
pub fn register_builtins(_registry: &mut BackendRegistry, _platform: &Platform) {
    // Intentionally empty: all backends are now script backends loaded from disk.
    // Rust-native backend registrations go here if needed in the future.
}

/// Register Rust-based [`executor::ExecutionEnvContributor`]s for the given platform.
///
/// # Current state: intentionally empty
///
/// Env contributions for builtin backends (`core/brew`, `core/mise`, …) are now
/// handled entirely by `env_pre.sh` / `env_post.sh` shell scripts in the backend
/// plugin directory (e.g. `backends/brew/env_pre.sh`). No Rust contributors are
/// registered by default.
///
/// # Extension point
///
/// `ContributorRegistry` is preserved as a **secondary Rust-only extension point**
/// for cases where shell scripts are not sufficient:
/// - OS-level API probes (e.g. Windows registry reads)
/// - Contributors generated from loadout's own metadata at runtime
/// - Dependency injection in unit tests (mock contributors)
///
/// To add a Rust contributor, implement [`executor::ExecutionEnvContributor`] and
/// call `registry.register_pre_action(key, Box::new(MyContributor))` here.
/// Prefer `env_pre.sh` for new work; use Rust contributors only when shell is
/// genuinely insufficient.
pub fn register_contributors(_registry: &mut ContributorRegistry, _platform: &Platform) {
    // Intentionally empty: all env contributions are handled by env_pre.sh /
    // env_post.sh scripts in the backend plugin directory.
    // See: backends/brew/env_pre.sh, backends/mise/env_pre.sh
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Construct a [`BackendId`] from a hardcoded string.
///
/// Panics if the string is not a valid canonical ID — callers must use
/// compile-time-known strings of the form `"<source>/<name>"`.
#[allow(dead_code)]
fn id(s: &str) -> BackendId {
    BackendId::new(s).expect("builtin backend IDs are always valid canonical IDs")
}
