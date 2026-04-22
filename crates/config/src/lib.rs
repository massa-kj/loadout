//! Configuration loading, validation, and normalization.
//!
//! This crate bridges raw YAML files on disk and domain model types in the `model` crate.
//! It handles three kinds of config files: profiles, strategies, and sources.
//!
//! ## Input format
//!
//! Profiles use *namespace grouping* syntax: the outer key is a `source_id`,
//! the inner key is the component name. Both bare names and canonical `source/name`
//! forms are **rejected**; grouping is the only accepted syntax.
//!
//! ```yaml
//! profile:
//!   components:
//!     core:
//!       git: {}
//!     local:
//!       nvim: {}
//!       python:
//!         version: "3.12"
//! ```
//!
//! ## Import expansion
//!
//! Config files may declare `imports:` to merge other config files before processing.
//! Imports are resolved recursively in order; the importing file always wins.
//!
//! ```yaml
//! imports:
//!   - bundles/base.yaml          # kind: relative (default) — relative to this file
//!   - path: dotfiles/base.yaml
//!     kind: home                 # relative to the user's home directory
//! ```
//!
//! Absolute paths are forbidden. Cycle detection (via a DFS stack of canonical paths)
//! and a depth limit of [`IMPORT_DEPTH_LIMIT`] prevent runaway recursion.
//!
//! ## Bundle expansion
//!
//! Bundles allow reusable component sets:
//!
//! ```yaml
//! bundle:
//!   use:
//!     - base
//!     - work          # last entry wins on conflict
//!
//! bundles:
//!   base:
//!     components:
//!       core:
//!         git: {}
//!   work:
//!     components:
//!       dev:
//!         terraform: {}
//!
//! profile:
//!   components:
//!     local:
//!       nvim: {}      # profile.components overrides all bundles
//! ```
//!
//! ## Pipeline
//!
//! ```text
//! YAML deserialize
//!   → import expansion  (cycle-free, depth-limited recursive merge)
//!   → bundle expansion  (bundle.use → merge bundles:)
//!   → grouped-component normalization  (source_id/name canonical IDs)
//!   → canonicalization
//! ```
//!
//! After expansion and normalization, all component keys are canonical `source_id/name`.
//! Source existence is NOT verified here; that happens at `SourceRegistry` construction.
//!
//! **Path resolution contract**: callers supply explicit `&Path` values.
//! Platform-aware path discovery belongs to the `platform` crate.
//!
//! See: `docs/specs/data/profile.md`, `docs/specs/data/strategy.md`,
//!      `docs/specs/data/sources.md`

pub mod write;
pub use write::{
    add_component, create_config, raw_set, raw_show, raw_unset, remove_component,
    rewrite_backend_source, rewrite_component_source,
};

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

pub use model::{
    profile::{Profile, ProfileComponentConfig},
    sources::{
        AllowList, AllowSpec, DetailedAllow, SourceEntry, SourceLockEntry, SourceRef, SourceType,
        SourcesLock, SourcesSpec, WildcardAll,
    },
    strategy::{
        FingerprintPolicy, FsStrategy, MatchKind, MatchSelector, Specificity, Strategy,
        StrategyGroup, StrategyRule,
    },
};
use thiserror::Error;

/// Errors produced by configuration loading or validation.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("I/O error: {0}")]
    Io(#[from] io::IoError),

    #[error("invalid profile: {reason}")]
    InvalidProfile { reason: String },

    #[error("invalid strategy: {reason}")]
    InvalidStrategy { reason: String },

    #[error("invalid sources: {reason}")]
    InvalidSources { reason: String },

    #[error("config file already exists: {}", path.display())]
    AlreadyExists { path: std::path::PathBuf },

    #[error("import cycle detected: {chain}")]
    ImportCycle { chain: String },

    #[error("import depth limit ({limit}) exceeded at: {path}")]
    ImportDepthExceeded { limit: usize, path: String },

    #[error("imported config not found: {path} (referenced from {from})")]
    ImportNotFound { path: String, from: String },

    #[error("absolute import paths are not allowed: {path}")]
    ImportAbsolutePath { path: String },
}

// ─── Raw profile types (config-crate-local) ─────────────────────────────────

/// Raw per-component config as parsed from YAML (grouping syntax inner value).
/// Mirrors `ProfileComponentConfig` but is local to this crate.
///
/// `deny_unknown_fields` rejects legacy `version` field and other typos early.
#[derive(Deserialize, Default, Clone)]
#[serde(deny_unknown_fields)]
struct RawComponentConfig {
    #[serde(default)]
    params: Option<HashMap<String, serde_yaml::Value>>,
}

/// Grouped component map: `source_id → (component_name → config)`.
/// This is the only accepted input shape; bare names and canonical direct form are rejected.
type GroupedComponents = HashMap<String, HashMap<String, RawComponentConfig>>;

/// Raw profile as read from a standalone profile YAML file.
/// `components` uses the grouped syntax.
#[derive(Deserialize)]
struct RawProfile {
    #[serde(default)]
    components: GroupedComponents,
}

// ─── Raw bundle types (config-crate-local) ──────────────────────────────────

/// `bundle:` section in config.yaml — lists which bundles to apply.
/// Values are bundle names (strings). Future `file:` prefix scheme is intentionally
/// excluded from this type to keep the distinction clear.
#[derive(Deserialize, Default)]
struct RawBundleRef {
    #[serde(default, rename = "use")]
    use_list: Vec<String>,
}

/// A single bundle definition. Uses the same grouped-components syntax as profiles.
#[derive(Deserialize)]
struct RawBundle {
    #[serde(default)]
    components: GroupedComponents,
}

/// `bundles:` section in config.yaml — named bundle definitions.
type RawBundlesMap = HashMap<String, RawBundle>;

// ─── Import types ─────────────────────────────────────────────────────────────

/// Maximum import recursion depth.
///
/// Prevents runaway recursion even when cycle detection has edge-case gaps
/// (e.g., unusual symlink layouts). Depth 0 = root config, depth 1 = direct imports.
const IMPORT_DEPTH_LIMIT: usize = 8;

/// The base directory kind for resolving an import path.
///
/// `Relative` (default): relative to the directory containing the importing file.
/// `Home`: relative to the user's home directory.
///
/// Implicit expansion (`~`, environment variables) is never performed regardless of kind.
#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "snake_case")]
enum ImportKind {
    #[default]
    Relative,
    Home,
}

/// A single `imports:` entry in a config file.
///
/// Supports two forms:
///
/// ```yaml
/// # String shorthand — kind defaults to `relative`.
/// imports:
///   - bundles/base.yaml
///
/// # Explicit object form — kind is optional, defaults to `relative`.
/// imports:
///   - path: bundles/base.yaml
///   - path: dotfiles/loadout/shared.yaml
///     kind: home
/// ```
///
/// Absolute paths are rejected at expansion time.
#[derive(Deserialize, Clone)]
#[serde(untagged)]
enum RawImportEntry {
    /// `- path/to/file.yaml` shorthand; equivalent to `kind: relative`.
    Shorthand(String),
    /// `- path: path/to/file.yaml` (with optional `kind:` field).
    Explicit {
        path: String,
        #[serde(default)]
        kind: ImportKind,
    },
}

impl RawImportEntry {
    fn path_str(&self) -> &str {
        match self {
            RawImportEntry::Shorthand(s) => s,
            RawImportEntry::Explicit { path, .. } => path,
        }
    }

    fn kind(&self) -> &ImportKind {
        match self {
            RawImportEntry::Shorthand(_) => &ImportKind::Relative,
            RawImportEntry::Explicit { kind, .. } => kind,
        }
    }
}

// ─── Unified config top-level struct ─────────────────────────────────────────

/// Top-level structure for a config.yaml file.
///
/// `deny_unknown_fields` is intentionally absent so future sections do not break
/// existing versions (serde default: unknown keys are silently ignored).
#[derive(Deserialize)]
struct RawConfig {
    /// List of config files to import before applying this file's values.
    #[serde(default)]
    imports: Vec<RawImportEntry>,
    profile: Option<RawProfile>,
    strategy: Option<Strategy>,
    #[serde(default)]
    bundle: RawBundleRef,
    #[serde(default)]
    bundles: RawBundlesMap,
}

/// Intermediate accumulator built during import expansion.
///
/// Holds merged values from all imported files before assembling the final `RawConfig`.
/// `profile_components` uses `Option` to preserve the "is profile section present" signal:
/// `None` means no file provided a `profile:` section; `Some({})` means a profile
/// section was present but declared zero components.
#[derive(Default)]
struct MergedConfig {
    profile_components: Option<GroupedComponents>,
    strategy: Option<Strategy>,
    bundle_use: Vec<String>,
    bundles: RawBundlesMap,
}

