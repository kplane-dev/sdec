//! Packet decoding and section framing.

use crate::error::{DecodeError, EncodeError, LimitKind, SectionFramingError, WireResult};
use crate::header::{PacketFlags, PacketHeader, HEADER_SIZE, MAGIC, VERSION};
use crate::limits::Limits;

/// Section tags for version 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
#[repr(u8)]
pub enum SectionTag {
    EntityCreate = 1,
    EntityDestroy = 2,
    EntityUpdate = 3,
    EntityUpdateSparse = 4,
    EntityUpdateSparsePacked = 5,
    SessionInit = 6,
}

impl SectionTag {
    /// Parses a section tag from a raw byte.
    pub fn parse(tag: u8) -> Result<Self, DecodeError> {
        match tag {
            1 => Ok(Self::EntityCreate),
            2 => Ok(Self::EntityDestroy),
            3 => Ok(Self::EntityUpdate),
            4 => Ok(Self::EntityUpdateSparse),
            5 => Ok(Self::EntityUpdateSparsePacked),
            6 => Ok(Self::SessionInit),
            _ => Err(DecodeError::UnknownSectionTag { tag }),
        }
    }
}

/// A section within a wire packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireSection<'a> {
    pub tag: SectionTag,
    pub body: &'a [u8],
}

/// A decoded wire packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WirePacket<'a> {
    pub header: PacketHeader,
    pub sections: Vec<WireSection<'a>>,
}

/// Decodes a wire packet into header + section slices.
pub fn decode_packet<'a>(buf: &'a [u8], limits: &Limits) -> WireResult<WirePacket<'a>> {
    if buf.len() < HEADER_SIZE {
        return Err(DecodeError::PacketTooSmall {
            actual: buf.len(),
            required: HEADER_SIZE,
        });
    }
    if buf.len() > limits.max_packet_bytes {
        return Err(DecodeError::LimitsExceeded {
            kind: LimitKind::PacketBytes,
            limit: limits.max_packet_bytes,
            actual: buf.len(),
        });
    }

    let magic = u32::from_le_bytes(buf[0..4].try_into().unwrap());
    if magic != MAGIC {
        return Err(DecodeError::InvalidMagic { found: magic });
    }

    let version = u16::from_le_bytes(buf[4..6].try_into().unwrap());
    if version != VERSION {
        return Err(DecodeError::UnsupportedVersion { found: version });
    }

    let flags_raw = u16::from_le_bytes(buf[6..8].try_into().unwrap());
    let flags = PacketFlags::from_raw(flags_raw);
    if !flags.is_valid_v2() {
        return Err(DecodeError::InvalidFlags { flags: flags_raw });
    }

    let schema_hash = u64::from_le_bytes(buf[8..16].try_into().unwrap());
    let tick = u32::from_le_bytes(buf[16..20].try_into().unwrap());
    let baseline_tick = u32::from_le_bytes(buf[20..24].try_into().unwrap());
    let payload_len = u32::from_le_bytes(buf[24..28].try_into().unwrap());

    if !flags.is_session_init() && flags.is_full_snapshot() && baseline_tick != 0 {
        return Err(DecodeError::InvalidBaselineTick {
            baseline_tick,
            flags: flags_raw,
        });
    }
    if !flags.is_session_init() && flags.is_delta_snapshot() && baseline_tick == 0 {
        return Err(DecodeError::InvalidBaselineTick {
            baseline_tick,
            flags: flags_raw,
        });
    }

    let actual_payload_len = buf.len() - HEADER_SIZE;
    if payload_len as usize != actual_payload_len {
        return Err(DecodeError::PayloadLengthMismatch {
            header_len: payload_len,
            actual_len: actual_payload_len,
        });
    }

    let header = PacketHeader {
        version,
        flags,
        schema_hash,
        tick,
        baseline_tick,
        payload_len,
    };

    let payload = &buf[HEADER_SIZE..];
    let sections = decode_sections(payload, limits)?;

    Ok(WirePacket { header, sections })
}

/// Decodes sections from a payload buffer (no packet header).
pub fn decode_sections<'a>(payload: &'a [u8], limits: &Limits) -> WireResult<Vec<WireSection<'a>>> {
    let mut offset = 0usize;
    let mut sections = Vec::new();

    while offset < payload.len() {
        if sections.len() >= limits.max_sections {
            return Err(DecodeError::LimitsExceeded {
                kind: LimitKind::SectionCount,
                limit: limits.max_sections,
                actual: sections.len() + 1,
            });
        }

        let tag = payload[offset];
        offset += 1;
        let (len, new_offset) = read_varu32(payload, offset)?;
        offset = new_offset;
        let len_usize = usize::try_from(len).unwrap();

        if len_usize > limits.max_section_len {
            return Err(DecodeError::LimitsExceeded {
                kind: LimitKind::SectionLength,
                limit: limits.max_section_len,
                actual: len_usize,
            });
        }
        if offset + len_usize > payload.len() {
            return Err(DecodeError::SectionFraming(
                SectionFramingError::Truncated {
                    needed: offset + len_usize,
                    available: payload.len(),
                },
            ));
        }

        let tag = SectionTag::parse(tag)?;
        let body = &payload[offset..offset + len_usize];
        sections.push(WireSection { tag, body });
        offset += len_usize;
    }

    Ok(sections)
}

