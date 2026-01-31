//! Error types for bitstream operations.

use std::fmt;

/// Result type for bitstream operations.
pub type BitResult<T> = Result<T, BitError>;

/// Errors that can occur during bit-level encoding/decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BitError {
    /// Attempted to read past the end of the buffer.
    UnexpectedEof {
        /// Number of bits requested.
        requested: usize,
        /// Number of bits available.
        available: usize,
    },

    /// Attempted to write more bits than the buffer can hold.
    WriteOverflow {
        /// Number of bits attempted to write.
        attempted: usize,
        /// Number of bits available.
        available: usize,
    },

    /// Invalid bit count for the operation.
    InvalidBitCount {
        /// The invalid bit count provided.
        bits: u8,
        /// Maximum allowed bits for this operation.
        max_bits: u8,
    },

    /// Value exceeds the range representable by the specified number of bits.
    ValueOutOfRange {
        /// The value that was out of range.
        value: u64,
        /// Number of bits available.
        bits: u8,
    },

    /// Invalid varint encoding (too many bytes or overflow).
    InvalidVarint,

    /// Attempted to read or write a byte-aligned value when not aligned.
    MisalignedAccess {
        /// Current bit position.
        bit_position: usize,
    },
}

impl fmt::Display for BitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof {
                requested,
                available,
            } => {
                write!(
                    f,
                    "unexpected EOF: requested {requested} bits, {available} available"
                )
            }
            Self::WriteOverflow {
                attempted,
                available,
            } => {
                write!(
                    f,
                    "write overflow: attempted {attempted} bits, {available} available"
                )
            }
            Self::InvalidBitCount { bits, max_bits } => {
                write!(f, "invalid bit count {bits}, maximum allowed is {max_bits}")
            }
            Self::ValueOutOfRange { value, bits } => {
                write!(f, "value {value} cannot be represented in {bits} bits")
            }
            Self::InvalidVarint => {
                write!(f, "invalid varint encoding")
            }
            Self::MisalignedAccess { bit_position } => {
                write!(f, "misaligned access at bit position {bit_position}")
            }
        }
    }
}

impl std::error::Error for BitError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_unexpected_eof() {
        let err = BitError::UnexpectedEof {
            requested: 8,
            available: 3,
        };
        let msg = err.to_string();
        assert!(msg.contains("8"), "should mention requested bits");
        assert!(msg.contains("3"), "should mention available bits");
        assert!(msg.contains("EOF"), "should mention EOF");
    }

    #[test]
    fn error_display_write_overflow() {
        let err = BitError::WriteOverflow {
            attempted: 100,
            available: 64,
        };
        let msg = err.to_string();
        assert!(msg.contains("100"), "should mention attempted bits");
        assert!(msg.contains("64"), "should mention available bits");
        assert!(msg.contains("overflow"), "should mention overflow");
    }

    #[test]
    fn error_display_invalid_bit_count() {
        let err = BitError::InvalidBitCount {
            bits: 128,
            max_bits: 64,
        };
        let msg = err.to_string();
        assert!(msg.contains("128"), "should mention invalid count");
        assert!(msg.contains("64"), "should mention maximum");
    }

    #[test]
    fn error_display_value_out_of_range() {
        let err = BitError::ValueOutOfRange {
            value: 256,
            bits: 8,
        };
        let msg = err.to_string();
        assert!(msg.contains("256"), "should mention the value");
        assert!(msg.contains("8"), "should mention bit count");
    }

    #[test]
    fn error_display_invalid_varint() {
        let err = BitError::InvalidVarint;
        assert!(err.to_string().contains("varint"));
    }

    #[test]
    fn error_display_misaligned_access() {
        let err = BitError::MisalignedAccess { bit_position: 3 };
        let msg = err.to_string();
        assert!(msg.contains("3"), "should mention bit position");
        assert!(msg.contains("misaligned"));
    }

    #[test]
    fn error_equality() {
        let err1 = BitError::UnexpectedEof {
            requested: 8,
            available: 3,
        };
        let err2 = BitError::UnexpectedEof {
            requested: 8,
            available: 3,
        };
        let err3 = BitError::UnexpectedEof {
            requested: 8,
            available: 4,
        };
        assert_eq!(err1, err2);
        assert_ne!(err1, err3);
    }

    #[test]
    fn error_clone() {
        let err = BitError::InvalidBitCount {
            bits: 65,
            max_bits: 64,
        };
        let cloned = err.clone();
        assert_eq!(err, cloned);
    }

    #[test]
    fn error_debug() {
        let err = BitError::UnexpectedEof {
            requested: 1,
            available: 0,
        };
        let debug = format!("{err:?}");
        assert!(debug.contains("UnexpectedEof"));
    }

    #[test]
    fn error_is_std_error() {
        fn assert_error<E: std::error::Error>() {}
        assert_error::<BitError>();
    }
}