impl MergedConfig {
    /// Assemble the accumulator into a `RawConfig` for further processing.
    fn into_raw_config(self) -> RawConfig {
        RawConfig {
            imports: Vec::new(), // already expanded; not used downstream
            profile: self
                .profile_components
                .map(|c| RawProfile { components: c }),
            strategy: self.strategy,
            bundle: RawBundleRef {
                use_list: self.bundle_use,
            },
            bundles: self.bundles,
        }
    }
}

/// Merge a `RawConfig` into the accumulator following import merge rules.
///
/// Rules applied:
/// - `profile.components`: source_id-level shallow merge; component-level replace (no field merge).
/// - `strategy.groups`: group-name-level replace (later file wins per group name).
/// - `strategy.rules`: replace entirely if non-empty (array; later file wins entirely).
/// - `strategy.fs`: field-level replace — replaced independently if the overlay provides a
///   non-`None` value.
/// - `bundle.use`: replace (array; no list concatenation — later value wins entirely).
/// - `bundles`: bundle-name-level replace (the entire bundle definition is replaced).
/// - `imports`: not merged (each file's imports are expanded independently before reaching here).
fn merge_into_acc(acc: &mut MergedConfig, raw: RawConfig) {
    // profile.components: source_id shallow merge, component-level replace.
    if let Some(p) = raw.profile {
        let acc_components = acc.profile_components.get_or_insert_with(HashMap::new);
        for (source_id, names) in p.components {
            let source_entry = acc_components.entry(source_id).or_default();
            for (name, cfg) in names {
                source_entry.insert(name, cfg);
            }
        }
    }

    // strategy: field-level / collection merge.
    if let Some(s) = raw.strategy {
        let base_s = acc.strategy.get_or_insert_with(Strategy::default);
        if s.strategy.is_some() {
            base_s.strategy = s.strategy;
        }
        // groups: group-name-level replace (later file wins per group name).
        for (name, group) in s.groups {
            base_s.groups.insert(name, group);
        }
        // rules: replace entirely if non-empty.
        if !s.rules.is_empty() {
            base_s.rules = s.rules;
        }
        if s.fs.is_some() {
            base_s.fs = s.fs;
        }
    }

    // bundle.use: replace (array; no concatenation).
    if !raw.bundle.use_list.is_empty() {
        acc.bundle_use = raw.bundle.use_list;
    }

    // bundles: bundle-name-level replace.
    for (name, bundle) in raw.bundles {
        acc.bundles.insert(name, bundle);
    }
}

// ─── Expansion helpers ───────────────────────────────────────────────────────

/// Expand grouped components into a flat canonical map.
///
/// `{source_id: {name: config}}` → `{"source_id/name": ProfileComponentConfig}`
///
/// Validates:
/// - `source_id` must not be empty
/// - component name must not be empty
/// - duplicate canonical IDs are rejected
///
/// Does NOT verify that `source_id` exists in the source registry;
/// that check happens later at `SourceRegistry` construction.
fn expand_grouped_components(
    grouped: GroupedComponents,
) -> Result<HashMap<String, ProfileComponentConfig>, ConfigError> {
    let mut out: HashMap<String, ProfileComponentConfig> = HashMap::new();

    for (source_id, names) in grouped {
        if source_id.is_empty() {
            return Err(ConfigError::InvalidProfile {
                reason: "source_id must not be empty".into(),
            });
        }
        for (name, cfg) in names {
            if name.is_empty() {
                return Err(ConfigError::InvalidProfile {
                    reason: format!("component name under source '{source_id}' must not be empty"),
                });
            }
            let canonical = format!("{source_id}/{name}");
            if out.contains_key(&canonical) {
                return Err(ConfigError::InvalidProfile {
                    reason: format!("duplicate component '{canonical}'"),
                });
            }
            out.insert(
                canonical,
                ProfileComponentConfig {
                    params: convert_raw_params(cfg.params)?,
                },
            );
        }
    }

    Ok(out)
}

/// Convert raw YAML param values into typed `ParamValue` map.
///
/// String values → `ParamValue::String`.
/// Mapping values with `kind` + `path` → `ParamValue::Source`.
fn convert_raw_params(
    raw: Option<HashMap<String, serde_yaml::Value>>,
) -> Result<Option<HashMap<String, model::params::ParamValue>>, ConfigError> {
    let raw = match raw {
        Some(r) if !r.is_empty() => r,
        _ => return Ok(None),
    };

    let mut out = HashMap::new();
    for (key, value) in raw {
        let pv = convert_one_param_value(&key, value)?;
        out.insert(key, pv);
    }
    Ok(Some(out))
}

/// Convert a single raw YAML value into a typed `ParamValue`.
fn convert_one_param_value(
    key: &str,
    value: serde_yaml::Value,
) -> Result<model::params::ParamValue, ConfigError> {
    use model::params::{ParamValue, SourceParamValue};

    match value {
        serde_yaml::Value::String(s) => Ok(ParamValue::String(s)),
        serde_yaml::Value::Number(n) => {
            // Numeric values are coerced to string (e.g., version: 20 → "20").
            Ok(ParamValue::String(n.to_string()))
        }
        serde_yaml::Value::Mapping(map) => {
            // Expect structured source: { kind: ..., path: ... }
            let kind_val = map
                .get(serde_yaml::Value::String("kind".into()))
                .ok_or_else(|| ConfigError::InvalidProfile {
                    reason: format!("param '{key}': object value must have 'kind' field"),
                })?;
            let path_val = map
                .get(serde_yaml::Value::String("path".into()))
                .ok_or_else(|| ConfigError::InvalidProfile {
                    reason: format!("param '{key}': object value must have 'path' field"),
                })?;

            let kind_str = kind_val
                .as_str()
                .ok_or_else(|| ConfigError::InvalidProfile {
                    reason: format!("param '{key}': 'kind' must be a string"),
                })?;

            let kind: model::fs::FsSourceKind = serde_yaml::from_value(serde_yaml::Value::String(
                kind_str.to_string(),
            ))
            .map_err(|_| ConfigError::InvalidProfile {
                reason: format!(
                    "param '{key}': invalid source kind '{kind_str}'; \
                             expected one of: home_relative, component_relative, absolute"
                ),
            })?;

            let path = path_val
                .as_str()
                .ok_or_else(|| ConfigError::InvalidProfile {
                    reason: format!("param '{key}': 'path' must be a string"),
                })?
                .to_string();

            Ok(ParamValue::Source(SourceParamValue { kind, path }))
        }
        _ => Err(ConfigError::InvalidProfile {
            reason: format!("param '{key}': unsupported value type; expected string or object"),
        }),
    }
}

/// Merge bundles in `use` list order (last entry wins), then overlay profile components.
///
/// Returns merged grouped components ready for `expand_grouped_components`.
/// Priority (lowest → highest): bundles[0], bundles[1], …, profile.components.
fn expand_bundles(
    bundle_ref: &RawBundleRef,
    bundles: &RawBundlesMap,
    profile_components: GroupedComponents,
) -> Result<GroupedComponents, ConfigError> {
    // Validate: all referenced bundle names must be defined.
    for name in &bundle_ref.use_list {
        if !bundles.contains_key(name) {
            return Err(ConfigError::InvalidProfile {
                reason: format!("undefined bundle '{name}': not found in 'bundles:' section"),
            });
        }
    }

    // Merge: iterate use list in order; last bundle wins per component.
    // Params are merged at key level (shallow merge): later values overwrite earlier ones.
    let mut merged: GroupedComponents = HashMap::new();
    for name in &bundle_ref.use_list {
        let bundle = &bundles[name];
        for (source_id, names) in &bundle.components {
            let source_entry = merged.entry(source_id.clone()).or_default();
            for (feat_name, cfg) in names {
                merge_raw_component_config(source_entry, feat_name.clone(), cfg.clone());
            }
        }
    }

    // Overlay: profile.components overwrites bundle params at key level.
    for (source_id, names) in profile_components {
        let source_entry = merged.entry(source_id).or_default();
        for (feat_name, cfg) in names {
            merge_raw_component_config(source_entry, feat_name, cfg);
        }
    }

    Ok(merged)
}

