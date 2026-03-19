//! Source registry — canonical ID to filesystem path resolution and allow-list enforcement.
//!
//! The registry maps canonical feature and backend IDs to concrete directories on disk,
//! and enforces the allow-list rules declared in `sources.yaml` for external sources.
//!
//! # Source Kinds
//!
//! Three source kinds are recognized:
//!
//! | Source ID   | Kind       | Declared in `sources.yaml` |
//! |-------------|------------|----------------------------|
//! | `core`      | implicit   | No — always available       |
//! | `user`      | implicit   | No — always available       |
//! | `<other>`   | external   | Yes — must be declared      |
//!
//! # Path Resolution
//!
//! **Features:**
//! - `core/<name>` → `{repo_root}/features/<name>`
//! - `user/<name>` → `{config_home}/features/<name>`
//! - `<ext>/<name>` → `{data_home}/sources/<ext>/features/<name>`
//!
//! **Backends:**
//! - `core/<name>` → `{repo_root}/backends/<name>`
//! - `user/<name>` → `{config_home}/backends/<name>`
//! - `<ext>/<name>` → `{data_home}/sources/<ext>/backends/<name>`
//!
//! # Allow-List Enforcement
//!
//! - `core` and `user`: always allowed — no allow-list applies.
//! - External sources: `allow` field in the source entry is checked.
//!   - `allow` absent → deny-all (error).
//!   - `allow: "*"` → allow everything.
//!   - `allow: { features: ..., backends: ... }` → check respective allow-list.
//!
//! No implicit fallback between `user`, external, and `core` is permitted.
//!
//! # Phase 3 Contract
//!
//! Path resolution for base directories (XDG, AppData) is NOT performed here.
//! Callers supply explicit `&Path` values. Platform-aware path discovery is the
//! responsibility of the `platform` crate (Phase 4).
//!
//! See: `docs/specs/data/sources.md`

use std::path::{Path, PathBuf};

use model::{
    id::{CanonicalBackendId, CanonicalFeatureId},
    sources::{AllowList, AllowSpec, SourceEntry, SourcesSpec},
};
use thiserror::Error;

/// Errors produced by source registry operations.
#[derive(Debug, Error)]
pub enum RegistryError {
    /// An external source ID was referenced but is not declared in `sources.yaml`.
    #[error("unknown source '{source_id}': not declared in sources.yaml")]
    UnknownSource { source_id: String },

    /// The allow-list for an external source has no `allow` field — source is deny-all.
    #[error(
        "feature '{feature_id}' is not allowed by source '{source_id}': \
         source has no allow-list (deny-all)"
    )]
    FeatureDeniedNoAllowList {
        feature_id: String,
        source_id: String,
    },

    /// The allow-list for an external source does not permit this feature.
    #[error("feature '{feature_id}' is not in the allow-list of source '{source_id}'")]
    FeatureNotAllowed {
        feature_id: String,
        source_id: String,
    },

    /// The allow-list for an external source has no `allow` field — source is deny-all.
    #[error(
        "backend '{backend_id}' is not allowed by source '{source_id}': \
         source has no allow-list (deny-all)"
    )]
    BackendDeniedNoAllowList {
        backend_id: String,
        source_id: String,
    },

    /// The allow-list for an external source does not permit this backend.
    #[error("backend '{backend_id}' is not in the allow-list of source '{source_id}'")]
    BackendNotAllowed {
        backend_id: String,
        source_id: String,
    },
}

/// The source registry resolves canonical IDs to filesystem paths and enforces allow-lists.
///
/// Construct via [`SourceRegistry::new`] using a loaded [`SourcesSpec`] and three base paths.
pub struct SourceRegistry {
    /// External sources declared in `sources.yaml`. `core` and `user` are not listed here.
    sources: Vec<SourceEntry>,
    /// Absolute path to the loadout repository root (contains `features/` and `backends/`).
    repo_root: PathBuf,
    /// Absolute path to the user config home (contains `features/` and `backends/` overrides).
    config_home: PathBuf,
    /// Absolute path to the data home (contains `sources/<id>/features/` and `sources/<id>/backends/`).
    data_home: PathBuf,
}

