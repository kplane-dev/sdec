//! Error types for wire format operations.

use std::fmt;

/// Result type for wire format operations.
pub type WireResult<T> = Result<T, WireError>;

/// Errors that can occur during wire format encoding/decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    /// Packet is too small to contain the required header.
    PacketTooSmall {
        /// Actual size in bytes.
        actual: usize,
        /// Minimum required size in bytes.
        required: usize,
    },

    /// Invalid magic number in packet header.
    InvalidMagic {
        /// The invalid magic value found.
        found: u32,
    },

    /// Unsupported wire version.
    UnsupportedVersion {
        /// The version found in the packet.
        found: u16,
    },

    /// Invalid flags combination.
    InvalidFlags {
        /// The invalid flags value.
        flags: u16,
    },

    /// Schema hash mismatch between packet and expected schema.
    SchemaMismatch {
        /// Schema hash in the packet.
        packet_hash: u64,
        /// Expected schema hash.
        expected_hash: u64,
    },

    /// Packet exceeds configured size limit.
    PacketTooLarge {
        /// Actual size in bytes.
        actual: usize,
        /// Maximum allowed size in bytes.
        limit: usize,
    },

    /// Section count exceeds limit.
    TooManySections {
        /// Number of sections found.
        count: usize,
        /// Maximum allowed sections.
        limit: usize,
    },

    /// Entity count exceeds limit for the operation.
    TooManyEntities {
        /// Operation type (create, update, destroy).
        operation: &'static str,
        /// Number of entities.
        count: usize,
        /// Maximum allowed.
        limit: usize,
    },

    /// Unknown section tag encountered.
    UnknownSectionTag {
        /// The unknown tag value.
        tag: u8,
    },

    /// Payload length mismatch.
    PayloadLengthMismatch {
        /// Declared length in header.
        declared: usize,
        /// Actual available bytes.
        actual: usize,
    },

    /// Underlying bitstream error.
    BitError(bitstream::BitError),
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PacketTooSmall { actual, required } => {
                write!(
                    f,
                    "packet too small: {actual} bytes, need at least {required}"
                )
            }
            Self::InvalidMagic { found } => {
                write!(f, "invalid magic number: 0x{found:08X}")
            }
            Self::UnsupportedVersion { found } => {
                write!(f, "unsupported wire version: {found}")
            }
            Self::InvalidFlags { flags } => {
                write!(f, "invalid flags: 0x{flags:04X}")
            }
            Self::SchemaMismatch {
                packet_hash,
                expected_hash,
            } => {
                write!(
                    f,
                    "schema mismatch: packet has 0x{packet_hash:016X}, expected 0x{expected_hash:016X}"
                )
            }
            Self::PacketTooLarge { actual, limit } => {
                write!(
                    f,
                    "packet too large: {actual} bytes exceeds limit of {limit}"
                )
            }
            Self::TooManySections { count, limit } => {
                write!(f, "too many sections: {count} exceeds limit of {limit}")
            }
            Self::TooManyEntities {
                operation,
                count,
                limit,
            } => {
                write!(
                    f,
                    "too many entities in {operation}: {count} exceeds limit of {limit}"
                )
            }
            Self::UnknownSectionTag { tag } => {
                write!(f, "unknown section tag: {tag}")
            }
            Self::PayloadLengthMismatch { declared, actual } => {
                write!(
                    f,
                    "payload length mismatch: declared {declared} bytes but {actual} available"
                )
            }
            Self::BitError(e) => {
                write!(f, "bitstream error: {e}")
            }
        }
    }
}

impl std::error::Error for WireError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::BitError(e) => Some(e),
            _ => None,
        }
    }
}

