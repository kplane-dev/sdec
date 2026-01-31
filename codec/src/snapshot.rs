//! Full snapshot encoding/decoding.

use bitstream::{BitReader, BitWriter};
use schema::{schema_hash, ComponentDef, ComponentId, FieldCodec, FieldDef, FieldId};
use wire::{decode_packet, encode_header, SectionTag, WirePacket};

use crate::error::{CodecError, CodecResult, LimitKind, MaskKind, MaskReason, ValueReason};
use crate::limits::CodecLimits;
use crate::types::{EntityId, SnapshotTick};

const VARINT_MAX_BYTES: usize = 5;

/// A decoded snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub tick: SnapshotTick,
    pub entities: Vec<EntitySnapshot>,
}

/// An entity snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntitySnapshot {
    pub id: EntityId,
    pub components: Vec<ComponentSnapshot>,
}

/// A component snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentSnapshot {
    pub id: ComponentId,
    /// Field values in schema order.
    pub fields: Vec<FieldValue>,
}

/// A field value in decoded form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldValue {
    Bool(bool),
    UInt(u64),
    SInt(i64),
    VarUInt(u64),
    VarSInt(i64),
    FixedPoint(i64),
}

/// Encodes a full snapshot into the provided output buffer.
///
/// Entities must be in deterministic order (ascending `EntityId` recommended).
pub fn encode_full_snapshot(
    schema: &schema::Schema,
    tick: SnapshotTick,
    entities: &[EntitySnapshot],
    limits: &CodecLimits,
    out: &mut [u8],
) -> CodecResult<usize> {
    if out.len() < wire::HEADER_SIZE {
        return Err(CodecError::OutputTooSmall {
            needed: wire::HEADER_SIZE,
            available: out.len(),
        });
    }

    if entities.len() > limits.max_entities_create {
        return Err(CodecError::LimitsExceeded {
            kind: LimitKind::EntitiesCreate,
            limit: limits.max_entities_create,
            actual: entities.len(),
        });
    }

    let mut offset = wire::HEADER_SIZE;
    if !entities.is_empty() {
        let written = write_section(
            SectionTag::EntityCreate,
            &mut out[offset..],
            limits,
            |writer| encode_create_body(schema, entities, limits, writer),
        )?;
        offset += written;
    }

    let payload_len = offset - wire::HEADER_SIZE;
    let header =
        wire::PacketHeader::full_snapshot(schema_hash(schema), tick.raw(), payload_len as u32);
    encode_header(&header, &mut out[..wire::HEADER_SIZE]).map_err(|_| {
        CodecError::OutputTooSmall {
            needed: wire::HEADER_SIZE,
            available: out.len(),
        }
    })?;

    Ok(offset)
}

/// Decodes a full snapshot from raw packet bytes.
pub fn decode_full_snapshot(
    schema: &schema::Schema,
    bytes: &[u8],
    wire_limits: &wire::Limits,
    limits: &CodecLimits,
) -> CodecResult<Snapshot> {
    let packet = decode_packet(bytes, wire_limits)?;
    decode_full_snapshot_from_packet(schema, &packet, limits)
}

/// Decodes a full snapshot from a parsed wire packet.
pub fn decode_full_snapshot_from_packet(
    schema: &schema::Schema,
    packet: &WirePacket<'_>,
    limits: &CodecLimits,
) -> CodecResult<Snapshot> {
    let header = packet.header;
    if !header.flags.is_full_snapshot() {
        return Err(CodecError::Wire(wire::DecodeError::InvalidFlags {
            flags: header.flags.raw(),
        }));
    }
    if header.baseline_tick != 0 {
        return Err(CodecError::Wire(wire::DecodeError::InvalidBaselineTick {
            baseline_tick: header.baseline_tick,
            flags: header.flags.raw(),
        }));
    }

    let expected_hash = schema_hash(schema);
    if header.schema_hash != expected_hash {
        return Err(CodecError::SchemaMismatch {
            expected: expected_hash,
            found: header.schema_hash,
        });
    }

    let mut entities: Vec<EntitySnapshot> = Vec::new();
    let mut create_seen = false;
    for section in &packet.sections {
        match section.tag {
            SectionTag::EntityCreate => {
                if create_seen {
                    return Err(CodecError::DuplicateSection {
                        section: section.tag,
                    });
                }
                create_seen = true;
                let decoded = decode_create_section(schema, section.body, limits)?;
                entities = decoded;
            }
            _ => {
                return Err(CodecError::UnexpectedSection {
                    section: section.tag,
                });
            }
        }
    }

    Ok(Snapshot {
        tick: SnapshotTick::new(header.tick),
        entities,
    })
}