impl SourceRegistry {
    /// Build a registry from a loaded `SourcesSpec` and explicit base paths.
    ///
    /// # Arguments
    ///
    /// - `sources` — loaded and validated [`SourcesSpec`]; `core`/`user` must not appear here.
    /// - `repo_root` — the repository root directory (where `features/` and `backends/` live).
    /// - `config_home` — user config home (for `user/` source resolution).
    /// - `data_home` — user data home (for external source resolution).
    pub fn new(
        sources: SourcesSpec,
        repo_root: &Path,
        config_home: &Path,
        data_home: &Path,
    ) -> Self {
        Self {
            sources: sources.sources,
            repo_root: repo_root.to_path_buf(),
            config_home: config_home.to_path_buf(),
            data_home: data_home.to_path_buf(),
        }
    }

    /// Resolve the filesystem directory for a canonical feature ID.
    ///
    /// Does NOT check whether the directory exists on disk.
    /// Does NOT enforce the allow-list — call [`check_feature_allowed`](Self::check_feature_allowed) separately.
    pub fn feature_dir(&self, id: &CanonicalFeatureId) -> Result<PathBuf, RegistryError> {
        self.resolve_dir(id.source(), id.name(), ResourceKind::Feature)
    }

    /// Resolve the filesystem directory for a canonical backend ID.
    ///
    /// Does NOT check whether the directory exists on disk.
    /// Does NOT enforce the allow-list — call [`check_backend_allowed`](Self::check_backend_allowed) separately.
    pub fn backend_dir(&self, id: &CanonicalBackendId) -> Result<PathBuf, RegistryError> {
        self.resolve_dir(id.source(), id.name(), ResourceKind::Backend)
    }

    /// Check whether a canonical feature ID is permitted by the source's allow-list.
    ///
    /// - `core` and `user` sources are always allowed.
    /// - External sources without an `allow` field are deny-all.
    /// - Returns `Ok(())` if allowed, `Err(RegistryError::Feature*)` otherwise.
    pub fn check_feature_allowed(&self, id: &CanonicalFeatureId) -> Result<(), RegistryError> {
        let source_id = id.source();
        if is_implicit(source_id) {
            return Ok(());
        }
        let entry = self.find_source(source_id)?;
        check_allowed_feature(entry, id)
    }

    /// Check whether a canonical backend ID is permitted by the source's allow-list.
    ///
    /// - `core` and `user` sources are always allowed.
    /// - External sources without an `allow` field are deny-all.
    /// - Returns `Ok(())` if allowed, `Err(RegistryError::Backend*)` otherwise.
    pub fn check_backend_allowed(&self, id: &CanonicalBackendId) -> Result<(), RegistryError> {
        let source_id = id.source();
        if is_implicit(source_id) {
            return Ok(());
        }
        let entry = self.find_source(source_id)?;
        check_allowed_backend(entry, id)
    }

    // ─── Internal helpers ────────────────────────────────────────────────────

    fn find_source<'a>(&'a self, source_id: &str) -> Result<&'a SourceEntry, RegistryError> {
        self.sources
            .iter()
            .find(|e| e.id == source_id)
            .ok_or_else(|| RegistryError::UnknownSource {
                source_id: source_id.to_string(),
            })
    }

    fn resolve_dir(
        &self,
        source_id: &str,
        name: &str,
        kind: ResourceKind,
    ) -> Result<PathBuf, RegistryError> {
        match source_id {
            "core" => Ok(self.repo_root.join(kind.dir_segment()).join(name)),
            "user" => Ok(self.config_home.join(kind.dir_segment()).join(name)),
            ext => {
                // External source must be declared.
                if !self.sources.iter().any(|e| e.id == ext) {
                    return Err(RegistryError::UnknownSource {
                        source_id: ext.to_string(),
                    });
                }
                Ok(self
                    .data_home
                    .join("sources")
                    .join(ext)
                    .join(kind.dir_segment())
                    .join(name))
            }
        }
    }
}

