//! Parameter validation and default resolution.
//!
//! Pure-function crate: validates profile params against a component's `ParamsSchema`,
//! applies defaults, and produces `ResolvedParams` ready for materialization.
//!
//! See: `tmp/20260417_parameter.md` — Phase 4

use std::collections::HashMap;

use model::params::{ParamProperty, ParamType, ParamValue, ParamsSchema, ResolvedParams};
use thiserror::Error;

/// Errors produced during parameter validation.
#[derive(Debug, Error)]
pub enum ParamsValidationError {
    /// Profile supplies params but the component declares no `params_schema`.
    #[error("component '{component_id}': params are not accepted (no params_schema declared)")]
    ParamsNotAccepted { component_id: String },

    /// A required param is missing from the profile.
    #[error("component '{component_id}': required param '{key}' is missing")]
    MissingRequired { component_id: String, key: String },

    /// An unknown key was provided and `additional_properties` is false.
    #[error("component '{component_id}': unknown param '{key}'")]
    UnknownKey { component_id: String, key: String },

    /// The value type does not match the schema's declared type.
    #[error(
        "component '{component_id}': param '{key}' type mismatch: expected {expected}, found {found}"
    )]
    TypeMismatch {
        component_id: String,
        key: String,
        expected: String,
        found: String,
    },

    /// An enum param value is not one of the allowed values.
    #[error(
        "component '{component_id}': param '{key}' value '{value}' is not in allowed values: {allowed:?}"
    )]
    InvalidEnumValue {
        component_id: String,
        key: String,
        value: String,
        allowed: Vec<String>,
    },
}

/// Validate profile params against a component's schema and resolve defaults.
///
/// # Cases
///
/// - `schema` is `None` and `params` is `None` or empty → empty `ResolvedParams` (OK)
/// - `schema` is `None` but `params` is non-empty → `ParamsNotAccepted` error
/// - `schema` is `Some` → validate keys, types, and required constraints; apply defaults
pub fn validate_and_resolve(
    component_id: &str,
    schema: Option<&ParamsSchema>,
    params: Option<&HashMap<String, ParamValue>>,
) -> Result<ResolvedParams, ParamsValidationError> {
    let has_params = params.is_some_and(|p| !p.is_empty());

    // No schema declared.
    let Some(schema) = schema else {
        if has_params {
            return Err(ParamsValidationError::ParamsNotAccepted {
                component_id: component_id.to_string(),
            });
        }
        return Ok(ResolvedParams::default());
    };

    let params = params.cloned().unwrap_or_default();

    // Reject unknown keys when additional_properties is false.
    if !schema.additional_properties {
        for key in params.keys() {
            if !schema.properties.contains_key(key) {
                return Err(ParamsValidationError::UnknownKey {
                    component_id: component_id.to_string(),
                    key: key.clone(),
                });
            }
        }
    }

    let mut resolved: HashMap<String, ParamValue> = HashMap::new();

    // Iterate over declared properties: validate provided values and apply defaults.
    for (key, prop) in &schema.properties {
        match params.get(key) {
            Some(value) => {
                validate_type(component_id, key, prop, value)?;
                resolved.insert(key.clone(), value.clone());
            }
            None => {
                // Apply default if available.
                if let Some(default) = &prop.default {
                    resolved.insert(key.clone(), default.clone());
                }
            }
        }
    }

    // Pass through extra keys when additional_properties is true.
    if schema.additional_properties {
        for (key, value) in &params {
            if !schema.properties.contains_key(key) {
                resolved.insert(key.clone(), value.clone());
            }
        }
    }

    // Check all required keys are present in the resolved map.
    for key in &schema.required {
        if !resolved.contains_key(key) {
            return Err(ParamsValidationError::MissingRequired {
                component_id: component_id.to_string(),
                key: key.clone(),
            });
        }
    }

    Ok(ResolvedParams { values: resolved })
}

