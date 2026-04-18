//! Parameter types for component parameterization.
//!
//! Params allow profiles to inject identity attribute values into component resource
//! templates. Params must not change resource count, kind, or id (structural invariance).
//!
//! See: `tmp/20260417_parameter.md`

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::fs::FsSourceKind;

// ---------------------------------------------------------------------------
// ParamsSchema — declared in component.yaml
// ---------------------------------------------------------------------------

/// Schema for parameters accepted by a component.
///
/// Absent `params_schema` means the component accepts no params.
/// If a profile provides params for a component without a schema, validation fails.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamsSchema {
    /// Property definitions keyed by param name.
    #[serde(default)]
    pub properties: HashMap<String, ParamProperty>,

    /// Param names that must be provided by the profile.
    /// A key listed here must NOT have a `default` in its `ParamProperty`.
    #[serde(default)]
    pub required: Vec<String>,

    /// When `false` (default), unknown param keys are rejected.
    #[serde(default = "default_false")]
    pub additional_properties: bool,
}

fn default_false() -> bool {
    false
}

/// Schema definition for a single parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamProperty {
    /// Type constraint for this parameter.
    #[serde(rename = "type", flatten)]
    pub param_type: ParamType,

    /// Default value applied when the profile omits this param.
    /// Must not coexist with `required` for the same property (validated at index build time).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<ParamValue>,
}

/// Type constraint for a parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ParamType {
    /// Plain string value.
    String,

    /// Structured source object with `kind` and `path`.
    Object {
        /// Nested schema for the source object.
        /// V1: only `SourceParamSchema` is supported.
        #[serde(skip_serializing_if = "Option::is_none")]
        properties: Option<SourceParamObjectSchema>,
    },

    /// One of a fixed set of string values.
    Enum {
        /// Allowed values.
        #[serde(rename = "enum")]
        values: Vec<String>,
    },
}

/// Nested object schema for source params.
///
/// V1 only supports the fs source pattern: `{ kind: FsSourceKind, path: string }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceParamObjectSchema {
    /// Schema for the `kind` field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<SourceKindSchema>,
    /// Schema for the `path` field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<SourcePathSchema>,
}

/// Schema for the `kind` field in a source param object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceKindSchema {
    /// Allowed source kind values.
    #[serde(rename = "enum")]
    pub values: Vec<String>,
}

/// Schema for the `path` field in a source param object (must be string).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourcePathSchema {
    /// Must be `"string"`.
    #[serde(rename = "type")]
    pub path_type: String,
}

// ---------------------------------------------------------------------------
// ParamValue — runtime representation of a resolved param value
// ---------------------------------------------------------------------------

/// A concrete parameter value, either from profile or from a default.
///
/// This is the typed representation used after config parsing.
/// The `config` crate converts raw YAML into this type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    /// A plain string value (covers `string` and `enum` param types).
    String(String),

    /// A structured source reference (covers `object` param type for fs sources).
    Source(SourceParamValue),
}

/// A structured source reference provided as a param value.
///
/// Represents the user-facing input that will be resolved into `ConcreteFsSource`
/// by the materializer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceParamValue {
    /// Source kind.
    pub kind: FsSourceKind,
    /// Path (interpretation depends on `kind`).
    pub path: String,
}

// ---------------------------------------------------------------------------
// ResolvedParams — output of validation + default resolution
// ---------------------------------------------------------------------------

/// Params after schema validation and default value resolution.
///
/// All required params are present. All defaults have been applied.
/// Types have been checked. Ready for materialization.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ResolvedParams {
    /// Resolved param values keyed by param name.
    pub values: HashMap<String, ParamValue>,
}

// ---------------------------------------------------------------------------
// MaterializedComponentSpec — template-resolved ComponentSpec
// ---------------------------------------------------------------------------

/// A `ComponentSpec` after all `${params.*}` references have been resolved.
///
/// This is the input to `ComponentCompiler`. No template variables remain.
/// The compiler treats this identically to a regular `ComponentSpec`.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterializedComponentSpec {
    /// Resources with all param references resolved to concrete values.
    pub resources: Vec<crate::component_index::SpecResource>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_value_string_round_trip() {
        let val = ParamValue::String("22.17.1".to_string());
        let json = serde_json::to_string(&val).unwrap();
        let parsed: ParamValue = serde_json::from_str(&json).unwrap();
        assert_eq!(val, parsed);
    }

    #[test]
    fn param_value_source_round_trip() {
        let val = ParamValue::Source(SourceParamValue {
            kind: FsSourceKind::HomeRelative,
            path: "~/dotfiles/git/.gitconfig".to_string(),
        });
        let json = serde_json::to_string(&val).unwrap();
        let parsed: ParamValue = serde_json::from_str(&json).unwrap();
        assert_eq!(val, parsed);
    }

    #[test]
    fn resolved_params_default_is_empty() {
        let rp = ResolvedParams::default();
        assert!(rp.values.is_empty());
    }

    #[test]
    fn params_schema_round_trip() {
        let schema = ParamsSchema {
            properties: {
                let mut m = HashMap::new();
                m.insert(
                    "version".to_string(),
                    ParamProperty {
                        param_type: ParamType::String,
                        default: Some(ParamValue::String("22.17.1".to_string())),
                    },
                );
                m
            },
            required: vec![],
            additional_properties: false,
        };
        let json = serde_json::to_string(&schema).unwrap();
        let parsed: ParamsSchema = serde_json::from_str(&json).unwrap();
        assert_eq!(schema, parsed);
    }

    #[test]
    fn param_type_enum_round_trip() {
        let prop = ParamProperty {
            param_type: ParamType::Enum {
                values: vec!["copy".to_string(), "link".to_string()],
            },
            default: None,
        };
        let json = serde_json::to_string(&prop).unwrap();
        let parsed: ParamProperty = serde_json::from_str(&json).unwrap();
        assert_eq!(prop, parsed);
    }

    #[test]
    fn materialized_component_spec_holds_resources() {
        use crate::component_index::{SpecResource, SpecResourceKind};
        let spec = MaterializedComponentSpec {
            resources: vec![SpecResource {
                id: "rt:node".to_string(),
                kind: SpecResourceKind::Runtime {
                    name: "node".to_string(),
                    version: "22.17.1".to_string(),
                },
            }],
        };
        assert_eq!(spec.resources.len(), 1);
    }
}
