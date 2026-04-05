// Config mutation use cases.
//
// These use cases modify config files on behalf of the CLI. Each function:
//   1. Resolves the target config path (from explicit arg or active context).
//   2. Delegates the actual file mutation to the `config` crate.
//   3. Returns the path of the file that was modified.

use std::path::PathBuf;

use crate::context::{AppContext, AppError};

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
