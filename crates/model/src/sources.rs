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

    /// Source type. Currently only `git` is supported.
    #[serde(rename = "type")]
    pub source_type: SourceType,

    /// Git repository URL.
    pub url: String,

    /// Pinned revision identifier. Declarative metadata; core does not fetch automatically.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,

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
        let json = r#"{
            "sources": [
                { "id": "community", "type": "git", "url": "https://github.com/ex/loadout", "commit": "abcdef0" }
            ]
        }"#;
        let s: SourcesSpec = serde_json::from_str(json).unwrap();
        assert_eq!(s.sources[0].id, "community");
        assert!(s.sources[0].allow.is_none());
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
    fn empty_sources() {
        let json = r#"{}"#;
        let s: SourcesSpec = serde_json::from_str(json).unwrap();
        assert!(s.sources.is_empty());
    }
}
