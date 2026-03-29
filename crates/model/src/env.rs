//! Execution environment types.
//!
//! Defines the structured representation of environment variable mutations,
//! evidence (provenance), and the accumulated execution context that the
//! executor maintains across actions in a single apply session.
//!
//! Key types:
//! - [`EnvMutation`]        – a single change to the environment
//! - [`ExecutionEnvDelta`]  – a set of mutations from one contributor
//! - [`ExecutionEnvContext`] – accumulated state the executor builds up
//! - [`ExecutionEnvPlan`]   – snapshot used by `loadout activate`
//!
//! See: `tmp/20260322_backend-activate問題2.md`

use std::collections::BTreeMap;
use std::path::PathBuf;

use platform::Platform;

// ---------------------------------------------------------------------------
// EnvMutation
// ---------------------------------------------------------------------------

/// A single mutation to the execution environment.
///
/// Mutations are applied in sequence; later `Set` mutations win for scalar
/// variables. Path-like variables are handled by the `*Path` variants which
/// preserve ordering and deduplicate entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvMutation {
    /// Set a scalar variable, overwriting any previous value.
    Set { key: String, value: String },

    /// Remove a variable from the environment.
    Unset { key: String },

    /// Prepend entries to a path-like variable (e.g. `PATH`).
    /// New entries are placed before existing ones; duplicates are removed.
    PrependPath {
        key: String,
        entries: Vec<PathEntry>,
    },

    /// Append entries to a path-like variable.
    /// Appended entries go after existing ones; duplicates are removed.
    AppendPath {
        key: String,
        entries: Vec<PathEntry>,
    },

    /// Remove specific entries from a path-like variable.
    RemovePath {
        key: String,
        entries: Vec<PathEntry>,
    },
}

// ---------------------------------------------------------------------------
// PathEntry
// ---------------------------------------------------------------------------

/// A single entry in a path-like environment variable.
///
/// Stored as a raw string; normalization (trailing-separator removal, platform
/// case comparison) is applied during merge operations and is not destructive.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PathEntry(pub String);

impl PathEntry {
    /// Construct a `PathEntry` from any string-like value.
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Return a normalized form used for deduplication comparisons.
    ///
    /// Strips trailing `/` and `\`. Does **not** apply case folding; callers
    /// that need case-insensitive comparison must lower-case the result.
    pub fn normalize(&self) -> String {
        self.0.trim_end_matches(['/', '\\']).to_string()
    }
}

impl std::fmt::Display for PathEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ---------------------------------------------------------------------------
// EnvEvidence
// ---------------------------------------------------------------------------

/// Provenance of an [`ExecutionEnvDelta`].
///
/// Kept alongside every delta so apply reports can explain where values came
/// from. Used for debugging and future `loadout activate --from-last-apply`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvEvidence {
    /// Derived from a known static default (e.g. Linuxbrew default prefix).
    StaticDefault,

    /// Obtained by running a command at probe time (e.g. `brew --prefix`).
    Probed { command: String },

    /// Read from a configuration file (e.g. `.tool-versions`).
    ConfigFile { path: PathBuf },
}

// ---------------------------------------------------------------------------
// ExecutionEnvDelta
// ---------------------------------------------------------------------------

/// A set of mutations produced by one [`ExecutionEnvContributor`] invocation.
///
/// The `evidence` field records the provenance so apply reports remain
/// auditable.
#[derive(Debug, Clone)]
pub struct ExecutionEnvDelta {
    pub mutations: Vec<EnvMutation>,
    pub evidence: EnvEvidence,
}

impl ExecutionEnvDelta {
    /// Construct an empty delta (no-op).
    pub fn empty() -> Self {
        Self {
            mutations: vec![],
            evidence: EnvEvidence::StaticDefault,
        }
    }
}

// ---------------------------------------------------------------------------
// ExecutionEnvContext
// ---------------------------------------------------------------------------