pub(crate) fn write_section<F>(
    tag: SectionTag,
    out: &mut [u8],
    limits: &CodecLimits,
    write_body: F,
) -> CodecResult<usize>
where
    F: FnOnce(&mut BitWriter<'_>) -> CodecResult<()>,
{
    if out.len() < 1 + VARINT_MAX_BYTES {
        return Err(CodecError::OutputTooSmall {
            needed: 1 + VARINT_MAX_BYTES,
            available: out.len(),
        });
    }

    let body_start = 1 + VARINT_MAX_BYTES;
    let mut writer = BitWriter::new(&mut out[body_start..]);
    write_body(&mut writer)?;
    let body_len = writer.finish();

    if body_len > limits.max_section_bytes {
        return Err(CodecError::LimitsExceeded {
            kind: LimitKind::SectionBytes,
            limit: limits.max_section_bytes,
            actual: body_len,
        });
    }

    let len_u32 = u32::try_from(body_len).map_err(|_| CodecError::OutputTooSmall {
        needed: body_len,
        available: out.len(),
    })?;
    let len_bytes = varu32_len(len_u32);
    let total_needed = 1 + len_bytes + body_len;
    if out.len() < total_needed {
        return Err(CodecError::OutputTooSmall {
            needed: total_needed,
            available: out.len(),
        });
    }

    out[0] = tag as u8;
    write_varu32(len_u32, &mut out[1..1 + len_bytes]);
    let shift = VARINT_MAX_BYTES - len_bytes;
    if shift > 0 {
        let src = body_start..body_start + body_len;
        out.copy_within(src, 1 + len_bytes);
    }
    Ok(total_needed)
}

fn encode_create_body(
    schema: &schema::Schema,
    entities: &[EntitySnapshot],
    limits: &CodecLimits,
    writer: &mut BitWriter<'_>,
) -> CodecResult<()> {
    if schema.components.len() > limits.max_components_per_entity {
        return Err(CodecError::LimitsExceeded {
            kind: LimitKind::ComponentsPerEntity,
            limit: limits.max_components_per_entity,
            actual: schema.components.len(),
        });
    }

    writer.align_to_byte()?;
    writer.write_varu32(entities.len() as u32)?;

    let mut prev_id: Option<u32> = None;
    for entity in entities {
        if let Some(prev) = prev_id {
            if entity.id.raw() <= prev {
                return Err(CodecError::InvalidEntityOrder {
                    previous: prev,
                    current: entity.id.raw(),
                });
            }
        }
        prev_id = Some(entity.id.raw());

        writer.align_to_byte()?;
        writer.write_u32_aligned(entity.id.raw())?;

        if entity.components.len() > limits.max_components_per_entity {
            return Err(CodecError::LimitsExceeded {
                kind: LimitKind::ComponentsPerEntity,
                limit: limits.max_components_per_entity,
                actual: entity.components.len(),
            });
        }

        ensure_known_components(schema, entity)?;

        write_component_mask(schema, entity, writer)?;

        for component in schema.components.iter() {
            if let Some(snapshot) = find_component(entity, component.id) {
                write_component_fields(component, snapshot, limits, writer)?;
            }
        }
    }

    writer.align_to_byte()?;
    Ok(())
}

fn write_component_mask(
    schema: &schema::Schema,
    entity: &EntitySnapshot,
    writer: &mut BitWriter<'_>,
) -> CodecResult<()> {
    for component in &schema.components {
        let present = find_component(entity, component.id).is_some();
        writer.write_bit(present)?;
    }
    Ok(())
}

fn write_component_fields(
    component: &ComponentDef,
    snapshot: &ComponentSnapshot,
    limits: &CodecLimits,
    writer: &mut BitWriter<'_>,
) -> CodecResult<()> {
    if component.fields.len() > limits.max_fields_per_component {
        return Err(CodecError::LimitsExceeded {
            kind: LimitKind::FieldsPerComponent,
            limit: limits.max_fields_per_component,
            actual: component.fields.len(),
        });
    }
    if snapshot.fields.len() != component.fields.len() {
        return Err(CodecError::InvalidMask {
            kind: MaskKind::FieldMask {
                component: component.id,
            },
            reason: MaskReason::FieldCountMismatch {
                expected: component.fields.len(),
                actual: snapshot.fields.len(),
            },
        });
    }
    if snapshot.fields.len() > limits.max_fields_per_component {
        return Err(CodecError::LimitsExceeded {
            kind: LimitKind::FieldsPerComponent,
            limit: limits.max_fields_per_component,
            actual: snapshot.fields.len(),
        });
    }

    for _field in &component.fields {
        writer.write_bit(true)?;
    }

    for (field, value) in component.fields.iter().zip(snapshot.fields.iter()) {
        write_field_value(component.id, *field, *value, writer)?;
    }
    Ok(())
}

pub(crate) fn write_field_value(
    component_id: ComponentId,
    field: FieldDef,
    value: FieldValue,
    writer: &mut BitWriter<'_>,
) -> CodecResult<()> {
    match (field.codec, value) {
        (FieldCodec::Bool, FieldValue::Bool(v)) => writer.write_bit(v)?,
        (FieldCodec::UInt { bits }, FieldValue::UInt(v)) => {
            validate_uint(component_id, field.id, bits, v)?;
            writer.write_bits(v, bits)?;
        }
        (FieldCodec::SInt { bits }, FieldValue::SInt(v)) => {
            let encoded = encode_sint(component_id, field.id, bits, v)?;
            writer.write_bits(encoded, bits)?;
        }
        (FieldCodec::VarUInt, FieldValue::VarUInt(v)) => {
            if v > u32::MAX as u64 {
                return Err(CodecError::InvalidValue {
                    component: component_id,
                    field: field.id,
                    reason: ValueReason::VarUIntOutOfRange { value: v },
                });
            }
            writer.align_to_byte()?;
            writer.write_varu32(v as u32)?;
        }
        (FieldCodec::VarSInt, FieldValue::VarSInt(v)) => {
            if v < i32::MIN as i64 || v > i32::MAX as i64 {
                return Err(CodecError::InvalidValue {
                    component: component_id,
                    field: field.id,
                    reason: ValueReason::VarSIntOutOfRange { value: v },
                });
            }
            writer.align_to_byte()?;
            writer.write_vars32(v as i32)?;
        }
        (FieldCodec::FixedPoint(fp), FieldValue::FixedPoint(v)) => {
            if v < fp.min_q || v > fp.max_q {
                return Err(CodecError::InvalidValue {
                    component: component_id,
                    field: field.id,
                    reason: ValueReason::FixedPointOutOfRange {
                        min_q: fp.min_q,
                        max_q: fp.max_q,
                        value: v,
                    },
                });
            }
            let offset = (v - fp.min_q) as u64;
            let range = (fp.max_q - fp.min_q) as u64;
            let bits = required_bits(range);
            if bits > 0 {
                writer.write_bits(offset, bits)?;
            }
        }
        _ => {
            return Err(CodecError::InvalidValue {
                component: component_id,
                field: field.id,
                reason: ValueReason::TypeMismatch {
                    expected: codec_name(field.codec),
                    found: value_name(value),
                },
            });
        }
    }
    Ok(())
}

fn decode_create_section(
    schema: &schema::Schema,
    body: &[u8],
    limits: &CodecLimits,
) -> CodecResult<Vec<EntitySnapshot>> {
    if body.len() > limits.max_section_bytes {
        return Err(CodecError::LimitsExceeded {
            kind: LimitKind::SectionBytes,
            limit: limits.max_section_bytes,
            actual: body.len(),
        });
    }

    let mut reader = BitReader::new(body);
    reader.align_to_byte()?;
    let count = reader.read_varu32()? as usize;

    if count > limits.max_entities_create {
        return Err(CodecError::LimitsExceeded {
            kind: LimitKind::EntitiesCreate,
            limit: limits.max_entities_create,
            actual: count,
        });
    }

    if schema.components.len() > limits.max_components_per_entity {
        return Err(CodecError::LimitsExceeded {
            kind: LimitKind::ComponentsPerEntity,
            limit: limits.max_components_per_entity,
            actual: schema.components.len(),
        });
    }

    let mut entities = Vec::with_capacity(count);
    let mut prev_id: Option<u32> = None;
    for _ in 0..count {
        reader.align_to_byte()?;
        let entity_id = reader.read_u32_aligned()?;
        if let Some(prev) = prev_id {
            if entity_id <= prev {
                return Err(CodecError::InvalidEntityOrder {
                    previous: prev,
                    current: entity_id,
                });
            }
        }
        prev_id = Some(entity_id);

        let component_mask = read_mask(
            &mut reader,
            schema.components.len(),
            MaskKind::ComponentMask,
        )?;

        let mut components = Vec::new();
        for (idx, component) in schema.components.iter().enumerate() {
            if component_mask[idx] {
                let fields = decode_component_fields(component, &mut reader, limits)?;
                components.push(ComponentSnapshot {
                    id: component.id,
                    fields,
                });
            }
        }

        entities.push(EntitySnapshot {
            id: EntityId::new(entity_id),
            components,
        });
    }

    reader.align_to_byte()?;
    let remaining_bits = reader.bits_remaining();
    if remaining_bits != 0 {
        return Err(CodecError::TrailingSectionData {
            section: SectionTag::EntityCreate,
            remaining_bits,
        });
    }

    Ok(entities)
}

fn decode_component_fields(
    component: &ComponentDef,
    reader: &mut BitReader<'_>,
    limits: &CodecLimits,
) -> CodecResult<Vec<FieldValue>> {
    if component.fields.len() > limits.max_fields_per_component {
        return Err(CodecError::LimitsExceeded {
            kind: LimitKind::FieldsPerComponent,
            limit: limits.max_fields_per_component,
            actual: component.fields.len(),
        });
    }

    let mask = read_mask(
        reader,
        component.fields.len(),
        MaskKind::FieldMask {
            component: component.id,
        },
    )?;

    let mut values = Vec::with_capacity(component.fields.len());
    for (idx, field) in component.fields.iter().enumerate() {
        if !mask[idx] {
            return Err(CodecError::InvalidMask {
                kind: MaskKind::FieldMask {
                    component: component.id,
                },
                reason: MaskReason::MissingField { field: field.id },
            });
        }
        let value = read_field_value(component.id, *field, reader)?;
        values.push(value);
    }
    Ok(values)
}

pub(crate) fn read_field_value(
    component_id: ComponentId,
    field: FieldDef,
    reader: &mut BitReader<'_>,
) -> CodecResult<FieldValue> {
    match field.codec {
        FieldCodec::Bool => Ok(FieldValue::Bool(reader.read_bit()?)),
        FieldCodec::UInt { bits } => {
            let value = reader.read_bits(bits)?;
            validate_uint(component_id, field.id, bits, value)?;
            Ok(FieldValue::UInt(value))
        }
        FieldCodec::SInt { bits } => {
            let raw = reader.read_bits(bits)?;
            let value = decode_sint(bits, raw)?;
            Ok(FieldValue::SInt(value))
        }
        FieldCodec::VarUInt => {
            reader.align_to_byte()?;
            let value = reader.read_varu32()? as u64;
            Ok(FieldValue::VarUInt(value))
        }
        FieldCodec::VarSInt => {
            reader.align_to_byte()?;
            let value = reader.read_vars32()? as i64;
            Ok(FieldValue::VarSInt(value))
        }
        FieldCodec::FixedPoint(fp) => {
            let range = (fp.max_q - fp.min_q) as u64;
            let bits = required_bits(range);
            let offset = if bits == 0 {
                0
            } else {
                reader.read_bits(bits)?
            };
            let value = fp.min_q + offset as i64;
            if value < fp.min_q || value > fp.max_q {
                return Err(CodecError::InvalidValue {
                    component: component_id,
                    field: field.id,
                    reason: ValueReason::FixedPointOutOfRange {
                        min_q: fp.min_q,
                        max_q: fp.max_q,
                        value,
                    },
                });
            }
            Ok(FieldValue::FixedPoint(value))
        }
    }
}

pub(crate) fn read_mask(
    reader: &mut BitReader<'_>,
    expected_bits: usize,
    kind: MaskKind,
) -> CodecResult<Vec<bool>> {
    if reader.bits_remaining() < expected_bits {
        return Err(CodecError::InvalidMask {
            kind,
            reason: MaskReason::NotEnoughBits {
                expected: expected_bits,
                available: reader.bits_remaining(),
            },
        });
    }

    let mut mask = Vec::with_capacity(expected_bits);
    for _ in 0..expected_bits {
        mask.push(reader.read_bit()?);
    }
    Ok(mask)
}

pub(crate) fn ensure_known_components(
    schema: &schema::Schema,
    entity: &EntitySnapshot,
) -> CodecResult<()> {
    for component in &entity.components {
        if schema.components.iter().all(|c| c.id != component.id) {
            return Err(CodecError::InvalidMask {
                kind: MaskKind::ComponentMask,
                reason: MaskReason::UnknownComponent {
                    component: component.id,
                },
            });
        }
    }
    Ok(())
}

fn find_component(entity: &EntitySnapshot, id: ComponentId) -> Option<&ComponentSnapshot> {
    entity.components.iter().find(|c| c.id == id)
}

fn validate_uint(
    component_id: ComponentId,
    field_id: FieldId,
    bits: u8,
    value: u64,
) -> CodecResult<()> {
    if bits == 64 {
        return Ok(());
    }
    let max = 1u128 << bits;
    if value as u128 >= max {
        return Err(CodecError::InvalidValue {
            component: component_id,
            field: field_id,
            reason: ValueReason::UnsignedOutOfRange { bits, value },
        });
    }
    Ok(())
}

fn encode_sint(
    component_id: ComponentId,
    field_id: FieldId,
    bits: u8,
    value: i64,
) -> CodecResult<u64> {
    if bits == 64 {
        return Ok(value as u64);
    }
    let min = -(1i128 << (bits - 1));
    let max = (1i128 << (bits - 1)) - 1;
    let value_i128 = value as i128;
    if value_i128 < min || value_i128 > max {
        return Err(CodecError::InvalidValue {
            component: component_id,
            field: field_id,
            reason: ValueReason::SignedOutOfRange { bits, value },
        });
    }
    let mask = (1u64 << bits) - 1;
    Ok((value as u64) & mask)
}

fn decode_sint(bits: u8, raw: u64) -> CodecResult<i64> {
    if bits == 64 {
        return Ok(raw as i64);
    }
    if bits == 0 {
        return Ok(0);
    }
    let sign_bit = 1u64 << (bits - 1);
    if raw & sign_bit == 0 {
        Ok(raw as i64)
    } else {
        let mask = (1u64 << bits) - 1;
        let value = (raw & mask) as i64;
        Ok(value - (1i64 << bits))
    }
}

pub(crate) fn required_bits(range: u64) -> u8 {
    if range == 0 {
        return 0;
    }
    (64 - range.leading_zeros()) as u8
}

fn codec_name(codec: FieldCodec) -> &'static str {
    match codec {
        FieldCodec::Bool => "bool",
        FieldCodec::UInt { .. } => "uint",
        FieldCodec::SInt { .. } => "sint",
        FieldCodec::VarUInt => "varuint",
        FieldCodec::VarSInt => "varsint",
        FieldCodec::FixedPoint(_) => "fixed-point",
    }
}

