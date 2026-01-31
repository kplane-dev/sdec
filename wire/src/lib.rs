//! Wire framing and packet layout for the sdec codec.
//!
//! This crate handles the binary wire format: packet headers, section framing,
//! and limit enforcement. It does not know about game state types—only the
//! structure of packets.
//!
//! # Design Principles
//!
//! - **Stable wire format** - The format is versioned and changes are documented.
//! - **Bounded decoding** - All length fields are validated against limits before iteration.
//! - **No domain knowledge** - This crate handles framing, not game logic.
//!
//! See `WIRE_FORMAT.md` for the complete specification.

mod error;
mod header;
mod limits;

pub use error::{WireError, WireResult};
pub use header::{PacketFlags, PacketHeader, HEADER_SIZE, MAGIC, VERSION};
pub use limits::Limits;

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn public_api_exports() {
        // Verify all expected items are exported
        let _ = MAGIC;
        let _ = VERSION;
        let _ = HEADER_SIZE;
        let _ = PacketFlags::full_snapshot();
        let _ = PacketHeader::full_snapshot(0, 0, 0);
        let _ = Limits::default();

        // Error types
        let _: WireResult<()> = Ok(());
    }

    #[test]
    fn limits_default_is_reasonable() {
        let limits = Limits::default();
        // Should be able to handle typical FPS scenarios
        assert!(
            limits.max_packet_bytes >= 1024,
            "should allow at least 1KB packets"
        );
        assert!(
            limits.max_entities_update >= 64,
            "should allow at least 64 entity updates"
        );
    }

    #[test]
    fn header_size_constant_correct() {
        // Sanity check the header size calculation
        assert_eq!(
            HEADER_SIZE,
            size_of::<u32>() // magic
                + size_of::<u16>() // version
                + size_of::<u16>() // flags
                + size_of::<u64>() // schema_hash
                + size_of::<u32>() // tick
                + size_of::<u32>() // baseline_tick
                + size_of::<u32>() // payload_len
        );
    }

    #[test]
    fn packet_flags_and_header_integration() {
        let flags = PacketFlags::delta_snapshot();
        let header = PacketHeader::delta_snapshot(0x1234, 100, 95, 512);

        assert_eq!(header.flags, flags);
        assert!(header.flags.is_valid_v0());
    }
}
