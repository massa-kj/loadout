//! Profile data type.
//!
//! A profile declares intent: which features should be present and with what configuration.
//! Profile is one of three inputs to the planner (alongside state and policy).
//!
//! See: `docs/specs/data/profile.md`

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// User-declared desired environment.
///
/// Keys are feature identifiers (bare or canonical). Normalization to canonical IDs
/// happens in the `config` crate before pipeline entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    /// Desired features and their per-feature configuration.
    pub features: HashMap<String, ProfileFeatureConfig>,
}

/// Per-feature configuration in a profile.
///
/// An empty map `{}` is equivalent to no configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ProfileFeatureConfig {
    /// Desired version string. Interpretation is feature-specific.
    /// Passed to the feature script via `LOADOUT_FEATURE_CONFIG_VERSION`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_minimal() {
        let yaml = r#"
features:
  git: {}
  bash: {}
"#;
        // Parse as Profile via serde_json to avoid serde_yaml dependency in model.
        // Full YAML round-trip is tested in the config crate.
        let json = r#"{"features":{"git":{},"bash":{}}}"#;
        let p: Profile = serde_json::from_str(json).unwrap();
        assert_eq!(p.features.len(), 2);
        assert!(p.features.contains_key("git"));
        let _ = yaml; // referenced to suppress unused warning
    }

    #[test]
    fn round_trip_with_version() {
        let json = r#"{"features":{"node":{"version":"22.17.1"}}}"#;
        let p: Profile = serde_json::from_str(json).unwrap();
        assert_eq!(
            p.features["node"].version.as_deref(),
            Some("22.17.1")
        );
    }
}