/// Whether a source ID is an implicit source (always available, no allow-list).
fn is_implicit(source_id: &str) -> bool {
    matches!(source_id, "core" | "user")
}

enum ResourceKind {
    Feature,
    Backend,
}

impl ResourceKind {
    fn dir_segment(&self) -> &'static str {
        match self {
            ResourceKind::Feature => "features",
            ResourceKind::Backend => "backends",
        }
    }
}

fn check_allowed_feature(
    entry: &SourceEntry,
    id: &CanonicalFeatureId,
) -> Result<(), RegistryError> {
    match &entry.allow {
        None => Err(RegistryError::FeatureDeniedNoAllowList {
            feature_id: id.as_str().to_string(),
            source_id: entry.id.clone(),
        }),
        Some(AllowSpec::All(_)) => Ok(()),
        Some(AllowSpec::Detailed(detail)) => match &detail.features {
            None => Err(RegistryError::FeatureNotAllowed {
                feature_id: id.as_str().to_string(),
                source_id: entry.id.clone(),
            }),
            Some(AllowList::All(_)) => Ok(()),
            Some(AllowList::Names(names)) => {
                if names.iter().any(|n| n == id.name()) {
                    Ok(())
                } else {
                    Err(RegistryError::FeatureNotAllowed {
                        feature_id: id.as_str().to_string(),
                        source_id: entry.id.clone(),
                    })
                }
            }
        },
    }
}

