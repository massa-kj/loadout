//! Strategy data type.
//!
//! A strategy declares which backend to use for each resource, via a rule-based
//! selector model. Rules are evaluated against every resource; the most specific
//! matching rule wins, with ties broken by rule order (last wins).
//!
//! Key types:
//! - [`Strategy`] — top-level strategy section in a config file
//! - [`StrategyGroup`] — static named set of resources, keyed by kind
//! - [`StrategyRule`] — single backend selection rule
//! - [`MatchSelector`] — AND-predicate selector within a rule
//! - [`MatchKind`] — resource kind discriminant (`package` | `runtime`)
//! - [`Specificity`] — fixed-priority comparison tuple for rule ranking
//!
//! See: `docs/specs/data/strategy.md`

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Top-level strategy
// ---------------------------------------------------------------------------

/// User-declared implementation strategy embedded in a config file.
///
/// `strategy` (the identifier label) is optional metadata not used by core logic.
/// `fs` settings are independent of the rule-based backend selection.
///
/// See: `docs/specs/data/strategy.md`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Strategy {
    /// Optional strategy identifier label (not used by core logic).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,

    /// Named resource groups for use in rule selectors.
    ///
    /// Groups are static enumerations of resource names, keyed by kind string.
    /// Condition expressions and version constraints are not permitted.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub groups: HashMap<String, StrategyGroup>,

    /// Ordered list of backend selection rules.
    ///
    /// All rules are evaluated against each resource. The most specific match
    /// wins. Equal specificity is broken by position: the last matching rule
    /// (highest index) wins.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<StrategyRule>,

    /// Filesystem operation settings (independent of backend selection).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fs: Option<FsStrategy>,
}

// ---------------------------------------------------------------------------
// Groups
// ---------------------------------------------------------------------------

/// A named, static set of resources keyed by resource kind.
///
/// Each key is a kind string (`"package"` or `"runtime"`), and the value is a
/// list of resource names belonging to that group.
///
/// Groups are purely enumerative. Condition expressions, globs, regexes, and
/// version constraints are forbidden. If filtering logic is needed in the
/// future, a separate concept should be introduced rather than extending groups.
///
/// ```yaml
/// groups:
///   npm_global:
///     package:
///       - eslint
///       - prettier
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct StrategyGroup(pub HashMap<String, Vec<String>>);

impl StrategyGroup {
    /// Returns the resource names registered for the given kind, if any.
    pub fn names_for_kind(&self, kind: &str) -> Option<&[String]> {
        self.0.get(kind).map(Vec::as_slice)
    }
}

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

/// A single backend selection rule.
///
/// ```yaml
/// - match:
///     kind: package
///     name: git
///   use: core/apt
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrategyRule {
    /// Selector that determines which resources this rule applies to.
    ///
    /// All specified fields are ANDed. An empty selector matches everything.
    #[serde(rename = "match")]
    pub selector: MatchSelector,

    /// Backend identifier to use when this rule matches.
    ///
    /// Accepts bare names (normalized to `core/<name>`) or canonical IDs
    /// (`core/brew`, `local/custom`).
    #[serde(rename = "use")]
    pub use_backend: String,
}

// ---------------------------------------------------------------------------
// Selector
// ---------------------------------------------------------------------------

/// Predicate selector for a strategy rule.
///
/// All non-`None` fields are ANDed. A resource must satisfy every specified
/// field for the rule to match. An all-`None` selector matches every resource.
///
/// Constraints:
/// - `kind` must be `package` or `runtime` (never `fs` or `tool`).
/// - When `component` is specified, `kind` is required.
/// - `group` membership is evaluated against the named group in `Strategy::groups`.
///
/// See: `docs/specs/data/strategy.md`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MatchSelector {
    /// Resource kind to match. Required when `component` is specified.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<MatchKind>,

    /// Exact resource name to match (e.g. `"git"`, `"node"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Canonical component ID to match (e.g. `"core/cli-tools"`).
    ///
    /// When present, `kind` is required to prevent accidental cross-kind matches.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,

    /// Group name to match. The resource must be listed in `Strategy::groups[group][kind]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

impl MatchSelector {
    /// Returns `true` if this selector matches the given resource attributes.
    ///
    /// `group_member` must be pre-computed by the caller from `Strategy::groups`.
    /// All specified fields are ANDed; an all-`None` selector always returns `true`.
    pub fn matches(
        &self,
        resource_kind: &MatchKind,
        resource_name: &str,
        resource_component: &str,
        group_member: bool,
    ) -> bool {
        if let Some(ref k) = self.kind {
            if k != resource_kind {
                return false;
            }
        }
        if let Some(ref n) = self.name {
            if n != resource_name {
                return false;
            }
        }
        if let Some(ref c) = self.component {
            if c != resource_component {
                return false;
            }
        }
        if self.group.is_some() && !group_member {
            return false;
        }
        true
    }

