//! Packet header types and constants.

/// Magic number identifying sdec packets.
///
/// This value is fixed and must never change across versions.
pub const MAGIC: u32 = 0x5344_4543; // "SDEC" in ASCII

/// Current wire format version.
pub const VERSION: u16 = 2;

/// Header size in bytes (28 total).
pub const HEADER_SIZE: usize = 4 + 2 + 2 + 8 + 4 + 4 + 4;

/// Packet flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PacketFlags(u16);

impl PacketFlags {
    /// Flag indicating a full snapshot packet.
    pub const FULL_SNAPSHOT: u16 = 1 << 0;

    /// Flag indicating a delta snapshot packet.
    pub const DELTA_SNAPSHOT: u16 = 1 << 1;

    /// Flag indicating a session init packet.
    pub const SESSION_INIT: u16 = 1 << 2;

    /// Reserved bits mask (must be zero in version 2).
    const RESERVED_MASK: u16 = !0b111;

    /// Creates new flags from a raw value.
    #[must_use]
    pub const fn from_raw(raw: u16) -> Self {
        Self(raw)
    }

    /// Returns the raw flag bits.
    #[must_use]
    pub const fn raw(self) -> u16 {
        self.0
    }

    /// Returns `true` if this is a full snapshot.
    #[must_use]
    pub const fn is_full_snapshot(self) -> bool {
        self.0 & Self::FULL_SNAPSHOT != 0
    }

    /// Returns `true` if this is a delta snapshot.
    #[must_use]
    pub const fn is_delta_snapshot(self) -> bool {
        self.0 & Self::DELTA_SNAPSHOT != 0
    }

    /// Returns `true` if the flags are valid for version 0.
    ///
    /// Valid means exactly one of `FULL_SNAPSHOT` or `DELTA_SNAPSHOT` is set,
    /// and no reserved bits are set.
    #[must_use]
    pub const fn is_valid_v0(self) -> bool {
        let has_full = self.is_full_snapshot();
        let has_delta = self.is_delta_snapshot();
        let has_reserved = self.0 & Self::RESERVED_MASK != 0;

        has_full ^ has_delta && !has_reserved
    }

    /// Returns `true` if this is a session init packet.
    #[must_use]
    pub const fn is_session_init(self) -> bool {
        self.0 & Self::SESSION_INIT != 0
    }

    /// Returns `true` if the flags are valid for version 2.
    ///
    /// Valid means either:
    /// - session init set alone (no full/delta), or
    /// - exactly one of full/delta set, with no session init,
    /// and no reserved bits are set.
    #[must_use]
    pub const fn is_valid_v2(self) -> bool {
        let has_full = self.is_full_snapshot();
        let has_delta = self.is_delta_snapshot();
        let has_session = self.is_session_init();
        let has_reserved = self.0 & Self::RESERVED_MASK != 0;
        if has_reserved {
            return false;
        }
        if has_session {
            return !has_full && !has_delta;
        }
        has_full ^ has_delta
    }

    /// Creates flags for a full snapshot.
    #[must_use]
    pub const fn full_snapshot() -> Self {
        Self(Self::FULL_SNAPSHOT)
    }

    /// Creates flags for a delta snapshot.
    #[must_use]
    pub const fn delta_snapshot() -> Self {
        Self(Self::DELTA_SNAPSHOT)
    }

    /// Creates flags for a session init packet.
    #[must_use]
    pub const fn session_init() -> Self {
        Self(Self::SESSION_INIT)
    }
}

/// Packet header (version 0).
///
/// This struct represents the header fields *after* the magic number.
/// The magic number is validated separately during decoding and is not
/// stored in this struct.
///
/// See `WIRE_FORMAT.md` for the complete specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketHeader {
    /// Wire format version.
    pub version: u16,
    /// Packet flags.
    pub flags: PacketFlags,
    /// Schema hash for compatibility checking.
    pub schema_hash: u64,
    /// Simulation tick this snapshot represents.
    pub tick: u32,
    /// Baseline tick for delta packets (0 for full snapshots).
    pub baseline_tick: u32,
    /// Payload length in bytes.
    pub payload_len: u32,
}

impl PacketHeader {
    /// Creates a new header for a full snapshot.
    #[must_use]
    pub const fn full_snapshot(schema_hash: u64, tick: u32, payload_len: u32) -> Self {
        Self {
            version: VERSION,
            flags: PacketFlags::full_snapshot(),
            schema_hash,
            tick,
            baseline_tick: 0,
            payload_len,
        }
    }