fn check_allowed_backend(
    entry: &SourceEntry,
    id: &CanonicalBackendId,
) -> Result<(), RegistryError> {
    match &entry.allow {
        None => Err(RegistryError::BackendDeniedNoAllowList {
            backend_id: id.as_str().to_string(),
            source_id: entry.id.clone(),
        }),
        Some(AllowSpec::All(_)) => Ok(()),
        Some(AllowSpec::Detailed(detail)) => match &detail.backends {
            None => Err(RegistryError::BackendNotAllowed {
                backend_id: id.as_str().to_string(),
                source_id: entry.id.clone(),
            }),
            Some(AllowList::All(_)) => Ok(()),
            Some(AllowList::Names(names)) => {
                if names.iter().any(|n| n == id.name()) {
                    Ok(())
                } else {
                    Err(RegistryError::BackendNotAllowed {
                        backend_id: id.as_str().to_string(),
                        source_id: entry.id.clone(),
                    })
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::sources::{
        AllowList, AllowSpec, DetailedAllow, SourceType, SourcesSpec, WildcardAll,
    };
    use std::path::PathBuf;

    fn repo() -> PathBuf {
        PathBuf::from("/repo")
    }
    fn cfg() -> PathBuf {
        PathBuf::from("/cfg")
    }
    fn data() -> PathBuf {
        PathBuf::from("/data")
    }

    fn feature(s: &str) -> CanonicalFeatureId {
        CanonicalFeatureId::new(s).unwrap()
    }

    fn backend(s: &str) -> CanonicalBackendId {
        CanonicalBackendId::new(s).unwrap()
    }

    fn registry_with(sources: SourcesSpec) -> SourceRegistry {
        SourceRegistry::new(sources, &repo(), &cfg(), &data())
    }

    fn empty_registry() -> SourceRegistry {
        registry_with(SourcesSpec::default())
    }

    fn source_entry(id: &str, allow: Option<AllowSpec>) -> model::sources::SourceEntry {
        model::sources::SourceEntry {
            id: id.into(),
            source_type: SourceType::Git,
            url: format!("https://example.com/{id}"),
            commit: None,
            allow,
        }
    }

    fn spec_with(entries: Vec<model::sources::SourceEntry>) -> SourcesSpec {
        SourcesSpec { sources: entries }
    }

    // ── feature_dir ───────────────────────────────────────────────────────────

    #[test]
    fn feature_dir_core() {
        let r = empty_registry();
        let dir = r.feature_dir(&feature("core/git")).unwrap();
        assert_eq!(dir, PathBuf::from("/repo/features/git"));
    }

    #[test]
    fn feature_dir_user() {
        let r = empty_registry();
        let dir = r.feature_dir(&feature("user/myvim")).unwrap();
        assert_eq!(dir, PathBuf::from("/cfg/features/myvim"));
    }

    #[test]
    fn feature_dir_external() {
        let r = registry_with(spec_with(vec![source_entry("community", None)]));
        let dir = r.feature_dir(&feature("community/node")).unwrap();
        assert_eq!(dir, PathBuf::from("/data/sources/community/features/node"));
    }

    #[test]
    fn feature_dir_unknown_source_errors() {
        let r = empty_registry();
        let err = r.feature_dir(&feature("unknown/node")).unwrap_err();
        assert!(matches!(err, RegistryError::UnknownSource { .. }));
    }

    // ── backend_dir ───────────────────────────────────────────────────────────

    #[test]
    fn backend_dir_core() {
        let r = empty_registry();
        let dir = r.backend_dir(&backend("core/brew")).unwrap();
        assert_eq!(dir, PathBuf::from("/repo/backends/brew"));
    }

    #[test]
    fn backend_dir_user() {
        let r = empty_registry();
        let dir = r.backend_dir(&backend("user/mypkg")).unwrap();
        assert_eq!(dir, PathBuf::from("/cfg/backends/mypkg"));
    }

    #[test]
    fn backend_dir_external() {
        let r = registry_with(spec_with(vec![source_entry("tools", None)]));
        let dir = r.backend_dir(&backend("tools/npm")).unwrap();
        assert_eq!(dir, PathBuf::from("/data/sources/tools/backends/npm"));
    }

    // ── check_feature_allowed ─────────────────────────────────────────────────

    #[test]
    fn core_feature_always_allowed() {
        let r = empty_registry();
        r.check_feature_allowed(&feature("core/git")).unwrap();
    }

    #[test]
    fn user_feature_always_allowed() {
        let r = empty_registry();
        r.check_feature_allowed(&feature("user/myfeature")).unwrap();
    }

    #[test]
    fn external_no_allow_is_deny_all() {
        let r = registry_with(spec_with(vec![source_entry("community", None)]));
        let err = r
            .check_feature_allowed(&feature("community/node"))
            .unwrap_err();
        assert!(matches!(
            err,
            RegistryError::FeatureDeniedNoAllowList { .. }
        ));
    }

    #[test]
    fn external_allow_star_permits_all_features() {
        let r = registry_with(spec_with(vec![source_entry(
            "community",
            Some(AllowSpec::All(WildcardAll)),
        )]));
        r.check_feature_allowed(&feature("community/node")).unwrap();
        r.check_feature_allowed(&feature("community/python"))
            .unwrap();
    }

    #[test]
    fn external_allow_detailed_features_star_permits_all() {
        let r = registry_with(spec_with(vec![source_entry(
            "tools",
            Some(AllowSpec::Detailed(DetailedAllow {
                features: Some(AllowList::All(WildcardAll)),
                backends: None,
            })),
        )]));
        r.check_feature_allowed(&feature("tools/node")).unwrap();
    }

    #[test]
    fn external_allow_detailed_features_list_permits_listed() {
        let r = registry_with(spec_with(vec![source_entry(
            "tools",
            Some(AllowSpec::Detailed(DetailedAllow {
                features: Some(AllowList::Names(vec!["node".into(), "python".into()])),
                backends: None,
            })),
        )]));
        r.check_feature_allowed(&feature("tools/node")).unwrap();
        r.check_feature_allowed(&feature("tools/python")).unwrap();
    }

    #[test]
    fn external_allow_detailed_features_list_rejects_unlisted() {
        let r = registry_with(spec_with(vec![source_entry(
            "tools",
            Some(AllowSpec::Detailed(DetailedAllow {
                features: Some(AllowList::Names(vec!["node".into()])),
                backends: None,
            })),
        )]));
        let err = r.check_feature_allowed(&feature("tools/ruby")).unwrap_err();
        assert!(matches!(err, RegistryError::FeatureNotAllowed { .. }));
    }

    #[test]
    fn external_allow_detailed_no_features_key_denies_feature() {
        // `allow: { backends: ... }` with no `features:` key — features are denied.
        let r = registry_with(spec_with(vec![source_entry(
            "tools",
            Some(AllowSpec::Detailed(DetailedAllow {
                features: None,
                backends: Some(AllowList::All(WildcardAll)),
            })),
        )]));
        let err = r.check_feature_allowed(&feature("tools/node")).unwrap_err();
        assert!(matches!(err, RegistryError::FeatureNotAllowed { .. }));
    }

    // ── check_backend_allowed ─────────────────────────────────────────────────

    #[test]
    fn core_backend_always_allowed() {
        let r = empty_registry();
        r.check_backend_allowed(&backend("core/brew")).unwrap();
    }

    #[test]
    fn user_backend_always_allowed() {
        let r = empty_registry();
        r.check_backend_allowed(&backend("user/mypkg")).unwrap();
    }

    #[test]
    fn external_no_allow_denies_backend() {
        let r = registry_with(spec_with(vec![source_entry("community", None)]));
        let err = r
            .check_backend_allowed(&backend("community/brew"))
            .unwrap_err();
        assert!(matches!(
            err,
            RegistryError::BackendDeniedNoAllowList { .. }
        ));
    }

    #[test]
    fn external_allow_star_permits_all_backends() {
        let r = registry_with(spec_with(vec![source_entry(
            "community",
            Some(AllowSpec::All(WildcardAll)),
        )]));
        r.check_backend_allowed(&backend("community/npm")).unwrap();
    }

    #[test]
    fn external_allow_detailed_backends_list_permits_listed() {
        let r = registry_with(spec_with(vec![source_entry(
            "tools",
            Some(AllowSpec::Detailed(DetailedAllow {
                features: None,
                backends: Some(AllowList::Names(vec!["npm".into(), "uv".into()])),
            })),
        )]));
        r.check_backend_allowed(&backend("tools/npm")).unwrap();
        r.check_backend_allowed(&backend("tools/uv")).unwrap();
    }

    #[test]
    fn external_allow_detailed_backends_list_rejects_unlisted() {
        let r = registry_with(spec_with(vec![source_entry(
            "tools",
            Some(AllowSpec::Detailed(DetailedAllow {
                features: None,
                backends: Some(AllowList::Names(vec!["npm".into()])),
            })),
        )]));
        let err = r.check_backend_allowed(&backend("tools/brew")).unwrap_err();
        assert!(matches!(err, RegistryError::BackendNotAllowed { .. }));
    }

    #[test]
    fn external_allow_detailed_no_backends_key_denies_backend() {
        let r = registry_with(spec_with(vec![source_entry(
            "tools",
            Some(AllowSpec::Detailed(DetailedAllow {
                features: Some(AllowList::All(WildcardAll)),
                backends: None,
            })),
        )]));
        let err = r.check_backend_allowed(&backend("tools/npm")).unwrap_err();
        assert!(matches!(err, RegistryError::BackendNotAllowed { .. }));
    }

    #[test]
    fn unknown_source_in_check_backend_errors() {
        let r = empty_registry();
        let err = r.check_backend_allowed(&backend("ghost/brew")).unwrap_err();
        assert!(matches!(err, RegistryError::UnknownSource { .. }));
    }
}
