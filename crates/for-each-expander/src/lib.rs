//! for_each expansion — expand templated resources over a params array.
//!
//! Pure-function crate: takes a `MaterializedComponentSpec` (with any `for_each` fields
//! still in place) and `ResolvedParams`, expands each `for_each` resource into one
//! `SpecResource` per array element, and returns an `ExpandedComponentSpec`.
//!
//! Placement in the pipeline:
//!   params-materializer → **for-each-expander** → compiler
//!
//! Invariants guaranteed by this crate:
//! - `for_each` is consumed here; no `for_each` field survives in the expanded output.
//! - Expanded resource `id` must be unique within the component (checked: `DuplicateId`).
//! - A `for_each` resource's `id` must contain `${item}` (checked: `ItemNotInId`).
//! - `${item}` placeholders in `id` and string-valued kind fields are replaced by each
//!   element value. Only `ParamValue::String` elements are supported; non-string elements
//!   produce `NonStringItem`.
//!
//! See `docs/specs/data/component_index.md` — for_each contract.

use model::component_index::{SpecResource, SpecResourceKind};
use model::params::{MaterializedComponentSpec, ParamValue, ResolvedParams};
use std::collections::HashSet;
use thiserror::Error;

/// Errors produced during for_each expansion.
#[derive(Debug, Error)]
pub enum ExpanderError {
    /// `for_each` references a params key that does not exist in the resolved params.
    #[error(
        "component '{component_id}': resource '{resource_id}': \
         for_each references unknown param key '{key}'"
    )]
    UnknownParam {
        component_id: String,
        resource_id: String,
        key: String,
    },

    /// The params value referenced by `for_each` is not an array.
    #[error(
        "component '{component_id}': resource '{resource_id}': \
         for_each param '{key}' is not an array"
    )]
    NotAnArray {
        component_id: String,
        resource_id: String,
        key: String,
    },

    /// A `for_each` resource id does not contain `${item}`.
    #[error(
        "component '{component_id}': resource '{resource_id}': \
         for_each resource id must contain '${{item}}'"
    )]
    ItemNotInId {
        component_id: String,
        resource_id: String,
    },

    /// Two expanded resources have the same id.
    #[error(
        "component '{component_id}': for_each expansion produced duplicate resource id '{id}'"
    )]
    DuplicateId { component_id: String, id: String },

    /// A `for_each` array element is not a string.
    #[error(
        "component '{component_id}': resource '{resource_id}': \
         for_each param '{key}' element [{index}] must be a string"
    )]
    NonStringItem {
        component_id: String,
        resource_id: String,
        key: String,
        index: usize,
    },
}

/// A `ComponentSpec` after `for_each` expansion.
///
/// All resources have `for_each = None`. Resource count may differ from the
/// pre-expansion `MaterializedComponentSpec` if any `for_each` resources were present.
#[derive(Debug, Clone, PartialEq)]
pub struct ExpandedComponentSpec {
    /// Resources with all `for_each` fields expanded.
    pub resources: Vec<SpecResource>,
}