    /// Creates a new header for a delta snapshot.
    #[must_use]
    pub const fn delta_snapshot(
        schema_hash: u64,
        tick: u32,
        baseline_tick: u32,
        payload_len: u32,
    ) -> Self {
        Self {
            version: VERSION,
            flags: PacketFlags::delta_snapshot(),
            schema_hash,
            tick,
            baseline_tick,
            payload_len,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Constants tests
    #[test]
    fn magic_is_sdec_ascii() {
        // S=0x53, D=0x44, E=0x45, C=0x43
        assert_eq!(MAGIC, 0x5344_4543);
        let bytes = MAGIC.to_be_bytes();
        assert_eq!(&bytes, b"SDEC");
    }

    #[test]
    fn version_is_two() {
        assert_eq!(VERSION, 2);
    }

    #[test]
    fn header_size_is_correct() {
        // magic(4) + version(2) + flags(2) + schema_hash(8) + tick(4) + baseline_tick(4) + payload_len(4)
        assert_eq!(HEADER_SIZE, 28);
    }

    // PacketFlags tests
    #[test]
    fn flags_full_snapshot() {
        let flags = PacketFlags::full_snapshot();
        assert!(flags.is_full_snapshot());
        assert!(!flags.is_delta_snapshot());
        assert_eq!(flags.raw(), 0b01);
    }

    #[test]
    fn flags_delta_snapshot() {
        let flags = PacketFlags::delta_snapshot();
        assert!(!flags.is_full_snapshot());
        assert!(flags.is_delta_snapshot());
        assert_eq!(flags.raw(), 0b10);
    }

    #[test]
    fn flags_from_raw_roundtrip() {
        let flags = PacketFlags::from_raw(0b01);
        assert_eq!(flags.raw(), 0b01);
        assert!(flags.is_full_snapshot());
    }

    #[test]
    fn flags_validity_full() {
        assert!(PacketFlags::full_snapshot().is_valid_v0());
    }

    #[test]
    fn flags_validity_delta() {
        assert!(PacketFlags::delta_snapshot().is_valid_v0());
    }

    #[test]
    fn flags_invalid_neither_set() {
        assert!(!PacketFlags::from_raw(0).is_valid_v0());
    }

    #[test]
    fn flags_invalid_both_set() {
        assert!(!PacketFlags::from_raw(0b11).is_valid_v2());
    }

    #[test]
    fn flags_invalid_reserved_bits() {
        // Full snapshot + reserved bit
        assert!(!PacketFlags::from_raw(0b1001).is_valid_v2());
        // High bits set
        assert!(!PacketFlags::from_raw(0xFF01).is_valid_v2());
    }

    #[test]
    fn flags_default() {
        let flags = PacketFlags::default();
        assert_eq!(flags.raw(), 0);
        assert!(!flags.is_valid_v2()); // default is invalid (neither set)
    }

    #[test]
    fn flags_equality() {
        assert_eq!(PacketFlags::full_snapshot(), PacketFlags::from_raw(0b01));
        assert_ne!(PacketFlags::full_snapshot(), PacketFlags::delta_snapshot());
    }

    #[test]
    fn flags_clone_copy() {
        let flags = PacketFlags::full_snapshot();
        let copied = flags; // Copy
        assert_eq!(flags, copied);
    }

    // PacketHeader tests
    #[test]
    fn header_full_snapshot() {
        let header = PacketHeader::full_snapshot(0x1234_5678_9ABC_DEF0, 100, 512);

        assert_eq!(header.version, VERSION);
        assert!(header.flags.is_full_snapshot());
        assert!(!header.flags.is_delta_snapshot());
        assert_eq!(header.schema_hash, 0x1234_5678_9ABC_DEF0);
        assert_eq!(header.tick, 100);
        assert_eq!(header.baseline_tick, 0);
        assert_eq!(header.payload_len, 512);
    }

    #[test]
    fn header_delta_snapshot() {
        let header = PacketHeader::delta_snapshot(0xABCD, 100, 95, 256);

        assert_eq!(header.version, VERSION);
        assert!(header.flags.is_delta_snapshot());
        assert!(!header.flags.is_full_snapshot());
        assert_eq!(header.schema_hash, 0xABCD);
        assert_eq!(header.tick, 100);
        assert_eq!(header.baseline_tick, 95);
        assert_eq!(header.payload_len, 256);
    }

    #[test]
    fn header_equality() {
        let h1 = PacketHeader::full_snapshot(0x1234, 100, 512);
        let h2 = PacketHeader::full_snapshot(0x1234, 100, 512);
        let h3 = PacketHeader::full_snapshot(0x1234, 101, 512);

        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }

    #[test]
    fn header_clone_copy() {
        let header = PacketHeader::full_snapshot(0x1234, 100, 512);
        let copied = header; // Copy
        assert_eq!(header, copied);
    }

    #[test]
    fn header_debug() {
        let header = PacketHeader::full_snapshot(0x1234, 100, 512);
        let debug = format!("{header:?}");
        assert!(debug.contains("PacketHeader"));
        assert!(debug.contains("100")); // tick
    }

    #[test]
    fn header_const_constructible() {
        const HEADER: PacketHeader = PacketHeader::full_snapshot(0, 0, 0);
        assert_eq!(HEADER.tick, 0);
    }
}
