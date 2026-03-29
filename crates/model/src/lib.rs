//! Shared data types for loadout.
//!
//! This crate defines the canonical data structures passed between pipeline stages.
//! It has no I/O dependencies; serialization uses serde derives only.

pub mod desired_resource_graph;
pub mod env;
pub mod error;
pub mod feature_index;
pub mod id;
pub mod plan;
pub mod profile;
pub mod sources;
pub mod state;
pub mod strategy;

pub use desired_resource_graph::DesiredResourceGraph;
pub use feature_index::FeatureIndex;
pub use id::{CanonicalBackendId, CanonicalFeatureId, ResolvedFeatureOrder, SourceId};
pub use plan::Plan;
pub use profile::Profile;
pub use sources::SourcesSpec;
pub use state::State;
pub use strategy::Strategy;
