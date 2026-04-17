//! Shared data types for loadout.
//!
//! This crate defines the canonical data structures passed between pipeline stages.
//! It has no I/O dependencies; serialization uses serde derives only.

pub mod component_index;
pub mod desired_resource_graph;
pub mod env;
pub mod error;
pub mod fs;
pub mod id;
pub mod plan;
pub mod profile;
pub mod sources;
pub mod state;
pub mod strategy;
pub mod tool;

pub use component_index::ComponentIndex;
pub use desired_resource_graph::DesiredResourceGraph;
pub use id::{CanonicalBackendId, CanonicalComponentId, ResolvedComponentOrder, SourceId};
pub use plan::Plan;
pub use profile::Profile;
pub use sources::SourcesSpec;
pub use state::State;
pub use strategy::Strategy;