/// Merge a `RawComponentConfig` into an existing component entry.
///
/// Params are merged at key level (shallow merge): if both the existing entry and the
/// new entry have params, the new entry's keys overwrite the existing ones while
/// preserving keys only present in the existing entry.
fn merge_raw_component_config(
    target: &mut HashMap<String, RawComponentConfig>,
    feat_name: String,
    incoming: RawComponentConfig,
) {
    match target.get_mut(&feat_name) {
        Some(existing) => {
            match (&mut existing.params, incoming.params) {
                // Both have params: merge keys (incoming wins per key).
                (Some(base), Some(overlay)) => {
                    for (k, v) in overlay {
                        base.insert(k, v);
                    }
                }
                // Only incoming has params: replace.
                (None, Some(overlay)) => {
                    existing.params = Some(overlay);
                }
                // Only existing has params, or neither: keep existing.
                (_, None) => {}
            }
        }
        None => {
            target.insert(feat_name, incoming);
        }
    }
}

// ─── Import expansion ────────────────────────────────────────────────────────

/// Expand `imports:` directives in `config_path` recursively, returning a single
/// merged `RawConfig` ready for bundle expansion.
///
/// The `stack` parameter holds the canonical paths of all config files in the
/// current DFS call chain. It is used for cycle detection and is not the same as
/// a simple "visited" set — diamond-shaped imports (A → B, A → C, B → D, C → D)
/// are allowed and processed correctly.
///
/// Merge order: each import in the `imports:` list is processed in order;
/// later entries override earlier ones. The current file always takes priority
/// over all its imports.
fn expand_imports(
    config_path: &Path,
    home_dir: &Path,
    from: &str,
    depth: usize,
    stack: &mut Vec<PathBuf>,
) -> Result<RawConfig, ConfigError> {
    if depth > IMPORT_DEPTH_LIMIT {
        return Err(ConfigError::ImportDepthExceeded {
            limit: IMPORT_DEPTH_LIMIT,
            path: config_path.display().to_string(),
        });
    }

    if !config_path.exists() {
        return Err(ConfigError::ImportNotFound {
            path: config_path.display().to_string(),
            from: from.to_string(),
        });
    }

    // Canonicalize for robust cycle detection (resolves symlinks and `..` components).
    // Fall back to the absolute-looking path if canonicalize fails.
    let canonical =
        std::fs::canonicalize(config_path).unwrap_or_else(|_| config_path.to_path_buf());

    if stack.contains(&canonical) {
        let mut chain: Vec<String> = stack.iter().map(|p| p.display().to_string()).collect();
        chain.push(canonical.display().to_string());
        return Err(ConfigError::ImportCycle {
            chain: chain.join(" \u{2192} "),
        });
    }

    // Push onto the DFS stack; pop on return (success or error) for correct diamond handling.
    stack.push(canonical);
    let result = expand_imports_inner(config_path, home_dir, depth, stack);
    stack.pop();
    result
}

/// Inner body of `expand_imports`, called after cycle/depth guards have passed.
fn expand_imports_inner(
    config_path: &Path,
    home_dir: &Path,
    depth: usize,
    stack: &mut Vec<PathBuf>,
) -> Result<RawConfig, ConfigError> {
    let raw: RawConfig = io::load_yaml(config_path)?;
    let base_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    let from = config_path.display().to_string();

    let mut acc = MergedConfig::default();

    for entry in &raw.imports {
        let path_str = entry.path_str();

        // Absolute paths are forbidden — they break portability.
        // Also reject Unix-style root paths (e.g. /etc/...) on Windows, where
        // is_absolute() returns false for paths without a drive letter but such
        // paths are still non-portable.
        let looks_absolute = std::path::Path::new(path_str).is_absolute()
            || path_str.starts_with('/')
            || path_str.starts_with('\\');
        if looks_absolute {
            return Err(ConfigError::ImportAbsolutePath {
                path: path_str.to_string(),
            });
        }

        // Resolve against the appropriate base directory.
        let resolved = match entry.kind() {
            ImportKind::Relative => normalize_path(&base_dir.join(path_str)),
            ImportKind::Home => normalize_path(&home_dir.join(path_str)),
        };

        let imported = expand_imports(&resolved, home_dir, &from, depth + 1, stack)?;
        merge_into_acc(&mut acc, imported);
    }

    // Overlay the current file on top of all its imports.
    merge_into_acc(&mut acc, raw);

    Ok(acc.into_raw_config())
}

// ─── Profile ────────────────────────────────────────────────────────────────

/// Load and normalize a profile from a standalone profile YAML file.
///
/// The file must use grouping syntax:
/// ```yaml
/// components:
///   core:
///     git: {}
///   local:
///     nvim: {}
/// ```
pub fn load_profile(path: &Path) -> Result<Profile, ConfigError> {
    let raw: RawProfile = io::load_yaml(path)?;
    let flat = expand_grouped_components(raw.components)?;
    Ok(Profile { components: flat })
}

// ─── Strategy ───────────────────────────────────────────────────────────────

/// Load and validate a strategy from a YAML file.
pub fn load_strategy(path: &Path) -> Result<Strategy, ConfigError> {
    let raw: Strategy = io::load_yaml(path)?;
    validate_strategy(raw)
}

fn validate_strategy(strategy: Strategy) -> Result<Strategy, ConfigError> {
    // Validate group kind keys: only 'package' and 'runtime' are permitted.
    for (group_name, group) in &strategy.groups {
        for kind_key in group.0.keys() {
            if kind_key != "package" && kind_key != "runtime" {
                return Err(ConfigError::InvalidStrategy {
                    reason: format!(
                        "groups.{group_name}: invalid kind '{kind_key}'; \
                         must be 'package' or 'runtime'"
                    ),
                });
            }
        }
    }

    // Validate each rule.
    for (i, rule) in strategy.rules.iter().enumerate() {
        // use_backend must be a non-empty string.
        if rule.use_backend.is_empty() {
            return Err(ConfigError::InvalidStrategy {
                reason: format!("rules[{i}].use must not be empty"),
            });
        }

        // component-only rule is forbidden: kind must accompany component.
        if rule.selector.component.is_some() && rule.selector.kind.is_none() {
            return Err(ConfigError::InvalidStrategy {
                reason: format!(
                    "rules[{i}]: component-only rule is forbidden because it matches \
                     multiple resource kinds; add 'kind' alongside 'component'"
                ),
            });
        }

        // group-only rule is forbidden: kind must accompany group.
        // Without kind, specificity is (0,0,0,1) which is lower than a kind-only
        // catch-all (0,1,0,0), so the group rule can never win.
        if rule.selector.group.is_some() && rule.selector.kind.is_none() {
            return Err(ConfigError::InvalidStrategy {
                reason: format!(
                    "rules[{i}]: 'group' requires 'kind' to be present; \
                     a group-only rule has lower specificity than a kind-only rule \
                     and can never be selected"
                ),
            });
        }

        // group reference must resolve to a defined group.
        if let Some(ref group_name) = rule.selector.group {
            let group =
                strategy
                    .groups
                    .get(group_name)
                    .ok_or_else(|| ConfigError::InvalidStrategy {
                        reason: format!(
                            "rules[{i}]: group '{group_name}' is not defined in 'groups'"
                        ),
                    })?;

            // kind consistency: if kind=runtime, the group must have runtime entries.
            if let Some(MatchKind::Runtime) = rule.selector.kind {
                if group.names_for_kind("runtime").is_none() {
                    return Err(ConfigError::InvalidStrategy {
                        reason: format!(
                            "rules[{i}]: match.kind is 'runtime' but group '{group_name}' \
                             has no 'runtime' entries"
                        ),
                    });
                }
            }
        }
    }

    Ok(strategy)
}

// ─── Unified config ──────────────────────────────────────────────────────────

/// Load a unified `config.yaml` and return the resolved `Profile` and `Strategy`.
///
/// serde ignores unknown top-level keys by default (no `deny_unknown_fields`),
/// so future sections added to the format will not break existing versions.
///
/// Sections:
/// - `profile` — required. Components use grouping syntax `source_id: { name: {} }`.
/// - `strategy` — optional. Absent → `Strategy::default()` (no overrides).
/// - `bundle`   — optional. Lists which bundles to apply (`use: [base, work]`).
/// - `bundles`  — optional. Named bundle definitions (same grouping syntax as profile).
///
/// Pipeline: bundle expansion → grouped-component normalization → canonical Profile.
///
/// # Format
///
/// ```yaml
/// bundle:
///   use:
///     - base
///
/// bundles:
///   base:
///     components:
///       core:
///         git: {}
///
/// profile:
///   components:
///     local:
///       nvim: {}
///
/// strategy:                  # optional
///   rules:
///     - match:
///         kind: package
///       use: local/brew
///
/// future_section: ...        # silently ignored
/// ```
pub fn load_config(path: &Path) -> Result<(Profile, Strategy), ConfigError> {
    // Resolve home directory once and pass it into the recursive expander.
    // Using an empty PathBuf as fallback means `kind: home` imports will not
    // resolve when $HOME is unset — the error manifests as ImportNotFound.
    let home = home_dir().unwrap_or_default();
    let mut stack: Vec<PathBuf> = Vec::new();

    // Expand all `imports:` directives recursively before bundle/profile processing.
    let raw = expand_imports(path, &home, "<root>", 0, &mut stack)?;

    // profile is required.
    let raw_profile = raw.profile.ok_or_else(|| ConfigError::InvalidProfile {
        reason: "config.yaml must contain a 'profile' section".into(),
    })?;

    // Bundle expansion: merge bundles in use-list order (last wins), then overlay profile.
    let merged = expand_bundles(&raw.bundle, &raw.bundles, raw_profile.components)?;

    // Normalize grouped components to canonical flat map.
    let flat = expand_grouped_components(merged)?;
    let profile = Profile { components: flat };

    // strategy is optional; absent → Strategy::default().
    let strategy = match raw.strategy {
        Some(p) => validate_strategy(p)?,
        None => Strategy::default(),
    };

    Ok((profile, strategy))
}

