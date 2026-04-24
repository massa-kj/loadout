//! Parameter materialization — resolve `${params.*}` references in component specs.
//!
//! Pure-function crate: takes a `ComponentSpec` (template) and `ResolvedParams`,
//! replaces all `${params.<key>}` placeholders, and produces a `MaterializedComponentSpec`.
//!
//! See: `tmp/20260417_parameter.md` — Phase 5

use model::component_index::{SpecResource, SpecResourceKind};
use model::params::{MaterializedComponentSpec, ParamValue, ResolvedParams};
use thiserror::Error;

/// Errors produced during parameter materialization.
#[derive(Debug, Error)]
pub enum MaterializeError {
    /// A `${params.<key>}` reference could not be resolved.
    #[error(
        "component '{component_id}': resource '{resource_id}' field '{field}' \
         references unresolved param '{key}'"
    )]
    UnresolvedParam {
        component_id: String,
        resource_id: String,
        field: String,
        key: String,
    },

    /// A param value is invalid for the field it targets.
    #[error("component '{component_id}': resource '{resource_id}' field '{field}': {reason}")]
    InvalidParamValue {
        component_id: String,
        resource_id: String,
        field: String,
        reason: String,
    },
}

/// Prefix for param template references.
const PARAM_PREFIX: &str = "${params.";
/// Suffix for param template references.
const PARAM_SUFFIX: &str = "}";

/// Materialize a component spec by resolving all `${params.*}` references.
///
/// Resources are cloned with string fields replaced. Resource count, kind, and id
/// are never changed (structural invariance).
///
/// If `resolved_params` is empty, any remaining `${params.*}` references produce an error.
pub fn materialize(
    component_id: &str,
    resources: &[SpecResource],
    resolved_params: &ResolvedParams,
) -> Result<MaterializedComponentSpec, MaterializeError> {
    let mut out = Vec::with_capacity(resources.len());
    for resource in resources {
        let materialized = materialize_resource(component_id, resource, resolved_params)?;
        out.push(materialized);
    }
    Ok(MaterializedComponentSpec { resources: out })
}

/// Materialize a single resource.
fn materialize_resource(
    component_id: &str,
    resource: &SpecResource,
    params: &ResolvedParams,
) -> Result<SpecResource, MaterializeError> {
    let kind = match &resource.kind {
        SpecResourceKind::Runtime { name, version } => {
            let resolved_version =
                resolve_string(component_id, &resource.id, "version", version, params)?;
            SpecResourceKind::Runtime {
                name: name.clone(),
                version: resolved_version,
            }
        }
        SpecResourceKind::Package { name, version } => SpecResourceKind::Package {
            name: name.clone(),
            version: version.clone(),
        },
        SpecResourceKind::Fs {
            source,
            path,
            entry_type,
            op,
        } => {
            let resolved_path = resolve_string(component_id, &resource.id, "path", path, params)?;
            let resolved_source = match source {
                Some(s) => Some(resolve_string(
                    component_id,
                    &resource.id,
                    "source",
                    s,
                    params,
                )?),
                None => None,
            };
            SpecResourceKind::Fs {
                source: resolved_source,
                path: resolved_path,
                entry_type: entry_type.clone(),
                op: op.clone(),
            }
        }
        SpecResourceKind::Tool { name, verify } => SpecResourceKind::Tool {
            name: name.clone(),
            verify: verify.clone(),
        },
    };

    Ok(SpecResource {
        id: resource.id.clone(),
        kind,
        // for_each is passed through unchanged; the for-each-expander consumes it.
        for_each: resource.for_each.clone(),
    })
}

