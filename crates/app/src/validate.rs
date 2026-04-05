// Validation use cases: check feature/backend directories for correctness.
//
// Each validate function:
//   1. Loads the target via the normal read path (catches YAML/schema errors).
//   2. Runs supplementary checks not covered by the index builder:
//      - Script file presence (script mode features / required backend scripts)
//      - Resource ID uniqueness within a single feature
//      - Depends entries resolved against the full feature index

use std::collections::HashSet;
use std::path::PathBuf;

use crate::context::{AppContext, AppError};
use crate::pipeline::{build_source_roots, load_sources_optional, to_fi_platform};

// ── Output types ─────────────────────────────────────────────────────────────

/// Severity level of a single validation issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueLevel {
    Error,
    Warning,
}

/// A single validation issue found during feature or backend validation.
#[derive(Debug)]
pub struct ValidationIssue {
    pub level: IssueLevel,
    pub message: String,
}

impl ValidationIssue {
    fn error(msg: impl Into<String>) -> Self {
        Self {
            level: IssueLevel::Error,
            message: msg.into(),
        }
    }

    fn warning(msg: impl Into<String>) -> Self {
        Self {
            level: IssueLevel::Warning,
            message: msg.into(),
        }
    }
}

/// Aggregated result of validating a single feature or backend.
pub struct ValidationReport {
    pub id: String,
    /// Directory containing the validated plugin.
    pub path: PathBuf,
    pub issues: Vec<ValidationIssue>,
}

impl ValidationReport {
    /// `true` if all issues are warnings (no errors).
    pub fn is_ok(&self) -> bool {
        self.issues.iter().all(|i| i.level == IssueLevel::Warning)
    }
}

// ── feature validate ─────────────────────────────────────────────────────────

/// Validate a feature identified by canonical ID (or bare name = `local/<name>`).
///
/// Checks performed:
/// 1. YAML parseable + `spec_version` / `mode` valid (via `feature_index::build`).
/// 2. `install.sh` and `uninstall.sh` exist (script mode only).
/// 3. Resource IDs are unique within the feature (declarative mode).
/// 4. Each `depends` entry is present in the full feature index.
pub fn feature_validate(ctx: &AppContext, id: &str) -> Result<ValidationReport, AppError> {
    let sources = load_sources_optional(ctx)?;
    let roots = build_source_roots(ctx, &sources);
    let fi_platform = to_fi_platform(&ctx.platform);

    // Build full index — validates YAML, spec_version, mode, depends format.
    let mut index = feature_index::build(&roots, &fi_platform)?;

    let meta = index
        .features
        .remove(id)
        .ok_or_else(|| AppError::FeatureNotFound { id: id.to_string() })?;

    let dir = PathBuf::from(&meta.source_dir);
    let mut issues: Vec<ValidationIssue> = Vec::new();

    // ── Check 2: script files ────────────────────────────────────────────────
    if matches!(meta.mode, model::feature_index::FeatureMode::Script) {
        let install = dir.join("install.sh");
        let uninstall = dir.join("uninstall.sh");
        if !install.is_file() {
            issues.push(ValidationIssue::error(
                "install.sh is missing (required for script mode)",
            ));
        }
        if !uninstall.is_file() {
            issues.push(ValidationIssue::error(
                "uninstall.sh is missing (required for script mode)",
            ));
        }
    }

    // ── Check 3: resource ID uniqueness ─────────────────────────────────────
    if let Some(spec) = &meta.spec {
        let mut seen: HashSet<&str> = HashSet::new();
        for res in &spec.resources {
            if !seen.insert(res.id.as_str()) {
                issues.push(ValidationIssue::error(format!(
                    "duplicate resource id: '{}'",
                    res.id
                )));
            }
        }
    }

    // ── Check 4: depends entries exist in the index ──────────────────────────
    for dep_id in &meta.dep.depends {
        if !index.features.contains_key(dep_id.as_str()) {
            issues.push(ValidationIssue::warning(format!(
                "depends on '{}' which was not found in the feature index \
                 (may come from a source not yet cloned)",
                dep_id
            )));
        }
    }

    Ok(ValidationReport {
        id: id.to_string(),
        path: dir,
        issues,
    })
}

// ── backend validate ─────────────────────────────────────────────────────────

/// Validate a backend identified by canonical ID (or bare name = `local/<name>`).
///
/// Checks performed:
/// 1. `backend.yaml` parseable + `api_version` valid (via `show_backend`).
/// 2. Required scripts present for the current platform:
///    `apply`, `remove`, and `status` (error if absent).
pub fn backend_validate(ctx: &AppContext, id: &str) -> Result<ValidationReport, AppError> {
    let detail = crate::read::show_backend(ctx, id)?;
    let path = PathBuf::from(&detail.dir);
    let mut issues: Vec<ValidationIssue> = Vec::new();

    if !detail.scripts.apply {
        issues.push(ValidationIssue::error("apply script is missing (required)"));
    }
    if !detail.scripts.remove {
        issues.push(ValidationIssue::error(
            "remove script is missing (required)",
        ));
    }
    if !detail.scripts.status {
        issues.push(ValidationIssue::error(
            "status script is missing (required)",
        ));
    }

    Ok(ValidationReport {
        id: id.to_string(),
        path,
        issues,
    })
}