/// Validate that a param value matches the schema property's declared type.
fn validate_type(
    component_id: &str,
    key: &str,
    prop: &ParamProperty,
    value: &ParamValue,
) -> Result<(), ParamsValidationError> {
    match (&prop.param_type, value) {
        (ParamType::String, ParamValue::String(_)) => Ok(()),
        (ParamType::String, _) => Err(ParamsValidationError::TypeMismatch {
            component_id: component_id.to_string(),
            key: key.to_string(),
            expected: "string".to_string(),
            found: describe_value(value),
        }),
        (ParamType::Enum { values }, ParamValue::String(s)) => {
            if values.contains(s) {
                Ok(())
            } else {
                Err(ParamsValidationError::InvalidEnumValue {
                    component_id: component_id.to_string(),
                    key: key.to_string(),
                    value: s.clone(),
                    allowed: values.clone(),
                })
            }
        }
        (ParamType::Enum { .. }, _) => Err(ParamsValidationError::TypeMismatch {
            component_id: component_id.to_string(),
            key: key.to_string(),
            expected: "string (enum)".to_string(),
            found: describe_value(value),
        }),
        (ParamType::Object { .. }, ParamValue::Source(_)) => Ok(()),
        (ParamType::Object { .. }, _) => Err(ParamsValidationError::TypeMismatch {
            component_id: component_id.to_string(),
            key: key.to_string(),
            expected: "object (source)".to_string(),
            found: describe_value(value),
        }),
    }
}

