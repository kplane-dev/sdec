//! Error types for wire format operations.

use std::fmt;

/// Result type for wire format operations.
pub type WireResult<T> = Result<T, DecodeError>;

/// High-level decode errors for wire framing.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecodeError {
    /// Packet is too small to contain the required header.
    PacketTooSmall { actual: usize, required: usize },

    /// Invalid magic number in packet header.
    InvalidMagic { found: u32 },

    /// Unsupported wire version.
    UnsupportedVersion { found: u16 },

    /// Invalid flags combination.
    InvalidFlags { flags: u16 },

    /// Invalid baseline tick for the packet kind.
    InvalidBaselineTick { baseline_tick: u32, flags: u16 },

    /// Payload length mismatch.
    PayloadLengthMismatch { header_len: u32, actual_len: usize },

    /// Unknown section tag encountered.
    UnknownSectionTag { tag: u8 },

    /// Limits exceeded.
    LimitsExceeded {
        kind: LimitKind,
        limit: usize,
        actual: usize,
    },

    /// Section framing error.
    SectionFraming(SectionFramingError),
}

/// Specific wire limits that can be exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitKind {
    PacketBytes,
    SectionCount,
    SectionLength,
}

/// Errors that can occur while framing sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SectionFramingError {
    InvalidVarint,
    LengthOverflow { value: u64 },
    Truncated { needed: usize, available: usize },
}

/// Errors that can occur during encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodeError {
    BufferTooSmall { needed: usize, available: usize },
    LengthOverflow { length: usize },
}

impl fmt::Display for DecodeError {
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
            Self::InvalidBaselineTick {
                baseline_tick,
                flags,
            } => {
                write!(
                    f,
                    "invalid baseline tick {baseline_tick} for flags 0x{flags:04X}"
                )
            }
            Self::PayloadLengthMismatch {
                header_len,
                actual_len,
            } => {
                write!(
                    f,
                    "payload length mismatch: header {header_len} bytes but {actual_len} available"
                )
            }
            Self::UnknownSectionTag { tag } => {
                write!(f, "unknown section tag: {tag}")
            }
            Self::LimitsExceeded {
                kind,
                limit,
                actual,
            } => {
                write!(f, "{kind} limit exceeded: {actual} > {limit}")
            }
            Self::SectionFraming(err) => write!(f, "section framing error: {err}"),
        }
    }
}

impl fmt::Display for LimitKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::PacketBytes => "packet bytes",
            Self::SectionCount => "section count",
            Self::SectionLength => "section length",
        };
        write!(f, "{name}")
    }
}

impl fmt::Display for SectionFramingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidVarint => write!(f, "invalid varint"),
            Self::LengthOverflow { value } => write!(f, "length overflow: {value}"),
            Self::Truncated { needed, available } => {
                write!(
                    f,
                    "truncated section: need {needed} bytes, have {available}"
                )
            }
        }
    }
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BufferTooSmall { needed, available } => {
                write!(f, "buffer too small: need {needed}, have {available}")
            }
            Self::LengthOverflow { length } => {
                write!(f, "length overflow: {length}")
            }
        }
    }
}

impl std::error::Error for DecodeError {}

impl std::error::Error for EncodeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_error_display_invalid_magic() {
        let err = DecodeError::InvalidMagic { found: 0xDEAD_BEEF };
        let msg = err.to_string();
        assert!(msg.contains("DEADBEEF"));
    }

    #[test]
    fn decode_error_display_limits_exceeded() {
        let err = DecodeError::LimitsExceeded {
            kind: LimitKind::SectionCount,
            limit: 4,
            actual: 10,
        };
        let msg = err.to_string();
        assert!(msg.contains("section count"));
        assert!(msg.contains("10"));
    }

    #[test]
    fn section_framing_display() {
        let err = SectionFramingError::Truncated {
            needed: 10,
            available: 4,
        };
        let msg = err.to_string();
        assert!(msg.contains("truncated"));
        assert!(msg.contains("10"));
    }

    #[test]
    fn encode_error_display() {
        let err = EncodeError::BufferTooSmall {
            needed: 10,
            available: 4,
        };
        let msg = err.to_string();
        assert!(msg.contains("buffer too small"));
    }
}