// ─── Sources ─────────────────────────────────────────────────────────────────

const RESERVED_SOURCE_IDS: &[&str] =
    &["core", "local", "official", "sample", "example", "external"];

/// Load, validate, and path-resolve a sources spec from a YAML file.
///
/// For `type: path` entries, the `path` field is resolved to an absolute path
/// relative to the directory containing `sources.yaml`.
/// `~`-prefixed paths are expanded using the user's home directory.
pub fn load_sources(path: &Path) -> Result<SourcesSpec, ConfigError> {
    let raw: SourcesSpec = io::load_yaml(path)?;
    let sources_dir = path.parent().unwrap_or_else(|| Path::new("."));
    validate_and_resolve_sources(raw, sources_dir)
}

/// Load a sources lock file.
///
/// Returns an empty `SourcesLock` if the file does not exist.
pub fn load_sources_lock(path: &Path) -> Result<SourcesLock, ConfigError> {
    if !path.exists() {
        return Ok(SourcesLock::default());
    }
    let lock: SourcesLock = io::load_yaml(path)?;
    Ok(lock)
}

/// Write a sources spec to a YAML file atomically.
pub fn save_sources(path: &Path, spec: &SourcesSpec) -> Result<(), ConfigError> {
    io::write_yaml_atomic(path, spec)?;
    Ok(())
}

/// Write a sources lock file atomically.
pub fn save_sources_lock(path: &Path, lock: &SourcesLock) -> Result<(), ConfigError> {
    io::write_yaml_atomic(path, lock)?;
    Ok(())
}

/// Resolve a raw source path (possibly relative, `~`-prefixed, or absolute)
/// relative to `sources_yaml_path`'s parent directory.
///
/// This mirrors the path resolution applied by [`load_sources`] for `type: path` entries.
/// Useful when building a new `SourceEntry` before writing it to `sources.yaml`.
pub fn resolve_path_relative_to_sources(raw: &str, sources_yaml_path: &Path) -> std::path::PathBuf {
    let sources_dir = sources_yaml_path.parent().unwrap_or(sources_yaml_path);
    resolve_source_path(raw, sources_dir)
}

fn validate_and_resolve_sources(
    mut spec: SourcesSpec,
    sources_dir: &Path,
) -> Result<SourcesSpec, ConfigError> {
    let mut seen_ids: HashSet<String> = HashSet::new();

    for entry in &mut spec.sources {
        // Reserved ID check.
        if RESERVED_SOURCE_IDS.contains(&entry.id.as_str()) {
            return Err(ConfigError::InvalidSources {
                reason: format!(
                    "source id '{}' is reserved and must not appear in sources.yaml",
                    entry.id
                ),
            });
        }

        // Uniqueness check.
        if !seen_ids.insert(entry.id.clone()) {
            return Err(ConfigError::InvalidSources {
                reason: format!("duplicate source id '{}'", entry.id),
            });
        }

        match entry.source_type {
            SourceType::Git => {
                // url required and non-empty.
                match &entry.url {
                    None => {
                        return Err(ConfigError::InvalidSources {
                            reason: format!("source '{}': url is required for type: git", entry.id),
                        });
                    }
                    Some(u) if u.is_empty() => {
                        return Err(ConfigError::InvalidSources {
                            reason: format!("source '{}': url is required for type: git", entry.id),
                        });
                    }
                    _ => {}
                }

                // path (git repo subpath): no absolute path, no `..` components.
                if let Some(ref p) = entry.path {
                    if p.is_empty() {
                        return Err(ConfigError::InvalidSources {
                            reason: format!("source '{}': path must not be empty", entry.id),
                        });
                    }
                    let subpath = std::path::Path::new(p.as_str());
                    if subpath.is_absolute() || p.starts_with('/') || p.starts_with('\\') {
                        return Err(ConfigError::InvalidSources {
                            reason: format!(
                                "source '{}': path must be relative (no absolute paths in git repo subpath)",
                                entry.id
                            ),
                        });
                    }
                    if subpath
                        .components()
                        .any(|c| c == std::path::Component::ParentDir)
                    {
                        return Err(ConfigError::InvalidSources {
                            reason: format!(
                                "source '{}': path must not contain '..' components",
                                entry.id
                            ),
                        });
                    }
                }

                // ref: exactly one of branch, tag, or commit.
                if let Some(ref r) = entry.source_ref {
                    let set_count = [r.branch.is_some(), r.tag.is_some(), r.commit.is_some()]
                        .iter()
                        .filter(|&&b| b)
                        .count();
                    if set_count != 1 {
                        return Err(ConfigError::InvalidSources {
                            reason: format!(
                                "source '{}': ref must specify exactly one of branch, tag, or commit",
                                entry.id
                            ),
                        });
                    }
                }
            }
            SourceType::Path => {
                // url must not be specified for type:path.
                if entry.url.is_some() {
                    return Err(ConfigError::InvalidSources {
                        reason: format!(
                            "source '{}': url must not be specified for type: path",
                            entry.id
                        ),
                    });
                }

                // path required and non-empty.
                let raw_path = match &entry.path {
                    None => {
                        return Err(ConfigError::InvalidSources {
                            reason: format!(
                                "source '{}': path is required for type: path",
                                entry.id
                            ),
                        });
                    }
                    Some(p) if p.is_empty() => {
                        return Err(ConfigError::InvalidSources {
                            reason: format!("source '{}': path must not be empty", entry.id),
                        });
                    }
                    Some(p) => p.clone(),
                };

                // Resolve to absolute path.
                let resolved = resolve_source_path(&raw_path, sources_dir);
                entry.path = Some(resolved.display().to_string());
            }
        }

        // Validate allow-list names if Detailed (applies to both source types).
        if let Some(AllowSpec::Detailed(ref detail)) = entry.allow {
            if let Some(AllowList::Names(ref names)) = detail.components {
                for n in names {
                    if n.is_empty() {
                        return Err(ConfigError::InvalidSources {
                            reason: format!(
                                "source '{}': allow.components contains empty name",
                                entry.id
                            ),
                        });
                    }
                }
            }
            if let Some(AllowList::Names(ref names)) = detail.backends {
                for n in names {
                    if n.is_empty() {
                        return Err(ConfigError::InvalidSources {
                            reason: format!(
                                "source '{}': allow.backends contains empty name",
                                entry.id
                            ),
                        });
                    }
                }
            }
        }
    }

    Ok(spec)
}

/// Resolve a `type: path` source path to an absolute `PathBuf`.
///
/// Resolution rules (in order):
/// 1. `~` or `~/...` — expanded against the user's home directory.
/// 2. Absolute path — used as-is.
/// 3. Relative path — resolved against `sources_dir` (parent of `sources.yaml`).
fn resolve_source_path(raw: &str, sources_dir: &Path) -> std::path::PathBuf {
    // Home directory expansion.
    if raw == "~" {
        if let Some(home) = home_dir() {
            return home;
        }
    } else if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }

    let p = std::path::Path::new(raw);
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        sources_dir.join(p)
    };
    // Normalize away `.` components without requiring the path to exist.
    normalize_path(&joined)
}

