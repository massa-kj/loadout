//! Sources specification data type.
//!
//! Sources declare where features and backends are discovered (git repos or local paths),
//! and define the allow-list for plugin admission control.
//!
//! See: `docs/specs/data/sources.md`

use serde::{Deserialize, Serialize};

/// Specification of external plugin source locations and admission rules.
///
/// Implicit sources (`core` and `local`) are always available and must NOT be listed here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SourcesSpec {
    /// External source declarations.
    /// Reserved IDs (`core`, `local`, `official`) must not appear here.
    #[serde(default)]
    pub sources: Vec<SourceEntry>,
}

/// A single external source entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceEntry {
    /// Canonical source identifier (e.g. `community`). Must not be a reserved ID.
    pub id: String,

    /// Source type.
    #[serde(rename = "type")]
    pub source_type: SourceType,

    /// Git repository URL. Required for `type: git`; must be absent for `type: path`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// Mutable ref for the git source (branch, tag, or commit).
    ///
    /// Exactly one of `branch`, `tag`, or `commit` must be set.
    /// Validated at config load time. Absent for `type: path`.
    ///
    /// The resolved commit hash is stored in `sources.lock.yaml`, not here.
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<SourceRef>,

    /// Path field. Semantics depend on `source_type`:
    ///
    /// - `type: git`: sub-path within the repository, relative to the repo root (default `"."`).
    ///   Must be a relative path without `..` components.
    /// - `type: path`: filesystem path to the source directory.
    ///   May be absolute, `~`-prefixed (home-relative), or relative to `sources.yaml`'s
    ///   parent directory. Pre-resolved to an absolute path by `config::load_sources`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,

    /// Allow-list for resources importable from this source.
    /// If absent, the source is deny-all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow: Option<AllowSpec>,
}

/// Source type discriminant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    Git,
    Path,
}

/// Mutable ref for a git source.
///
/// Exactly one of `branch`, `tag`, or `commit` must be set.
/// Validated at config load time (see `config::load_sources`).
///
/// Serializes to a YAML mapping with a single key, e.g.:
/// ```yaml
/// ref:
///   branch: main
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SourceRef {
    /// Track the HEAD of a named branch (floating/mutable ref).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Pin to a specific tag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Pin to a specific commit hash directly, bypassing lock-file management.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
}

/// Lock file for external git sources (`sources.lock.yaml`).
///
/// Records the resolved commit hash and fetch metadata for each `type: git` source.
/// `type: path` sources are not included in the lock file.
///
/// See: `docs/specs/data/sources.md`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SourcesLock {
    /// Map of source ID to lock entry.
    #[serde(default)]
    pub sources: std::collections::HashMap<String, SourceLockEntry>,
}

/// Lock entry for a single git source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceLockEntry {
    /// Full 40-character commit hash. Short hashes are not permitted.
    pub resolved_commit: String,
    /// UTC timestamp of the last successful fetch, in RFC3339 format.
    pub fetched_at: String,
    /// SHA-256 hash of the source's loadout manifests (`feature.yaml`, `backend.yaml` files).
    /// Computed over manifest files within this source only, not the entire repository.
    pub manifest_hash: String,
}

/// Allow-list for an external source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AllowSpec {
    /// `allow: "*"` — allow all features and backends.
    All(WildcardAll),
    /// `allow: { features: ..., backends: ... }` — fine-grained allow-list.
    Detailed(DetailedAllow),
}

/// Marker for `"*"` wildcard (all allowed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct WildcardAll;

impl TryFrom<String> for WildcardAll {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        if s == "*" {
            Ok(Self)
        } else {
            Err(format!("expected \"*\", got \"{s}\""))
        }
    }
}

impl From<WildcardAll> for String {
    fn from(_: WildcardAll) -> String {
        "*".into()
    }
}

/// Fine-grained allow-list by resource kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DetailedAllow {
    /// Feature names allowed from this source, or `"*"` for all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub features: Option<AllowList>,

    /// Backend names allowed from this source, or `"*"` for all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backends: Option<AllowList>,
}

/// Either a wildcard `"*"` or an explicit name list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AllowList {
    /// Allow all (`"*"`).
    All(WildcardAll),
    /// Allow only the listed names.
    Names(Vec<String>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_deny_all() {
        // Minimal type:git source without allow-list (deny-all).
        let json = r#"{
            "sources": [
                { "id": "community", "type": "git", "url": "https://github.com/ex/loadout" }
            ]
        }"#;
        let s: SourcesSpec = serde_json::from_str(json).unwrap();
        assert_eq!(s.sources[0].id, "community");
        assert!(s.sources[0].allow.is_none());
        assert!(s.sources[0].source_ref.is_none());
    }

    #[test]
    fn round_trip_git_with_ref() {
        let json = r#"{
            "sources": [
                {
                    "id": "community",
                    "type": "git",
                    "url": "https://github.com/ex/loadout",
                    "ref": { "branch": "main" }
                }
            ]
        }"#;
        let s: SourcesSpec = serde_json::from_str(json).unwrap();
        let r = s.sources[0].source_ref.as_ref().unwrap();
        assert_eq!(r.branch.as_deref(), Some("main"));
        assert!(r.tag.is_none());
        assert!(r.commit.is_none());
    }

    #[test]
    fn round_trip_type_path() {
        let json = r#"{
            "sources": [
                { "id": "mylab", "type": "path", "path": "/home/user/mylab" }
            ]
        }"#;
        let s: SourcesSpec = serde_json::from_str(json).unwrap();
        assert_eq!(s.sources[0].source_type, SourceType::Path);
        assert_eq!(s.sources[0].path.as_deref(), Some("/home/user/mylab"));
        assert!(s.sources[0].url.is_none());
    }

    #[test]
    fn round_trip_wildcard_all() {
        let json = r#"{
            "sources": [
                { "id": "tools", "type": "git", "url": "https://example.com", "allow": "*" }
            ]
        }"#;
        let s: SourcesSpec = serde_json::from_str(json).unwrap();
        assert!(matches!(s.sources[0].allow, Some(AllowSpec::All(_))));
    }

    #[test]
    fn round_trip_detailed_allow() {
        let json = r#"{
            "sources": [
                {
                    "id": "tools",
                    "type": "git",
                    "url": "https://example.com",
                    "allow": { "features": "*", "backends": ["npm", "uv"] }
                }
            ]
        }"#;
        let s: SourcesSpec = serde_json::from_str(json).unwrap();
        match &s.sources[0].allow {
            Some(AllowSpec::Detailed(d)) => {
                assert!(matches!(d.features, Some(AllowList::All(_))));
                match &d.backends {
                    Some(AllowList::Names(names)) => assert_eq!(names, &["npm", "uv"]),
                    _ => panic!("expected names"),
                }
            }
            _ => panic!("expected detailed"),
        }
    }

    #[test]
    fn round_trip_sources_lock() {
        let yaml = "sources:\n  community:\n    resolved_commit: abcdef1234567890abcdef1234567890abcdef12\n    fetched_at: '2026-04-07T00:00:00Z'\n    manifest_hash: 'sha256:abc'\n";
        let lock: SourcesLock = serde_yaml::from_str(yaml).unwrap();
        let entry = lock.sources.get("community").unwrap();
        assert_eq!(
            entry.resolved_commit,
            "abcdef1234567890abcdef1234567890abcdef12"
        );
    }

    #[test]
    fn empty_sources() {
        let json = r#"{}"#;
        let s: SourcesSpec = serde_json::from_str(json).unwrap();
        assert!(s.sources.is_empty());
    }
}
