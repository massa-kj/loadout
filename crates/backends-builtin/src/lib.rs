//! Builtin backends — Rust implementations of the core package manager adapters.
//!
//! Each backend implements [`backend_host::Backend`] and shells out to the
//! appropriate package manager CLI. The glue code is in Rust rather than shell,
//! eliminating bash as a dependency for the core installation path.
//!
//! # Available backends
//!
//! | Backend ID    | Tool        | Supported kinds | Platforms         |
//! |---------------|-------------|-----------------|-------------------|
//! | `core/brew`   | Homebrew    | package         | Linux, macOS, WSL |
//! | `core/apt`    | APT         | package         | Linux, WSL        |
//! | `core/mise`   | mise        | runtime         | all               |
//! | `core/npm`    | npm         | package         | all               |
//! | `core/uv`     | uv          | package         | all               |
//! | `core/scoop`  | Scoop       | package         | Windows           |
//! | `core/winget` | winget      | package         | Windows           |
//!
//! # Registration
//!
//! Call [`register_builtins`] once during app startup to populate a
//! [`backend_host::BackendRegistry`] with all platform-appropriate backends.
//! Script backends from `backends/` on disk are registered afterwards and can
//! override or extend the builtins.
//!
//! See: `docs/specs/api/backend.md`

pub mod apt;
pub mod brew;
pub mod mise;
pub mod npm;
pub mod scoop;
pub mod uv;
pub mod winget;

mod cmd;

use backend_host::{BackendId, BackendRegistry};
use platform::Platform;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Register all builtin backends appropriate for `platform` into `registry`.
///
/// Builtin backends are registered under `core/<name>` IDs.  Script backends
/// loaded from disk afterwards can override individual entries.
///
/// # Platform mapping
///
/// | Platform      | Backends registered                                |
/// |---------------|----------------------------------------------------|
/// | Linux / WSL   | `core/brew`, `core/apt`, `core/mise`, `core/npm`, `core/uv` |
/// | Windows       | `core/scoop`, `core/winget`, `core/mise`, `core/npm`, `core/uv` |
pub fn register_builtins(registry: &mut BackendRegistry, platform: &Platform) {
    match platform {
        Platform::Linux | Platform::Wsl => {
            registry.register(id("core/brew"), Box::new(brew::BrewBackend));
            registry.register(id("core/apt"), Box::new(apt::AptBackend));
            registry.register(id("core/mise"), Box::new(mise::MiseBackend));
            registry.register(id("core/npm"), Box::new(npm::NpmBackend));
            registry.register(id("core/uv"), Box::new(uv::UvBackend));
        }
        Platform::Windows => {
            registry.register(id("core/scoop"), Box::new(scoop::ScoopBackend));
            registry.register(id("core/winget"), Box::new(winget::WingetBackend));
            registry.register(id("core/mise"), Box::new(mise::MiseBackend));
            registry.register(id("core/npm"), Box::new(npm::NpmBackend));
            registry.register(id("core/uv"), Box::new(uv::UvBackend));
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Construct a [`BackendId`] from a hardcoded string.
///
/// Panics if the string is not a valid canonical ID — callers must use
/// compile-time-known strings of the form `"<source>/<name>"`.
fn id(s: &str) -> BackendId {
    BackendId::new(s).expect("builtin backend IDs are always valid canonical IDs")
}