/// Accumulated execution environment maintained by the executor across actions.
///
/// Starts empty; grows as contributors return deltas after actions succeed.
/// The `vars` map is passed as process-level environment variables to backend
/// subprocesses, so they see the cumulative environment from earlier actions.
#[derive(Debug, Clone, Default)]
pub struct ExecutionEnvContext {
    /// Current snapshot: variable name → value as it would appear in a subprocess.
    pub vars: BTreeMap<String, String>,
}

impl ExecutionEnvContext {
    /// Create an empty context.
    ///
    /// **WARNING:** using an empty context with `PrependPath`/`AppendPath` mutations
    /// will discard the current process PATH. Use [`from_process_env`] instead when
    /// the executor is going to export the accumulated vars back to `std::env::set_var`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a context pre-seeded with the **current process environment**.
    ///
    /// Use this as the starting point for executor sessions so that
    /// `PrependPath`/`AppendPath` mutations correctly extend the existing system
    /// PATH rather than replacing it entirely. Without seeding, the first
    /// `env_pre.sh` script that prepends a PATH entry would overwrite the system
    /// PATH with only that one entry, causing `bash` to be unfindable for all
    /// subsequent subprocess spawns.
    pub fn from_process_env() -> Self {
        Self {
            vars: std::env::vars().collect(),
        }
    }

    /// Merge a delta into this context, applying its mutations in order.
    ///
    /// - `Set`         – overwrites any previous value.
    /// - `Unset`       – removes the variable (no-op if absent).
    /// - `PrependPath` – prepends entries, then deduplicates.
    /// - `AppendPath`  – appends entries, then deduplicates.
    /// - `RemovePath`  – removes matching entries.
    pub fn merge(&mut self, delta: &ExecutionEnvDelta, platform: Platform) {
        for mutation in &delta.mutations {
            self.apply_mutation(mutation, platform);
        }
    }

    fn apply_mutation(&mut self, mutation: &EnvMutation, platform: Platform) {
        match mutation {
            EnvMutation::Set { key, value } => {
                self.vars.insert(key.clone(), value.clone());
            }
            EnvMutation::Unset { key } => {
                self.vars.remove(key);
            }
            EnvMutation::PrependPath { key, entries } => {
                let existing = self.vars.get(key).cloned().unwrap_or_default();
                let sep = path_separator(platform);
                let existing_parts = split_path(&existing, sep);
                let merged = prepend_dedup(entries, &existing_parts, platform);
                if merged.is_empty() {
                    self.vars.remove(key);
                } else {
                    self.vars.insert(key.clone(), merged.join(sep));
                }
            }
            EnvMutation::AppendPath { key, entries } => {
                let existing = self.vars.get(key).cloned().unwrap_or_default();
                let sep = path_separator(platform);
                let existing_parts = split_path(&existing, sep);
                let merged = append_dedup(&existing_parts, entries, platform);
                if merged.is_empty() {
                    self.vars.remove(key);
                } else {
                    self.vars.insert(key.clone(), merged.join(sep));
                }
            }
            EnvMutation::RemovePath { key, entries } => {
                if let Some(existing) = self.vars.get(key).cloned() {
                    let sep = path_separator(platform);
                    let existing_parts = split_path(&existing, sep);
                    let kept: Vec<String> = existing_parts
                        .into_iter()
                        .filter(|e| !entries.iter().any(|r| path_eq(e, &r.normalize(), platform)))
                        .map(|s| s.to_string())
                        .collect();
                    if kept.is_empty() {
                        self.vars.remove(key);
                    } else {
                        self.vars.insert(key.clone(), kept.join(sep));
                    }
                }
            }
        }
    }

