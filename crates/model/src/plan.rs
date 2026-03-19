//! Plan data types.
//!
//! A Plan is the output of the Planner and the only authoritative instruction set
//! consumed by the Executor. The Executor must not re-classify or modify plan decisions.
//!
//! See: `docs/specs/algorithms/planner.md`

use serde::{Deserialize, Serialize};
use crate::id::CanonicalFeatureId;

/// Ordered set of actions the Executor must apply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    /// Ordered list of operations to execute (create/destroy/replace/strengthen).
    pub actions: Vec<PlanAction>,

    /// Features already in the correct state; not included in `actions`.
    #[serde(default)]
    pub noops: Vec<NoopEntry>,

    /// Features skipped due to planner-level classification (unknown kind, invariant).
    #[serde(default)]
    pub blocked: Vec<BlockedEntry>,

    /// Count of each operation type.
    pub summary: PlanSummary,
}

/// A single planned operation for a feature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanAction {
    /// Canonical feature ID this action applies to.
    pub feature: CanonicalFeatureId,

    /// Type of operation to perform.
    pub operation: Operation,

    /// Optional operation-specific details. Present for `replace` and `strengthen`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<ActionDetails>,
}

/// Type of operation in a plan action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Create,
    Destroy,
    Replace,
    ReplaceBackend,
    Strengthen,
}

/// Operation-specific detail payload.
///
/// Deserialized by matching field names (untagged).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ActionDetails {
    /// Details for a `strengthen` action.
    Strengthen(StrengthenDetails),
    /// Details for a `replace` or `replace_backend` action.
    Replace(ReplaceDetails),
}

/// Details for a `replace`/`replace_backend` action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ReplaceDetails {
    /// Version being replaced (if applicable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_version: Option<String>,

    /// Version being installed (if applicable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_version: Option<String>,
}

/// Details for a `strengthen` action.
///
/// The Executor reads `add_resources` to determine which resources to install
/// without re-reading `desired_resource_graph` directly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrengthenDetails {
    /// Resources to add (those present in desired but absent in current state).
    pub add_resources: Vec<ResourceRef>,
}

/// Lightweight reference to a resource within a feature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceRef {
    /// Resource kind string (e.g. `package`, `runtime`, `fs`).
    pub kind: String,

    /// Resource stable identifier.
    pub id: String,
}

/// Record of a feature that required no changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoopEntry {
    pub feature: CanonicalFeatureId,
}

/// Record of a feature that was blocked by the planner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockedEntry {
    pub feature: CanonicalFeatureId,
    pub reason: String,
}

/// Per-operation count summary for display and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PlanSummary {
    pub create: u32,
    pub destroy: u32,
    pub replace: u32,
    pub replace_backend: u32,
    pub strengthen: u32,
    pub blocked: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_plan() {
        let json = r#"{
            "actions": [
                { "feature": "core/git", "operation": "create" },
                {
                    "feature": "core/node",
                    "operation": "replace",
                    "details": { "from_version": "18", "to_version": "20" }
                },
                {
                    "feature": "core/git",
                    "operation": "strengthen",
                    "details": {
                        "add_resources": [{ "kind": "fs", "id": "fs:gitconfig" }]
                    }
                }
            ],
            "noops": [{ "feature": "core/bash" }],
            "blocked": [{ "feature": "user/legacy", "reason": "unknown resource kind: registry" }],
            "summary": {
                "create": 1, "destroy": 0, "replace": 1,
                "replace_backend": 0, "strengthen": 1, "blocked": 1
            }
        }"#;
        let plan: Plan = serde_json::from_str(json).unwrap();
        assert_eq!(plan.actions.len(), 3);
        assert_eq!(plan.actions[0].operation, Operation::Create);
        assert_eq!(plan.actions[1].operation, Operation::Replace);

        match &plan.actions[1].details {
            Some(ActionDetails::Replace(d)) => {
                assert_eq!(d.from_version.as_deref(), Some("18"));
                assert_eq!(d.to_version.as_deref(), Some("20"));
            }
            _ => panic!("expected replace details"),
        }

        match &plan.actions[2].details {
            Some(ActionDetails::Strengthen(d)) => {
                assert_eq!(d.add_resources.len(), 1);
                assert_eq!(d.add_resources[0].id, "fs:gitconfig");
            }
            _ => panic!("expected strengthen details"),
        }

        assert_eq!(plan.noops[0].feature.as_str(), "core/bash");
        assert_eq!(plan.blocked[0].reason, "unknown resource kind: registry");
        assert_eq!(plan.summary.create, 1);
    }
}
