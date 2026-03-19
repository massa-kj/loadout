//! Error types for model-level validation.

use std::fmt;

/// Error produced when a schema version field has an unexpected value.
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaVersionError {
    pub expected: u32,
    pub found: u32,
    pub context: &'static str,
}

impl fmt::Display for SchemaVersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: unsupported schema version {} (expected {})",
            self.context, self.found, self.expected
        )
    }
}

impl std::error::Error for SchemaVersionError {}

/// High-level validation errors for model types.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationError {
    /// Schema version field is not the expected value.
    SchemaVersion(SchemaVersionError),
    /// A required field is absent or empty.
    MissingField { field: String, context: String },
    /// A field value does not match the expected format or constraint.
    InvalidField {
        field: String,
        value: String,
        reason: String,
    },
    /// A duplicate identifier was found where uniqueness is required.
    DuplicateId { id: String, context: String },
    /// An absolute path is required but a relative path was provided.
    RelativePath { path: String, context: String },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SchemaVersion(e) => write!(f, "{e}"),
            Self::MissingField { field, context } => {
                write!(f, "{context}: missing required field '{field}'")
            }
            Self::InvalidField {
                field,
                value,
                reason,
            } => {
                write!(f, "invalid value for '{field}': {value} ({reason})")
            }
            Self::DuplicateId { id, context } => {
                write!(f, "{context}: duplicate id '{id}'")
            }
            Self::RelativePath { path, context } => {
                write!(f, "{context}: path must be absolute, got '{path}'")
            }
        }
    }
}

impl std::error::Error for ValidationError {}

impl From<SchemaVersionError> for ValidationError {
    fn from(e: SchemaVersionError) -> Self {
        Self::SchemaVersion(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_version_error_display() {
        let e = SchemaVersionError {
            expected: 3,
            found: 2,
            context: "state",
        };
        assert_eq!(
            e.to_string(),
            "state: unsupported schema version 2 (expected 3)"
        );
    }

    #[test]
    fn validation_error_missing_field() {
        let e = ValidationError::MissingField {
            field: "resources".into(),
            context: "core/git".into(),
        };
        assert_eq!(
            e.to_string(),
            "core/git: missing required field 'resources'"
        );
    }
}