/// Describe a `ParamValue` for error messages.
fn describe_value(value: &ParamValue) -> String {
    match value {
        ParamValue::String(_) => "string".to_string(),
        ParamValue::Source(_) => "object (source)".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::fs::FsSourceKind;
    use model::params::{ParamProperty, ParamType, SourceParamValue};

    fn schema_with_version() -> ParamsSchema {
        ParamsSchema {
            properties: HashMap::from([(
                "version".to_string(),
                ParamProperty {
                    param_type: ParamType::String,
                    default: None,
                },
            )]),
            required: vec!["version".to_string()],
            additional_properties: false,
        }
    }

    fn schema_with_default_version() -> ParamsSchema {
        ParamsSchema {
            properties: HashMap::from([(
                "version".to_string(),
                ParamProperty {
                    param_type: ParamType::String,
                    default: Some(ParamValue::String("22.17.1".to_string())),
                },
            )]),
            required: vec![],
            additional_properties: false,
        }
    }

    // ── Happy paths ──────────────────────────────────────────────────────

    #[test]
    fn all_required_provided() {
        let schema = schema_with_version();
        let params = HashMap::from([("version".into(), ParamValue::String("20.0".into()))]);
        let resolved = validate_and_resolve("core/node", Some(&schema), Some(&params)).unwrap();
        assert_eq!(
            resolved.values["version"],
            ParamValue::String("20.0".into())
        );
    }

    #[test]
    fn default_applied_when_omitted() {
        let schema = schema_with_default_version();
        let resolved = validate_and_resolve("core/node", Some(&schema), None).unwrap();
        assert_eq!(
            resolved.values["version"],
            ParamValue::String("22.17.1".into())
        );
    }

    #[test]
    fn no_schema_no_params_ok() {
        let resolved = validate_and_resolve("core/git", None, None).unwrap();
        assert!(resolved.values.is_empty());
    }

    #[test]
    fn no_schema_empty_params_ok() {
        let resolved = validate_and_resolve("core/git", None, Some(&HashMap::new())).unwrap();
        assert!(resolved.values.is_empty());
    }

    #[test]
    fn enum_valid_value() {
        let schema = ParamsSchema {
            properties: HashMap::from([(
                "op".to_string(),
                ParamProperty {
                    param_type: ParamType::Enum {
                        values: vec!["copy".into(), "link".into()],
                    },
                    default: None,
                },
            )]),
            required: vec!["op".to_string()],
            additional_properties: false,
        };
        let params = HashMap::from([("op".into(), ParamValue::String("copy".into()))]);
        let resolved = validate_and_resolve("core/git", Some(&schema), Some(&params)).unwrap();
        assert_eq!(resolved.values["op"], ParamValue::String("copy".into()));
    }

    #[test]
    fn object_source_valid() {
        let schema = ParamsSchema {
            properties: HashMap::from([(
                "source".to_string(),
                ParamProperty {
                    param_type: ParamType::Object { properties: None },
                    default: None,
                },
            )]),
            required: vec!["source".to_string()],
            additional_properties: false,
        };
        let params = HashMap::from([(
            "source".into(),
            ParamValue::Source(SourceParamValue {
                kind: FsSourceKind::HomeRelative,
                path: "~/dotfiles/.gitconfig".into(),
            }),
        )]);
        let resolved = validate_and_resolve("core/git", Some(&schema), Some(&params)).unwrap();
        assert!(matches!(resolved.values["source"], ParamValue::Source(_)));
    }

    #[test]
    fn additional_properties_true_passes_extra_keys() {
        let schema = ParamsSchema {
            properties: HashMap::new(),
            required: vec![],
            additional_properties: true,
        };
        let params = HashMap::from([("custom".into(), ParamValue::String("val".into()))]);
        let resolved = validate_and_resolve("core/x", Some(&schema), Some(&params)).unwrap();
        assert_eq!(resolved.values["custom"], ParamValue::String("val".into()));
    }

    // ── Error paths ──────────────────────────────────────────────────────

    #[test]
    fn no_schema_with_params_rejected() {
        let params = HashMap::from([("version".into(), ParamValue::String("1.0".into()))]);
        let err = validate_and_resolve("core/git", None, Some(&params)).unwrap_err();
        assert!(matches!(
            err,
            ParamsValidationError::ParamsNotAccepted { .. }
        ));
    }

    #[test]
    fn missing_required_rejected() {
        let schema = schema_with_version();
        let err = validate_and_resolve("core/node", Some(&schema), None).unwrap_err();
        assert!(
            matches!(err, ParamsValidationError::MissingRequired { ref key, .. } if key == "version"),
        );
    }

    #[test]
    fn unknown_key_rejected() {
        let schema = schema_with_version();
        let params = HashMap::from([
            ("version".into(), ParamValue::String("20".into())),
            ("typo".into(), ParamValue::String("oops".into())),
        ]);
        let err = validate_and_resolve("core/node", Some(&schema), Some(&params)).unwrap_err();
        assert!(matches!(err, ParamsValidationError::UnknownKey { ref key, .. } if key == "typo"),);
    }

    #[test]
    fn type_mismatch_string_expects_source() {
        let schema = schema_with_version();
        let params = HashMap::from([(
            "version".into(),
            ParamValue::Source(SourceParamValue {
                kind: FsSourceKind::Absolute,
                path: "/a".into(),
            }),
        )]);
        let err = validate_and_resolve("core/node", Some(&schema), Some(&params)).unwrap_err();
        assert!(matches!(err, ParamsValidationError::TypeMismatch { .. }));
    }

    #[test]
    fn type_mismatch_object_expects_string() {
        let schema = ParamsSchema {
            properties: HashMap::from([(
                "source".to_string(),
                ParamProperty {
                    param_type: ParamType::Object { properties: None },
                    default: None,
                },
            )]),
            required: vec!["source".to_string()],
            additional_properties: false,
        };
        let params = HashMap::from([("source".into(), ParamValue::String("wrong".into()))]);
        let err = validate_and_resolve("core/git", Some(&schema), Some(&params)).unwrap_err();
        assert!(matches!(err, ParamsValidationError::TypeMismatch { .. }));
    }

    #[test]
    fn enum_invalid_value_rejected() {
        let schema = ParamsSchema {
            properties: HashMap::from([(
                "op".to_string(),
                ParamProperty {
                    param_type: ParamType::Enum {
                        values: vec!["copy".into(), "link".into()],
                    },
                    default: None,
                },
            )]),
            required: vec!["op".to_string()],
            additional_properties: false,
        };
        let params = HashMap::from([("op".into(), ParamValue::String("move".into()))]);
        let err = validate_and_resolve("core/git", Some(&schema), Some(&params)).unwrap_err();
        assert!(matches!(
            err,
            ParamsValidationError::InvalidEnumValue { ref value, .. } if value == "move"
        ));
    }

    #[test]
    fn default_overridden_by_explicit_value() {
        let schema = schema_with_default_version();
        let params = HashMap::from([("version".into(), ParamValue::String("18.0".into()))]);
        let resolved = validate_and_resolve("core/node", Some(&schema), Some(&params)).unwrap();
        assert_eq!(
            resolved.values["version"],
            ParamValue::String("18.0".into()),
            "explicit value must override default"
        );
    }
}