/// Remove `.` components from a path without hitting the filesystem.
fn normalize_path(path: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut out = std::path::PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Return the user's home directory from the environment.
fn home_dir() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        std::env::var("USERPROFILE")
            .ok()
            .map(std::path::PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var("HOME").ok().map(std::path::PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_yaml_file(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, content).unwrap();
        p
    }

    // ── Profile (grouping) tests ───────────────────────────────────────────

    #[test]
    fn grouped_components_normalized_to_canonical() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "components:\n  core:\n    git: {}\n  local:\n    nvim: {}\n";
        let p = write_yaml_file(dir.path(), "profile.yaml", yaml);
        let profile = load_profile(&p).unwrap();
        assert!(
            profile.components.contains_key("core/git"),
            "core/git must be present"
        );
        assert!(
            profile.components.contains_key("local/nvim"),
            "local/nvim must be present"
        );
        assert_eq!(profile.components.len(), 2);
    }

    #[test]
    fn profile_empty_components_ok() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml_file(dir.path(), "profile.yaml", "components: {}\n");
        let profile = load_profile(&p).unwrap();
        assert!(profile.components.is_empty());
    }

    #[test]
    fn profile_params_forwarded_through_grouping() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "components:\n  local:\n    node:\n      params:\n        version: \"20\"\n";
        let p = write_yaml_file(dir.path(), "profile.yaml", yaml);
        let profile = load_profile(&p).unwrap();
        let cfg = profile.components.get("local/node").unwrap();
        let params = cfg.params.as_ref().unwrap();
        assert_eq!(
            params["version"],
            model::params::ParamValue::String("20".to_string())
        );
    }

    #[test]
    fn profile_empty_source_id_rejected() {
        // YAML: components: { "": { git: {} } }
        // serde_yaml will parse "" as an empty key
        let dir = tempfile::tempdir().unwrap();
        let yaml = "components:\n  \"\":\n    git: {}\n";
        let p = write_yaml_file(dir.path(), "profile.yaml", yaml);
        let err = load_profile(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidProfile { .. }));
    }

    #[test]
    fn profile_empty_component_name_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "components:\n  core:\n    \"\": {}\n";
        let p = write_yaml_file(dir.path(), "profile.yaml", yaml);
        let err = load_profile(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidProfile { .. }));
    }

    #[test]
    fn profile_duplicate_canonical_rejected() {
        // Two sources that would produce the same canonical id via different paths
        // are not possible in grouping syntax (source_id is the outer key, so
        // "core: { git: {} }" appears once). Duplicates can only occur within
        // the same source group — e.g. outer key "core" appearing twice, which
        // YAML/serde handles by last-write-wins (HashMap). So verify the happy
        // path instead: same source, two different components are both present.
        let dir = tempfile::tempdir().unwrap();
        let yaml = "components:\n  core:\n    git: {}\n    bash: {}\n";
        let p = write_yaml_file(dir.path(), "profile.yaml", yaml);
        let profile = load_profile(&p).unwrap();
        assert!(profile.components.contains_key("core/git"));
        assert!(profile.components.contains_key("core/bash"));
    }

    #[test]
    fn profile_bare_name_at_top_level_rejected() {
        // Old format: "components:\n  git: {}\n" where the value is an empty map.
        // Now "git" is treated as a source_id mapping to component-map {"git": {}}.
        // This is actually valid (source "git" with component "{}") - but since the
        // value `{}` is an empty HashMap, "git" source has no components.
        // The resulting canonical map will be empty, not an error.
        // The important invariant is: you cannot sneak a bare name through as canonical.
        let dir = tempfile::tempdir().unwrap();
        // "components:\n  git: {}\n" — source_id=git, inner map is empty → 0 components
        let yaml = "components:\n  git: {}\n";
        let p = write_yaml_file(dir.path(), "profile.yaml", yaml);
        let profile = load_profile(&p).unwrap();
        // Inner {} means empty sourced components, not "git" as a canonical ID.
        assert!(
            !profile.components.contains_key("git"),
            "bare 'git' must not appear as canonical"
        );
        assert!(
            !profile.components.contains_key("core/git"),
            "must not auto-prefix to core/git"
        );
        assert!(
            profile.components.is_empty(),
            "source 'git' has no declared components"
        );
    }

    // ── Strategy tests ─────────────────────────────────────────────────────

    #[test]
    fn strategy_empty_is_valid() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml_file(dir.path(), "strategy.yaml", "{}\n");
        let strategy = load_strategy(&p).unwrap();
        assert!(strategy.rules.is_empty());
        assert!(strategy.groups.is_empty());
    }

    #[test]
    fn strategy_simple_rule_loads() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "rules:\n  - match:\n      kind: package\n    use: core/brew\n";
        let p = write_yaml_file(dir.path(), "strategy.yaml", yaml);
        let strategy = load_strategy(&p).unwrap();
        assert_eq!(strategy.rules.len(), 1);
        assert_eq!(strategy.rules[0].use_backend, "core/brew");
    }

    #[test]
    fn strategy_empty_use_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "rules:\n  - match:\n      kind: package\n    use: \"\"\n";
        let p = write_yaml_file(dir.path(), "strategy.yaml", yaml);
        let err = load_strategy(&p).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidStrategy { reason } if reason.contains("rules[0].use"))
        );
    }

    #[test]
    fn strategy_component_without_kind_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "rules:\n  - match:\n      component: core/cli-tools\n    use: core/npm\n";
        let p = write_yaml_file(dir.path(), "strategy.yaml", yaml);
        let err = load_strategy(&p).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidStrategy { reason } if reason.contains("component-only"))
        );
    }

    #[test]
    fn strategy_component_with_kind_is_valid() {
        let dir = tempfile::tempdir().unwrap();
        let yaml =
            "rules:\n  - match:\n      component: core/cli-tools\n      kind: package\n    use: core/npm\n";
        let p = write_yaml_file(dir.path(), "strategy.yaml", yaml);
        let strategy = load_strategy(&p).unwrap();
        assert_eq!(strategy.rules.len(), 1);
    }

    #[test]
    fn strategy_group_without_kind_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
groups:\n  npm_global:\n    package:\n      - eslint\nrules:\n  - match:\n      group: npm_global\n    use: core/npm\n";
        let p = write_yaml_file(dir.path(), "strategy.yaml", yaml);
        let err = load_strategy(&p).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidStrategy { reason } if reason.contains("'group' requires 'kind'")),
        );
    }

    #[test]
    fn strategy_undefined_group_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml =
            "rules:\n  - match:\n      kind: package\n      group: npm_global\n    use: core/npm\n";
        let p = write_yaml_file(dir.path(), "strategy.yaml", yaml);
        let err = load_strategy(&p).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidStrategy { reason } if reason.contains("npm_global")),
        );
    }

    #[test]
    fn strategy_group_and_rule_consistent_kind_is_valid() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
groups:\n  npm_global:\n    package:\n      - eslint\nrules:\n  - match:\n      kind: package\n      group: npm_global\n    use: core/npm\n";
        let p = write_yaml_file(dir.path(), "strategy.yaml", yaml);
        let strategy = load_strategy(&p).unwrap();
        assert_eq!(strategy.rules.len(), 1);
    }

    #[test]
    fn strategy_runtime_rule_referencing_package_group_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
groups:\n  my_group:\n    package:\n      - eslint\nrules:\n  - match:\n      kind: runtime\n      group: my_group\n    use: core/mise\n";
        let p = write_yaml_file(dir.path(), "strategy.yaml", yaml);
        let err = load_strategy(&p).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidStrategy { reason } if reason.contains("runtime")),
        );
    }

    #[test]
    fn strategy_group_invalid_kind_key_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "groups:\n  my_group:\n    tool:\n      - something\n";
        let p = write_yaml_file(dir.path(), "strategy.yaml", yaml);
        let err = load_strategy(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidStrategy { reason } if reason.contains("tool")),);
    }

    // ── Sources tests ──────────────────────────────────────────────────────

    #[test]
    fn sources_valid_external_source() {
        let dir = tempfile::tempdir().unwrap();
        let yaml =
            "sources:\n  - id: community\n    type: git\n    url: https://github.com/ex/repo\n";
        let p = write_yaml_file(dir.path(), "sources.yaml", yaml);
        let spec = load_sources(&p).unwrap();
        assert_eq!(spec.sources[0].id, "community");
    }

    #[test]
    fn sources_reserved_id_rejected() {
        for reserved in &["core", "local", "official"] {
            let dir = tempfile::tempdir().unwrap();
            let yaml =
                format!("sources:\n  - id: {reserved}\n    type: git\n    url: https://x.com\n");
            let p = write_yaml_file(dir.path(), "sources.yaml", &yaml);
            let err = load_sources(&p).unwrap_err();
            assert!(
                matches!(err, ConfigError::InvalidSources { .. }),
                "expected error for reserved id '{reserved}'"
            );
        }
    }

    #[test]
    fn sources_duplicate_id_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "sources:\n  - id: tools\n    type: git\n    url: https://a.com\n  - id: tools\n    type: git\n    url: https://b.com\n";
        let p = write_yaml_file(dir.path(), "sources.yaml", yaml);
        let err = load_sources(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidSources { .. }));
    }

    #[test]
    fn sources_empty_url_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "sources:\n  - id: tools\n    type: git\n    url: \"\"\n";
        let p = write_yaml_file(dir.path(), "sources.yaml", yaml);
        let err = load_sources(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidSources { .. }));
    }

    #[test]
    fn sources_empty_list_ok() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml_file(dir.path(), "sources.yaml", "{}\n");
        let spec = load_sources(&p).unwrap();
        assert!(spec.sources.is_empty());
    }

    #[test]
    fn sources_allow_wildcard_ok() {
        let dir = tempfile::tempdir().unwrap();
        let yaml =
            "sources:\n  - id: ext\n    type: git\n    url: https://x.com\n    allow: \"*\"\n";
        let p = write_yaml_file(dir.path(), "sources.yaml", yaml);
        let spec = load_sources(&p).unwrap();
        assert!(matches!(spec.sources[0].allow, Some(AllowSpec::All(_))));
    }

    // ── load_config (grouping) tests ───────────────────────────────────────

    #[test]
    fn config_profile_and_strategy_parsed() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
