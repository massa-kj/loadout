//! Profile data type.
//!
//! A profile declares intent: which components should be present and with what configuration.
//! Profile is one of three inputs to the planner (alongside state and strategy).
//!
//! See: `docs/specs/data/profile.md`

use crate::params::ParamValue;
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
    /// Parameter values to inject into the component's resource templates.
    /// Keys must match the component's `params_schema.properties`.
    /// Validated and resolved by the params validator before materialization.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<HashMap<String, ParamValue>>,
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
    fn round_trip_with_params() {
        let json = r#"{"components":{"node":{"params":{"version":"22.17.1"}}}}"#;
        let p: Profile = serde_json::from_str(json).unwrap();
        let params = p.components["node"].params.as_ref().unwrap();
        assert_eq!(
            params["version"],
            crate::params::ParamValue::String("22.17.1".to_string())
        );
    }

    #[test]
    fn empty_config_has_no_params() {
        let json = r#"{"components":{"git":{}}}"#;
        let p: Profile = serde_json::from_str(json).unwrap();
        assert!(p.components["git"].params.is_none());
    }
}
