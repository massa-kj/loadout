//! Canonical identifier types used throughout the pipeline.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Canonical component identifier: `<source_id>/<name>` (e.g. `core/git`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CanonicalComponentId(String);

/// Canonical backend identifier: `<source_id>/<name>` (e.g. `core/brew`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CanonicalBackendId(String);

/// Source identifier (e.g. `core`, `local`, `community`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourceId(String);

/// Topologically sorted list of component IDs produced by the resolver.
pub type ResolvedComponentOrder = Vec<CanonicalComponentId>;

// --- CanonicalComponentId impl ---

impl CanonicalComponentId {
    /// Construct from a string that is already in canonical form.
    /// Returns an error if the string does not contain exactly one `/`
    /// or if either part is empty.
    pub fn new(s: impl Into<String>) -> Result<Self, IdError> {
        let s = s.into();
        validate_canonical(&s)?;
        Ok(Self(s))
    }

    /// Return the raw string value.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Return the source part (before `/`).
    pub fn source(&self) -> &str {
        split_canonical(&self.0).0
    }

    /// Return the name part (after `/`).
    pub fn name(&self) -> &str {
        split_canonical(&self.0).1
    }
}

impl fmt::Display for CanonicalComponentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<CanonicalComponentId> for String {
    fn from(id: CanonicalComponentId) -> Self {
        id.0
    }
}

// --- CanonicalBackendId impl ---

impl CanonicalBackendId {
    /// Construct from a string that is already in canonical form.
    pub fn new(s: impl Into<String>) -> Result<Self, IdError> {
        let s = s.into();
        validate_canonical(&s)?;
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn source(&self) -> &str {
        split_canonical(&self.0).0
    }

    pub fn name(&self) -> &str {
        split_canonical(&self.0).1
    }
}

impl fmt::Display for CanonicalBackendId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<CanonicalBackendId> for String {
    fn from(id: CanonicalBackendId) -> Self {
        id.0
    }
}

// --- SourceId impl ---

impl SourceId {
    pub fn new(s: impl Into<String>) -> Result<Self, IdError> {
        let s = s.into();
        if s.is_empty() {
            return Err(IdError {
                msg: "source id must not be empty".into(),
            });
        }
        if s.contains('/') {
            return Err(IdError {
                msg: format!("source id must not contain '/': {s}"),
            });
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// --- helpers ---

fn validate_canonical(s: &str) -> Result<(), IdError> {
    let sep_count = s.chars().filter(|&c| c == '/').count();
    if sep_count != 1 {
        return Err(IdError {
            msg: format!("canonical id must contain exactly one '/': {s}"),
        });
    }
    let (source, name) = split_canonical(s);
    if source.is_empty() {
        return Err(IdError {
            msg: format!("source part must not be empty in: {s}"),
        });
    }
    if name.is_empty() {
        return Err(IdError {
            msg: format!("name part must not be empty in: {s}"),
        });
    }
    Ok(())
}

fn split_canonical(s: &str) -> (&str, &str) {
    let pos = s.find('/').expect("canonical id must contain '/'");
    (&s[..pos], &s[pos + 1..])
}

// --- error ---

/// Error returned when constructing an ID from an invalid string.
#[derive(Debug, Clone, PartialEq)]
pub struct IdError {
    msg: String,
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid id: {}", self.msg)
    }
}

impl std::error::Error for IdError {}

// --- tests ---

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_component_id_valid() {
        let id = CanonicalComponentId::new("core/git").unwrap();
        assert_eq!(id.source(), "core");
        assert_eq!(id.name(), "git");
        assert_eq!(id.as_str(), "core/git");
    }

    #[test]
    fn canonical_component_id_no_slash() {
        assert!(CanonicalComponentId::new("git").is_err());
    }

    #[test]
    fn canonical_component_id_multiple_slashes() {
        assert!(CanonicalComponentId::new("core/local/git").is_err());
    }

    #[test]
    fn canonical_component_id_empty_source() {
        assert!(CanonicalComponentId::new("/git").is_err());
    }

    #[test]
    fn canonical_component_id_empty_name() {
        assert!(CanonicalComponentId::new("core/").is_err());
    }

    #[test]
    fn source_id_valid() {
        let id = SourceId::new("core").unwrap();
        assert_eq!(id.as_str(), "core");
    }

    #[test]
    fn source_id_empty() {
        assert!(SourceId::new("").is_err());
    }

    #[test]
    fn source_id_no_slash_allowed() {
        assert!(SourceId::new("core/local").is_err());
    }
}