profile:
  components:
    core:
      git: {}
    local:
      myapp: {}

strategy:
  rules:
    - match:
        kind: package
      use: local/brew
";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let (profile, strategy) = load_config(&p).unwrap();
        assert!(
            profile.components.contains_key("core/git"),
            "core/git must be present"
        );
        assert!(
            profile.components.contains_key("local/myapp"),
            "local/myapp must be present"
        );
        assert_eq!(strategy.rules.len(), 1);
        assert_eq!(strategy.rules[0].use_backend, "local/brew");
    }

    #[test]
    fn config_strategy_optional_defaults_to_empty() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "profile:\n  components:\n    core:\n      git: {}\n";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let (profile, strategy) = load_config(&p).unwrap();
        assert!(profile.components.contains_key("core/git"));
        assert!(strategy.rules.is_empty());
        assert!(strategy.groups.is_empty());
    }

    #[test]
    fn config_empty_profile_components_ok() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "profile:\n  components: {}\n";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let (profile, _) = load_config(&p).unwrap();
        assert!(profile.components.is_empty());
    }

    #[test]
    fn config_extra_unknown_keys_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
profile:
  components:
    core:
      git: {}
future_section:
  some_key: value
another_unknown: 42
";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        assert!(load_config(&p).is_ok());
    }

    #[test]
    fn config_missing_profile_section_errors() {
        let dir = tempfile::tempdir().unwrap();
        let yaml =
            "strategy:\n  rules:\n    - match:\n        kind: package\n      use: local/brew\n";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let err = load_config(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidProfile { .. }));
    }

    #[test]
    fn config_invalid_strategy_propagates_error() {
        let dir = tempfile::tempdir().unwrap();
        // component-only rule (no kind) must be rejected.
        let yaml = "\
profile:
  components:
    core:
      git: {}
strategy:
  rules:
    - match:
        component: core/cli-tools
      use: core/npm
";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let err = load_config(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidStrategy { .. }));
    }

    #[test]
    fn config_params_config_forwarded() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "profile:\n  components:\n    local:\n      node:\n        params:\n          version: \"20\"\n";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let (profile, _) = load_config(&p).unwrap();
        let cfg = profile.components.get("local/node").unwrap();
        let params = cfg.params.as_ref().unwrap();
        assert_eq!(
            params["version"],
            model::params::ParamValue::String("20".to_string())
        );
    }

    // ── Bundle tests ───────────────────────────────────────────────────────

    #[test]
    fn bundle_components_merged_into_profile() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
bundle:
  use:
    - base

bundles:
  base:
    components:
      core:
        git: {}

profile:
  components:
    local:
      nvim: {}
";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let (profile, _) = load_config(&p).unwrap();
        assert!(
            profile.components.contains_key("core/git"),
            "bundle component must be merged"
        );
        assert!(
            profile.components.contains_key("local/nvim"),
            "profile component must be present"
        );
        assert_eq!(profile.components.len(), 2);
    }

    #[test]
    fn bundle_use_order_last_wins() {
        // base: core/git params.version "1.0", lang: core/git params.version "2.0"
        // use: [base, lang] → lang wins
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
bundle:
  use:
    - base
    - lang

bundles:
  base:
    components:
      core:
        git:
          params:
            version: \"1.0\"
  lang:
    components:
      core:
        git:
          params:
            version: \"2.0\"

profile:
  components: {}
";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let (profile, _) = load_config(&p).unwrap();
        let cfg = profile.components.get("core/git").unwrap();
        let params = cfg.params.as_ref().unwrap();
        assert_eq!(
            params["version"],
            model::params::ParamValue::String("2.0".to_string()),
            "last bundle in use list must win"
        );
    }

    #[test]
    fn bundle_profile_components_override_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
bundle:
  use:
    - base

bundles:
  base:
    components:
      core:
        git:
          params:
            version: \"1.0\"

profile:
  components:
    core:
      git:
        params:
          version: \"override\"
";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let (profile, _) = load_config(&p).unwrap();
        let cfg = profile.components.get("core/git").unwrap();
        let params = cfg.params.as_ref().unwrap();
        assert_eq!(
            params["version"],
            model::params::ParamValue::String("override".to_string()),
            "profile.components must override bundle"
        );
    }

    #[test]
    fn bundle_undefined_reference_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
bundle:
  use:
    - nonexistent

profile:
  components: {}
