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

mod baseline;
mod delta;
mod error;
mod limits;
mod scratch;
mod session;
mod snapshot;
mod types;

pub use baseline::{BaselineError, BaselineStore};
pub use delta::{
    apply_delta_snapshot, apply_delta_snapshot_from_packet, decode_delta_packet,
    encode_delta_snapshot, encode_delta_snapshot_for_client,
    encode_delta_snapshot_for_client_session,
    encode_delta_snapshot_for_client_session_with_scratch,
    encode_delta_snapshot_for_client_with_scratch, encode_delta_snapshot_with_scratch,
    select_baseline_tick, DeltaDecoded, DeltaUpdateComponent, DeltaUpdateEntity,
};
pub use error::{CodecError, CodecResult, LimitKind, MaskKind, MaskReason, ValueReason};
pub use limits::CodecLimits;
pub use scratch::CodecScratch;
pub use session::{
    decode_session_init_packet, decode_session_packet, encode_session_init_packet,
    CompactHeaderMode, SessionState,
};
pub use snapshot::{
    decode_full_snapshot, decode_full_snapshot_from_packet, encode_full_snapshot,
    ComponentSnapshot, EntitySnapshot, FieldValue, Snapshot,
};
pub use types::{EntityId, SnapshotTick};
pub use wire::Limits as WireLimits;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_api_exports() {
        // Verify all expected items are exported
        let _ = SnapshotTick::new(0);
        let _ = EntityId::new(0);
        let _ = WireLimits::default();
        let _ = CodecLimits::default();

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
        let limits = WireLimits::default();
        assert!(limits.max_packet_bytes > 0);
    }
}