/// Encodes a packet header into the provided output buffer.
pub fn encode_header(header: &PacketHeader, out: &mut [u8]) -> Result<usize, EncodeError> {
    if out.len() < HEADER_SIZE {
        return Err(EncodeError::BufferTooSmall {
            needed: HEADER_SIZE,
            available: out.len(),
        });
    }

    out[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    out[4..6].copy_from_slice(&header.version.to_le_bytes());
    out[6..8].copy_from_slice(&header.flags.raw().to_le_bytes());
    out[8..16].copy_from_slice(&header.schema_hash.to_le_bytes());
    out[16..20].copy_from_slice(&header.tick.to_le_bytes());
    out[20..24].copy_from_slice(&header.baseline_tick.to_le_bytes());
    out[24..28].copy_from_slice(&header.payload_len.to_le_bytes());

    Ok(HEADER_SIZE)
}

/// Encodes a single section into the provided output buffer.
pub fn encode_section(tag: SectionTag, body: &[u8], out: &mut [u8]) -> Result<usize, EncodeError> {
    let len_u32 = u32::try_from(body.len())
        .map_err(|_| EncodeError::LengthOverflow { length: body.len() })?;
    let len_bytes = varu32_len(len_u32);
    let needed = 1 + len_bytes + body.len();
    if out.len() < needed {
        return Err(EncodeError::BufferTooSmall {
            needed,
            available: out.len(),
        });
    }

    out[0] = tag as u8;
    let mut offset = 1;
    offset += write_varu32(len_u32, &mut out[offset..]);
    out[offset..offset + body.len()].copy_from_slice(body);
    Ok(needed)
}

fn read_varu32(buf: &[u8], mut offset: usize) -> Result<(u32, usize), DecodeError> {
    let mut value = 0u32;
    let mut shift = 0u32;
    for _ in 0..5 {
        if offset >= buf.len() {
            return Err(DecodeError::SectionFraming(
                SectionFramingError::Truncated {
                    needed: offset + 1,
                    available: buf.len(),
                },
            ));
        }
        let byte = buf[offset];
        offset += 1;
        value |= u32::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, offset));
        }
        shift += 7;
    }
    Err(DecodeError::SectionFraming(
        SectionFramingError::InvalidVarint,
    ))
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