";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let err = load_config(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidProfile { .. }));
    }

    #[test]
    fn bundle_section_absent_ok() {
        // No bundle/bundles sections: behaves identically to profile-only config.
        let dir = tempfile::tempdir().unwrap();
        let yaml = "profile:\n  components:\n    core:\n      git: {}\n";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let (profile, _) = load_config(&p).unwrap();
        assert!(profile.components.contains_key("core/git"));
        assert_eq!(profile.components.len(), 1);
    }

    #[test]
    fn bundle_params_only_no_profile_params() {
        // bundle provides params; profile does not specify params → bundle params pass through.
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
bundle:
  use:
    - base

bundles:
  base:
    components:
      core:
        git:
          params:
            version: \"2.40\"

profile:
  components:
    core:
      git: {}
";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let (profile, _) = load_config(&p).unwrap();
        let cfg = profile.components.get("core/git").unwrap();
        let params = cfg
            .params
            .as_ref()
            .expect("bundle params must pass through");
        assert_eq!(
            params["version"],
            model::params::ParamValue::String("2.40".to_string()),
        );
    }

    #[test]
    fn bundle_params_shallow_merge_preserves_unspecified_keys() {
        // bundle provides {a: "1", b: "2"}; profile provides {b: "override"}
        // result: {a: "1", b: "override"}
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
bundle:
  use:
    - base

bundles:
  base:
    components:
      core:
        git:
          params:
            a: \"1\"
            b: \"2\"

profile:
  components:
    core:
      git:
        params:
          b: \"override\"
";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let (profile, _) = load_config(&p).unwrap();
        let cfg = profile.components.get("core/git").unwrap();
        let params = cfg.params.as_ref().unwrap();
        assert_eq!(
            params["a"],
            model::params::ParamValue::String("1".to_string()),
            "key only in bundle must be preserved"
        );
        assert_eq!(
            params["b"],
            model::params::ParamValue::String("override".to_string()),
            "profile key must override bundle"
        );
    }

    #[test]
    fn bundle_inter_bundle_params_shallow_merge() {
        // bundle base: {a: "1", b: "2"}, bundle lang: {b: "3", c: "4"}
        // use: [base, lang] → {a: "1", b: "3", c: "4"}
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
bundle:
  use:
    - base
    - lang

bundles:
  base:
    components:
      core:
        git:
          params:
            a: \"1\"
            b: \"2\"
  lang:
    components:
      core:
        git:
          params:
            b: \"3\"
            c: \"4\"

profile:
  components: {}
";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let (profile, _) = load_config(&p).unwrap();
        let cfg = profile.components.get("core/git").unwrap();
        let params = cfg.params.as_ref().unwrap();
        assert_eq!(
            params["a"],
            model::params::ParamValue::String("1".to_string()),
        );
        assert_eq!(
            params["b"],
            model::params::ParamValue::String("3".to_string()),
            "later bundle must win"
        );
        assert_eq!(
            params["c"],
            model::params::ParamValue::String("4".to_string()),
        );
    }

    #[test]
    fn empty_params_does_not_clear_bundle_params() {
        // profile specifies empty params {} → should NOT erase bundle params.
        // Empty params map deserializes as Some(empty HashMap) → convert_raw_params returns None.
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
bundle:
  use:
    - base

bundles:
  base:
    components:
      core:
        git:
          params:
            version: \"2.40\"

profile:
  components:
    core:
      git:
        params: {}
";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let (profile, _) = load_config(&p).unwrap();
        let cfg = profile.components.get("core/git").unwrap();
        let params = cfg
            .params
            .as_ref()
            .expect("bundle params must survive empty overlay");
        assert_eq!(
            params["version"],
            model::params::ParamValue::String("2.40".to_string()),
        );
    }

    #[test]
    fn legacy_version_field_rejected() {
        // The old `version` field must be rejected now that `params` is the only accepted key.
        let dir = tempfile::tempdir().unwrap();
        let yaml = "profile:\n  components:\n    core:\n      git:\n        version: \"2.40\"\n";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let err = load_config(&p).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("version"),
            "error must mention the unknown field 'version': {msg}"
        );
    }

    // ── Additional Sources tests ───────────────────────────────────────────

    #[test]
    fn sources_type_git_with_ref_ok() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "sources:\n  - id: ext\n    type: git\n    url: https://x.com\n    ref:\n      branch: main\n";
        let p = write_yaml_file(dir.path(), "sources.yaml", yaml);
        let spec = load_sources(&p).unwrap();
        let r = spec.sources[0].source_ref.as_ref().unwrap();
        assert_eq!(r.branch.as_deref(), Some("main"));
        assert!(r.tag.is_none());
        assert!(r.commit.is_none());
    }

    #[test]
    fn sources_type_git_ref_multiple_fields_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "sources:\n  - id: ext\n    type: git\n    url: https://x.com\n    ref:\n      branch: main\n      tag: v1\n";
        let p = write_yaml_file(dir.path(), "sources.yaml", yaml);
        let err = load_sources(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidSources { .. }));
    }

    #[test]
    fn sources_type_git_dotdot_subpath_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml =
            "sources:\n  - id: ext\n    type: git\n    url: https://x.com\n    path: ../sibling\n";
        let p = write_yaml_file(dir.path(), "sources.yaml", yaml);
        let err = load_sources(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidSources { .. }));
    }

    #[test]
    fn sources_type_git_absolute_subpath_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml =
            "sources:\n  - id: ext\n    type: git\n    url: https://x.com\n    path: /absolute\n";
        let p = write_yaml_file(dir.path(), "sources.yaml", yaml);
        let err = load_sources(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidSources { .. }));
    }

    #[test]
    fn sources_type_path_valid() {
        let dir = tempfile::tempdir().unwrap();
        // Use absolute path so resolution doesn't depend on tempdir.
        let yaml = format!(
            "sources:\n  - id: mylab\n    type: path\n    path: {}\n",
            dir.path().display()
        );
        let p = write_yaml_file(dir.path(), "sources.yaml", &yaml);
        let spec = load_sources(&p).unwrap();
        assert_eq!(spec.sources[0].source_type, SourceType::Path);
        // After resolution, path is absolute (was already absolute).
        assert!(std::path::Path::new(spec.sources[0].path.as_deref().unwrap()).is_absolute());
    }

    #[test]
    fn sources_type_path_no_path_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "sources:\n  - id: mylab\n    type: path\n";
        let p = write_yaml_file(dir.path(), "sources.yaml", yaml);
        let err = load_sources(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidSources { .. }));
    }

    #[test]
    fn sources_type_path_with_url_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = format!(
            "sources:\n  - id: mylab\n    type: path\n    path: {}\n    url: https://x.com\n",
            dir.path().display()
        );
        let p = write_yaml_file(dir.path(), "sources.yaml", &yaml);
        let err = load_sources(&p).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidSources { .. }));
    }

    #[test]
    fn sources_type_path_relative_resolved_against_sources_dir() {
        let dir = tempfile::tempdir().unwrap();
        // Create the subdir so it's a plausible path (resolve doesn't check existence).
        let yaml = "sources:\n  - id: mylab\n    type: path\n    path: ./subdir\n";
        let p = write_yaml_file(dir.path(), "sources.yaml", yaml);
        let spec = load_sources(&p).unwrap();
        let resolved = spec.sources[0].path.as_deref().unwrap();
        let expected = dir.path().join("subdir").display().to_string();
        assert_eq!(resolved, expected);
    }

    #[test]
    fn sources_lock_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "sources:\n  community:\n    resolved_commit: abcdef1234567890abcdef1234567890abcdef12\n    fetched_at: '2026-04-07T00:00:00Z'\n    manifest_hash: 'sha256:abc'\n";
        let p = write_yaml_file(dir.path(), "sources.lock.yaml", yaml);
        let lock = load_sources_lock(&p).unwrap();
        assert_eq!(
            lock.sources["community"].resolved_commit,
            "abcdef1234567890abcdef1234567890abcdef12"
        );
        // Round-trip: save and reload.
        let p2 = dir.path().join("sources2.lock.yaml");
        save_sources_lock(&p2, &lock).unwrap();
        let lock2 = load_sources_lock(&p2).unwrap();
        assert_eq!(lock, lock2);
    }

    #[test]
    fn sources_lock_absent_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sources.lock.yaml");
        let lock = load_sources_lock(&p).unwrap();
        assert!(lock.sources.is_empty());
    }

    // ── Import tests ───────────────────────────────────────────────────────

    #[test]
    fn import_single_file_bundles_available() {
        // Main config imports a file that defines a bundle; bundle.use references it.
        let dir = tempfile::tempdir().unwrap();

        let base_yaml = "\
bundles:
  base:
    components:
      core:
        git: {}
";
        write_yaml_file(dir.path(), "base.yaml", base_yaml);

        let main_yaml = "\
imports:
  - base.yaml

bundle:
  use:
    - base

profile:
  components:
    local:
      nvim: {}
";
        let p = write_yaml_file(dir.path(), "config.yaml", main_yaml);
        let (profile, _) = load_config(&p).unwrap();
        assert!(
            profile.components.contains_key("core/git"),
            "imported bundle must be applied"
        );
        assert!(
            profile.components.contains_key("local/nvim"),
            "main profile must be present"
        );
    }

    #[test]
    fn import_multiple_files_last_wins() {
        // Two imports each provide 'base' bundle; the later one wins.
        let dir = tempfile::tempdir().unwrap();

        write_yaml_file(
            dir.path(),
            "a.yaml",
            "\
bundles:
  base:
    components:
      core:
        git: {}
",
        );
        write_yaml_file(
            dir.path(),
            "b.yaml",
            "\
bundles:
  base:
    components:
      core:
        bash: {}
",
        );
        let main_yaml = "\
imports:
  - a.yaml
  - b.yaml

bundle:
  use:
    - base

profile:
  components: {}
";
        let p = write_yaml_file(dir.path(), "config.yaml", main_yaml);
        let (profile, _) = load_config(&p).unwrap();
        // b.yaml is later → its 'base' bundle wins
        assert!(
            profile.components.contains_key("core/bash"),
            "later import must win"
        );
        // a.yaml's bundle was completely replaced at bundle-name level
        assert!(
            !profile.components.contains_key("core/git"),
            "earlier bundle must be replaced"
        );
    }

    #[test]
    fn import_main_overrides_imported_profile() {
        // Imported file provides profile.components; main file overrides the same component.
        let dir = tempfile::tempdir().unwrap();

        write_yaml_file(
            dir.path(),
            "imported.yaml",
            "\
profile:
  components:
    core:
      git:
        params:
          version: \"2.40\"
",
        );
        let main_yaml = "\
imports:
  - imported.yaml

profile:
  components:
    core:
      git:
        params:
          version: \"2.44\"
";
        let p = write_yaml_file(dir.path(), "config.yaml", main_yaml);
        let (profile, _) = load_config(&p).unwrap();
        let cfg = profile.components.get("core/git").unwrap();
        let params = cfg.params.as_ref().unwrap();
        assert_eq!(
            params["version"],
            model::params::ParamValue::String("2.44".to_string()),
            "main file must override imported file"
        );
    }

    #[test]
    fn import_profile_merge_different_sources() {
        // Imported file provides core/git; main file provides local/nvim.
        // Both must appear in the final profile.
        let dir = tempfile::tempdir().unwrap();

        write_yaml_file(
            dir.path(),
            "imported.yaml",
            "\
profile:
  components:
    core:
      git: {}
",
        );
        let main_yaml = "\
imports:
  - imported.yaml

profile:
  components:
    local:
      nvim: {}
";
        let p = write_yaml_file(dir.path(), "config.yaml", main_yaml);
        let (profile, _) = load_config(&p).unwrap();
        assert!(
            profile.components.contains_key("core/git"),
            "imported component must be present"
        );
        assert!(
            profile.components.contains_key("local/nvim"),
            "main component must be present"
        );
        assert_eq!(profile.components.len(), 2);
    }

    #[test]
    fn import_recursive_a_b_c() {
        // main → a.yaml → b.yaml; each layer adds a component.
        let dir = tempfile::tempdir().unwrap();

        write_yaml_file(
            dir.path(),
            "b.yaml",
            "\
profile:
  components:
    core:
      bash: {}
",
        );
        write_yaml_file(
            dir.path(),
            "a.yaml",
            "\
imports:
  - b.yaml

profile:
  components:
    core:
      git: {}
",
        );
        let main_yaml = "\
imports:
  - a.yaml

profile:
  components:
    local:
      nvim: {}
";
        let p = write_yaml_file(dir.path(), "config.yaml", main_yaml);
        let (profile, _) = load_config(&p).unwrap();
        assert!(
            profile.components.contains_key("core/bash"),
            "b.yaml component must be present"
        );
        assert!(
            profile.components.contains_key("core/git"),
            "a.yaml component must be present"
        );
        assert!(
            profile.components.contains_key("local/nvim"),
            "main component must be present"
        );
    }

    #[test]
    fn import_diamond_allowed() {
        // main → a.yaml and b.yaml; both → shared.yaml.  Not a cycle.
        let dir = tempfile::tempdir().unwrap();

        write_yaml_file(
            dir.path(),
            "shared.yaml",
            "\
profile:
  components:
    core:
      git: {}
",
        );
        write_yaml_file(
            dir.path(),
            "a.yaml",
            "\
imports:
  - shared.yaml
profile:
  components:
    local:
      tool-a: {}
",
        );
        write_yaml_file(
            dir.path(),
            "b.yaml",
            "\
imports:
  - shared.yaml
profile:
  components:
    local:
      tool-b: {}
",
        );
        let main_yaml = "\
imports:
  - a.yaml
  - b.yaml
profile:
  components: {}
";
        let p = write_yaml_file(dir.path(), "config.yaml", main_yaml);
        // Should succeed without cycle error
        let (profile, _) = load_config(&p).unwrap();
        assert!(profile.components.contains_key("core/git"));
        assert!(profile.components.contains_key("local/tool-a"));
        assert!(profile.components.contains_key("local/tool-b"));
    }

    #[test]
    fn import_cycle_detected() {
        let dir = tempfile::tempdir().unwrap();

        // a.yaml imports b.yaml; b.yaml imports a.yaml → cycle
        write_yaml_file(
            dir.path(),
            "b.yaml",
            "\
imports:
  - a.yaml
profile:
  components: {}
",
        );
        write_yaml_file(
            dir.path(),
            "a.yaml",
            "\
imports:
  - b.yaml
profile:
  components: {}
",
        );
        let main_yaml = "\
imports:
  - a.yaml
profile:
  components:
    local:
      nvim: {}
";
        let p = write_yaml_file(dir.path(), "config.yaml", main_yaml);
        let err = load_config(&p).unwrap_err();
        assert!(
            matches!(err, ConfigError::ImportCycle { .. }),
            "expected ImportCycle error, got: {err}"
        );
    }

    #[test]
    fn import_depth_exceeded() {
        // Create a chain of files each importing the next, exceeding IMPORT_DEPTH_LIMIT.
        let dir = tempfile::tempdir().unwrap();

        // Create files depth+2 down to 1, each importing the next.
        let limit = IMPORT_DEPTH_LIMIT + 2;
        for i in (1..limit).rev() {
            let next = format!("file{}.yaml", i + 1);
            let content = format!("imports:\n  - {next}\nprofile:\n  components: {{}}\n");
            write_yaml_file(dir.path(), &format!("file{i}.yaml"), &content);
        }
        // Leaf file (no imports)
        write_yaml_file(
            dir.path(),
            &format!("file{limit}.yaml"),
            "profile:\n  components: {}\n",
        );

        let main_yaml = "imports:\n  - file1.yaml\nprofile:\n  components: {}\n".to_string();
        let p = write_yaml_file(dir.path(), "config.yaml", &main_yaml);
        let err = load_config(&p).unwrap_err();
        assert!(
            matches!(err, ConfigError::ImportDepthExceeded { .. }),
            "expected ImportDepthExceeded, got: {err}"
        );
    }

    #[test]
    fn import_not_found_error() {
        let dir = tempfile::tempdir().unwrap();
        let main_yaml = "\
imports:
  - nonexistent.yaml
profile:
  components:
    local:
      nvim: {}
";
        let p = write_yaml_file(dir.path(), "config.yaml", main_yaml);
        let err = load_config(&p).unwrap_err();
        assert!(
            matches!(err, ConfigError::ImportNotFound { .. }),
            "expected ImportNotFound, got: {err}"
        );
    }

    #[test]
    fn import_absolute_path_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let main_yaml = "\
imports:
  - /etc/loadout/base.yaml