/// Expand a component spec by processing all `for_each` resource declarations.
///
/// Resources without `for_each` are passed through unchanged (with `for_each` set to
/// `None`). Resources with `for_each` are expanded into one resource per array element.
///
/// # Errors
///
/// Returns `ExpanderError` if:
/// - A `for_each` key is not present in `resolved_params`.
/// - The referenced param value is not an array.
/// - The resource id does not contain `${item}`.
/// - An array element is not a string.
/// - Two expanded resources share the same id.
pub fn expand(
    component_id: &str,
    materialized: &MaterializedComponentSpec,
    resolved_params: &ResolvedParams,
) -> Result<ExpandedComponentSpec, ExpanderError> {
    let mut out: Vec<SpecResource> = Vec::with_capacity(materialized.resources.len());
    let mut seen_ids: HashSet<String> = HashSet::new();

    for resource in &materialized.resources {
        match &resource.for_each {
            None => {
                // Pass through unchanged, clearing for_each (already None).
                let r = SpecResource {
                    id: resource.id.clone(),
                    kind: resource.kind.clone(),
                    for_each: None,
                };
                if !seen_ids.insert(r.id.clone()) {
                    return Err(ExpanderError::DuplicateId {
                        component_id: component_id.to_string(),
                        id: r.id.clone(),
                    });
                }
                out.push(r);
            }
            Some(param_path) => {
                // Validate that id contains ${item}.
                if !resource.id.contains("${item}") {
                    return Err(ExpanderError::ItemNotInId {
                        component_id: component_id.to_string(),
                        resource_id: resource.id.clone(),
                    });
                }

                // Resolve "params.X" → look up key "X" in resolved_params.
                let key = strip_params_prefix(param_path);
                let param_value =
                    resolved_params
                        .values
                        .get(key)
                        .ok_or_else(|| ExpanderError::UnknownParam {
                            component_id: component_id.to_string(),
                            resource_id: resource.id.clone(),
                            key: key.to_string(),
                        })?;

                let elements = match param_value {
                    ParamValue::Array(elems) => elems,
                    _ => {
                        return Err(ExpanderError::NotAnArray {
                            component_id: component_id.to_string(),
                            resource_id: resource.id.clone(),
                            key: key.to_string(),
                        })
                    }
                };

                for (i, elem) in elements.iter().enumerate() {
                    let item_str = match elem {
                        ParamValue::String(s) => s.as_str(),
                        _ => {
                            return Err(ExpanderError::NonStringItem {
                                component_id: component_id.to_string(),
                                resource_id: resource.id.clone(),
                                key: key.to_string(),
                                index: i,
                            })
                        }
                    };

                    let expanded_id = replace_item(&resource.id, item_str);
                    if !seen_ids.insert(expanded_id.clone()) {
                        return Err(ExpanderError::DuplicateId {
                            component_id: component_id.to_string(),
                            id: expanded_id,
                        });
                    }

                    let expanded_kind = expand_kind(&resource.kind, item_str);
                    out.push(SpecResource {
                        id: expanded_id,
                        kind: expanded_kind,
                        for_each: None, // consumed
                    });
                }
            }
        }
    }

    Ok(ExpandedComponentSpec { resources: out })
}

/// Replace all occurrences of `${item}` in `s` with `value`.
fn replace_item(s: &str, value: &str) -> String {
    s.replace("${item}", value)
}

/// Strip the `params.` prefix from a for_each path like `"params.versions"` → `"versions"`.
fn strip_params_prefix(path: &str) -> &str {
    path.strip_prefix("params.").unwrap_or(path)
}