fn varu32_len(mut value: u32) -> usize {
    let mut len = 1;
    while value >= 0x80 {
        value >>= 7;
        len += 1;
    }
    len
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_header_roundtrip_empty_payload() {
        let header = PacketHeader::full_snapshot(0xABCD, 42, 0);
        let mut buf = [0u8; HEADER_SIZE];
        let written = encode_header(&header, &mut buf).unwrap();
        assert_eq!(written, HEADER_SIZE);

        let limits = Limits::for_testing();
        let packet = decode_packet(&buf, &limits).unwrap();
        assert_eq!(packet.header, header);
        assert!(packet.sections.is_empty());
    }

    #[test]
    fn decode_rejects_invalid_magic() {
        let mut buf = [0u8; HEADER_SIZE];
        buf[0..4].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        buf[4..6].copy_from_slice(&VERSION.to_le_bytes());
        buf[6..8].copy_from_slice(&PacketFlags::full_snapshot().raw().to_le_bytes());
        let limits = Limits::for_testing();
        let err = decode_packet(&buf, &limits).unwrap_err();
        assert!(matches!(err, DecodeError::InvalidMagic { .. }));
    }

    #[test]
    fn decode_payload_length_mismatch() {
        let header = PacketHeader::full_snapshot(0, 1, 10);
        let mut buf = [0u8; HEADER_SIZE];
        encode_header(&header, &mut buf).unwrap();
        let limits = Limits::for_testing();
        let err = decode_packet(&buf, &limits).unwrap_err();
        assert!(matches!(err, DecodeError::PayloadLengthMismatch { .. }));
    }

    #[test]
    fn decode_payload_length_mismatch_with_extra_bytes() {
        let header = PacketHeader::full_snapshot(0, 1, 0);
        let mut buf = vec![0u8; HEADER_SIZE + 4];
        encode_header(&header, &mut buf).unwrap();
        let limits = Limits::for_testing();
        let err = decode_packet(&buf, &limits).unwrap_err();
        assert!(matches!(err, DecodeError::PayloadLengthMismatch { .. }));
    }

    #[test]
    fn decode_rejects_invalid_baseline_full() {
        let header = PacketHeader {
            version: VERSION,
            flags: PacketFlags::full_snapshot(),
            schema_hash: 0,
            tick: 1,
            baseline_tick: 1,
            payload_len: 0,
        };
        let mut buf = [0u8; HEADER_SIZE];
        encode_header(&header, &mut buf).unwrap();
        let limits = Limits::for_testing();
        let err = decode_packet(&buf, &limits).unwrap_err();
        assert!(matches!(err, DecodeError::InvalidBaselineTick { .. }));
    }

    #[test]
    fn decode_rejects_invalid_baseline_delta() {
        let header = PacketHeader {
            version: VERSION,
            flags: PacketFlags::delta_snapshot(),
            schema_hash: 0,
            tick: 1,
            baseline_tick: 0,
            payload_len: 0,
        };
        let mut buf = [0u8; HEADER_SIZE];
        encode_header(&header, &mut buf).unwrap();
        let limits = Limits::for_testing();
        let err = decode_packet(&buf, &limits).unwrap_err();
        assert!(matches!(err, DecodeError::InvalidBaselineTick { .. }));
    }

    #[test]
    fn decode_rejects_invalid_flags_reserved_bits() {
        let mut buf = [0u8; HEADER_SIZE];
        buf[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        buf[4..6].copy_from_slice(&VERSION.to_le_bytes());
        let flags = PacketFlags::from_raw(0b101).raw(); // reserved bit set
        buf[6..8].copy_from_slice(&flags.to_le_bytes());
        let limits = Limits::for_testing();
        let err = decode_packet(&buf, &limits).unwrap_err();
        assert!(matches!(err, DecodeError::InvalidFlags { .. }));
    }

    #[test]
    fn decode_accepts_session_init_flags() {
        let mut buf = [0u8; HEADER_SIZE];
        buf[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        buf[4..6].copy_from_slice(&VERSION.to_le_bytes());
        let flags = PacketFlags::session_init().raw();
        buf[6..8].copy_from_slice(&flags.to_le_bytes());
        let limits = Limits::for_testing();
        let packet = decode_packet(&buf, &limits).unwrap();
        assert!(packet.header.flags.is_session_init());
    }

    #[test]
    fn decode_rejects_unsupported_version() {
        let mut buf = [0u8; HEADER_SIZE];
        buf[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        let version = 0u16;
        buf[4..6].copy_from_slice(&version.to_le_bytes());
        let flags = PacketFlags::full_snapshot().raw();
        buf[6..8].copy_from_slice(&flags.to_le_bytes());
        let limits = Limits::for_testing();
        let err = decode_packet(&buf, &limits).unwrap_err();
        assert!(matches!(err, DecodeError::UnsupportedVersion { found: 0 }));
    }

    #[test]
    fn decode_rejects_invalid_varint_len() {
        let header = PacketHeader::full_snapshot(0, 1, 6);
        let mut buf = vec![0u8; HEADER_SIZE + 6];
        encode_header(&header, &mut buf).unwrap();
        let payload = &mut buf[HEADER_SIZE..];
        payload[0] = SectionTag::EntityCreate as u8;
        payload[1..6].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
        let limits = Limits::for_testing();
        let err = decode_packet(&buf, &limits).unwrap_err();
        assert!(matches!(
            err,
            DecodeError::SectionFraming(SectionFramingError::InvalidVarint)
        ));
    }

    #[test]
    fn decode_sections() {
        let mut payload = [0u8; 16];
        let body = [1u8, 2, 3];
        let section_len = encode_section(SectionTag::EntityUpdate, &body, &mut payload).unwrap();

        let header = PacketHeader::full_snapshot(0, 1, section_len as u32);
        let mut buf = vec![0u8; HEADER_SIZE + section_len];
        encode_header(&header, &mut buf).unwrap();
        buf[HEADER_SIZE..HEADER_SIZE + section_len].copy_from_slice(&payload[..section_len]);

        let limits = Limits::for_testing();
        let packet = decode_packet(&buf, &limits).unwrap();
        assert_eq!(packet.sections.len(), 1);
        assert_eq!(packet.sections[0].tag, SectionTag::EntityUpdate);
        assert_eq!(packet.sections[0].body, &body);
    }

    #[test]
    fn decode_enforces_section_limits() {
        let mut payload = [0u8; 8];
        let body = [0u8; 5];
        let section_len = encode_section(SectionTag::EntityCreate, &body, &mut payload).unwrap();

        let header = PacketHeader::full_snapshot(0, 1, section_len as u32);
        let mut buf = vec![0u8; HEADER_SIZE + section_len];
        encode_header(&header, &mut buf).unwrap();
        buf[HEADER_SIZE..HEADER_SIZE + section_len].copy_from_slice(&payload[..section_len]);

        let limits = Limits {
            max_packet_bytes: 4096,
            max_sections: 1,
            max_section_len: 4,
        };
        let err = decode_packet(&buf, &limits).unwrap_err();
        assert!(matches!(
            err,
            DecodeError::LimitsExceeded {
                kind: LimitKind::SectionLength,
                ..
            }
        ));
    }
}
