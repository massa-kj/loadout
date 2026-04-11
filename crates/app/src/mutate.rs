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
// Config component mutations
// ---------------------------------------------------------------------------

/// Add a component to a config file's `profile.components` section.
///
/// `component_id` may be canonical (`source/name`) or a bare name (resolved to
/// `local/<name>`). If `name_or_path` is `None`, the active context is used.
/// Returns the path of the modified config file.
pub fn config_component_add(
    ctx: &AppContext,
    name_or_path: Option<&str>,
    component_id: &str,
) -> Result<PathBuf, AppError> {
    let path = resolve_config_required(ctx, name_or_path)?;
    let (source, name) = split_component_id(component_id);
    config::add_component(&path, &source, &name)?;
    Ok(path)
}

/// Remove a component from a config file's `profile.components` section.
///
/// Returns `(path, found)` — `found` is `false` if the component was not present
/// and no change was made.
pub fn config_component_remove(
    ctx: &AppContext,
    name_or_path: Option<&str>,
    component_id: &str,
) -> Result<(PathBuf, bool), AppError> {
    let path = resolve_config_required(ctx, name_or_path)?;
    let (source, name) = split_component_id(component_id);
    let found = config::remove_component(&path, &source, &name)?;
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

/// Split a component ID into `(source, name)`.
///
/// - `core/git` → `("core", "git")`
/// - `git`      → `("local", "git")` (bare name = local source)
fn split_component_id(component_id: &str) -> (String, String) {
    match component_id.find('/') {
        Some(pos) => (
            component_id[..pos].to_string(),
            component_id[pos + 1..].to_string(),
        ),
        None => ("local".to_string(), component_id.to_string()),
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

    // Require at least one of components/ or backends/ to exist.
    if !resolved.join("components").exists() && !resolved.join("backends").exists() {
        return Err(config::ConfigError::InvalidSources {
            reason: format!(
                "neither components/ nor backends/ found under: {}",
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
/// Merges `components` and `backends` into the source's existing `allow` field.
/// At least one of `components` or `backends` must be `Some``.
/// Returns the path of the modified `sources.yaml`.
pub fn source_trust(
    ctx: &AppContext,
    id: &str,
    components: Option<config::AllowList>,
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
        merge_allow(&mut entry.allow, components, backends);
    }

    let sources_path = ctx.sources_path();
    config::save_sources(&sources_path, &spec)?;
    Ok(sources_path)
}

/// Revoke allow-list entries for an external source.
///
/// Passing `AllowList::All("*")` as components or backends requires `force = true`.
/// If both dimensions become empty after removal, the `allow` field is omitted
/// (deny-all state).
/// Returns the path of the modified `sources.yaml`.
pub fn source_untrust(
    ctx: &AppContext,
    id: &str,
    components: Option<config::AllowList>,
    backends: Option<config::AllowList>,
    force: bool,
) -> Result<PathBuf, AppError> {
    // Reject wildcard removal without --force.
    let wildcard_in = matches!(components, Some(config::AllowList::All(_)))
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
        if matches!(components, Some(config::AllowList::Names(_)))
            && is_effective_wildcard_for_components(&entry.allow)
        {
            return Err(AppError::UntrustNamesFromWildcard {
                id: id.to_string(),
                dimension: "components",
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

    apply_untrust(&mut entry.allow, components, backends, force);

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

    // Check state: any installed component whose ID begins with `<id>/`.
    if let Ok(st) = state::load(&ctx.state_path()) {
        for key in st.components.keys() {
            if key.starts_with(&prefix) {
                found.push(format!("state: component '{key}' is installed"));
                break;
            }
        }
    }

    // Check all config YAML files: profile components and strategy backend references.
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

                for fid in profile.components.keys() {
                    if fid.starts_with(&prefix) {
                        found.push(format!(
                            "config '{config_name}': component '{fid}' is declared"
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

/// Returns `true` if the components dimension of the allow-list is effectively a wildcard (`"*"`).
///
/// Both `AllowSpec::All` (top-level `"*"`) and a `Detailed` entry with `components: "*"` count.
fn is_effective_wildcard_for_components(allow: &Option<config::AllowSpec>) -> bool {
    match allow {
        Some(config::AllowSpec::All(_)) => true,
        Some(config::AllowSpec::Detailed(d)) => {
            matches!(d.components, Some(config::AllowList::All(_)))
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

/// Merge `components` and `backends` AllowLists into an existing AllowSpec.
fn merge_allow(
    allow: &mut Option<config::AllowSpec>,
    components: Option<config::AllowList>,
    backends: Option<config::AllowList>,
) {
    // AllowSpec::All already grants everything; caller should guard against this.
    let detail = match allow {
        Some(config::AllowSpec::All(_)) => return,
        Some(config::AllowSpec::Detailed(d)) => d,
        None => {
            *allow = Some(config::AllowSpec::Detailed(config::DetailedAllow {
                components: None,
                backends: None,
            }));
            match allow {
                Some(config::AllowSpec::Detailed(d)) => d,
                _ => unreachable!(),
            }
        }
    };

    if let Some(new_components) = components {
        merge_allow_list(&mut detail.components, new_components);
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

/// Remove `components` and `backends` entries from an existing AllowSpec.
///
/// After removal, if both dimensions are empty the allow field is set to `None` (deny-all).
fn apply_untrust(
    allow: &mut Option<config::AllowSpec>,
    components: Option<config::AllowList>,
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
            if let Some(f) = components {
                remove_from_allow_list(&mut d.components, f, force);
            }
            if let Some(b) = backends {
                remove_from_allow_list(&mut d.backends, b, force);
            }
            // Revert to deny-all when both dimensions are cleared.
            if d.components.is_none() && d.backends.is_none() {
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
// source update
// ---------------------------------------------------------------------------

/// Update a `type: git` external source: fetch, checkout, and rewrite the lock file.
///
/// Modes:
/// - Default (`to_commit = None`, `relock = false`): fetch latest and checkout the
///   ref declared in `sources.yaml` (e.g. tip of a branch).
/// - `--to-commit <hash>`: fetch, then checkout the specified commit hash.
/// - `--relock`: skip fetch/checkout; only recompute `manifest_hash` and update
///   the lock entry. Useful when the working tree is already at the desired state.
///
/// Returns `()` on success. The only file mutated is `sources.lock.yaml`.
pub fn source_update(
    ctx: &AppContext,
    id: &str,
    to_commit: Option<&str>,
    relock: bool,
) -> Result<(), AppError> {
    let spec = load_sources_spec_optional(ctx)?;

    let entry = spec
        .sources
        .iter()
        .find(|e| e.id == id)
        .ok_or_else(|| AppError::SourceNotFound { id: id.to_string() })?;

    // type: path sources cannot be updated via git.
    if matches!(entry.source_type, config::SourceType::Path) {
        return Err(AppError::GitSourceRequired { id: id.to_string() });
    }

    let repo_dir = ctx.dirs.data_home.join("sources").join(id);

    if !relock {
        // Fetch from remote.
        git_run(id, &["fetch", "--prune"], &repo_dir)?;

        // Determine the target ref to checkout.
        let target = if let Some(commit) = to_commit {
            commit.to_string()
        } else {
            // Resolve from the declared ref in sources.yaml.
            match &entry.source_ref {
                Some(r) if r.branch.is_some() => {
                    format!("origin/{}", r.branch.as_deref().unwrap())
                }
                Some(r) if r.tag.is_some() => r.tag.clone().unwrap(),
                Some(r) if r.commit.is_some() => r.commit.clone().unwrap(),
                _ => {
                    // No ref declared; resolve HEAD of the tracked remote.
                    git_rev_parse(id, "FETCH_HEAD", &repo_dir)?
                }
            }
        };

        // Detach HEAD at the resolved target.
        git_run(id, &["checkout", "--detach", &target], &repo_dir)?;
    }

    // Resolve the current HEAD to a full commit hash.
    let resolved_commit = git_rev_parse(id, "HEAD", &repo_dir)?;

    // Compute manifest_hash over components/**/*.yaml and backends/**/*.yaml.
    let manifest_hash = compute_manifest_hash(id, &repo_dir, entry)?;

    // Update the lock file.
    let lock_path = ctx.sources_lock_path();
    let mut lock = config::load_sources_lock(&lock_path).unwrap_or_default();

    lock.sources.insert(
        id.to_string(),
        config::SourceLockEntry {
            resolved_commit,
            fetched_at: utc_now_rfc3339(),
            manifest_hash,
        },
    );

    config::save_sources_lock(&lock_path, &lock)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// git helpers
// ---------------------------------------------------------------------------

/// Run a git sub-command inside `repo_dir`. Returns `Ok(())` on exit code 0,
/// or `AppError::GitCommandFailed` with captured stderr on failure.
fn git_run(source_id: &str, args: &[&str], repo_dir: &std::path::Path) -> Result<(), AppError> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo_dir)
        .output()
        .map_err(|e| AppError::GitCommandFailed {
            source_id: source_id.to_string(),
            stderr: format!("failed to spawn git: {e}"),
        })?;

    if output.status.success() {
        Ok(())
    } else {
        Err(AppError::GitCommandFailed {
            source_id: source_id.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Run `git rev-parse <rev>` and return the trimmed output as a full commit hash.
fn git_rev_parse(
    source_id: &str,
    rev: &str,
    repo_dir: &std::path::Path,
) -> Result<String, AppError> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", rev])
        .current_dir(repo_dir)
        .output()
        .map_err(|e| AppError::GitCommandFailed {
            source_id: source_id.to_string(),
            stderr: format!("failed to spawn git: {e}"),
        })?;

    if !output.status.success() {
        return Err(AppError::GitCommandFailed {
            source_id: source_id.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Compute a `sha256:<hex>` manifest hash over all `*.yaml` files found under
/// `components/` and `backends/` within the source's subtree (as declared by
/// `entry.path`). Files are sorted by relative path for determinism.
fn compute_manifest_hash(
    source_id: &str,
    repo_dir: &std::path::Path,
    entry: &config::SourceEntry,
) -> Result<String, AppError> {
    use std::io::Read;

    // The repo subtree path declared in the source entry (default ".").
    let subtree = entry
        .path
        .as_deref()
        .filter(|p| !p.is_empty())
        .unwrap_or(".");
    let root = repo_dir.join(subtree);

    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    for subdir in ["components", "backends"] {
        let dir = root.join(subdir);
        if !dir.exists() {
            continue;
        }
        collect_yaml_files(&dir, &mut paths);
    }
    paths.sort();

    // Build a SHA-256 over pair of (relative path bytes, file content).
    let mut hasher = Sha256::new();
    for abs_path in &paths {
        let rel = abs_path
            .strip_prefix(repo_dir)
            .unwrap_or(abs_path)
            .to_string_lossy();
        hasher.update(rel.as_bytes());
        hasher.update(b"\0");

        let mut f = std::fs::File::open(abs_path).map_err(|e| AppError::GitCommandFailed {
            source_id: source_id.to_string(),
            stderr: format!("failed to read manifest file {}: {e}", abs_path.display()),
        })?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)
            .map_err(|e| AppError::GitCommandFailed {
                source_id: source_id.to_string(),
                stderr: format!("failed to read manifest file {}: {e}", abs_path.display()),
            })?;
        hasher.update(&buf);
        hasher.update(b"\0");
    }

    Ok(format!("sha256:{}", hex_encode(hasher.finalize())))
}

/// Recursively collect `*.yaml` files under `dir` into `out`.
fn collect_yaml_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    children.sort();
    for path in children {
        if path.is_dir() {
            collect_yaml_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
            out.push(path);
        }
    }
}

/// Minimal SHA-256 state machine (no external dependency).
///
/// Implements FIPS 180-4 SHA-256.
struct Sha256 {
    h: [u32; 8],
    buf: [u8; 64],
    buf_len: usize,
    total_len: u64,
}

impl Sha256 {
    #[allow(clippy::unreadable_literal)]
    fn new() -> Self {
        Self {
            h: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buf: [0u8; 64],
            buf_len: 0,
            total_len: 0,
        }
    }

    fn update(&mut self, data: &[u8]) {
        let mut offset = 0;
        while offset < data.len() {
            let to_copy = (64 - self.buf_len).min(data.len() - offset);
            self.buf[self.buf_len..self.buf_len + to_copy]
                .copy_from_slice(&data[offset..offset + to_copy]);
            self.buf_len += to_copy;
            offset += to_copy;
            self.total_len += to_copy as u64;
            if self.buf_len == 64 {
                let block = self.buf;
                Self::compress(&mut self.h, &block);
                self.buf_len = 0;
            }
        }
    }

    fn finalize(mut self) -> [u8; 32] {
        // Padding
        let bit_len = self.total_len * 8;
        self.update(&[0x80]);
        while self.buf_len != 56 {
            self.update(&[0x00]);
        }
        self.update(&bit_len.to_be_bytes());

        let mut out = [0u8; 32];
        for (i, &word) in self.h.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    #[allow(clippy::unreadable_literal)]
    fn compress(h: &mut [u32; 8], block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];

        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(block[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = *h;

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
}

/// Encode a byte slice as a lowercase hex string.
fn hex_encode(bytes: [u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Return the current UTC time as an RFC 3339 string (`YYYY-MM-DDTHH:MM:SSZ`).
///
/// Uses only `std` to avoid adding external time dependencies.
fn utc_now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    secs_to_rfc3339(secs)
}

/// Convert a Unix timestamp (seconds since epoch) to `YYYY-MM-DDTHH:MM:SSZ`.
fn secs_to_rfc3339(secs: u64) -> String {
    // Days since 1970-01-01
    let days = secs / 86400;
    let time = secs % 86400;
    let h = time / 3600;
    let m = (time % 3600) / 60;
    let s = time % 60;

    // Gregorian calendar calculation from Julian Day Number.
    let jd = days + 2440588; // 2440588 = Julian day of 1970-01-01
    let a = jd + 32044;
    let b = (4 * a + 3) / 146097;
    let c = a - (146097 * b) / 4;
    let d = (4 * c + 3) / 1461;
    let e = c - (1461 * d) / 4;
    let m_raw = (5 * e + 2) / 153;

    let day = e - (153 * m_raw + 2) / 5 + 1;
    let month = m_raw + 3 - 12 * (m_raw / 10);
    let year = 100 * b + d - 4800 + m_raw / 10;

    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}Z")
}

// ---------------------------------------------------------------------------
// Import result
// ---------------------------------------------------------------------------

/// Summary returned by `component_import` and `backend_import`.
pub struct ImportReport {
    /// Source directory that was (or would be) copied from.
    pub source_dir: PathBuf,
    /// Destination directory that was (or would be) copied to.
    pub dest_dir: PathBuf,
    /// Config files that were (or would be) rewritten.
    pub config_files_updated: Vec<PathBuf>,
    /// Bare depends found in the imported component (warnings only).
    pub bare_depends_warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// component import
// ---------------------------------------------------------------------------

/// Copy a component from an external source into the `local` source directory.
///
/// - `canonical_id` must be `<source_id>/<component_name>` (external source only).
/// - `move_config`: also rewrite all config files to reference `local/<name>`.
/// - `dry_run`: compute what would happen but do not write any files.
pub fn component_import(
    ctx: &AppContext,
    canonical_id: &str,
    move_config: bool,
    dry_run: bool,
) -> Result<ImportReport, AppError> {
    let (source_id, name) = split_canonical_id(canonical_id)?;

    // Reject implicit sources (local, core).
    match source_id {
        "local" => {
            return Err(AppError::NotImportable {
                id: source_id.to_string(),
                kind: "local",
            })
        }
        "core" => {
            return Err(AppError::NotImportable {
                id: source_id.to_string(),
                kind: "core",
            })
        }
        _ => {}
    }

    // Load sources spec and find the entry.
    let spec = load_sources_spec_optional(ctx)?;
    let entry = spec
        .sources
        .iter()
        .find(|e| e.id == source_id)
        .ok_or_else(|| AppError::SourceNotFound {
            id: source_id.to_string(),
        })?;

    let source_dir = resolve_external_component_dir(ctx, entry, name);
    if !source_dir.exists() {
        return Err(AppError::ComponentNotFound {
            id: canonical_id.to_string(),
        });
    }

    let dest_dir = ctx.local_root.join("components").join(name);
    if dest_dir.exists() {
        return Err(AppError::ImportDestinationExists {
            path: dest_dir.clone(),
        });
    }

    // Collect bare-depend warnings before writing anything.
    let bare_depends_warnings = read_bare_depends(&source_dir);

    if dry_run {
        let config_files_updated = if move_config {
            find_configs_with_component(ctx, source_id, name)
        } else {
            vec![]
        };
        return Ok(ImportReport {
            source_dir,
            dest_dir,
            config_files_updated,
            bare_depends_warnings,
        });
    }

    // Copy the component directory to local.
    copy_dir_recursive(&source_dir, &dest_dir)?;

    // Optionally rewrite config references.
    let mut config_files_updated = vec![];
    if move_config {
        for cfg_path in list_config_files(ctx) {
            if config::rewrite_component_source(&cfg_path, source_id, name, "local")? {
                config_files_updated.push(cfg_path);
            }
        }
    }

    Ok(ImportReport {
        source_dir,
        dest_dir,
        config_files_updated,
        bare_depends_warnings,
    })
}

// ---------------------------------------------------------------------------
// backend import
// ---------------------------------------------------------------------------

/// Copy a backend from an external source into the `local` source directory.
///
/// - `canonical_id` must be `<source_id>/<backend_name>` (external source only).
/// - `move_strategy`: also rewrite strategy sections to reference `local/<name>`.
/// - `dry_run`: compute what would happen but do not write any files.
pub fn backend_import(
    ctx: &AppContext,
    canonical_id: &str,
    move_strategy: bool,
    dry_run: bool,
) -> Result<ImportReport, AppError> {
    let (source_id, name) = split_canonical_id(canonical_id)?;

    match source_id {
        "local" => {
            return Err(AppError::NotImportable {
                id: source_id.to_string(),
                kind: "local",
            })
        }
        "core" => {
            return Err(AppError::NotImportable {
                id: source_id.to_string(),
                kind: "core",
            })
        }
        _ => {}
    }

    let spec = load_sources_spec_optional(ctx)?;
    let entry = spec
        .sources
        .iter()
        .find(|e| e.id == source_id)
        .ok_or_else(|| AppError::SourceNotFound {
            id: source_id.to_string(),
        })?;

    let source_dir = resolve_external_backend_dir(ctx, entry, name);
    if !source_dir.exists() {
        return Err(AppError::BackendNotFound {
            id: canonical_id.to_string(),
        });
    }

    let dest_dir = ctx.local_root.join("backends").join(name);
    if dest_dir.exists() {
        return Err(AppError::ImportDestinationExists {
            path: dest_dir.clone(),
        });
    }

    if dry_run {
        let config_files_updated = if move_strategy {
            find_configs_with_backend(ctx, source_id, name)
        } else {
            vec![]
        };
        return Ok(ImportReport {
            source_dir,
            dest_dir,
            config_files_updated,
            bare_depends_warnings: vec![],
        });
    }

    copy_dir_recursive(&source_dir, &dest_dir)?;

    let mut config_files_updated = vec![];
    if move_strategy {
        for cfg_path in list_config_files(ctx) {
            if config::rewrite_backend_source(&cfg_path, source_id, name, "local")? {
                config_files_updated.push(cfg_path);
            }
        }
    }

    Ok(ImportReport {
        source_dir,
        dest_dir,
        config_files_updated,
        bare_depends_warnings: vec![],
    })
}

// ---------------------------------------------------------------------------
// Import helpers
// ---------------------------------------------------------------------------

/// Split a canonical ID `source/name` into `(source_id, name)`.
///
/// Returns an error if the string is not in `<source>/<name>` form.
fn split_canonical_id(canonical_id: &str) -> Result<(&str, &str), AppError> {
    let mut parts = canonical_id.splitn(2, '/');
    let source_id = parts.next().unwrap_or("");
    let name = parts.next().unwrap_or("");
    if source_id.is_empty() || name.is_empty() || name.contains('/') {
        return Err(config::ConfigError::InvalidSources {
            reason: format!("invalid canonical ID '{canonical_id}': expected '<source>/<name>'"),
        }
        .into());
    }
    Ok((source_id, name))
}

/// Resolve the filesystem directory for a component in an external source.
fn resolve_external_component_dir(
    ctx: &AppContext,
    entry: &config::SourceEntry,
    name: &str,
) -> PathBuf {
    match entry.source_type {
        config::SourceType::Git => ctx
            .dirs
            .data_home
            .join("sources")
            .join(&entry.id)
            .join("components")
            .join(name),
        config::SourceType::Path => {
            let base = entry.path.as_deref().unwrap_or("");
            std::path::Path::new(base).join("components").join(name)
        }
    }
}

/// Resolve the filesystem directory for a backend in an external source.
fn resolve_external_backend_dir(
    ctx: &AppContext,
    entry: &config::SourceEntry,
    name: &str,
) -> PathBuf {
    match entry.source_type {
        config::SourceType::Git => ctx
            .dirs
            .data_home
            .join("sources")
            .join(&entry.id)
            .join("backends")
            .join(name),
        config::SourceType::Path => {
            let base = entry.path.as_deref().unwrap_or("");
            std::path::Path::new(base).join("backends").join(name)
        }
    }
}

/// Recursively copy a directory from `src` to `dst`, skipping `.git` subdirectories.
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<(), AppError> {
    std::fs::create_dir_all(dst).map_err(|e| AppError::ScaffoldIo {
        path: dst.to_path_buf(),
        source: e,
    })?;
    for entry in std::fs::read_dir(src).map_err(|e| AppError::ScaffoldIo {
        path: src.to_path_buf(),
        source: e,
    })? {
        let entry = entry.map_err(|e| AppError::ScaffoldIo {
            path: src.to_path_buf(),
            source: e,
        })?;
        let file_type = entry.file_type().map_err(|e| AppError::ScaffoldIo {
            path: entry.path(),
            source: e,
        })?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            if entry.file_name() == ".git" {
                continue;
            }
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path).map_err(|e| AppError::ScaffoldIo {
                path: src_path.clone(),
                source: e,
            })?;
        }
    }
    Ok(())
}

/// List all `*.yaml` config files in `{config_home}/configs/`.
fn list_config_files(ctx: &AppContext) -> Vec<PathBuf> {
    let configs_dir = ctx.dirs.config_home.join("configs");
    let Ok(entries) = std::fs::read_dir(&configs_dir) else {
        return vec![];
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("yaml"))
        .collect()
}

/// Find config files that are likely to reference `source_id/name` in components sections.
///
/// Used for `--dry-run` output. Uses a fast text search heuristic.
fn find_configs_with_component(ctx: &AppContext, source_id: &str, name: &str) -> Vec<PathBuf> {
    list_config_files(ctx)
        .into_iter()
        .filter(|p| {
            std::fs::read_to_string(p)
                .map(|content| content.contains(source_id) && content.contains(name))
                .unwrap_or(false)
        })
        .collect()
}

/// Find config files that are likely to reference `source_id/name` in strategy sections.
///
/// Used for `--dry-run` output. Uses a fast text search heuristic.
fn find_configs_with_backend(ctx: &AppContext, source_id: &str, name: &str) -> Vec<PathBuf> {
    let canonical = format!("{source_id}/{name}");
    list_config_files(ctx)
        .into_iter()
        .filter(|p| {
            std::fs::read_to_string(p)
                .map(|content| content.contains(&canonical))
                .unwrap_or(false)
        })
        .collect()
}

/// Read `component.yaml` and return any bare depends (depends entries without a `/` prefix).
///
/// Returns an empty vec if the file is absent or cannot be parsed.
fn read_bare_depends(component_dir: &std::path::Path) -> Vec<String> {
    let component_yaml_path = component_dir.join("component.yaml");
    let Ok(content) = std::fs::read_to_string(&component_yaml_path) else {
        return vec![];
    };

    // Minimal serde parse to extract dep.depends without pulling in the full component schema.
    #[derive(serde::Deserialize, Default)]
    struct DepSection {
        #[serde(default)]
        depends: Vec<String>,
    }
    #[derive(serde::Deserialize, Default)]
    struct MinimalComponent {
        #[serde(default)]
        dep: Option<DepSection>,
    }

    let Ok(f): Result<MinimalComponent, _> = serde_yaml::from_str(&content) else {
        return vec![];
    };
    f.dep
        .unwrap_or_default()
        .depends
        .into_iter()
        .filter(|d| !d.contains('/'))
        .collect()
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
            components: Some(names(&["node"])),
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
        // Create components/ so the structural check passes; the local-root check comes next.
        std::fs::create_dir_all(ctx.local_root.join("components")).unwrap();
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
        // Create components/ under local_root.
        std::fs::create_dir_all(ctx.local_root.join("components")).unwrap();
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
        // Create external repo with components/.
        let repo = tmpdir.path().join("external");
        std::fs::create_dir_all(repo.join("components")).unwrap();
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
        // Create components/ under local_root.
        std::fs::create_dir_all(ctx.local_root.join("components")).unwrap();
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
        // Create external repo with components/.
        let repo = tmpdir.path().join("external");
        std::fs::create_dir_all(repo.join("components")).unwrap();
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
        // Create external repo with components/.
        let repo = tmpdir.path().join("external");
        std::fs::create_dir_all(repo.join("components")).unwrap();
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
    fn source_trust_adds_components() {
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
            Some(config::AllowSpec::Detailed(d)) if matches!(&d.components, Some(config::AllowList::Names(v)) if v.contains(&"node".to_string()))
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
    fn source_untrust_names_from_detailed_wildcard_components_rejected() {
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
        // Trust with wildcard on components dimension.
        source_trust(&ctx, "myrepo", Some(all()), None).unwrap();
        // Attempting to untrust specific names should be rejected.
        let err = source_untrust(&ctx, "myrepo", Some(names(&["node"])), None, false);
        assert!(
            matches!(
                err,
                Err(AppError::UntrustNamesFromWildcard { dimension, .. }) if dimension == "components"
            ),
            "expected UntrustNamesFromWildcard for components, got {err:?}"
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
                    if matches!(d.components, Some(config::AllowList::All(_)))
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