/// Expand `${item}` in all string fields of a `SpecResourceKind`.
fn expand_kind(kind: &SpecResourceKind, item: &str) -> SpecResourceKind {
    match kind {
        SpecResourceKind::Package { name, version } => SpecResourceKind::Package {
            name: replace_item(name, item),
            version: version.as_deref().map(|v| replace_item(v, item)),
        },
        SpecResourceKind::Runtime { name, version } => SpecResourceKind::Runtime {
            name: replace_item(name, item),
            version: replace_item(version, item),
        },
        SpecResourceKind::Fs {
            source,
            path,
            entry_type,
            op,
        } => SpecResourceKind::Fs {
            source: source.as_deref().map(|s| replace_item(s, item)),
            path: replace_item(path, item),
            entry_type: entry_type.clone(),
            op: op.clone(),
        },
        SpecResourceKind::Tool { name, verify } => SpecResourceKind::Tool {
            name: replace_item(name, item),
            verify: verify.clone(),
        },
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use model::component_index::{FsOp, SpecFsEntryType};
    use model::params::{MaterializedComponentSpec, ParamValue, ResolvedParams};
    use std::collections::HashMap;

    fn resolved(pairs: &[(&str, ParamValue)]) -> ResolvedParams {
        ResolvedParams {
            values: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect::<HashMap<_, _>>(),
        }
    }

    fn array_param(items: &[&str]) -> ParamValue {
        ParamValue::Array(
            items
                .iter()
                .map(|s| ParamValue::String(s.to_string()))
                .collect(),
        )
    }

    fn runtime_resource(
        id: &str,
        name: &str,
        version: &str,
        for_each: Option<&str>,
    ) -> SpecResource {
        SpecResource {
            id: id.to_string(),
            kind: SpecResourceKind::Runtime {
                name: name.to_string(),
                version: version.to_string(),
            },
            for_each: for_each.map(str::to_string),
        }
    }

    fn package_resource(id: &str, name: &str, for_each: Option<&str>) -> SpecResource {
        SpecResource {
            id: id.to_string(),
            kind: SpecResourceKind::Package {
                name: name.to_string(),
                version: None,
            },
            for_each: for_each.map(str::to_string),
        }
    }

    // ── Pass-through (no for_each) ────────────────────────────────────────

    #[test]
    fn no_for_each_passes_through() {
        let spec = MaterializedComponentSpec {
            resources: vec![runtime_resource("rt:node", "node", "22.17.1", None)],
        };
        let params = ResolvedParams::default();
        let expanded = expand("core/node", &spec, &params).unwrap();
        assert_eq!(expanded.resources.len(), 1);
        assert_eq!(expanded.resources[0].id, "rt:node");
        assert!(expanded.resources[0].for_each.is_none());
    }

    #[test]
    fn empty_resources_ok() {
        let spec = MaterializedComponentSpec { resources: vec![] };
        let params = ResolvedParams::default();
        let expanded = expand("core/empty", &spec, &params).unwrap();
        assert!(expanded.resources.is_empty());
    }

    // ── for_each expansion ────────────────────────────────────────────────

    #[test]
    fn expands_runtime_over_array() {
        let spec = MaterializedComponentSpec {
            resources: vec![runtime_resource(
                "rt:node@${item}",
                "node",
                "${item}",
                Some("params.versions"),
            )],
        };
        let params = resolved(&[("versions", array_param(&["18.20.0", "22.17.1"]))]);
        let expanded = expand("core/node", &spec, &params).unwrap();
        assert_eq!(expanded.resources.len(), 2);
        assert_eq!(expanded.resources[0].id, "rt:node@18.20.0");
        assert_eq!(expanded.resources[1].id, "rt:node@22.17.1");
        match &expanded.resources[0].kind {
            SpecResourceKind::Runtime { version, .. } => assert_eq!(version, "18.20.0"),
            _ => panic!("expected Runtime"),
        }
        match &expanded.resources[1].kind {
            SpecResourceKind::Runtime { version, .. } => assert_eq!(version, "22.17.1"),
            _ => panic!("expected Runtime"),
        }
    }

    #[test]
    fn single_element_array_yields_one_resource() {
        let spec = MaterializedComponentSpec {
            resources: vec![runtime_resource(
                "rt:node@${item}",
                "node",
                "${item}",
                Some("params.versions"),
            )],
        };
        let params = resolved(&[("versions", array_param(&["20.0.0"]))]);
        let expanded = expand("core/node", &spec, &params).unwrap();
        assert_eq!(expanded.resources.len(), 1);
        assert_eq!(expanded.resources[0].id, "rt:node@20.0.0");
    }

    #[test]
    fn empty_array_yields_zero_resources() {
        let spec = MaterializedComponentSpec {
            resources: vec![runtime_resource(
                "rt:node@${item}",
                "node",
                "${item}",
                Some("params.versions"),
            )],
        };
        let params = resolved(&[("versions", ParamValue::Array(vec![]))]);
        let expanded = expand("core/node", &spec, &params).unwrap();
        assert!(expanded.resources.is_empty());
    }

    #[test]
    fn mixed_for_each_and_plain_resources() {
        let spec = MaterializedComponentSpec {
            resources: vec![
                package_resource("pkg:git", "git", None),
                runtime_resource(
                    "rt:node@${item}",
                    "node",
                    "${item}",
                    Some("params.versions"),
                ),
            ],
        };
        let params = resolved(&[("versions", array_param(&["18.0", "22.0"]))]);
        let expanded = expand("core/mixed", &spec, &params).unwrap();
        assert_eq!(expanded.resources.len(), 3);
        assert_eq!(expanded.resources[0].id, "pkg:git");
        assert_eq!(expanded.resources[1].id, "rt:node@18.0");
        assert_eq!(expanded.resources[2].id, "rt:node@22.0");
    }

    #[test]
    fn for_each_consumes_field_in_output() {
        let spec = MaterializedComponentSpec {
            resources: vec![runtime_resource(
                "rt:node@${item}",
                "node",
                "${item}",
                Some("params.versions"),
            )],
        };
        let params = resolved(&[("versions", array_param(&["20.0"]))]);
        let expanded = expand("core/node", &spec, &params).unwrap();
        assert!(expanded.resources[0].for_each.is_none());
    }

    // ── Error paths ───────────────────────────────────────────────────────

    #[test]
    fn item_not_in_id_error() {
        let spec = MaterializedComponentSpec {
            resources: vec![runtime_resource(
                "rt:node", // missing ${item}
                "node",
                "${item}",
                Some("params.versions"),
            )],
        };
        let params = resolved(&[("versions", array_param(&["20.0"]))]);
        let err = expand("core/node", &spec, &params).unwrap_err();
        assert!(matches!(err, ExpanderError::ItemNotInId { .. }));
    }

    #[test]
    fn unknown_param_key_error() {
        let spec = MaterializedComponentSpec {
            resources: vec![runtime_resource(
                "rt:node@${item}",
                "node",
                "${item}",
                Some("params.typo"),
            )],
        };
        let params = resolved(&[("versions", array_param(&["20.0"]))]);
        let err = expand("core/node", &spec, &params).unwrap_err();
        assert!(matches!(err, ExpanderError::UnknownParam { ref key, .. } if key == "typo"));
    }

    #[test]
    fn not_an_array_error() {
        let spec = MaterializedComponentSpec {
            resources: vec![runtime_resource(
                "rt:node@${item}",
                "node",
                "${item}",
                Some("params.versions"),
            )],
        };
        let params = resolved(&[("versions", ParamValue::String("not-an-array".into()))]);
        let err = expand("core/node", &spec, &params).unwrap_err();
        assert!(matches!(err, ExpanderError::NotAnArray { .. }));
    }

    #[test]
    fn non_string_item_error() {
        let spec = MaterializedComponentSpec {
            resources: vec![runtime_resource(
                "rt:node@${item}",
                "node",
                "${item}",
                Some("params.versions"),
            )],
        };
        let params = resolved(&[(
            "versions",
            ParamValue::Array(vec![
                ParamValue::String("20.0".into()),
                ParamValue::Array(vec![]), // nested array — invalid item type
            ]),
        )]);
        let err = expand("core/node", &spec, &params).unwrap_err();
        assert!(
            matches!(err, ExpanderError::NonStringItem { index: 1, .. }),
            "expected NonStringItem at index 1"
        );
    }

    #[test]
    fn duplicate_id_error() {
        // Two for_each resources expanding to the same id
        let spec = MaterializedComponentSpec {
            resources: vec![
                runtime_resource("rt:node@${item}", "node", "${item}", Some("params.v1")),
                runtime_resource("rt:node@${item}", "node", "${item}", Some("params.v2")),
            ],
        };
        let params = resolved(&[
            ("v1", array_param(&["20.0"])),
            ("v2", array_param(&["20.0"])), // same expanded id
        ]);
        let err = expand("core/node", &spec, &params).unwrap_err();
        assert!(matches!(err, ExpanderError::DuplicateId { .. }));
    }

    #[test]
    fn fs_resource_item_replaced_in_path() {
        let spec = MaterializedComponentSpec {
            resources: vec![SpecResource {
                id: "fs:config-${item}".to_string(),
                kind: SpecResourceKind::Fs {
                    source: None,
                    path: "~/.config/${item}/rc".to_string(),
                    entry_type: SpecFsEntryType::File,
                    op: FsOp::Link,
                },
                for_each: Some("params.tools".to_string()),
            }],
        };
        let params = resolved(&[("tools", array_param(&["bash", "zsh"]))]);
        let expanded = expand("core/shells", &spec, &params).unwrap();
        assert_eq!(expanded.resources.len(), 2);
        match &expanded.resources[0].kind {
            SpecResourceKind::Fs { path, .. } => assert_eq!(path, "~/.config/bash/rc"),
            _ => panic!("expected Fs"),
        }
        match &expanded.resources[1].kind {
            SpecResourceKind::Fs { path, .. } => assert_eq!(path, "~/.config/zsh/rc"),
            _ => panic!("expected Fs"),
        }
    }
}
