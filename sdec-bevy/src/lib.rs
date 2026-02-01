//! Bevy adapter for sdec schema + delta encoding.

mod apply;
mod extract;
mod mapping;
mod metrics;
mod replicator;
mod schema;

pub use apply::{apply_changes, apply_delta_updates};
pub use extract::{extract_changes, BevyChangeSet};
pub use mapping::EntityMap;
pub use metrics::{EncodeMetrics, MetricsSink};
pub use replicator::BevyReplicator;
pub use schema::{BevySchema, BevySchemaBuilder, ReplicatedComponent, ReplicatedField};