    /// Produce a snapshot suitable for `loadout activate`.
    pub fn to_plan(&self) -> ExecutionEnvPlan {
        ExecutionEnvPlan {
            vars: self.vars.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// ExecutionEnvPlan
// ---------------------------------------------------------------------------

/// Final snapshot of the execution environment, used for `loadout activate`.
///
/// Contains the fully resolved variable values after all contributor merges.
/// Provenance is tracked per-action in `ExecutorReport`; this type carries
/// only the final state needed to generate shell activation scripts.
#[derive(Debug, Clone, Default)]
pub struct ExecutionEnvPlan {
    /// Variable name → final value to export.
    pub vars: BTreeMap<String, String>,
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Return the platform-appropriate PATH separator character.
fn path_separator(platform: Platform) -> &'static str {
    match platform {
        Platform::Windows => ";",
        Platform::Linux | Platform::Wsl => ":",
    }
}

/// Split a PATH-like value into its component entries, filtering empty strings.
fn split_path<'a>(value: &'a str, sep: &str) -> Vec<&'a str> {
    if value.is_empty() {
        return vec![];
    }
    value.split(sep).filter(|s| !s.is_empty()).collect()
}

/// Compare two path entries for equality, applying platform case semantics.
///
/// Trailing separators are stripped before comparison.
fn path_eq(a: &str, b: &str, platform: Platform) -> bool {
    let a = a.trim_end_matches(['/', '\\']);
    let b = b.trim_end_matches(['/', '\\']);
    match platform {
        Platform::Windows => a.eq_ignore_ascii_case(b),
        Platform::Linux | Platform::Wsl => a == b,
    }
}

/// Build a path with `new_entries` prepended before `existing`, deduplicating.
///
/// New entries are placed first and take precedence. Any existing entry that
/// matches (after normalization) a new entry is omitted from the result.
fn prepend_dedup(new_entries: &[PathEntry], existing: &[&str], platform: Platform) -> Vec<String> {
    let mut result: Vec<String> = Vec::with_capacity(new_entries.len() + existing.len());

    for entry in new_entries {
        let norm = entry.normalize();
        if !result.iter().any(|r| path_eq(r, &norm, platform)) {
            result.push(entry.0.clone());
        }
    }
    for entry in existing {
        let norm = entry.trim_end_matches(['/', '\\']);
        if !result.iter().any(|r| path_eq(r, norm, platform)) {
            result.push((*entry).to_string());
        }
    }
    result
}

/// Build a path with `new_entries` appended after `existing`, deduplicating.
///
/// Existing entries take precedence; any new entry already present is skipped.
fn append_dedup(existing: &[&str], new_entries: &[PathEntry], platform: Platform) -> Vec<String> {
    let mut result: Vec<String> = Vec::with_capacity(existing.len() + new_entries.len());

    for entry in existing {
        let norm = entry.trim_end_matches(['/', '\\']);
        if !result.iter().any(|r| path_eq(r, norm, platform)) {
            result.push((*entry).to_string());
        }
    }
    for entry in new_entries {
        let norm = entry.normalize();
        if !result.iter().any(|r| path_eq(r, &norm, platform)) {
            result.push(entry.0.clone());
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn linux() -> Platform {
        Platform::Linux
    }
    fn windows() -> Platform {
        Platform::Windows
    }
    fn pe(s: &str) -> PathEntry {
        PathEntry::new(s)
    }

    fn set_delta(key: &str, value: &str) -> ExecutionEnvDelta {
        ExecutionEnvDelta {
            mutations: vec![EnvMutation::Set {
                key: key.into(),
                value: value.into(),
            }],
            evidence: EnvEvidence::StaticDefault,
        }
    }

    fn prepend_delta(key: &str, entries: Vec<PathEntry>) -> ExecutionEnvDelta {
        ExecutionEnvDelta {
            mutations: vec![EnvMutation::PrependPath {
                key: key.into(),
                entries,
            }],
            evidence: EnvEvidence::StaticDefault,
        }
    }

    fn append_delta(key: &str, entries: Vec<PathEntry>) -> ExecutionEnvDelta {
        ExecutionEnvDelta {
            mutations: vec![EnvMutation::AppendPath {
                key: key.into(),
                entries,
            }],
            evidence: EnvEvidence::StaticDefault,
        }
    }

    fn remove_delta(key: &str, entries: Vec<PathEntry>) -> ExecutionEnvDelta {
        ExecutionEnvDelta {
            mutations: vec![EnvMutation::RemovePath {
                key: key.into(),
                entries,
            }],
            evidence: EnvEvidence::StaticDefault,
        }
    }

    // --- PathEntry::normalize ---

    #[test]
    fn normalize_strips_trailing_forward_slash() {
        assert_eq!(pe("/usr/bin/").normalize(), "/usr/bin");
        assert_eq!(pe("/usr/bin").normalize(), "/usr/bin");
    }

    #[test]
    fn normalize_strips_trailing_backslash() {
        assert_eq!(pe("C:\\Windows\\").normalize(), "C:\\Windows");
    }

    // --- EnvMutation::Set ---

    #[test]
    fn set_inserts_new_variable() {
        let mut ctx = ExecutionEnvContext::new();
        ctx.merge(&set_delta("FOO", "bar"), linux());
        assert_eq!(ctx.vars["FOO"], "bar");
    }

    #[test]
    fn set_overwrites_previous_value() {
        let mut ctx = ExecutionEnvContext::new();
        ctx.merge(&set_delta("FOO", "first"), linux());
        ctx.merge(&set_delta("FOO", "second"), linux());
        assert_eq!(ctx.vars["FOO"], "second");
    }

    // --- EnvMutation::Unset ---

    #[test]
    fn unset_removes_existing_variable() {
        let mut ctx = ExecutionEnvContext::new();
        ctx.vars.insert("FOO".into(), "bar".into());
        ctx.merge(
            &ExecutionEnvDelta {
                mutations: vec![EnvMutation::Unset { key: "FOO".into() }],
                evidence: EnvEvidence::StaticDefault,
            },
            linux(),
        );
        assert!(!ctx.vars.contains_key("FOO"));
    }

    #[test]
    fn unset_is_noop_when_absent() {
        let mut ctx = ExecutionEnvContext::new();
        ctx.merge(
            &ExecutionEnvDelta {
                mutations: vec![EnvMutation::Unset { key: "FOO".into() }],
                evidence: EnvEvidence::StaticDefault,
            },
            linux(),
        );
        assert!(ctx.vars.is_empty());
    }

    // --- PrependPath ---

    #[test]
    fn prepend_to_empty_path() {
        let mut ctx = ExecutionEnvContext::new();
        ctx.merge(
            &prepend_delta("PATH", vec![pe("/usr/local/bin"), pe("/opt/bin")]),
            linux(),
        );
        assert_eq!(ctx.vars["PATH"], "/usr/local/bin:/opt/bin");
    }

    #[test]
    fn prepend_goes_before_existing() {
        let mut ctx = ExecutionEnvContext::new();
        ctx.vars.insert("PATH".into(), "/usr/bin:/bin".into());
        ctx.merge(&prepend_delta("PATH", vec![pe("/opt/brew/bin")]), linux());
        assert_eq!(ctx.vars["PATH"], "/opt/brew/bin:/usr/bin:/bin");
    }

    #[test]
    fn prepend_deduplicates_trailing_slash() {
        let mut ctx = ExecutionEnvContext::new();
        ctx.vars
            .insert("PATH".into(), "/opt/brew/bin:/usr/bin".into());
        // "/opt/brew/bin/" normalizes to "/opt/brew/bin" → duplicate
        ctx.merge(&prepend_delta("PATH", vec![pe("/opt/brew/bin/")]), linux());
        let parts: Vec<&str> = ctx.vars["PATH"].split(':').collect();
        let count = parts
            .iter()
            .filter(|&&p| p.trim_end_matches('/') == "/opt/brew/bin")
            .count();
        assert_eq!(count, 1, "duplicate entry must be collapsed");
    }

    // --- AppendPath ---

    #[test]
    fn append_goes_after_existing() {
        let mut ctx = ExecutionEnvContext::new();
        ctx.vars.insert("PATH".into(), "/usr/bin".into());
        ctx.merge(&append_delta("PATH", vec![pe("/opt/extra/bin")]), linux());
        assert_eq!(ctx.vars["PATH"], "/usr/bin:/opt/extra/bin");
    }

    #[test]
    fn append_deduplicates_existing() {
        let mut ctx = ExecutionEnvContext::new();
        ctx.vars.insert("PATH".into(), "/usr/bin:/opt/extra".into());
        ctx.merge(&append_delta("PATH", vec![pe("/opt/extra")]), linux());
        assert_eq!(ctx.vars["PATH"], "/usr/bin:/opt/extra");
    }

    // --- RemovePath ---

    #[test]
    fn remove_path_removes_matching_entry() {
        let mut ctx = ExecutionEnvContext::new();
        ctx.vars
            .insert("PATH".into(), "/usr/bin:/opt/brew/bin:/bin".into());
        ctx.merge(&remove_delta("PATH", vec![pe("/opt/brew/bin")]), linux());
        assert_eq!(ctx.vars["PATH"], "/usr/bin:/bin");
    }

    #[test]
    fn remove_path_normalizes_trailing_slash() {
        let mut ctx = ExecutionEnvContext::new();
        ctx.vars
            .insert("PATH".into(), "/opt/brew/bin:/usr/bin".into());
        ctx.merge(&remove_delta("PATH", vec![pe("/opt/brew/bin/")]), linux());
        assert_eq!(ctx.vars["PATH"], "/usr/bin");
    }

    #[test]
    fn remove_all_entries_drops_key() {
        let mut ctx = ExecutionEnvContext::new();
        ctx.vars.insert("PATH".into(), "/only/path".into());
        ctx.merge(&remove_delta("PATH", vec![pe("/only/path")]), linux());
        assert!(!ctx.vars.contains_key("PATH"));
    }

    // --- Windows-specific ---

    #[test]
    fn windows_uses_semicolon_separator() {
        let mut ctx = ExecutionEnvContext::new();
        ctx.merge(
            &prepend_delta("PATH", vec![pe("C:\\tools\\bin"), pe("C:\\more")]),
            windows(),
        );
        assert_eq!(ctx.vars["PATH"], "C:\\tools\\bin;C:\\more");
    }

    #[test]
    fn windows_path_comparison_is_case_insensitive() {
        let mut ctx = ExecutionEnvContext::new();
        ctx.vars.insert("PATH".into(), "C:\\Windows".into());
        // "C:\WINDOWS" and "C:\Windows" should be treated as the same entry.
        ctx.merge(&prepend_delta("PATH", vec![pe("C:\\WINDOWS")]), windows());
        let parts: Vec<&str> = ctx.vars["PATH"].split(';').collect();
        assert_eq!(
            parts.len(),
            1,
            "case-insensitive duplicate must be collapsed"
        );
    }

    // --- to_plan ---

    #[test]
    fn to_plan_mirrors_current_vars() {
        let mut ctx = ExecutionEnvContext::new();
        ctx.vars.insert("FOO".into(), "bar".into());
        let plan = ctx.to_plan();
        assert_eq!(plan.vars["FOO"], "bar");
    }

    // --- Multiple mutations in one delta ---

    #[test]
    fn multiple_mutations_applied_in_order() {
        let mut ctx = ExecutionEnvContext::new();
        let delta = ExecutionEnvDelta {
            mutations: vec![
                EnvMutation::Set {
                    key: "FOO".into(),
                    value: "a".into(),
                },
                EnvMutation::PrependPath {
                    key: "PATH".into(),
                    entries: vec![pe("/opt/bin")],
                },
                EnvMutation::Set {
                    key: "FOO".into(),
                    value: "b".into(),
                }, // overwrites
            ],
            evidence: EnvEvidence::Probed {
                command: "brew --prefix".into(),
            },
        };
        ctx.merge(&delta, linux());
        assert_eq!(ctx.vars["FOO"], "b");
        assert_eq!(ctx.vars["PATH"], "/opt/bin");
    }
}