    /// Computes the fixed-priority specificity vector for this selector.
    ///
    /// Returns `(has_component, has_kind, has_name, has_group)`. Tuples compare
    /// lexicographically, so `component` dominates `kind`, which dominates `name`,
    /// which dominates `group`. Equal specificity is broken by rule index (last wins).
    ///
    /// See: `docs/specs/data/strategy.md`
    pub fn specificity(&self) -> Specificity {
        (
            u8::from(self.component.is_some()),
            u8::from(self.kind.is_some()),
            u8::from(self.name.is_some()),
            u8::from(self.group.is_some()),
        )
    }
}

// ---------------------------------------------------------------------------
// Kind discriminant
// ---------------------------------------------------------------------------

/// Resource kind discriminant for match selectors.
///
/// Only `package` and `runtime` participate in backend resolution.
/// `fs` and `tool` do not have backends and are forbidden in rule selectors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchKind {
    Package,
    Runtime,
}

impl MatchKind {
    /// Returns the kind as a static string, matching serde's `rename_all = "snake_case"`.
    ///
    /// Used to index into `StrategyGroup` entries.
    pub fn as_str(&self) -> &'static str {
        match self {
            MatchKind::Package => "package",
            MatchKind::Runtime => "runtime",
        }
    }
}

// ---------------------------------------------------------------------------
// Specificity
// ---------------------------------------------------------------------------

/// Fixed-priority specificity vector for backend rule selection.
///
/// Fields correspond to `(has_component, has_kind, has_name, has_group)`.
/// Each field is `1` when the selector specifies that axis, `0` otherwise.
/// Standard tuple ordering gives lexicographic comparison, so `component`
/// always outranks `kind`, `kind` always outranks `name`, and `name` always
/// outranks `group` — regardless of how many other axes are set.
///
/// This is deliberately a type alias over a primitive tuple so that `PartialOrd`
/// and `Ord` are derived automatically with the correct semantics.
///
/// See: `docs/specs/data/strategy.md`
pub type Specificity = (u8, u8, u8, u8);

/// Filesystem operation strategy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct FsStrategy {
    /// Whether to back up existing files before overwriting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<bool>,

    /// Directory where backups are stored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_dir: Option<String>,

    /// Controls which `copy` sources are fingerprinted for noop detection.
    ///
    /// - `managed_only` — only `component_relative` sources (loadout-managed assets).
    /// - `all_copy` (default) — all source kinds when `op = copy`.
    /// - `none` — disable fingerprinting entirely.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint_policy: Option<FingerprintPolicy>,
}

