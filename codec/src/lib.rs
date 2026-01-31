//! Snapshot and delta encoding/decoding for the sdec codec.
//!
//! This is the main codec crate that ties together bitstream, wire, and schema
//! to provide full snapshot and delta encoding/decoding capabilities.
//!
//! # Features
//!
//! - Full snapshot encoding/decoding
//! - Delta encoding relative to a baseline
//! - Baseline history management
//! - Entity create/update/destroy operations
//! - Per-component and per-field change masks
//!
//! # Design Principles
//!
//! - **Correctness first** - All invariants are documented and tested.
//! - **No steady-state allocations** - Uses caller-provided buffers.
//! - **Deterministic** - Same inputs produce same outputs.

mod error;
mod types;

pub use error::{CodecError, CodecResult};
pub use types::{EntityId, SnapshotTick};
pub use wire::Limits;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_api_exports() {
        // Verify all expected items are exported
        let _ = SnapshotTick::new(0);
        let _ = EntityId::new(0);
        let _ = Limits::default();

        // Error types
        let _: CodecResult<()> = Ok(());
    }

    #[test]
    fn snapshot_tick_usage() {
        let tick = SnapshotTick::new(100);
        assert_eq!(tick.raw(), 100);
        assert!(!tick.is_zero());
    }

    #[test]
    fn entity_id_usage() {
        let id = EntityId::new(42);
        assert_eq!(id.raw(), 42);
    }

    #[test]
    fn limits_reexported() {
        // Limits is re-exported from wire
        let limits = Limits::default();
        assert!(limits.max_packet_bytes > 0);
    }
}
