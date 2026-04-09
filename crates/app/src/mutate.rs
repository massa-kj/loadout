// Config mutation use cases.
//
// These use cases modify config files on behalf of the CLI. Each function:
//   1. Resolves the target config path (from explicit arg or active context).
//   2. Delegates the actual file mutation to the `config` crate.
//   3. Returns the path of the file that was modified.

use std::path::PathBuf;

use crate::context::{AppContext, AppError};

/// Source IDs that users may not use for external sources.
const RESERVED_SOURCE_IDS: &[&str] = &["core", "local", "official"];

// ---------------------------------------------------------------------------
// Config init
// ---------------------------------------------------------------------------

/// Create a new config file from the built-in template.
///
/// `name` is a bare config name (e.g. `linux`) and resolves to
/// `{config_home}/configs/{name}.yaml`. Returns the created file's path.
pub fn config_init(ctx: &AppContext, name: &str) -> Result<PathBuf, AppError> {
    let path = ctx.resolve_config_path(name);
    config::create_config(&path)?;
    Ok(path)
}

// ---------------------------------------------------------------------------
// Config feature mutations
// ---------------------------------------------------------------------------

/// Add a feature to a config file's `profile.features` section.
///
/// `feature_id` may be canonical (`source/name`) or a bare name (resolved to
/// `local/<name>`). If `name_or_path` is `None`, the active context is used.
/// Returns the path of the modified config file.
pub fn config_feature_add(
    ctx: &AppContext,
    name_or_path: Option<&str>,
    feature_id: &str,
) -> Result<PathBuf, AppError> {
    let path = resolve_config_required(ctx, name_or_path)?;
    let (source, name) = split_feature_id(feature_id);
    config::add_feature(&path, &source, &name)?;
    Ok(path)
}

/// Remove a feature from a config file's `profile.features` section.
///
/// Returns `(path, found)` — `found` is `false` if the feature was not present
/// and no change was made.
pub fn config_feature_remove(
    ctx: &AppContext,
    name_or_path: Option<&str>,
    feature_id: &str,
) -> Result<(PathBuf, bool), AppError> {
    let path = resolve_config_required(ctx, name_or_path)?;
    let (source, name) = split_feature_id(feature_id);
    let found = config::remove_feature(&path, &source, &name)?;
    Ok((path, found))
}

// ---------------------------------------------------------------------------
// Raw YAML access (escape hatch)
// ---------------------------------------------------------------------------

/// Return the raw YAML text of a config file without parsing or normalizing it.
pub fn config_raw_show(ctx: &AppContext, name_or_path: Option<&str>) -> Result<String, AppError> {
    let path = resolve_config_required(ctx, name_or_path)?;
    Ok(config::raw_show(&path)?)
}

/// Set the value at a dot-separated YAML key path in a config file.
///
/// `raw_value` is parsed as YAML (e.g. `{}`, `true`, `"hello"`).
/// Returns the path of the modified config file.
pub fn config_raw_set(
    ctx: &AppContext,
    name_or_path: Option<&str>,
    key_path: &str,
    raw_value: &str,
) -> Result<PathBuf, AppError> {
    let path = resolve_config_required(ctx, name_or_path)?;
    config::raw_set(&path, key_path, raw_value)?;
    Ok(path)
}

