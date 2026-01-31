//! Session state machine for compact headers.

use bitstream::{BitReader, BitWriter};
use schema::schema_hash;
use wire::{PacketFlags, PacketHeader, SectionTag, WirePacket, WireSection};

use crate::error::{CodecError, CodecResult};
use crate::limits::CodecLimits;
use crate::snapshot::write_section;
use crate::types::SnapshotTick;

/// Compact header mode negotiated via session init.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactHeaderMode {
    /// Compact session header v1.
    SessionV1 = 1,
}

impl CompactHeaderMode {
    fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::SessionV1),
            _ => None,
        }
    }
}

/// Session state for compact headers.
#[derive(Debug, Clone)]
pub struct SessionState {
    pub schema_hash: u64,
    pub session_id: Option<u64>,
    pub last_tick: SnapshotTick,
    pub compact_mode: CompactHeaderMode,
}

/// Encodes a session init packet.
pub fn encode_session_init_packet(
    schema: &schema::Schema,
    tick: SnapshotTick,
    session_id: Option<u64>,
    compact_mode: CompactHeaderMode,
    limits: &CodecLimits,
    out: &mut [u8],
) -> CodecResult<usize> {
    let mut offset = wire::HEADER_SIZE;
    let body_len = write_section(
        SectionTag::SessionInit,
        &mut out[offset..],
        limits,
        |writer| encode_session_init_body(session_id, compact_mode, writer),
    )?;
    offset += body_len;

    let payload_len = offset - wire::HEADER_SIZE;
    let header = PacketHeader {
        version: wire::VERSION,
        flags: PacketFlags::session_init(),
        schema_hash: schema_hash(schema),
        tick: tick.raw(),
        baseline_tick: 0,
        payload_len: payload_len as u32,
    };
    wire::encode_header(&header, &mut out[..wire::HEADER_SIZE]).map_err(|_| {
        CodecError::OutputTooSmall {
            needed: wire::HEADER_SIZE,
            available: out.len(),
        }
    })?;

    Ok(offset)
}

fn encode_session_init_body(
    session_id: Option<u64>,
    compact_mode: CompactHeaderMode,
    writer: &mut BitWriter<'_>,
) -> CodecResult<()> {
    writer.align_to_byte()?;
    writer.write_u64_aligned(session_id.unwrap_or(0))?;
    writer.write_u8_aligned(compact_mode as u8)?;
    writer.align_to_byte()?;
    Ok(())
}

/// Decodes a session init packet into session state.
pub fn decode_session_init_packet(
    schema: &schema::Schema,
    packet: &WirePacket<'_>,
    limits: &CodecLimits,
) -> CodecResult<SessionState> {
    let header = packet.header;
    if !header.flags.is_session_init() {
        return Err(CodecError::SessionMissing);
    }
    if header.flags.is_full_snapshot() || header.flags.is_delta_snapshot() {
        return Err(CodecError::SessionInitInvalid);
    }
    if header.baseline_tick != 0 {
        return Err(CodecError::SessionInitInvalid);
    }
    let expected_hash = schema_hash(schema);
    if header.schema_hash != expected_hash {
        return Err(CodecError::SchemaMismatch {
            expected: expected_hash,
            found: header.schema_hash,
        });
    }

    let mut init_section: Option<&WireSection<'_>> = None;
    for section in &packet.sections {
        match section.tag {
            SectionTag::SessionInit => {
                if init_section.is_some() {
                    return Err(CodecError::SessionInitInvalid);
                }
                init_section = Some(section);
            }
            _ => {
                return Err(CodecError::UnexpectedSection {
                    section: section.tag,
                });
            }
        }
    }
    let section = init_section.ok_or(CodecError::SessionInitInvalid)?;
    let (session_id, compact_mode) = decode_session_init_body(section.body, limits)?;

    Ok(SessionState {
        schema_hash: header.schema_hash,
        session_id,
        last_tick: SnapshotTick::new(header.tick),
        compact_mode,
    })
}

fn decode_session_init_body(
    body: &[u8],
    limits: &CodecLimits,
) -> CodecResult<(Option<u64>, CompactHeaderMode)> {
    if body.len() > limits.max_section_bytes {
        return Err(CodecError::LimitsExceeded {
            kind: crate::error::LimitKind::SectionBytes,
            limit: limits.max_section_bytes,
            actual: body.len(),
        });
    }
    let mut reader = BitReader::new(body);
    reader.align_to_byte()?;
    let session_id = reader.read_u64_aligned()?;
    let mode = reader.read_u8_aligned()?;
    reader.align_to_byte()?;
    if reader.bits_remaining() != 0 {
        return Err(CodecError::TrailingSectionData {
            section: SectionTag::SessionInit,
            remaining_bits: reader.bits_remaining(),
        });
    }
    let compact_mode =
        CompactHeaderMode::from_raw(mode).ok_or(CodecError::SessionUnsupportedMode { mode })?;
    Ok((
        if session_id == 0 {
            None
        } else {
            Some(session_id)
        },
        compact_mode,
    ))
}