/// Resolve all `${params.<key>}` references in a string field.
///
/// Supports:
/// - Full replacement: the entire string is `${params.<key>}` → replaced with the param value
/// - Partial/embedded references: `prefix-${params.<key>}-suffix` → string interpolation
///
/// Only `ParamValue::String` values can be interpolated into string fields.
fn resolve_string(
    component_id: &str,
    resource_id: &str,
    field: &str,
    template: &str,
    params: &ResolvedParams,
) -> Result<String, MaterializeError> {
    // Fast path: no template references.
    if !template.contains(PARAM_PREFIX) {
        return Ok(template.to_string());
    }

    let mut result = template.to_string();

    // Iteratively replace all ${params.<key>} references.
    while let Some(start) = result.find(PARAM_PREFIX) {
        let after_prefix = start + PARAM_PREFIX.len();
        let end = result[after_prefix..].find(PARAM_SUFFIX).ok_or_else(|| {
            MaterializeError::InvalidParamValue {
                component_id: component_id.to_string(),
                resource_id: resource_id.to_string(),
                field: field.to_string(),
                reason: format!("malformed template reference: unclosed '{{' at position {start}"),
            }
        })?;
        let key = &result[after_prefix..after_prefix + end];

        let value = params
            .values
            .get(key)
            .ok_or_else(|| MaterializeError::UnresolvedParam {
                component_id: component_id.to_string(),
                resource_id: resource_id.to_string(),
                field: field.to_string(),
                key: key.to_string(),
            })?;

        let replacement = match value {
            ParamValue::String(s) => s.clone(),
            ParamValue::Source(_) => {
                return Err(MaterializeError::InvalidParamValue {
                    component_id: component_id.to_string(),
                    resource_id: resource_id.to_string(),
                    field: field.to_string(),
                    reason: format!(
                        "param '{key}' is a source object but field '{field}' expects a string"
                    ),
                });
            }
            ParamValue::Array(_) => {
                return Err(MaterializeError::InvalidParamValue {
                    component_id: component_id.to_string(),
                    resource_id: resource_id.to_string(),
                    field: field.to_string(),
                    reason: format!(
                        "param '{key}' is an array; use for_each to expand over an array"
                    ),
                });
            }
        };

        let pattern = format!("{PARAM_PREFIX}{key}{PARAM_SUFFIX}");
        result = result.replacen(&pattern, &replacement, 1);
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::component_index::{FsOp, SpecFsEntryType};
    use model::fs::FsSourceKind;
    use model::params::{ParamValue, SourceParamValue};

    fn resolved(entries: &[(&str, ParamValue)]) -> ResolvedParams {
        ResolvedParams {
            values: entries
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        }
    }

    // ── Happy paths ──────────────────────────────────────────────────────

    #[test]
    fn runtime_version_resolved() {
        let resources = vec![SpecResource {
            id: "rt:node".into(),
            kind: SpecResourceKind::Runtime {
                name: "node".into(),
                version: "${params.version}".into(),
            },
            for_each: None,
        }];
        let params = resolved(&[("version", ParamValue::String("22.17.1".into()))]);
        let mat = materialize("core/node", &resources, &params).unwrap();
        assert_eq!(mat.resources.len(), 1);
        match &mat.resources[0].kind {
            SpecResourceKind::Runtime { version, .. } => {
                assert_eq!(version, "22.17.1");
            }
            _ => panic!("expected Runtime"),
        }
    }

    #[test]
    fn fs_path_resolved() {
        let resources = vec![SpecResource {
            id: "fs:gitconfig".into(),
            kind: SpecResourceKind::Fs {
                source: None,
                path: "${params.path}".into(),
                entry_type: SpecFsEntryType::File,
                op: FsOp::Link,
            },
            for_each: None,
        }];
        let params = resolved(&[("path", ParamValue::String("~/.gitconfig".into()))]);
        let mat = materialize("core/git", &resources, &params).unwrap();
        match &mat.resources[0].kind {
            SpecResourceKind::Fs { path, .. } => {
                assert_eq!(path, "~/.gitconfig");
            }
            _ => panic!("expected Fs"),
        }
    }

    #[test]
    fn literal_values_pass_through() {
        let resources = vec![SpecResource {
            id: "pkg:jq".into(),
            kind: SpecResourceKind::Package {
                name: "jq".into(),
                version: None,
            },
            for_each: None,
        }];
        let params = ResolvedParams::default();
        let mat = materialize("core/jq", &resources, &params).unwrap();
        match &mat.resources[0].kind {
            SpecResourceKind::Package { name, .. } => assert_eq!(name, "jq"),
            _ => panic!("expected Package"),
        }
    }

    #[test]
    fn runtime_literal_version_passes_through() {
        let resources = vec![SpecResource {
            id: "rt:python".into(),
            kind: SpecResourceKind::Runtime {
                name: "python".into(),
                version: "3.12".into(),
            },
            for_each: None,
        }];
        let params = ResolvedParams::default();
        let mat = materialize("core/python", &resources, &params).unwrap();
        match &mat.resources[0].kind {
            SpecResourceKind::Runtime { version, .. } => assert_eq!(version, "3.12"),
            _ => panic!("expected Runtime"),
        }
    }

    #[test]
    fn embedded_param_in_string() {
        let resources = vec![SpecResource {
            id: "rt:node".into(),
            kind: SpecResourceKind::Runtime {
                name: "node".into(),
                version: "v${params.version}-lts".into(),
            },
            for_each: None,
        }];
        let params = resolved(&[("version", ParamValue::String("20".into()))]);
        let mat = materialize("core/node", &resources, &params).unwrap();
        match &mat.resources[0].kind {
            SpecResourceKind::Runtime { version, .. } => assert_eq!(version, "v20-lts"),
            _ => panic!("expected Runtime"),
        }
    }

    #[test]
    fn multiple_params_in_one_field() {
        let resources = vec![SpecResource {
            id: "rt:node".into(),
            kind: SpecResourceKind::Runtime {
                name: "node".into(),
                version: "${params.major}.${params.minor}".into(),
            },
            for_each: None,
        }];
        let params = resolved(&[
            ("major", ParamValue::String("22".into())),
            ("minor", ParamValue::String("17".into())),
        ]);
        let mat = materialize("core/node", &resources, &params).unwrap();
        match &mat.resources[0].kind {
            SpecResourceKind::Runtime { version, .. } => assert_eq!(version, "22.17"),
            _ => panic!("expected Runtime"),
        }
    }

    #[test]
    fn fs_source_string_resolved() {
        let resources = vec![SpecResource {
            id: "fs:gitconfig".into(),
            kind: SpecResourceKind::Fs {
                source: Some("${params.source_path}".into()),
                path: "~/.gitconfig".into(),
                entry_type: SpecFsEntryType::File,
                op: FsOp::Link,
            },
            for_each: None,
        }];
        let params = resolved(&[(
            "source_path",
            ParamValue::String("files/custom.conf".into()),
        )]);
        let mat = materialize("core/git", &resources, &params).unwrap();
        match &mat.resources[0].kind {
            SpecResourceKind::Fs { source, .. } => {
                assert_eq!(source.as_deref(), Some("files/custom.conf"));
            }
            _ => panic!("expected Fs"),
        }
    }

    // ── Error paths ──────────────────────────────────────────────────────

    #[test]
    fn unresolved_param_error() {
        let resources = vec![SpecResource {
            id: "rt:node".into(),
            kind: SpecResourceKind::Runtime {
                name: "node".into(),
                version: "${params.version}".into(),
            },
            for_each: None,
        }];
        let params = ResolvedParams::default();
        let err = materialize("core/node", &resources, &params).unwrap_err();
        assert!(
            matches!(err, MaterializeError::UnresolvedParam { ref key, .. } if key == "version")
        );
    }

    #[test]
    fn source_value_in_string_field_error() {
        let resources = vec![SpecResource {
            id: "rt:node".into(),
            kind: SpecResourceKind::Runtime {
                name: "node".into(),
                version: "${params.src}".into(),
            },
            for_each: None,
        }];
        let params = resolved(&[(
            "src",
            ParamValue::Source(SourceParamValue {
                kind: FsSourceKind::HomeRelative,
                path: "~/x".into(),
            }),
        )]);
        let err = materialize("core/node", &resources, &params).unwrap_err();
        assert!(matches!(err, MaterializeError::InvalidParamValue { .. }));
    }

    #[test]
    fn unclosed_template_error() {
        let resources = vec![SpecResource {
            id: "rt:node".into(),
            kind: SpecResourceKind::Runtime {
                name: "node".into(),
                version: "${params.version".into(),
            },
            for_each: None,
        }];
        let params = resolved(&[("version", ParamValue::String("20".into()))]);
        let err = materialize("core/node", &resources, &params).unwrap_err();
        assert!(matches!(err, MaterializeError::InvalidParamValue { .. }));
    }

    #[test]
    fn empty_params_no_templates_ok() {
        let resources = vec![
            SpecResource {
                id: "pkg:git".into(),
                kind: SpecResourceKind::Package {
                    name: "git".into(),
                    version: None,
                },
                for_each: None,
            },
            SpecResource {
                id: "rt:python".into(),
                kind: SpecResourceKind::Runtime {
                    name: "python".into(),
                    version: "3.12".into(),
                },
                for_each: None,
            },
        ];
        let params = ResolvedParams::default();
        let mat = materialize("core/git", &resources, &params).unwrap();
        assert_eq!(mat.resources.len(), 2);
    }
}