/// Remove the value at a dot-separated YAML key path from a config file.
///
/// Returns `(path, found)` — `found` is `false` if the key did not exist.
pub fn config_raw_unset(
    ctx: &AppContext,
    name_or_path: Option<&str>,
    key_path: &str,
) -> Result<(PathBuf, bool), AppError> {
    let path = resolve_config_required(ctx, name_or_path)?;
    let found = config::raw_unset(&path, key_path)?;
    Ok((path, found))
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Resolve config name-or-path, falling back to the active context.
fn resolve_config_required(
    ctx: &AppContext,
    name_or_path: Option<&str>,
) -> Result<PathBuf, AppError> {
    let val = match name_or_path {
        Some(n) => n.to_string(),
        None => ctx.current_context().ok_or(AppError::NoActiveContext)?,
    };
    Ok(ctx.resolve_config_path(&val))
}

/// Split a feature ID into `(source, name)`.
///
/// - `core/git` → `("core", "git")`
/// - `git`      → `("local", "git")` (bare name = local source)
fn split_feature_id(feature_id: &str) -> (String, String) {
    match feature_id.find('/') {
        Some(pos) => (
            feature_id[..pos].to_string(),
            feature_id[pos + 1..].to_string(),
        ),
        None => ("local".to_string(), feature_id.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Source mutations
// ---------------------------------------------------------------------------

/// Add a `type: git` source entry to `sources.yaml`.
///
/// If `id` is `None`, derives the source ID from the URL's last path segment
/// (stripping the `.git` suffix if present).
/// Returns the path of the modified `sources.yaml`.
pub fn source_add_git(
    ctx: &AppContext,
    url: &str,
    id: Option<&str>,
    source_ref: Option<config::SourceRef>,
    repo_path: Option<&str>,
) -> Result<PathBuf, AppError> {
    if url.is_empty() {
        return Err(config::ConfigError::InvalidSources {
            reason: "url must not be empty".to_string(),
        }
        .into());
    }

    // Ref exclusivity: at most one of branch / tag / commit.
    if let Some(ref r) = source_ref {
        let set = r.branch.is_some() as u8 + r.tag.is_some() as u8 + r.commit.is_some() as u8;
        if set > 1 {
            return Err(config::ConfigError::InvalidSources {
                reason: "at most one of branch, tag, commit may be set".to_string(),
            }
            .into());
        }
    }

    let resolved_id = id.map_or_else(|| derive_id_from_url(url), str::to_string);
    validate_source_id(ctx, &resolved_id)?;

    let entry = config::SourceEntry {
        id: resolved_id,
        source_type: config::SourceType::Git,
        url: Some(url.to_string()),
        source_ref,
        path: repo_path.map(str::to_string),
        allow: None,
    };

    let sources_path = ctx.sources_path();
    let mut spec = load_sources_spec_optional(ctx)?;
    spec.sources.push(entry);
    config::save_sources(&sources_path, &spec)?;
    Ok(sources_path)
}

/// Add a `type: path` source entry to `sources.yaml`.
///
/// `path_str` is resolved relative to `sources.yaml`'s parent directory.
/// If `id` is `None`, derives the source ID from the directory name.
/// Returns the path of the modified `sources.yaml`.
pub fn source_add_path(
    ctx: &AppContext,
    path_str: &str,
    id: Option<&str>,
) -> Result<PathBuf, AppError> {
    if path_str.is_empty() {
        return Err(config::ConfigError::InvalidSources {
            reason: "path must not be empty".to_string(),
        }
        .into());
    }

    let sources_path = ctx.sources_path();
    let resolved = config::resolve_path_relative_to_sources(path_str, &sources_path);

    if !resolved.exists() {
        return Err(config::ConfigError::InvalidSources {
            reason: format!("path does not exist: {}", resolved.display()),
        }
        .into());
    }
    if !resolved.is_dir() {
        return Err(config::ConfigError::InvalidSources {
            reason: format!("path is not a directory: {}", resolved.display()),
        }
        .into());
    }

    // Require at least one of features/ or backends/ to exist.
    if !resolved.join("features").exists() && !resolved.join("backends").exists() {
        return Err(config::ConfigError::InvalidSources {
            reason: format!(
                "neither features/ nor backends/ found under: {}",
                resolved.display()
            ),
        }
        .into());
    }

    // Reject if the path resolves to the same real directory as the implicit
    // local source root. Canonicalize resolves symlinks, so alias registrations
    // via different symlink paths are also caught.
    if let Ok(real_local) = ctx.local_root.canonicalize() {
        if let Ok(real_resolved) = resolved.canonicalize() {
            if real_resolved == real_local {
                return Err(AppError::PathSourceIsLocalRoot {
                    path: path_str.to_string(),
                });
            }
        }
    }

    let resolved_id = id.map_or_else(|| derive_id_from_path(&resolved), str::to_string);
    validate_source_id(ctx, &resolved_id)?;

    // Determine the value written to sources.yaml.
    //
    // - Absolute paths  : canonicalize via the filesystem to strip `..` components
    //   and resolve symlinks to their real target. The path is known to exist here,
    //   so canonicalize should not fail; the raw string is kept as a fallback.
    //   (Shell expansion of `~` always produces an absolute path, so this case
    //   also handles the common `~/foo` → `/home/user/foo` scenario.)
    //
    // - Relative / `~`-prefixed paths : store verbatim. The config loader
    //   re-resolves them at load time, which preserves portability and the `~`
    //   shorthand when the user deliberately prevents shell expansion (e.g. by
    //   quoting the argument).
    let stored_path = if std::path::Path::new(path_str).is_absolute() {
        resolved
            .canonicalize()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| path_str.to_string())
    } else {
        path_str.to_string()
    };

    let entry = config::SourceEntry {
        id: resolved_id,
        source_type: config::SourceType::Path,
        url: None,
        source_ref: None,
        path: Some(stored_path),
        allow: None,
    };

    let mut spec = load_sources_spec_optional(ctx)?;

    // Reject if the new path resolves to the same real directory as any existing
    // type:path entry. Canonicalize resolves `..` and symlinks on both sides.
    if let Ok(real_resolved) = resolved.canonicalize() {
        for existing in spec
            .sources
            .iter()
            .filter(|e| matches!(e.source_type, config::SourceType::Path))
        {
            if let Some(ref p) = existing.path {
                if let Ok(real_existing) = std::path::Path::new(p).canonicalize() {
                    if real_resolved == real_existing {
                        return Err(AppError::PathSourceDuplicate {
                            path: path_str.to_string(),
                            existing_id: existing.id.clone(),
                        });
                    }
                }
            }
        }
    }

    spec.sources.push(entry);
    config::save_sources(&sources_path, &spec)?;
    Ok(sources_path)
}

/// Remove a source entry from `sources.yaml`.
///
/// Without `force`, aborts if the source is still referenced in state or any
/// config file. With `force`, removes unconditionally and also cleans up the
/// corresponding lock entry.
/// Returns the path of the modified `sources.yaml`.
pub fn source_remove(ctx: &AppContext, id: &str, force: bool) -> Result<PathBuf, AppError> {
    let mut spec = load_sources_spec_optional(ctx)?;

    let pos = spec
        .sources
        .iter()
        .position(|e| e.id == id)
        .ok_or_else(|| AppError::SourceNotFound { id: id.to_string() })?;

    if !force {
        let refs = find_source_references(ctx, id);
        if !refs.is_empty() {
            return Err(AppError::SourceStillReferenced {
                id: id.to_string(),
                reason: refs.join("; "),
            });
        }
    }

    spec.sources.remove(pos);
    let sources_path = ctx.sources_path();
    config::save_sources(&sources_path, &spec)?;

    // Clean up the lock entry (type:git sources only, but harmless for others).
    let lock_path = ctx.sources_lock_path();
    let mut lock = config::load_sources_lock(&lock_path).unwrap_or_default();
    if lock.sources.remove(id).is_some() {
        let _ = config::save_sources_lock(&lock_path, &lock);
    }

    Ok(sources_path)
}

/// Grant allow-list entries for an external source.
///
/// Merges `features` and `backends` into the source's existing `allow` field.
/// At least one of `features` or `backends` must be `Some`.
/// Returns the path of the modified `sources.yaml`.
pub fn source_trust(
    ctx: &AppContext,
    id: &str,
    features: Option<config::AllowList>,
    backends: Option<config::AllowList>,
) -> Result<PathBuf, AppError> {
    let mut spec = load_sources_spec_optional(ctx)?;

    let entry = spec
        .sources
        .iter_mut()
        .find(|e| e.id == id)
        .ok_or_else(|| AppError::SourceNotFound { id: id.to_string() })?;

    // AllowSpec::All already grants everything; no change needed.
    if !matches!(entry.allow, Some(config::AllowSpec::All(_))) {
        merge_allow(&mut entry.allow, features, backends);
    }

    let sources_path = ctx.sources_path();
    config::save_sources(&sources_path, &spec)?;
    Ok(sources_path)
}

/// Revoke allow-list entries for an external source.
///
/// Passing `AllowList::All("*")` as features or backends requires `force = true`.
/// If both dimensions become empty after removal, the `allow` field is omitted
/// (deny-all state).
/// Returns the path of the modified `sources.yaml`.
pub fn source_untrust(
    ctx: &AppContext,
    id: &str,
    features: Option<config::AllowList>,
    backends: Option<config::AllowList>,
    force: bool,
) -> Result<PathBuf, AppError> {
    // Reject wildcard removal without --force.
    let wildcard_in = matches!(features, Some(config::AllowList::All(_)))
        || matches!(backends, Some(config::AllowList::All(_)));
    if wildcard_in && !force {
        return Err(AppError::UntrustWildcardRequiresForce { id: id.to_string() });
    }

    let mut spec = load_sources_spec_optional(ctx)?;

    let entry = spec
        .sources
        .iter_mut()
        .find(|e| e.id == id)
        .ok_or_else(|| AppError::SourceNotFound { id: id.to_string() })?;

    // Reject removing specific names when the current allow-list is a wildcard.
    // A wildcard grants everything; narrowing it by name is not supported.
    // The user must revoke the wildcard first (--force), then re-trust specific entries.
    if !force {
        if matches!(features, Some(config::AllowList::Names(_)))
            && is_effective_wildcard_for_features(&entry.allow)
        {
            return Err(AppError::UntrustNamesFromWildcard {
                id: id.to_string(),
                dimension: "features",
            });
        }
        if matches!(backends, Some(config::AllowList::Names(_)))
            && is_effective_wildcard_for_backends(&entry.allow)
        {
            return Err(AppError::UntrustNamesFromWildcard {
                id: id.to_string(),
                dimension: "backends",
            });
        }
    }

    apply_untrust(&mut entry.allow, features, backends, force);

    let sources_path = ctx.sources_path();
    config::save_sources(&sources_path, &spec)?;
    Ok(sources_path)
}

// ---------------------------------------------------------------------------
// Source mutation helpers
// ---------------------------------------------------------------------------

/// Load the sources spec at `ctx.sources_path()`; return empty spec if absent.
fn load_sources_spec_optional(ctx: &AppContext) -> Result<config::SourcesSpec, AppError> {
    let path = ctx.sources_path();
    if path.exists() {
        Ok(config::load_sources(&path)?)
    } else {
        Ok(config::SourcesSpec::default())
    }
}

/// Validate a candidate source ID: must not be reserved and must not already exist.
fn validate_source_id(ctx: &AppContext, id: &str) -> Result<(), AppError> {
    if RESERVED_SOURCE_IDS.contains(&id) {
        return Err(config::ConfigError::InvalidSources {
            reason: format!("source id '{id}' is reserved"),
        }
        .into());
    }
    let spec = load_sources_spec_optional(ctx)?;
    if spec.sources.iter().any(|e| e.id == id) {
        return Err(AppError::SourceAlreadyExists { id: id.to_string() });
    }
    Ok(())
}

/// Derive a source ID from a git URL by taking the last path segment (without `.git`).
///
/// `https://github.com/example/community-loadout.git` → `"community-loadout"`
fn derive_id_from_url(url: &str) -> String {
    url.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(url)
        .trim_end_matches(".git")
        .to_string()
}

/// Derive a source ID from the last component of a filesystem path.
fn derive_id_from_path(resolved: &std::path::Path) -> String {
    resolved
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("external")
        .to_string()
}

/// Scan state and all config files for references to `source_id`.
///
/// Returns a list of human-readable descriptions for each reference found.
/// An empty list means no references exist.
fn find_source_references(ctx: &AppContext, source_id: &str) -> Vec<String> {
    let prefix = format!("{source_id}/");
    let mut found = Vec::new();

    // Check state: any installed feature whose ID begins with `<id>/`.
    if let Ok(st) = state::load(&ctx.state_path()) {
        for key in st.features.keys() {
            if key.starts_with(&prefix) {
                found.push(format!("state: feature '{key}' is installed"));
                break;
            }
        }
    }

    // Check all config YAML files: profile features and strategy backend references.
    let configs_dir = ctx.dirs.config_home.join("configs");
    if configs_dir.exists() {
        if let Ok(dir_entries) = std::fs::read_dir(&configs_dir) {
            for de in dir_entries.flatten() {
                let path = de.path();
                let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                    continue;
                };
                if ext != "yaml" && ext != "yml" {
                    continue;
                }
                let Ok((profile, strategy)) = config::load_config(&path) else {
                    continue; // skip unreadable configs rather than blocking
                };
                let config_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");

                for fid in profile.features.keys() {
                    if fid.starts_with(&prefix) {
                        found.push(format!(
                            "config '{config_name}': feature '{fid}' is declared"
                        ));
                        break;
                    }
                }

                for backend_id in collect_strategy_backends(&strategy) {
                    if backend_id.starts_with(&prefix) {
                        found.push(format!(
                            "config '{config_name}': backend '{backend_id}' is referenced"
                        ));
                        break;
                    }
                }
            }
        }
    }

    found
}

/// Collect all backend IDs declared in a strategy (default_backend + all override backends).
fn collect_strategy_backends(strategy: &config::Strategy) -> Vec<String> {
    let mut result = Vec::new();
    for kind_strategy in [&strategy.package, &strategy.runtime].into_iter().flatten() {
        if let Some(ref default) = kind_strategy.default_backend {
            result.push(default.clone());
        }
        for ovr in kind_strategy.overrides.values() {
            result.push(ovr.backend.clone());
        }
    }
    result
}

/// Returns `true` if the features dimension of the allow-list is effectively a wildcard (`"*"`).
///
/// Both `AllowSpec::All` (top-level `"*"`) and a `Detailed` entry with `features: "*"` count.
fn is_effective_wildcard_for_features(allow: &Option<config::AllowSpec>) -> bool {
    match allow {
        Some(config::AllowSpec::All(_)) => true,
        Some(config::AllowSpec::Detailed(d)) => {
            matches!(d.features, Some(config::AllowList::All(_)))
        }
        None => false,
    }
}

/// Returns `true` if the backends dimension of the allow-list is effectively a wildcard (`"*"`).
fn is_effective_wildcard_for_backends(allow: &Option<config::AllowSpec>) -> bool {
    match allow {
        Some(config::AllowSpec::All(_)) => true,
        Some(config::AllowSpec::Detailed(d)) => {
            matches!(d.backends, Some(config::AllowList::All(_)))
        }
        None => false,
    }
}

/// Merge `features` and `backends` AllowLists into an existing AllowSpec.
fn merge_allow(
    allow: &mut Option<config::AllowSpec>,
    features: Option<config::AllowList>,
    backends: Option<config::AllowList>,
) {
    // AllowSpec::All already grants everything; caller should guard against this.
    let detail = match allow {
        Some(config::AllowSpec::All(_)) => return,
        Some(config::AllowSpec::Detailed(d)) => d,
        None => {
            *allow = Some(config::AllowSpec::Detailed(config::DetailedAllow {
                features: None,
                backends: None,
            }));
            match allow {
                Some(config::AllowSpec::Detailed(d)) => d,
                _ => unreachable!(),
            }
        }
    };

    if let Some(new_features) = features {
        merge_allow_list(&mut detail.features, new_features);
    }
    if let Some(new_backends) = backends {
        merge_allow_list(&mut detail.backends, new_backends);
    }
}

/// Merge a single AllowList into an existing slot.
///
/// `AllowList::All` always wins; names are deduplicated and sorted.
fn merge_allow_list(existing: &mut Option<config::AllowList>, new: config::AllowList) {
    match (existing.take(), new) {
        // New wildcard overrides anything.
        (_, config::AllowList::All(a)) => *existing = Some(config::AllowList::All(a)),
        // Existing wildcard wins over new names.
        (Some(config::AllowList::All(a)), _) => *existing = Some(config::AllowList::All(a)),
        // Merge two name lists (deduplicate, sort).
        (Some(config::AllowList::Names(mut names)), config::AllowList::Names(new_names)) => {
            for n in new_names {
                if !names.contains(&n) {
                    names.push(n);
                }
            }
            names.sort();
            *existing = Some(config::AllowList::Names(names));
        }
        // First names entry.
        (None, config::AllowList::Names(names)) => {
            *existing = Some(config::AllowList::Names(names))
        }
    }
}

/// Remove `features` and `backends` entries from an existing AllowSpec.
///
/// After removal, if both dimensions are empty the allow field is set to `None` (deny-all).
fn apply_untrust(
    allow: &mut Option<config::AllowSpec>,
    features: Option<config::AllowList>,
    backends: Option<config::AllowList>,
    force: bool,
) {
    match allow {
        // Top-level "*": with force → deny-all; without force → already guarded by caller.
        Some(config::AllowSpec::All(_)) if force => {
            *allow = None;
        }
        Some(config::AllowSpec::All(_)) => (),
        None => (), // already deny-all
        Some(config::AllowSpec::Detailed(d)) => {
            if let Some(f) = features {
                remove_from_allow_list(&mut d.features, f, force);
            }
            if let Some(b) = backends {
                remove_from_allow_list(&mut d.backends, b, force);
            }
            // Revert to deny-all when both dimensions are cleared.
            if d.features.is_none() && d.backends.is_none() {
                *allow = None;
            }
        }
    }
}

/// Remove entries from a single AllowList slot.
///
/// - `AllowList::All` as `to_remove` with `force`: clears the slot.
/// - `AllowList::All` as `to_remove` without `force`: no-op (caller guards wildcard case).
/// - Removing names from a wildcard: no-op (wildcard still grants everything).
fn remove_from_allow_list(
    existing: &mut Option<config::AllowList>,
    to_remove: config::AllowList,
    force: bool,
) {
    let current = existing.take();
    *existing = match (current, to_remove) {
        (None, _) => None,
        // Wildcard → wildcard with force: remove entirely.
        (Some(config::AllowList::All(_)), config::AllowList::All(_)) if force => None,
        // Wildcard → wildcard without force: keep (caller should have prevented this).
        (Some(config::AllowList::All(_)), config::AllowList::All(a)) => {
            Some(config::AllowList::All(a))
        }
        // Wildcard → names: no-op; wildcard supersedes all names.
        (Some(config::AllowList::All(a)), config::AllowList::Names(_)) => {
            Some(config::AllowList::All(a))
        }
        // Names → wildcard with force: remove all names.
        (Some(config::AllowList::Names(_)), config::AllowList::All(_)) if force => None,
        // Names → wildcard without force: no-op (caller should have prevented this).
        (Some(config::AllowList::Names(names)), config::AllowList::All(_)) => {
            Some(config::AllowList::Names(names))
        }
        // Names → names: filter out the specified names.
        (Some(config::AllowList::Names(mut names)), config::AllowList::Names(to_remove)) => {
            names.retain(|n| !to_remove.contains(n));
            if names.is_empty() {
                None
            } else {
                Some(config::AllowList::Names(names))
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> config::AllowList {
        config::AllowList::Names(v.iter().map(|s| s.to_string()).collect())
    }
    fn all() -> config::AllowList {
        config::AllowList::All(config::WildcardAll)
    }

    // ── derive_id_from_url ────────────────────────────────────────────────────

    #[test]
    fn derive_url_strips_git_suffix() {
        assert_eq!(
            derive_id_from_url("https://github.com/example/community-loadout.git"),
            "community-loadout"
        );
    }

    #[test]
    fn derive_url_without_git_suffix() {
        assert_eq!(
            derive_id_from_url("https://github.com/example/myrepo"),
            "myrepo"
        );
    }

    // ── merge_allow_list ────────────────────────────────────────────────────────

    #[test]
    fn merge_names_into_empty() {
        let mut slot = None;
        merge_allow_list(&mut slot, names(&["brew", "mise"]));
        assert_eq!(slot, Some(names(&["brew", "mise"])));
    }

    #[test]
    fn merge_names_deduplicates_and_sorts() {
        let mut slot = Some(names(&["mise"]));
        merge_allow_list(&mut slot, names(&["brew", "mise"]));
        assert_eq!(slot, Some(names(&["brew", "mise"])));
    }

    #[test]
    fn merge_wildcard_wins_over_names() {
        let mut slot = Some(names(&["brew"]));
        merge_allow_list(&mut slot, all());
        assert!(matches!(slot, Some(config::AllowList::All(_))));
    }

    #[test]
    fn existing_wildcard_wins_over_new_names() {
        let mut slot = Some(all());
        merge_allow_list(&mut slot, names(&["brew"]));
        assert!(matches!(slot, Some(config::AllowList::All(_))));
    }

    // ── remove_from_allow_list ─────────────────────────────────────────────────

    #[test]
    fn remove_names_leaves_remainder() {
        let mut slot = Some(names(&["brew", "mise", "npm"]));
        remove_from_allow_list(&mut slot, names(&["mise"]), false);
        assert_eq!(slot, Some(names(&["brew", "npm"])));
    }

    #[test]
    fn remove_all_names_clears_slot() {
        let mut slot = Some(names(&["brew"]));
        remove_from_allow_list(&mut slot, names(&["brew"]), false);
        assert!(slot.is_none());
    }

    #[test]
    fn remove_wildcard_with_force_clears_slot() {
        let mut slot = Some(all());
        remove_from_allow_list(&mut slot, all(), true);
        assert!(slot.is_none());
    }

    #[test]
    fn remove_names_from_wildcard_is_noop() {
        let mut slot = Some(all());
        remove_from_allow_list(&mut slot, names(&["brew"]), false);
        assert!(matches!(slot, Some(config::AllowList::All(_))));
    }

    // ── apply_untrust deny-all after removal ────────────────────────────────────

    #[test]
    fn untrust_both_dimensions_yields_deny_all() {
        let mut allow = Some(config::AllowSpec::Detailed(config::DetailedAllow {
            features: Some(names(&["node"])),
            backends: Some(names(&["mise"])),
        }));
        apply_untrust(
            &mut allow,
            Some(names(&["node"])),
            Some(names(&["mise"])),
            false,
        );
        assert!(allow.is_none(), "expected deny-all (None)");
    }

    // ── source_add_git / source_add_path integration ─────────────────────────

    #[test]
    fn source_add_git_reserved_id_rejected() {
        let tmpdir = tempfile::tempdir().unwrap();
        let ctx = fake_ctx(&tmpdir);
        let err = source_add_git(
            &ctx,
            "https://example.com/repo.git",
            Some("core"),
            None,
            None,
        );
        assert!(
            matches!(err, Err(AppError::Config(_))),
            "expected Config error for reserved id"
        );
    }

    #[test]
    fn source_add_path_nonexistent_path_rejected() {
        let tmpdir = tempfile::tempdir().unwrap();
        let ctx = fake_ctx(&tmpdir);
        let err = source_add_path(&ctx, "/nonexistent/path", None);
        assert!(
            matches!(err, Err(AppError::Config(_))),
            "expected Config error for missing path"
        );
    }

    #[test]
    fn source_add_path_equal_to_local_root_rejected() {
        let tmpdir = tempfile::tempdir().unwrap();
        let ctx = fake_ctx(&tmpdir);
        // Create features/ so the structural check passes; the local-root check comes next.
        std::fs::create_dir_all(ctx.local_root.join("features")).unwrap();
        let err = source_add_path(&ctx, ctx.local_root.to_str().unwrap(), None);
        assert!(
            matches!(err, Err(AppError::PathSourceIsLocalRoot { .. })),
            "expected PathSourceIsLocalRoot error, got {err:?}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn source_add_path_symlink_to_local_root_rejected() {
        let tmpdir = tempfile::tempdir().unwrap();
        let ctx = fake_ctx(&tmpdir);
        // Create features/ under local_root.
        std::fs::create_dir_all(ctx.local_root.join("features")).unwrap();
        // Create a symlink pointing at local_root.
        let link = tmpdir.path().join("alias");
        std::os::unix::fs::symlink(&ctx.local_root, &link).unwrap();
        let err = source_add_path(&ctx, link.to_str().unwrap(), None);
        assert!(
            matches!(err, Err(AppError::PathSourceIsLocalRoot { .. })),
            "symlink alias should also be rejected, got {err:?}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn source_add_path_duplicate_real_path_rejected() {
        let tmpdir = tempfile::tempdir().unwrap();
        let ctx = fake_ctx(&tmpdir);
        // Create external repo with features/.
        let repo = tmpdir.path().join("external");
        std::fs::create_dir_all(repo.join("features")).unwrap();
        // Register the first time.
        source_add_path(&ctx, repo.to_str().unwrap(), Some("repo")).unwrap();
        // Attempt to register the same real directory again under a symlink.
        let link = tmpdir.path().join("alias");
        std::os::unix::fs::symlink(&repo, &link).unwrap();
        let err = source_add_path(&ctx, link.to_str().unwrap(), Some("repo2"));
        assert!(
            matches!(err, Err(AppError::PathSourceDuplicate { .. })),
            "same real dir via symlink should be rejected, got {err:?}"
        );
    }

    #[test]
    #[cfg(windows)]
    fn source_add_path_symlink_to_local_root_rejected_windows() {
        let tmpdir = tempfile::tempdir().unwrap();
        let ctx = fake_ctx(&tmpdir);
        // Create features/ under local_root.
        std::fs::create_dir_all(ctx.local_root.join("features")).unwrap();
        // Create a directory symlink pointing at local_root.
        let link = tmpdir.path().join("alias");
        match std::os::windows::fs::symlink_dir(&ctx.local_root, &link) {
            // 1314 = ERROR_PRIVILEGE_NOT_HELD; requires Developer Mode or elevated prompt
            Err(e) if e.raw_os_error() == Some(1314) => return,
            r => r.unwrap(),
        }
        let err = source_add_path(&ctx, link.to_str().unwrap(), None);
        assert!(
            matches!(err, Err(AppError::PathSourceIsLocalRoot { .. })),
            "symlink alias should also be rejected, got {err:?}"
        );
    }

    #[test]
    #[cfg(windows)]
    fn source_add_path_duplicate_real_path_rejected_windows() {
        let tmpdir = tempfile::tempdir().unwrap();
        let ctx = fake_ctx(&tmpdir);
        // Create external repo with features/.
        let repo = tmpdir.path().join("external");
        std::fs::create_dir_all(repo.join("features")).unwrap();
        // Register the first time.
        source_add_path(&ctx, repo.to_str().unwrap(), Some("repo")).unwrap();
        // Attempt to register the same real directory again under a symlink.
        let link = tmpdir.path().join("alias");
        match std::os::windows::fs::symlink_dir(&repo, &link) {
            // 1314 = ERROR_PRIVILEGE_NOT_HELD; requires Developer Mode or elevated prompt
            Err(e) if e.raw_os_error() == Some(1314) => return,
            r => r.unwrap(),
        }
        let err = source_add_path(&ctx, link.to_str().unwrap(), Some("repo2"));
        assert!(
            matches!(err, Err(AppError::PathSourceDuplicate { .. })),
            "same real dir via symlink should be rejected, got {err:?}"
        );
    }

    #[test]
    fn source_add_path_duplicate_dotdot_path_rejected() {
        let tmpdir = tempfile::tempdir().unwrap();
        let ctx = fake_ctx(&tmpdir);
        // Create external repo with features/.
        let repo = tmpdir.path().join("external");
        std::fs::create_dir_all(repo.join("features")).unwrap();
        // Register the first time.
        source_add_path(&ctx, repo.to_str().unwrap(), Some("repo")).unwrap();
        // Build a path that resolves to the same directory via `..`.
        let dotdot = repo
            .join("..")
            .join(repo.file_name().unwrap_or_default())
            .display()
            .to_string();
        let err = source_add_path(&ctx, &dotdot, Some("repo2"));
        assert!(
            matches!(err, Err(AppError::PathSourceDuplicate { .. })),
            "same real dir via '..' path should be rejected, got {err:?}"
        );
    }

    #[test]
    fn source_add_git_duplicate_id_rejected() {
        let tmpdir = tempfile::tempdir().unwrap();
        let ctx = fake_ctx(&tmpdir);
        // First add.
        source_add_git(
            &ctx,
            "https://example.com/repo.git",
            Some("myrepo"),
            None,
            None,
        )
        .unwrap();
        // Second add with same ID.
        let err = source_add_git(
            &ctx,
            "https://example.com/other.git",
            Some("myrepo"),
            None,
            None,
        );
        assert!(matches!(err, Err(AppError::SourceAlreadyExists { .. })));
    }

    #[test]
    fn source_add_git_derives_id_from_url() {
        let tmpdir = tempfile::tempdir().unwrap();
        let ctx = fake_ctx(&tmpdir);
        let path =
            source_add_git(&ctx, "https://example.com/community.git", None, None, None).unwrap();
        let spec = config::load_sources(&path).unwrap();
        assert_eq!(spec.sources[0].id, "community");
    }

    #[test]
    fn source_remove_nonexistent_returns_not_found() {
        let tmpdir = tempfile::tempdir().unwrap();
        let ctx = fake_ctx(&tmpdir);
        let err = source_remove(&ctx, "missing", false);
        assert!(matches!(err, Err(AppError::SourceNotFound { .. })));
    }

    #[test]
    fn source_remove_force_removes_entry() {
        let tmpdir = tempfile::tempdir().unwrap();
        let ctx = fake_ctx(&tmpdir);
        source_add_git(
            &ctx,
            "https://example.com/repo.git",
            Some("todelete"),
            None,
            None,
        )
        .unwrap();
        let path = source_remove(&ctx, "todelete", true).unwrap();
        let spec = config::load_sources(&path).unwrap();
        assert!(spec.sources.iter().all(|e| e.id != "todelete"));
    }

    #[test]
    fn source_trust_adds_features() {
        let tmpdir = tempfile::tempdir().unwrap();
        let ctx = fake_ctx(&tmpdir);
        source_add_git(
            &ctx,
            "https://example.com/repo.git",
            Some("myrepo"),
            None,
            None,
        )
        .unwrap();
        let path = source_trust(&ctx, "myrepo", Some(names(&["node", "python"])), None).unwrap();
        let spec = config::load_sources(&path).unwrap();
        let entry = spec.sources.iter().find(|e| e.id == "myrepo").unwrap();
        assert!(matches!(
            &entry.allow,
            Some(config::AllowSpec::Detailed(d)) if matches!(&d.features, Some(config::AllowList::Names(v)) if v.contains(&"node".to_string()))
        ));
    }

    #[test]
    fn source_untrust_wildcard_without_force_rejected() {
        let tmpdir = tempfile::tempdir().unwrap();
        let ctx = fake_ctx(&tmpdir);
        source_add_git(
            &ctx,
            "https://example.com/repo.git",
            Some("myrepo"),
            None,
            None,
        )
        .unwrap();
        let err = source_untrust(&ctx, "myrepo", Some(all()), None, false);
        assert!(matches!(
            err,
            Err(AppError::UntrustWildcardRequiresForce { .. })
        ));
    }

    #[test]
    fn source_untrust_names_from_detailed_wildcard_features_rejected() {
        let tmpdir = tempfile::tempdir().unwrap();
        let ctx = fake_ctx(&tmpdir);
        source_add_git(
            &ctx,
            "https://example.com/repo.git",
            Some("myrepo"),
            None,
            None,
        )
        .unwrap();
        // Trust with wildcard on features dimension.
        source_trust(&ctx, "myrepo", Some(all()), None).unwrap();
        // Attempting to untrust specific names should be rejected.
        let err = source_untrust(&ctx, "myrepo", Some(names(&["node"])), None, false);
        assert!(
            matches!(
                err,
                Err(AppError::UntrustNamesFromWildcard { dimension, .. }) if dimension == "features"
            ),
            "expected UntrustNamesFromWildcard for features, got {err:?}"
        );
    }

    #[test]
    fn source_untrust_names_from_detailed_wildcard_backends_rejected() {
        let tmpdir = tempfile::tempdir().unwrap();
        let ctx = fake_ctx(&tmpdir);
        source_add_git(
            &ctx,
            "https://example.com/repo.git",
            Some("myrepo"),
            None,
            None,
        )
        .unwrap();
        // Trust with wildcard on backends dimension.
        source_trust(&ctx, "myrepo", None, Some(all())).unwrap();
        // Attempting to untrust specific backends should be rejected.
        let err = source_untrust(&ctx, "myrepo", None, Some(names(&["brew"])), false);
        assert!(
            matches!(
                err,
                Err(AppError::UntrustNamesFromWildcard { dimension, .. }) if dimension == "backends"
            ),
            "expected UntrustNamesFromWildcard for backends, got {err:?}"
        );
    }

    #[test]
    fn source_untrust_names_from_wildcard_with_force_succeeds() {
        // With --force, the wildcard guard is bypassed entirely (wildcard itself is removed).
        let tmpdir = tempfile::tempdir().unwrap();
        let ctx = fake_ctx(&tmpdir);
        source_add_git(
            &ctx,
            "https://example.com/repo.git",
            Some("myrepo"),
            None,
            None,
        )
        .unwrap();
        source_trust(&ctx, "myrepo", Some(all()), None).unwrap();
        let path = source_untrust(&ctx, "myrepo", Some(names(&["node"])), None, true).unwrap();
        // With force, remove_from_allow_list's wildcard→names branch keeps the wildcard
        // (no-op for names removal), but the wildcard guard is skipped here.
        // This verifies no panic and the call succeeds (wildcard remains).
        let spec = config::load_sources(&path).unwrap();
        let entry = spec.sources.iter().find(|e| e.id == "myrepo").unwrap();
        // The wildcard was not cleared by removing a name (wildcard supersedes names).
        assert!(
            matches!(
                &entry.allow,
                Some(config::AllowSpec::Detailed(d))
                    if matches!(d.features, Some(config::AllowList::All(_)))
            ),
            "wildcard should still be present after force-remove of a name: {:?}",
            entry.allow
        );
    }

    /// Build a minimal `AppContext` rooted in `tmpdir`.
    fn fake_ctx(tmpdir: &tempfile::TempDir) -> AppContext {
        let root = tmpdir.path().to_path_buf();
        let dirs = platform::Dirs {
            config_home: root.join("config"),
            data_home: root.join("data"),
            cache_home: root.join("cache"),
            state_home: root.join("state"),
        };
        AppContext::new(platform::Platform::Linux, dirs)
    }
}