/// Decodes a compact packet using session state.
pub fn decode_session_packet<'a>(
    schema: &schema::Schema,
    session: &mut SessionState,
    bytes: &'a [u8],
    wire_limits: &wire::Limits,
) -> CodecResult<WirePacket<'a>> {
    if session.schema_hash != schema_hash(schema) {
        return Err(CodecError::SchemaMismatch {
            expected: schema_hash(schema),
            found: session.schema_hash,
        });
    }
    let header =
        wire::decode_session_header(bytes, session.last_tick.raw()).map_err(CodecError::Wire)?;
    if header.tick <= session.last_tick.raw() {
        return Err(CodecError::SessionOutOfOrder {
            previous: session.last_tick.raw(),
            current: header.tick,
        });
    }

    let payload_start = header.header_len;
    let payload_end = payload_start + header.payload_len as usize;
    if payload_end > bytes.len() {
        return Err(CodecError::Wire(wire::DecodeError::PayloadLengthMismatch {
            header_len: header.payload_len,
            actual_len: bytes.len().saturating_sub(payload_start),
        }));
    }
    let payload = &bytes[payload_start..payload_end];
    let sections = wire::decode_sections(payload, wire_limits).map_err(CodecError::Wire)?;

    session.last_tick = SnapshotTick::new(header.tick);
    let flags = if header.flags.is_full_snapshot() {
        PacketFlags::full_snapshot()
    } else {
        PacketFlags::delta_snapshot()
    };
    Ok(WirePacket {
        header: PacketHeader {
            version: wire::VERSION,
            flags,
            schema_hash: session.schema_hash,
            tick: header.tick,
            baseline_tick: header.baseline_tick,
            payload_len: header.payload_len,
        },
        sections,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{ComponentSnapshot, EntitySnapshot, FieldValue, Snapshot};
    use crate::types::EntityId;
    use schema::{ComponentDef, FieldCodec, FieldDef, FieldId, Schema};

    fn schema_one_bool() -> Schema {
        let component = ComponentDef::new(schema::ComponentId::new(1).unwrap())
            .field(FieldDef::new(FieldId::new(1).unwrap(), FieldCodec::bool()));
        Schema::new(vec![component]).unwrap()
    }

    #[test]
    fn session_init_roundtrip() {
        let schema = schema_one_bool();
        let mut buf = [0u8; 128];
        let bytes = encode_session_init_packet(
            &schema,
            SnapshotTick::new(5),
            Some(42),
            CompactHeaderMode::SessionV1,
            &CodecLimits::for_testing(),
            &mut buf,
        )
        .unwrap();
        let packet = wire::decode_packet(&buf[..bytes], &wire::Limits::for_testing()).unwrap();
        let session =
            decode_session_init_packet(&schema, &packet, &CodecLimits::for_testing()).unwrap();
        assert_eq!(session.session_id, Some(42));
        assert_eq!(session.last_tick.raw(), 5);
    }

    #[test]
    fn session_decode_compact_packet() {
        let schema = schema_one_bool();
        let baseline = Snapshot {
            tick: SnapshotTick::new(10),
            entities: vec![EntitySnapshot {
                id: EntityId::new(1),
                components: vec![ComponentSnapshot {
                    id: schema::ComponentId::new(1).unwrap(),
                    fields: vec![FieldValue::Bool(false)],
                }],
            }],
        };
        let current = Snapshot {
            tick: SnapshotTick::new(11),
            entities: vec![EntitySnapshot {
                id: EntityId::new(1),
                components: vec![ComponentSnapshot {
                    id: schema::ComponentId::new(1).unwrap(),
                    fields: vec![FieldValue::Bool(true)],
                }],
            }],
        };
        let mut session = SessionState {
            schema_hash: schema_hash(&schema),
            session_id: Some(1),
            last_tick: baseline.tick,
            compact_mode: CompactHeaderMode::SessionV1,
        };
        let mut buf = [0u8; 256];
        let bytes = crate::delta::encode_delta_snapshot_for_client_session_with_scratch(
            &schema,
            current.tick,
            baseline.tick,
            &baseline,
            &current,
            &CodecLimits::for_testing(),
            &mut crate::scratch::CodecScratch::default(),
            &mut session.last_tick,
            &mut buf,
        )
        .unwrap();
        let packet = decode_session_packet(
            &schema,
            &mut session,
            &buf[..bytes],
            &wire::Limits::for_testing(),
        )
        .unwrap();
        assert!(packet.header.flags.is_delta_snapshot());
    }
}