/// Policy controlling which `copy` sources the materializer fingerprints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FingerprintPolicy {
    /// Fingerprint only `component_relative` sources.
    ManagedOnly,
    /// Fingerprint all source kinds when `op = copy` (default).
    #[default]
    AllCopy,
    /// Disable fingerprinting entirely.
    None,
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- StrategyGroup ------------------------------------------------------

    #[test]
    fn group_names_for_kind_returns_slice() {
        let mut inner = HashMap::new();
        inner.insert(
            "package".to_string(),
            vec!["eslint".to_string(), "prettier".to_string()],
        );
        let group = StrategyGroup(inner);
        assert_eq!(
            group.names_for_kind("package"),
            Some(["eslint".to_string(), "prettier".to_string()].as_slice())
        );
        assert_eq!(group.names_for_kind("runtime"), None);
    }

    #[test]
    fn group_round_trip_yaml() {
        let yaml = "package:\n  - eslint\n  - prettier\n";
        let group: StrategyGroup = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            group.names_for_kind("package"),
            Some(["eslint".to_string(), "prettier".to_string()].as_slice())
        );
        let out = serde_yaml::to_string(&group).unwrap();
        let back: StrategyGroup = serde_yaml::from_str(&out).unwrap();
        assert_eq!(group, back);
    }

    // --- MatchKind ----------------------------------------------------------

    #[test]
    fn match_kind_as_str() {
        assert_eq!(MatchKind::Package.as_str(), "package");
        assert_eq!(MatchKind::Runtime.as_str(), "runtime");
    }

    #[test]
    fn match_kind_serde() {
        let yaml = "package";
        let k: MatchKind = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(k, MatchKind::Package);
        let out = serde_yaml::to_string(&MatchKind::Runtime).unwrap();
        assert!(out.contains("runtime"));
    }

    // --- MatchSelector::specificity -----------------------------------------

    #[test]
    fn specificity_all_none_is_zero() {
        let s = MatchSelector::default();
        assert_eq!(s.specificity(), (0, 0, 0, 0));
    }

    #[test]
    fn specificity_kind_only() {
        let s = MatchSelector {
            kind: Some(MatchKind::Package),
            ..Default::default()
        };
        assert_eq!(s.specificity(), (0, 1, 0, 0));
    }

    #[test]
    fn specificity_component_dominates_name() {
        let with_component = MatchSelector {
            component: Some("core/cli-tools".to_string()),
            kind: Some(MatchKind::Package),
            ..Default::default()
        };
        let with_name = MatchSelector {
            kind: Some(MatchKind::Package),
            name: Some("git".to_string()),
            ..Default::default()
        };
        assert!(with_component.specificity() > with_name.specificity());
    }

    #[test]
    fn specificity_full_selector() {
        let s = MatchSelector {
            component: Some("core/cli-tools".to_string()),
            kind: Some(MatchKind::Package),
            name: Some("eslint".to_string()),
            group: Some("npm_global".to_string()),
        };
        assert_eq!(s.specificity(), (1, 1, 1, 1));
    }

    // --- MatchSelector::matches ---------------------------------------------

    #[test]
    fn matches_empty_selector_matches_everything() {
        let s = MatchSelector::default();
        assert!(s.matches(&MatchKind::Package, "git", "core/cli-tools", false));
        assert!(s.matches(&MatchKind::Runtime, "node", "core/node", true));
    }

    #[test]
    fn matches_kind_filters_correctly() {
        let s = MatchSelector {
            kind: Some(MatchKind::Package),
            ..Default::default()
        };
        assert!(s.matches(&MatchKind::Package, "git", "core/git", false));
        assert!(!s.matches(&MatchKind::Runtime, "node", "core/node", false));
    }

    #[test]
    fn matches_name_filters_correctly() {
        let s = MatchSelector {
            name: Some("git".to_string()),
            ..Default::default()
        };
        assert!(s.matches(&MatchKind::Package, "git", "core/git", false));
        assert!(!s.matches(&MatchKind::Package, "ripgrep", "core/rg", false));
    }

    #[test]
    fn matches_component_filters_correctly() {
        let s = MatchSelector {
            component: Some("core/cli-tools".to_string()),
            kind: Some(MatchKind::Package),
            ..Default::default()
        };
        assert!(s.matches(&MatchKind::Package, "eslint", "core/cli-tools", false));
        assert!(!s.matches(&MatchKind::Package, "eslint", "core/other", false));
    }

    #[test]
    fn matches_group_requires_member() {
        let s = MatchSelector {
            kind: Some(MatchKind::Package),
            group: Some("npm_global".to_string()),
            ..Default::default()
        };
        assert!(s.matches(&MatchKind::Package, "eslint", "core/node", true));
        assert!(!s.matches(&MatchKind::Package, "eslint", "core/node", false));
    }

    // --- StrategyRule round-trip --------------------------------------------

    #[test]
    fn rule_round_trip_yaml() {
        let yaml = "match:\n  kind: package\n  name: git\nuse: core/apt\n";
        let rule: StrategyRule = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(rule.selector.kind, Some(MatchKind::Package));
        assert_eq!(rule.selector.name.as_deref(), Some("git"));
        assert_eq!(rule.use_backend, "core/apt");
    }

    // --- Strategy round-trip ------------------------------------------------

    #[test]
    fn strategy_round_trip_yaml_full() {
        let yaml = r#"
strategy: linux-default
groups:
  npm_global:
    package:
      - eslint
      - prettier
rules:
  - match:
      kind: package
      name: git
    use: core/apt
  - match:
      kind: package
      group: npm_global
    use: core/npm
  - match:
      kind: runtime
    use: core/mise
fs:
  backup: true
  backup_dir: ~/.backup/loadout
"#;
        let s: Strategy = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(s.strategy.as_deref(), Some("linux-default"));
        assert_eq!(s.rules.len(), 3);
        assert_eq!(s.rules[0].use_backend, "core/apt");
        assert_eq!(s.rules[2].selector.kind, Some(MatchKind::Runtime));
        assert!(s.groups.contains_key("npm_global"));
        assert_eq!(s.fs.as_ref().unwrap().backup, Some(true));

        // round-trip
        let out = serde_yaml::to_string(&s).unwrap();
        let back: Strategy = serde_yaml::from_str(&out).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn strategy_empty_round_trip() {
        let yaml = "{}";
        let s: Strategy = serde_yaml::from_str(yaml).unwrap();
        assert!(s.strategy.is_none());
        assert!(s.groups.is_empty());
        assert!(s.rules.is_empty());
        assert!(s.fs.is_none());
    }

    // --- FingerprintPolicy (unchanged) --------------------------------------

    #[test]
    fn fingerprint_policy_serde() {
        let json = r#"{"fs": {"fingerprint_policy": "managed_only"}}"#;
        let s: Strategy = serde_json::from_str(json).unwrap();
        assert_eq!(
            s.fs.unwrap().fingerprint_policy,
            Some(FingerprintPolicy::ManagedOnly)
        );
    }

    #[test]
    fn fingerprint_policy_default_is_all_copy() {
        assert_eq!(FingerprintPolicy::default(), FingerprintPolicy::AllCopy);
    }

    #[test]
    fn fingerprint_policy_none_serde() {
        let json = r#"{"fs": {"fingerprint_policy": "none"}}"#;
        let s: Strategy = serde_json::from_str(json).unwrap();
        assert_eq!(
            s.fs.unwrap().fingerprint_policy,
            Some(FingerprintPolicy::None)
        );
    }
}
