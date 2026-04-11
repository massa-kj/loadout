//! Profile data type.
//!
//! A profile declares intent: which components should be present and with what configuration.
//! Profile is one of three inputs to the planner (alongside state and strategy).
//!
//! See: `docs/specs/data/profile.md`

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// User-declared desired environment.
///
/// Keys are canonical component identifiers of the form `source_id/name`.
/// Normalization from grouping syntax (`source_id: { name: {} }`) happens
/// in the `config` crate before pipeline entry. Bare names and canonical
/// direct form are rejected at parse time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    /// Desired components and their per-component configuration.
    /// All keys are canonical IDs (`source_id/name`).
    pub components: HashMap<String, ProfileComponentConfig>,
}

/// Per-component configuration in a profile.
///
/// An empty map `{}` is equivalent to no configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ProfileComponentConfig {
    /// Desired version string. Interpretation is component-specific.
    /// Passed to the component script via `LOADOUT_COMPONENT_CONFIG_VERSION`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_minimal() {
        let yaml = r#"
components:
  git: {}
  bash: {}
"#;
        // Parse as Profile via serde_json to avoid serde_yaml dependency in model.
        // Full YAML round-trip is tested in the config crate.
        let json = r#"{"components":{"git":{},"bash":{}}}"#;
        let p: Profile = serde_json::from_str(json).unwrap();
        assert_eq!(p.components.len(), 2);
        assert!(p.components.contains_key("git"));
        let _ = yaml; // referenced to suppress unused warning
    }

    #[test]
    fn round_trip_with_version() {
        let json = r#"{"components":{"node":{"version":"22.17.1"}}}"#;
        let p: Profile = serde_json::from_str(json).unwrap();
        assert_eq!(p.components["node"].version.as_deref(), Some("22.17.1"));
    }
}