fn value_name(value: FieldValue) -> &'static str {
    match value {
        FieldValue::Bool(_) => "bool",
        FieldValue::UInt(_) => "uint",
        FieldValue::SInt(_) => "sint",
        FieldValue::VarUInt(_) => "varuint",
        FieldValue::VarSInt(_) => "varsint",
        FieldValue::FixedPoint(_) => "fixed-point",
    }
}

fn varu32_len(mut value: u32) -> usize {
    let mut len = 1;
    while value >= 0x80 {
        value >>= 7;
        len += 1;
    }
    len
}

fn write_varu32(mut value: u32, out: &mut [u8]) {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use schema::{ComponentDef, FieldCodec, FieldDef, FieldId, Schema};

    fn schema_one_bool() -> Schema {
        let component = ComponentDef::new(ComponentId::new(1).unwrap())
            .field(FieldDef::new(FieldId::new(1).unwrap(), FieldCodec::bool()));
        Schema::new(vec![component]).unwrap()
    }

    fn schema_bool_uint10() -> Schema {
        let component = ComponentDef::new(ComponentId::new(1).unwrap())
            .field(FieldDef::new(FieldId::new(1).unwrap(), FieldCodec::bool()))
            .field(FieldDef::new(
                FieldId::new(2).unwrap(),
                FieldCodec::uint(10),
            ));
        Schema::new(vec![component]).unwrap()
    }

    #[test]
    fn full_snapshot_roundtrip_minimal() {
        let schema = schema_one_bool();
        let snapshot = Snapshot {
            tick: SnapshotTick::new(1),
            entities: vec![EntitySnapshot {
                id: EntityId::new(1),
                components: vec![ComponentSnapshot {
                    id: ComponentId::new(1).unwrap(),
                    fields: vec![FieldValue::Bool(true)],
                }],
            }],
        };

        let mut buf = [0u8; 128];
        let bytes = encode_full_snapshot(
            &schema,
            snapshot.tick,
            &snapshot.entities,
            &CodecLimits::for_testing(),
            &mut buf,
        )
        .unwrap();
        let decoded = decode_full_snapshot(
            &schema,
            &buf[..bytes],
            &wire::Limits::for_testing(),
            &CodecLimits::for_testing(),
        )
        .unwrap();
        assert_eq!(decoded.entities, snapshot.entities);
    }

    #[test]
    fn full_snapshot_golden_bytes() {
        let schema = schema_one_bool();
        let entities = vec![EntitySnapshot {
            id: EntityId::new(1),
            components: vec![ComponentSnapshot {
                id: ComponentId::new(1).unwrap(),
                fields: vec![FieldValue::Bool(true)],
            }],
        }];

        let mut buf = [0u8; 128];
        let bytes = encode_full_snapshot(
            &schema,
            SnapshotTick::new(1),
            &entities,
            &CodecLimits::for_testing(),
            &mut buf,
        )
        .unwrap();

        let mut expected = Vec::new();
        expected.extend_from_slice(&wire::MAGIC.to_le_bytes());
        expected.extend_from_slice(&wire::VERSION.to_le_bytes());
        expected.extend_from_slice(&wire::PacketFlags::full_snapshot().raw().to_le_bytes());
        expected.extend_from_slice(&0x32F5_A224_657B_EE15u64.to_le_bytes());
        expected.extend_from_slice(&1u32.to_le_bytes());
        expected.extend_from_slice(&0u32.to_le_bytes());
        expected.extend_from_slice(&8u32.to_le_bytes());
        expected.extend_from_slice(&[SectionTag::EntityCreate as u8, 6, 1, 1, 0, 0, 0, 0xE0]);

        assert_eq!(&buf[..bytes], expected.as_slice());
    }

    #[test]
    fn full_snapshot_golden_fixture_two_fields() {
        let schema = schema_bool_uint10();
        let entities = vec![EntitySnapshot {
            id: EntityId::new(1),
            components: vec![ComponentSnapshot {
                id: ComponentId::new(1).unwrap(),
                fields: vec![FieldValue::Bool(true), FieldValue::UInt(513)],
            }],
        }];

        let mut buf = [0u8; 128];
        let bytes = encode_full_snapshot(
            &schema,
            SnapshotTick::new(1),
            &entities,
            &CodecLimits::for_testing(),
            &mut buf,
        )
        .unwrap();

        let mut expected = Vec::new();
        expected.extend_from_slice(&wire::MAGIC.to_le_bytes());
        expected.extend_from_slice(&wire::VERSION.to_le_bytes());
        expected.extend_from_slice(&wire::PacketFlags::full_snapshot().raw().to_le_bytes());
        expected.extend_from_slice(&0x57B2_2433_26F2_2706u64.to_le_bytes());
        expected.extend_from_slice(&1u32.to_le_bytes());
        expected.extend_from_slice(&0u32.to_le_bytes());
        expected.extend_from_slice(&9u32.to_le_bytes());
        expected.extend_from_slice(&[SectionTag::EntityCreate as u8, 7, 1, 1, 0, 0, 0, 0xF8, 0x04]);

        assert_eq!(&buf[..bytes], expected.as_slice());
    }

    #[test]
    fn decode_rejects_trailing_bytes() {
        let schema = schema_one_bool();
        let entities = vec![EntitySnapshot {
            id: EntityId::new(1),
            components: vec![ComponentSnapshot {
                id: ComponentId::new(1).unwrap(),
                fields: vec![FieldValue::Bool(true)],
            }],
        }];

        let mut buf = [0u8; 128];
        let bytes = encode_full_snapshot(
            &schema,
            SnapshotTick::new(1),
            &entities,
            &CodecLimits::for_testing(),
            &mut buf,
        )
        .unwrap();

        // Add a trailing padding byte to the section body and patch lengths.
        let mut extra = buf[..bytes].to_vec();
        extra[wire::HEADER_SIZE + 1] = 7; // section length varint
        let payload_len = 9u32;
        extra[24..28].copy_from_slice(&payload_len.to_le_bytes());
        extra.push(0);

        let err = decode_full_snapshot(
            &schema,
            &extra,
            &wire::Limits::for_testing(),
            &CodecLimits::for_testing(),
        )
        .unwrap_err();
        assert!(matches!(err, CodecError::TrailingSectionData { .. }));
    }

    #[test]
    fn decode_rejects_excessive_entity_count_early() {
        let schema = schema_one_bool();
        let limits = CodecLimits::for_testing();
        let count = (limits.max_entities_create as u32) + 1;

        let mut body = [0u8; 8];
        write_varu32(count, &mut body);
        let body_len = varu32_len(count);
        let mut section_buf = [0u8; 16];
        let section_len = wire::encode_section(
            SectionTag::EntityCreate,
            &body[..body_len],
            &mut section_buf,
        )
        .unwrap();

        let payload_len = section_len as u32;
        let header = wire::PacketHeader::full_snapshot(schema_hash(&schema), 1, payload_len);
        let mut buf = [0u8; wire::HEADER_SIZE + 16];
        encode_header(&header, &mut buf[..wire::HEADER_SIZE]).unwrap();
        buf[wire::HEADER_SIZE..wire::HEADER_SIZE + section_len]
            .copy_from_slice(&section_buf[..section_len]);
        let buf = &buf[..wire::HEADER_SIZE + section_len];

        let err =
            decode_full_snapshot(&schema, buf, &wire::Limits::for_testing(), &limits).unwrap_err();
        assert!(matches!(
            err,
            CodecError::LimitsExceeded {
                kind: LimitKind::EntitiesCreate,
                ..
            }
        ));
    }

    #[test]
    fn decode_rejects_truncated_prefixes() {
        let schema = schema_one_bool();
        let entities = vec![EntitySnapshot {
            id: EntityId::new(1),
            components: vec![ComponentSnapshot {
                id: ComponentId::new(1).unwrap(),
                fields: vec![FieldValue::Bool(true)],
            }],
        }];

        let mut buf = [0u8; 128];
        let bytes = encode_full_snapshot(
            &schema,
            SnapshotTick::new(1),
            &entities,
            &CodecLimits::for_testing(),
            &mut buf,
        )
        .unwrap();

        for len in 0..bytes {
            let result = decode_full_snapshot(
                &schema,
                &buf[..len],
                &wire::Limits::for_testing(),
                &CodecLimits::for_testing(),
            );
            assert!(result.is_err());
        }
    }

    #[test]
    fn encode_is_deterministic_for_same_input() {
        let schema = schema_one_bool();
        let entities = vec![EntitySnapshot {
            id: EntityId::new(1),
            components: vec![ComponentSnapshot {
                id: ComponentId::new(1).unwrap(),
                fields: vec![FieldValue::Bool(true)],
            }],
        }];

        let mut buf1 = [0u8; 128];
        let mut buf2 = [0u8; 128];
        let bytes1 = encode_full_snapshot(
            &schema,
            SnapshotTick::new(1),
            &entities,
            &CodecLimits::for_testing(),
            &mut buf1,
        )
        .unwrap();
        let bytes2 = encode_full_snapshot(
            &schema,
            SnapshotTick::new(1),
            &entities,
            &CodecLimits::for_testing(),
            &mut buf2,
        )
        .unwrap();

        assert_eq!(&buf1[..bytes1], &buf2[..bytes2]);
    }

    #[test]
    fn decode_rejects_missing_field_mask() {
        let schema = schema_one_bool();
        let entities = vec![EntitySnapshot {
            id: EntityId::new(1),
            components: vec![ComponentSnapshot {
                id: ComponentId::new(1).unwrap(),
                fields: vec![FieldValue::Bool(true)],
            }],
        }];

        let mut buf = [0u8; 128];
        let bytes = encode_full_snapshot(
            &schema,
            SnapshotTick::new(1),
            &entities,
            &CodecLimits::for_testing(),
            &mut buf,
        )
        .unwrap();

        // Flip the field mask bit off (component mask stays on).
        let payload_start = wire::HEADER_SIZE;
        let mask_offset = payload_start + 2 + 1 + 4; // tag + len + count + entity_id
        buf[mask_offset] &= 0b1011_1111;

        let err = decode_full_snapshot(
            &schema,
            &buf[..bytes],
            &wire::Limits::for_testing(),
            &CodecLimits::for_testing(),
        )
        .unwrap_err();
        assert!(matches!(err, CodecError::InvalidMask { .. }));
    }

    #[test]
    fn encode_rejects_unsorted_entities() {
        let schema = schema_one_bool();
        let entities = vec![
            EntitySnapshot {
                id: EntityId::new(2),
                components: vec![ComponentSnapshot {
                    id: ComponentId::new(1).unwrap(),
                    fields: vec![FieldValue::Bool(true)],
                }],
            },
            EntitySnapshot {
                id: EntityId::new(1),
                components: vec![ComponentSnapshot {
                    id: ComponentId::new(1).unwrap(),
                    fields: vec![FieldValue::Bool(false)],
                }],
            },
        ];

        let mut buf = [0u8; 128];
        let err = encode_full_snapshot(
            &schema,
            SnapshotTick::new(1),
            &entities,
            &CodecLimits::for_testing(),
            &mut buf,
        )
        .unwrap_err();
        assert!(matches!(err, CodecError::InvalidEntityOrder { .. }));
    }
}
