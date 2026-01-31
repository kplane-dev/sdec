//! Error types for bitstream operations.

use std::fmt;

/// Result type for bitstream operations.
pub type BitResult<T> = Result<T, BitError>;

/// Errors that can occur during bit-level encoding/decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BitError {
    /// Attempted to read past the end of the buffer.
    EndOfBuffer {
        /// Number of bits requested.
        requested: usize,
        /// Number of bits available.
        available: usize,
    },

    /// Attempted to write more bits than the buffer can hold.
    ///
    /// Note: This error is reserved for future bounded-capacity writer modes.
    /// The current `BitWriter` uses a growable `Vec` and will not return this error.
    BufferOverflow {
        /// Number of bits attempted to write.
        attempted: usize,
        /// Maximum capacity in bits.
        capacity: usize,
    },

    /// Invalid bit count for the operation.
    InvalidBitCount {
        /// The invalid bit count provided.
        bits: usize,
        /// Maximum allowed bits for this operation.
        max_bits: usize,
    },

    /// Value exceeds the range representable by the specified number of bits.
    ValueOutOfRange {
        /// The value that was out of range.
        value: u64,
        /// Number of bits available.
        bits: usize,
    },
}

impl fmt::Display for BitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EndOfBuffer {
                requested,
                available,
            } => {
                write!(
                    f,
                    "attempted to read {requested} bits but only {available} bits available"
                )
            }
            Self::BufferOverflow {
                attempted,
                capacity,
            } => {
                write!(
                    f,
                    "attempted to write {attempted} bits but buffer capacity is {capacity} bits"
                )
            }
            Self::InvalidBitCount { bits, max_bits } => {
                write!(f, "invalid bit count {bits}, maximum allowed is {max_bits}")
            }
            Self::ValueOutOfRange { value, bits } => {
                write!(f, "value {value} cannot be represented in {bits} bits")
            }
        }
    }
}

impl std::error::Error for BitError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_end_of_buffer() {
        let err = BitError::EndOfBuffer {
            requested: 8,
            available: 3,
        };
        let msg = err.to_string();
        assert!(msg.contains("8 bits"), "should mention requested bits");
        assert!(msg.contains("3 bits"), "should mention available bits");
        assert!(msg.contains("read"), "should mention read operation");
    }

    #[test]
    fn error_display_buffer_overflow() {
        let err = BitError::BufferOverflow {
            attempted: 100,
            capacity: 64,
        };
        let msg = err.to_string();
        assert!(msg.contains("100"), "should mention attempted bits");
        assert!(msg.contains("64"), "should mention capacity");
        assert!(msg.contains("write"), "should mention write operation");
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
        assert!(msg.contains("8 bits"), "should mention bit count");
    }

    #[test]
    fn error_equality() {
        let err1 = BitError::EndOfBuffer {
            requested: 8,
            available: 3,
        };
        let err2 = BitError::EndOfBuffer {
            requested: 8,
            available: 3,
        };
        let err3 = BitError::EndOfBuffer {
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
        let err = BitError::EndOfBuffer {
            requested: 1,
            available: 0,
        };
        let debug = format!("{err:?}");
        assert!(debug.contains("EndOfBuffer"));
    }

    #[test]
    fn error_is_std_error() {
        fn assert_error<E: std::error::Error>() {}
        assert_error::<BitError>();
    }
}