impl From<bitstream::BitError> for WireError {
    fn from(err: bitstream::BitError) -> Self {
        Self::BitError(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_packet_too_small() {
        let err = WireError::PacketTooSmall {
            actual: 10,
            required: 28,
        };
        let msg = err.to_string();
        assert!(msg.contains("10"), "should mention actual size");
        assert!(msg.contains("28"), "should mention required size");
    }

    #[test]
    fn error_display_invalid_magic() {
        let err = WireError::InvalidMagic { found: 0xDEAD_BEEF };
        let msg = err.to_string();
        assert!(msg.contains("DEADBEEF"), "should show hex magic");
    }

    #[test]
    fn error_display_unsupported_version() {
        let err = WireError::UnsupportedVersion { found: 99 };
        let msg = err.to_string();
        assert!(msg.contains("99"), "should mention version");
    }

    #[test]
    fn error_display_invalid_flags() {
        let err = WireError::InvalidFlags { flags: 0xFF00 };
        let msg = err.to_string();
        assert!(msg.contains("FF00"), "should show hex flags");
    }

    #[test]
    fn error_display_schema_mismatch() {
        let err = WireError::SchemaMismatch {
            packet_hash: 0x1234,
            expected_hash: 0x5678,
        };
        let msg = err.to_string();
        assert!(msg.contains("1234"), "should show packet hash");
        assert!(msg.contains("5678"), "should show expected hash");
    }

    #[test]
    fn error_display_packet_too_large() {
        let err = WireError::PacketTooLarge {
            actual: 100_000,
            limit: 65536,
        };
        let msg = err.to_string();
        assert!(msg.contains("100000"), "should mention actual size");
        assert!(msg.contains("65536"), "should mention limit");
    }

    #[test]
    fn error_display_too_many_sections() {
        let err = WireError::TooManySections {
            count: 100,
            limit: 16,
        };
        let msg = err.to_string();
        assert!(msg.contains("100"), "should mention count");
        assert!(msg.contains("16"), "should mention limit");
    }

    #[test]
    fn error_display_too_many_entities() {
        let err = WireError::TooManyEntities {
            operation: "create",
            count: 500,
            limit: 256,
        };
        let msg = err.to_string();
        assert!(msg.contains("create"), "should mention operation");
        assert!(msg.contains("500"), "should mention count");
        assert!(msg.contains("256"), "should mention limit");
    }

    #[test]
    fn error_display_unknown_section_tag() {
        let err = WireError::UnknownSectionTag { tag: 42 };
        let msg = err.to_string();
        assert!(msg.contains("42"), "should mention tag");
    }

    #[test]
    fn error_display_payload_length_mismatch() {
        let err = WireError::PayloadLengthMismatch {
            declared: 100,
            actual: 50,
        };
        let msg = err.to_string();
        assert!(msg.contains("100"), "should mention declared");
        assert!(msg.contains("50"), "should mention actual");
    }

    #[test]
    fn error_from_bit_error() {
        let bit_err = bitstream::BitError::EndOfBuffer {
            requested: 8,
            available: 0,
        };
        let wire_err: WireError = bit_err.into();
        assert!(matches!(wire_err, WireError::BitError(_)));

        let msg = wire_err.to_string();
        assert!(msg.contains("bitstream"), "should mention source");
    }

    #[test]
    fn error_source_bit_error() {
        let bit_err = bitstream::BitError::EndOfBuffer {
            requested: 8,
            available: 0,
        };
        let wire_err = WireError::BitError(bit_err);

        // Test std::error::Error::source()
        let source = std::error::Error::source(&wire_err);
        assert!(source.is_some(), "should have a source");
    }

    #[test]
    fn error_source_none_for_others() {
        let err = WireError::InvalidMagic { found: 0 };
        let source = std::error::Error::source(&err);
        assert!(source.is_none(), "non-wrapped errors should have no source");
    }

    #[test]
    fn error_equality() {
        let err1 = WireError::InvalidMagic { found: 0x1234 };
        let err2 = WireError::InvalidMagic { found: 0x1234 };
        let err3 = WireError::InvalidMagic { found: 0x5678 };
        assert_eq!(err1, err2);
        assert_ne!(err1, err3);
    }

    #[test]
    fn error_is_std_error() {
        fn assert_error<E: std::error::Error>() {}
        assert_error::<WireError>();
    }
}
