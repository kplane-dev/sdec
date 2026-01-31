//! Compact session header encoding for client replication.

use crate::error::{DecodeError, EncodeError, SectionFramingError, WireResult};

/// Maximum encoded size of a session header in bytes.
pub const SESSION_MAX_HEADER_SIZE: usize = 1 + 5 + 5 + 5;

/// Flags for session headers (compact, 1 byte).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SessionFlags(u8);

impl SessionFlags {
    /// Flag indicating a full snapshot packet.
    pub const FULL_SNAPSHOT: u8 = 1 << 0;
    /// Flag indicating a delta snapshot packet.
    pub const DELTA_SNAPSHOT: u8 = 1 << 1;
    /// Reserved bits mask (must be zero).
    const RESERVED_MASK: u8 = !0b11;

    /// Creates flags from a raw value.
    #[must_use]
    pub const fn from_raw(raw: u8) -> Self {
        Self(raw)
    }

    /// Returns the raw flag bits.
    #[must_use]
    pub const fn raw(self) -> u8 {
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

    /// Returns `true` if flags are valid (exactly one snapshot bit, no reserved).
    #[must_use]
    pub const fn is_valid(self) -> bool {
        let has_full = self.is_full_snapshot();
        let has_delta = self.is_delta_snapshot();
        let has_reserved = self.0 & Self::RESERVED_MASK != 0;
        (has_full ^ has_delta) && !has_reserved
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
}

/// Decoded session header (compact format).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionHeader {
    pub flags: SessionFlags,
    pub tick: u32,
    pub baseline_tick: u32,
    pub payload_len: u32,
    pub header_len: usize,
}

/// Encodes a compact session header into the provided buffer.
pub fn encode_session_header(
    out: &mut [u8],
    flags: SessionFlags,
    tick_delta: u32,
    baseline_delta: u32,
    payload_len: u32,
) -> Result<usize, EncodeError> {
    if out.len() < SESSION_MAX_HEADER_SIZE {
        return Err(EncodeError::BufferTooSmall {
            needed: SESSION_MAX_HEADER_SIZE,
            available: out.len(),
        });
    }
    if !flags.is_valid() {
        return Err(EncodeError::LengthOverflow { length: 0 });
    }

    let mut offset = 0;
    out[offset] = flags.raw();
    offset += 1;
    offset += write_varu32(tick_delta, &mut out[offset..]);
    offset += write_varu32(baseline_delta, &mut out[offset..]);
    offset += write_varu32(payload_len, &mut out[offset..]);
    Ok(offset)
}

/// Decodes a compact session header from the provided buffer.
pub fn decode_session_header(buf: &[u8], last_tick: u32) -> WireResult<SessionHeader> {
    if buf.is_empty() {
        return Err(DecodeError::PacketTooSmall {
            actual: buf.len(),
            required: 1,
        });
    }

    let flags = SessionFlags::from_raw(buf[0]);
    if !flags.is_valid() {
        return Err(DecodeError::InvalidFlags {
            flags: flags.raw() as u16,
        });
    }

    let mut offset = 1;
    let (tick_delta, new_offset) = read_varu32(buf, offset)?;
    offset = new_offset;
    if tick_delta == 0 {
        return Err(DecodeError::InvalidFlags {
            flags: flags.raw() as u16,
        });
    }
    let tick = last_tick
        .checked_add(tick_delta)
        .ok_or(DecodeError::InvalidFlags {
            flags: flags.raw() as u16,
        })?;

    let (baseline_delta, new_offset) = read_varu32(buf, offset)?;
    offset = new_offset;
    let baseline_tick =
        tick.checked_sub(baseline_delta)
            .ok_or(DecodeError::InvalidBaselineTick {
                baseline_tick: baseline_delta,
                flags: flags.raw() as u16,
            })?;
    if flags.is_full_snapshot() && baseline_delta != 0 {
        return Err(DecodeError::InvalidBaselineTick {
            baseline_tick,
            flags: flags.raw() as u16,
        });
    }
    if flags.is_delta_snapshot() && baseline_tick == 0 {
        return Err(DecodeError::InvalidBaselineTick {
            baseline_tick,
            flags: flags.raw() as u16,
        });
    }

    let (payload_len, new_offset) = read_varu32(buf, offset)?;
    offset = new_offset;

    Ok(SessionHeader {
        flags,
        tick,
        baseline_tick,
        payload_len,
        header_len: offset,
    })
}

fn read_varu32(buf: &[u8], mut offset: usize) -> Result<(u32, usize), DecodeError> {
    let mut value: u64 = 0;
    let mut shift = 0;
    let mut count = 0;
    loop {
        if offset >= buf.len() {
            return Err(DecodeError::SectionFraming(
                SectionFramingError::Truncated {
                    needed: 1,
                    available: buf.len().saturating_sub(offset),
                },
            ));
        }
        let byte = buf[offset];
        offset += 1;
        count += 1;
        value |= ((byte & 0x7F) as u64) << shift;
        if (byte & 0x80) == 0 {
            break;
        }
        shift += 7;
        if count >= 5 {
            return Err(DecodeError::SectionFraming(
                SectionFramingError::InvalidVarint,
            ));
        }
    }
    Ok((value as u32, offset))
}

fn write_varu32(mut value: u32, out: &mut [u8]) -> usize {
    let mut offset = 0;
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out[offset] = byte;
        offset += 1;
        if value == 0 {
            break;
        }
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_header_roundtrip_delta() {
        let mut buf = [0u8; SESSION_MAX_HEADER_SIZE];
        let len =
            encode_session_header(&mut buf, SessionFlags::delta_snapshot(), 2, 1, 123).unwrap();
        let decoded = decode_session_header(&buf[..len], 10).unwrap();
        assert_eq!(decoded.tick, 12);
        assert_eq!(decoded.baseline_tick, 11);
        assert_eq!(decoded.payload_len, 123);
    }

    #[test]
    fn session_header_rejects_zero_tick_delta() {
        let mut buf = [0u8; SESSION_MAX_HEADER_SIZE];
        let len =
            encode_session_header(&mut buf, SessionFlags::delta_snapshot(), 0, 1, 10).unwrap();
        let err = decode_session_header(&buf[..len], 1).unwrap_err();
        assert!(matches!(err, DecodeError::InvalidFlags { .. }));
    }
}