profile:
  components:
    local:
      nvim: {}
";
        let p = write_yaml_file(dir.path(), "config.yaml", main_yaml);
        let err = load_config(&p).unwrap_err();
        assert!(
            matches!(err, ConfigError::ImportAbsolutePath { .. }),
            "expected ImportAbsolutePath, got: {err}"
        );
    }

    #[test]
    fn import_object_form_accepted() {
        // Explicit `path:` form (kind: relative by default) works identically to shorthand.
        let dir = tempfile::tempdir().unwrap();

        write_yaml_file(
            dir.path(),
            "base.yaml",
            "\
profile:
  components:
    core:
      git: {}
",
        );
        let main_yaml = "\
imports:
  - path: base.yaml
profile:
  components:
    local:
      nvim: {}
";
        let p = write_yaml_file(dir.path(), "config.yaml", main_yaml);
        let (profile, _) = load_config(&p).unwrap();
        assert!(profile.components.contains_key("core/git"));
        assert!(profile.components.contains_key("local/nvim"));
    }

    #[test]
    fn import_kind_home_resolves_against_injected_home() {
        // We cannot reliably test against the real $HOME, but we can verify that
        // kind: home resolves against a known directory by placing the file there.
        // Since load_config uses home_dir() internally, we test expand_imports directly.
        let home_dir = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();

        // File located at <home_dir>/shared.yaml
        write_yaml_file(
            home_dir.path(),
            "shared.yaml",
            "\
profile:
  components:
    core:
      git: {}
",
        );
        let main_yaml = "\
imports:
  - path: shared.yaml
    kind: home
profile:
  components:
    local:
      nvim: {}
";
        let config_path = write_yaml_file(config_dir.path(), "config.yaml", main_yaml);

        // Call expand_imports directly to inject a controlled home_dir.
        let mut stack = Vec::new();
        let raw = expand_imports(&config_path, home_dir.path(), "<test>", 0, &mut stack).unwrap();

        // Verify profile was merged correctly.
        let merged_components = raw.profile.unwrap().components;
        assert!(merged_components
            .get("core")
            .and_then(|m| m.get("git"))
            .is_some());
        assert!(merged_components
            .get("local")
            .and_then(|m| m.get("nvim"))
            .is_some());
    }

    #[test]
    fn import_strategy_field_level_merge() {
        // Import provides a package rule; main provides a runtime rule.
        // The main file's rules replace entirely (rules array from main wins).
        let dir = tempfile::tempdir().unwrap();

        write_yaml_file(
            dir.path(),
            "base.yaml",
            "\
strategy:
  rules:
    - match:
        kind: package
      use: local/brew
profile:
  components: {}
",
        );
        let main_yaml = "\
imports:
  - base.yaml
profile:
  components:
    local:
      nvim: {}
strategy:
  rules:
    - match:
        kind: runtime
      use: local/mise
";
        let p = write_yaml_file(dir.path(), "config.yaml", main_yaml);
        let (_, strategy) = load_config(&p).unwrap();
        // Main's rules replace imported rules entirely.
        assert_eq!(strategy.rules.len(), 1);
        assert_eq!(
            strategy.rules[0].selector.kind,
            Some(MatchKind::Runtime),
            "runtime rule from main must win"
        );
        assert_eq!(strategy.rules[0].use_backend, "local/mise");
    }

    #[test]
    fn import_with_no_imports_field_unchanged() {
        // A config without `imports:` key must behave exactly as before.
        let dir = tempfile::tempdir().unwrap();
        let yaml = "\
profile:
  components:
    core:
      git: {}
";
        let p = write_yaml_file(dir.path(), "config.yaml", yaml);
        let (profile, _) = load_config(&p).unwrap();
        assert!(profile.components.contains_key("core/git"));
        assert_eq!(profile.components.len(), 1);
    }
}
