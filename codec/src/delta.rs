//! Delta snapshot encoding/decoding.

use bitstream::{BitReader, BitWriter};
use schema::{schema_hash, ChangePolicy, ComponentDef, ComponentId, FieldDef};
use wire::{decode_packet, encode_header, SectionTag, WirePacket};

use crate::baseline::BaselineStore;
use crate::error::{CodecError, CodecResult, LimitKind, MaskKind, MaskReason, ValueReason};
use crate::limits::CodecLimits;
use crate::scratch::CodecScratch;
use crate::snapshot::{
    ensure_known_components, read_field_value, read_mask, write_field_value, write_section,
    ComponentSnapshot, EntitySnapshot, FieldValue, Snapshot,
};
use crate::types::{EntityId, SnapshotTick};

/// Selects the latest baseline tick at or before the ack tick.
#[must_use]
pub fn select_baseline_tick<T>(
    store: &BaselineStore<T>,
    ack_tick: SnapshotTick,
) -> Option<SnapshotTick> {
    store.latest_at_or_before(ack_tick).map(|(tick, _)| tick)
}

/// Encodes a delta snapshot into the provided output buffer.
///
/// Baseline and current snapshots must have entities sorted by `EntityId`.
pub fn encode_delta_snapshot(
    schema: &schema::Schema,
    tick: SnapshotTick,
    baseline_tick: SnapshotTick,
    baseline: &Snapshot,
    current: &Snapshot,
    limits: &CodecLimits,
    out: &mut [u8],
) -> CodecResult<usize> {
    let mut scratch = CodecScratch::default();
    encode_delta_snapshot_with_scratch(
        schema,
        tick,
        baseline_tick,
        baseline,
        current,
        limits,
        &mut scratch,
        out,
    )
}

/// Encodes a delta snapshot using reusable scratch buffers.
#[allow(clippy::too_many_arguments)]
pub fn encode_delta_snapshot_with_scratch(
    schema: &schema::Schema,
    tick: SnapshotTick,
    baseline_tick: SnapshotTick,
    baseline: &Snapshot,
    current: &Snapshot,
    limits: &CodecLimits,
    scratch: &mut CodecScratch,
    out: &mut [u8],
) -> CodecResult<usize> {
    if out.len() < wire::HEADER_SIZE {
        return Err(CodecError::OutputTooSmall {
            needed: wire::HEADER_SIZE,
            available: out.len(),
        });
    }

    if baseline.tick != baseline_tick {
        return Err(CodecError::BaselineTickMismatch {
            expected: baseline.tick.raw(),
            found: baseline_tick.raw(),
        });
    }

    ensure_entities_sorted(&baseline.entities)?;
    ensure_entities_sorted(&current.entities)?;

    let mut counts = DiffCounts::default();
    diff_counts(schema, baseline, current, limits, &mut counts)?;

    if counts.creates > limits.max_entities_create {
        return Err(CodecError::LimitsExceeded {
            kind: LimitKind::EntitiesCreate,
            limit: limits.max_entities_create,
            actual: counts.creates,
        });
    }
    if counts.updates > limits.max_entities_update {
        return Err(CodecError::LimitsExceeded {
            kind: LimitKind::EntitiesUpdate,
            limit: limits.max_entities_update,
            actual: counts.updates,
        });
    }
    if counts.destroys > limits.max_entities_destroy {
        return Err(CodecError::LimitsExceeded {
            kind: LimitKind::EntitiesDestroy,
            limit: limits.max_entities_destroy,
            actual: counts.destroys,
        });
    }

    let mut offset = wire::HEADER_SIZE;
    if counts.destroys > 0 {
        let written = write_section(
            SectionTag::EntityDestroy,
            &mut out[offset..],
            limits,
            |writer| encode_destroy_body(baseline, current, counts.destroys, limits, writer),
        )?;
        offset += written;
    }
    if counts.creates > 0 {
        let written = write_section(
            SectionTag::EntityCreate,
            &mut out[offset..],
            limits,
            |writer| encode_create_body(schema, baseline, current, counts.creates, limits, writer),
        )?;
        offset += written;
    }
    if counts.updates > 0 {
        let written = write_section(
            SectionTag::EntityUpdate,
            &mut out[offset..],
            limits,
            |writer| {
                encode_update_body(
                    schema,
                    baseline,
                    current,
                    counts.updates,
                    limits,
                    scratch,
                    writer,
                )
            },
        )?;
        offset += written;
    }

    let payload_len = offset - wire::HEADER_SIZE;
    let header = wire::PacketHeader::delta_snapshot(
        schema_hash(schema),
        tick.raw(),
        baseline_tick.raw(),
        payload_len as u32,
    );
    encode_header(&header, &mut out[..wire::HEADER_SIZE]).map_err(|_| {
        CodecError::OutputTooSmall {
            needed: wire::HEADER_SIZE,
            available: out.len(),
        }
    })?;

    Ok(offset)
}

/// Applies a delta snapshot to a baseline snapshot.
pub fn apply_delta_snapshot(
    schema: &schema::Schema,
    baseline: &Snapshot,
    bytes: &[u8],
    wire_limits: &wire::Limits,
    limits: &CodecLimits,
) -> CodecResult<Snapshot> {
    let packet = decode_packet(bytes, wire_limits)?;
    apply_delta_snapshot_from_packet(schema, baseline, &packet, limits)
}

/// Applies a delta snapshot from a parsed wire packet.
pub fn apply_delta_snapshot_from_packet(
    schema: &schema::Schema,
    baseline: &Snapshot,
    packet: &WirePacket<'_>,
    limits: &CodecLimits,
) -> CodecResult<Snapshot> {
    let header = packet.header;
    if !header.flags.is_delta_snapshot() {
        return Err(CodecError::Wire(wire::DecodeError::InvalidFlags {
            flags: header.flags.raw(),
        }));
    }
    if header.baseline_tick == 0 {
        return Err(CodecError::Wire(wire::DecodeError::InvalidBaselineTick {
            baseline_tick: header.baseline_tick,
            flags: header.flags.raw(),
        }));
    }
    if header.baseline_tick != baseline.tick.raw() {
        return Err(CodecError::BaselineTickMismatch {
            expected: baseline.tick.raw(),
            found: header.baseline_tick,
        });
    }

    if header.baseline_tick == 0 {
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

    let (destroys, creates, updates) = decode_delta_sections(schema, packet, limits)?;

    ensure_entities_sorted(&baseline.entities)?;
    ensure_entities_sorted(&creates)?;

    let mut remaining = apply_destroys(&baseline.entities, &destroys)?;
    remaining = apply_creates(remaining, creates)?;
    if remaining.len() > limits.max_total_entities_after_apply {
        return Err(CodecError::LimitsExceeded {
            kind: LimitKind::TotalEntitiesAfterApply,
            limit: limits.max_total_entities_after_apply,
            actual: remaining.len(),
        });
    }
    apply_updates(&mut remaining, &updates)?;

    Ok(Snapshot {
        tick: SnapshotTick::new(header.tick),
        entities: remaining,
    })
}

/// Decodes a delta packet without applying it to a baseline.
pub fn decode_delta_packet(
    schema: &schema::Schema,
    packet: &WirePacket<'_>,
    limits: &CodecLimits,
) -> CodecResult<DeltaDecoded> {
    let header = packet.header;
    if !header.flags.is_delta_snapshot() {
        return Err(CodecError::Wire(wire::DecodeError::InvalidFlags {
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

    let (destroys, creates, updates) = decode_delta_sections(schema, packet, limits)?;

    Ok(DeltaDecoded {
        tick: SnapshotTick::new(header.tick),
        baseline_tick: SnapshotTick::new(header.baseline_tick),
        destroys,
        creates,
        updates,
    })
}

#[derive(Default)]
struct DiffCounts {
    creates: usize,
    updates: usize,
    destroys: usize,
}

fn diff_counts(
    schema: &schema::Schema,
    baseline: &Snapshot,
    current: &Snapshot,
    limits: &CodecLimits,
    counts: &mut DiffCounts,
) -> CodecResult<()> {
    let mut i = 0usize;
    let mut j = 0usize;
    while i < baseline.entities.len() || j < current.entities.len() {
        let base = baseline.entities.get(i);
        let curr = current.entities.get(j);
        match (base, curr) {
            (Some(b), Some(c)) => {
                if b.id.raw() < c.id.raw() {
                    counts.destroys += 1;
                    i += 1;
                } else if b.id.raw() > c.id.raw() {
                    counts.creates += 1;
                    j += 1;
                } else {
                    if entity_has_updates(schema, b, c, limits)? {
                        counts.updates += 1;
                    }
                    i += 1;
                    j += 1;
                }
            }
            (Some(_), None) => {
                counts.destroys += 1;
                i += 1;
            }
            (None, Some(_)) => {
                counts.creates += 1;
                j += 1;
            }
            (None, None) => break,
        }
    }
    Ok(())
}

fn encode_destroy_body(
    baseline: &Snapshot,
    current: &Snapshot,
    destroy_count: usize,
    limits: &CodecLimits,
    writer: &mut BitWriter<'_>,
) -> CodecResult<()> {
    if destroy_count > limits.max_entities_destroy {
        return Err(CodecError::LimitsExceeded {
            kind: LimitKind::EntitiesDestroy,
            limit: limits.max_entities_destroy,
            actual: destroy_count,
        });
    }

    writer.align_to_byte()?;
    writer.write_varu32(destroy_count as u32)?;

    let mut i = 0usize;
    let mut j = 0usize;
    while i < baseline.entities.len() || j < current.entities.len() {
        let base = baseline.entities.get(i);
        let curr = current.entities.get(j);
        match (base, curr) {
            (Some(b), Some(c)) => {
                if b.id.raw() < c.id.raw() {
                    writer.align_to_byte()?;
                    writer.write_u32_aligned(b.id.raw())?;
                    i += 1;
                } else if b.id.raw() > c.id.raw() {
                    j += 1;
                } else {
                    i += 1;
                    j += 1;
                }
            }
            (Some(b), None) => {
                writer.align_to_byte()?;
                writer.write_u32_aligned(b.id.raw())?;
                i += 1;
            }
            (None, Some(_)) => {
                j += 1;
            }
            (None, None) => break,
        }
    }

    writer.align_to_byte()?;
    Ok(())
}

fn encode_create_body(
    schema: &schema::Schema,
    baseline: &Snapshot,
    current: &Snapshot,
    create_count: usize,
    limits: &CodecLimits,
    writer: &mut BitWriter<'_>,
) -> CodecResult<()> {
    if create_count > limits.max_entities_create {
        return Err(CodecError::LimitsExceeded {
            kind: LimitKind::EntitiesCreate,
            limit: limits.max_entities_create,
            actual: create_count,
        });
    }

    writer.align_to_byte()?;
    writer.write_varu32(create_count as u32)?;

    let mut i = 0usize;
    let mut j = 0usize;
    while i < baseline.entities.len() || j < current.entities.len() {
        let base = baseline.entities.get(i);
        let curr = current.entities.get(j);
        match (base, curr) {
            (Some(b), Some(c)) => {
                if b.id.raw() < c.id.raw() {
                    i += 1;
                } else if b.id.raw() > c.id.raw() {
                    write_create_entity(schema, c, limits, writer)?;
                    j += 1;
                } else {
                    i += 1;
                    j += 1;
                }
            }
            (Some(_), None) => {
                i += 1;
            }
            (None, Some(c)) => {
                write_create_entity(schema, c, limits, writer)?;
                j += 1;
            }
            (None, None) => break,
        }
    }

    writer.align_to_byte()?;
    Ok(())
}

fn encode_update_body(
    schema: &schema::Schema,
    baseline: &Snapshot,
    current: &Snapshot,
    update_count: usize,
    limits: &CodecLimits,
    scratch: &mut CodecScratch,
    writer: &mut BitWriter<'_>,
) -> CodecResult<()> {
    if update_count > limits.max_entities_update {
        return Err(CodecError::LimitsExceeded {
            kind: LimitKind::EntitiesUpdate,
            limit: limits.max_entities_update,
            actual: update_count,
        });
    }

    writer.align_to_byte()?;
    writer.write_varu32(update_count as u32)?;

    let mut i = 0usize;
    let mut j = 0usize;
    while i < baseline.entities.len() || j < current.entities.len() {
        let base = baseline.entities.get(i);
        let curr = current.entities.get(j);
        match (base, curr) {
            (Some(b), Some(c)) => {
                if b.id.raw() < c.id.raw() {
                    i += 1;
                } else if b.id.raw() > c.id.raw() {
                    j += 1;
                } else {
                    if entity_has_updates(schema, b, c, limits)? {
                        writer.align_to_byte()?;
                        writer.write_u32_aligned(c.id.raw())?;
                        ensure_component_presence_matches(schema, b, c)?;
                        write_update_components(schema, b, c, limits, scratch, writer)?;
                    }
                    i += 1;
                    j += 1;
                }
            }
            (Some(_), None) => i += 1,
            (None, Some(_)) => j += 1,
            (None, None) => break,
        }
    }

    writer.align_to_byte()?;
    Ok(())
}

fn write_create_entity(
    schema: &schema::Schema,
    entity: &EntitySnapshot,
    limits: &CodecLimits,
    writer: &mut BitWriter<'_>,
) -> CodecResult<()> {
    writer.align_to_byte()?;
    writer.write_u32_aligned(entity.id.raw())?;
    ensure_known_components(schema, entity)?;
    write_component_mask(schema, entity, writer)?;
    for component in schema.components.iter() {
        if let Some(snapshot) = find_component(entity, component.id) {
            write_full_component(component, snapshot, limits, writer)?;
        }
    }
    Ok(())
}

fn decode_destroy_section(body: &[u8], limits: &CodecLimits) -> CodecResult<Vec<EntityId>> {
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
    if count > limits.max_entities_destroy {
        return Err(CodecError::LimitsExceeded {
            kind: LimitKind::EntitiesDestroy,
            limit: limits.max_entities_destroy,
            actual: count,
        });
    }

    let mut ids = Vec::with_capacity(count);
    let mut prev: Option<u32> = None;
    for _ in 0..count {
        reader.align_to_byte()?;
        let id = reader.read_u32_aligned()?;
        if let Some(prev_id) = prev {
            if id <= prev_id {
                return Err(CodecError::InvalidEntityOrder {
                    previous: prev_id,
                    current: id,
                });
            }
        }
        prev = Some(id);
        ids.push(EntityId::new(id));
    }
    reader.align_to_byte()?;
    if reader.bits_remaining() != 0 {
        return Err(CodecError::TrailingSectionData {
            section: SectionTag::EntityDestroy,
            remaining_bits: reader.bits_remaining(),
        });
    }
    Ok(ids)
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

    let mut entities = Vec::with_capacity(count);
    let mut prev: Option<u32> = None;
    for _ in 0..count {
        reader.align_to_byte()?;
        let id = reader.read_u32_aligned()?;
        if let Some(prev_id) = prev {
            if id <= prev_id {
                return Err(CodecError::InvalidEntityOrder {
                    previous: prev_id,
                    current: id,
                });
            }
        }
        prev = Some(id);

        let component_mask = read_mask(
            &mut reader,
            schema.components.len(),
            MaskKind::ComponentMask,
        )?;

        let mut components = Vec::new();
        for (idx, component) in schema.components.iter().enumerate() {
            if component_mask[idx] {
                let fields = decode_full_component(component, &mut reader, limits)?;
                components.push(ComponentSnapshot {
                    id: component.id,
                    fields,
                });
            }
        }

        let entity = EntitySnapshot {
            id: EntityId::new(id),
            components,
        };
        ensure_known_components(schema, &entity)?;
        entities.push(entity);
    }

    reader.align_to_byte()?;
    if reader.bits_remaining() != 0 {
        return Err(CodecError::TrailingSectionData {
            section: SectionTag::EntityCreate,
            remaining_bits: reader.bits_remaining(),
        });
    }
    Ok(entities)
}

fn decode_update_section(
    schema: &schema::Schema,
    body: &[u8],
    limits: &CodecLimits,
) -> CodecResult<Vec<DeltaUpdateEntity>> {
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
    if count > limits.max_entities_update {
        return Err(CodecError::LimitsExceeded {
            kind: LimitKind::EntitiesUpdate,
            limit: limits.max_entities_update,
            actual: count,
        });
    }

    let mut updates = Vec::with_capacity(count);
    let mut prev: Option<u32> = None;
    for _ in 0..count {
        reader.align_to_byte()?;
        let id = reader.read_u32_aligned()?;
        if let Some(prev_id) = prev {
            if id <= prev_id {
                return Err(CodecError::InvalidEntityOrder {
                    previous: prev_id,
                    current: id,
                });
            }
        }
        prev = Some(id);

        let component_mask = read_mask(
            &mut reader,
            schema.components.len(),
            MaskKind::ComponentMask,
        )?;
        let mut components = Vec::new();
        for (idx, component) in schema.components.iter().enumerate() {
            if component_mask[idx] {
                let fields = decode_update_component(component, &mut reader, limits)?;
                components.push(DeltaUpdateComponent {
                    id: component.id,
                    fields,
                });
            }
        }

        updates.push(DeltaUpdateEntity {
            id: EntityId::new(id),
            components,
        });
    }

    reader.align_to_byte()?;
    if reader.bits_remaining() != 0 {
        return Err(CodecError::TrailingSectionData {
            section: SectionTag::EntityUpdate,
            remaining_bits: reader.bits_remaining(),
        });
    }
    Ok(updates)
}

fn decode_delta_sections(
    schema: &schema::Schema,
    packet: &WirePacket<'_>,
    limits: &CodecLimits,
) -> CodecResult<(Vec<EntityId>, Vec<EntitySnapshot>, Vec<DeltaUpdateEntity>)> {
    let mut destroys: Option<Vec<EntityId>> = None;
    let mut creates: Option<Vec<EntitySnapshot>> = None;
    let mut updates: Option<Vec<DeltaUpdateEntity>> = None;

    for section in &packet.sections {
        match section.tag {
            SectionTag::EntityDestroy => {
                if destroys.is_some() {
                    return Err(CodecError::DuplicateSection {
                        section: section.tag,
                    });
                }
                destroys = Some(decode_destroy_section(section.body, limits)?);
            }
            SectionTag::EntityCreate => {
                if creates.is_some() {
                    return Err(CodecError::DuplicateSection {
                        section: section.tag,
                    });
                }
                creates = Some(decode_create_section(schema, section.body, limits)?);
            }
            SectionTag::EntityUpdate => {
                if updates.is_some() {
                    return Err(CodecError::DuplicateSection {
                        section: section.tag,
                    });
                }
                updates = Some(decode_update_section(schema, section.body, limits)?);
            }
            _ => {
                return Err(CodecError::UnexpectedSection {
                    section: section.tag,
                });
            }
        }
    }

    Ok((
        destroys.unwrap_or_default(),
        creates.unwrap_or_default(),
        updates.unwrap_or_default(),
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaDecoded {
    pub tick: SnapshotTick,
    pub baseline_tick: SnapshotTick,
    pub destroys: Vec<EntityId>,
    pub creates: Vec<EntitySnapshot>,
    pub updates: Vec<DeltaUpdateEntity>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaUpdateEntity {
    pub id: EntityId,
    pub components: Vec<DeltaUpdateComponent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaUpdateComponent {
    pub id: ComponentId,
    pub fields: Vec<(usize, FieldValue)>,
}

fn apply_destroys(
    baseline: &[EntitySnapshot],
    destroys: &[EntityId],
) -> CodecResult<Vec<EntitySnapshot>> {
    let mut result = Vec::with_capacity(baseline.len());
    let mut i = 0usize;
    let mut j = 0usize;
    while i < baseline.len() || j < destroys.len() {
        let base = baseline.get(i);
        let destroy = destroys.get(j);
        match (base, destroy) {
            (Some(b), Some(d)) => {
                if b.id.raw() < d.raw() {
                    result.push(b.clone());
                    i += 1;
                } else if b.id.raw() > d.raw() {
                    return Err(CodecError::EntityNotFound { entity_id: d.raw() });
                } else {
                    i += 1;
                    j += 1;
                }
            }
            (Some(b), None) => {
                result.push(b.clone());
                i += 1;
            }
            (None, Some(d)) => {
                return Err(CodecError::EntityNotFound { entity_id: d.raw() });
            }
            (None, None) => break,
        }
    }
    Ok(result)
}

fn apply_creates(
    baseline: Vec<EntitySnapshot>,
    creates: Vec<EntitySnapshot>,
) -> CodecResult<Vec<EntitySnapshot>> {
    let mut result = Vec::with_capacity(baseline.len() + creates.len());
    let mut i = 0usize;
    let mut j = 0usize;
    while i < baseline.len() || j < creates.len() {
        let base = baseline.get(i);
        let create = creates.get(j);
        match (base, create) {
            (Some(b), Some(c)) => {
                if b.id.raw() < c.id.raw() {
                    result.push(b.clone());
                    i += 1;
                } else if b.id.raw() > c.id.raw() {
                    result.push(c.clone());
                    j += 1;
                } else {
                    return Err(CodecError::EntityAlreadyExists {
                        entity_id: c.id.raw(),
                    });
                }
            }
            (Some(b), None) => {
                result.push(b.clone());
                i += 1;
            }
            (None, Some(c)) => {
                result.push(c.clone());
                j += 1;
            }
            (None, None) => break,
        }
    }
    Ok(result)
}

fn apply_updates(
    entities: &mut [EntitySnapshot],
    updates: &[DeltaUpdateEntity],
) -> CodecResult<()> {
    for update in updates {
        let idx = entities
            .binary_search_by_key(&update.id.raw(), |e| e.id.raw())
            .map_err(|_| CodecError::EntityNotFound {
                entity_id: update.id.raw(),
            })?;
        let entity = &mut entities[idx];
        for component_update in &update.components {
            let component = entity
                .components
                .iter_mut()
                .find(|c| c.id == component_update.id)
                .ok_or_else(|| CodecError::ComponentNotFound {
                    entity_id: update.id.raw(),
                    component_id: component_update.id.get(),
                })?;
            for (field_idx, value) in &component_update.fields {
                if *field_idx >= component.fields.len() {
                    return Err(CodecError::InvalidMask {
                        kind: MaskKind::FieldMask {
                            component: component_update.id,
                        },
                        reason: MaskReason::FieldCountMismatch {
                            expected: component.fields.len(),
                            actual: *field_idx + 1,
                        },
                    });
                }
                component.fields[*field_idx] = *value;
            }
        }
    }
    Ok(())
}

fn ensure_entities_sorted(entities: &[EntitySnapshot]) -> CodecResult<()> {
    let mut prev: Option<u32> = None;
    for entity in entities {
        if let Some(prev_id) = prev {
            if entity.id.raw() <= prev_id {
                return Err(CodecError::InvalidEntityOrder {
                    previous: prev_id,
                    current: entity.id.raw(),
                });
            }
        }
        prev = Some(entity.id.raw());
    }
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

fn write_full_component(
    component: &ComponentDef,
    snapshot: &ComponentSnapshot,
    limits: &CodecLimits,
    writer: &mut BitWriter<'_>,
) -> CodecResult<()> {
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

    for _ in &component.fields {
        writer.write_bit(true)?;
    }
    for (field, value) in component.fields.iter().zip(snapshot.fields.iter()) {
        write_field_value(component.id, *field, *value, writer)?;
    }
    Ok(())
}

fn decode_full_component(
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
        values.push(read_field_value(component.id, *field, reader)?);
    }
    Ok(values)
}

fn write_update_components(
    schema: &schema::Schema,
    baseline: &EntitySnapshot,
    current: &EntitySnapshot,
    limits: &CodecLimits,
    scratch: &mut CodecScratch,
    writer: &mut BitWriter<'_>,
) -> CodecResult<()> {
    let component_count = schema.components.len();
    let (component_changed, _) = scratch.component_and_field_masks_mut(component_count, 0);
    component_changed.fill(false);
    for (idx, component) in schema.components.iter().enumerate() {
        let base = find_component(baseline, component.id);
        let curr = find_component(current, component.id);
        if base.is_some() != curr.is_some() {
            return Err(CodecError::InvalidMask {
                kind: MaskKind::ComponentMask,
                reason: MaskReason::ComponentPresenceMismatch {
                    component: component.id,
                },
            });
        }
        if let (Some(base), Some(curr)) = (base, curr) {
            if base.fields.len() != component.fields.len()
                || curr.fields.len() != component.fields.len()
            {
                return Err(CodecError::InvalidMask {
                    kind: MaskKind::FieldMask {
                        component: component.id,
                    },
                    reason: MaskReason::FieldCountMismatch {
                        expected: component.fields.len(),
                        actual: base.fields.len().max(curr.fields.len()),
                    },
                });
            }
            if component.fields.len() > limits.max_fields_per_component {
                return Err(CodecError::LimitsExceeded {
                    kind: LimitKind::FieldsPerComponent,
                    limit: limits.max_fields_per_component,
                    actual: component.fields.len(),
                });
            }
            let (component_changed, field_mask) =
                scratch.component_and_field_masks_mut(component_count, component.fields.len());
            let any_changed = compute_field_mask_into(component, base, curr, field_mask)?
                .iter()
                .any(|b| *b);
            writer.write_bit(any_changed)?;
            if any_changed {
                component_changed[idx] = true;
            }
        } else {
            writer.write_bit(false)?;
        }
    }

    for (idx, component) in schema.components.iter().enumerate() {
        let (base, curr) = match (
            find_component(baseline, component.id),
            find_component(current, component.id),
        ) {
            (Some(base), Some(curr)) => (base, curr),
            _ => continue,
        };
        if component.fields.len() > limits.max_fields_per_component {
            return Err(CodecError::LimitsExceeded {
                kind: LimitKind::FieldsPerComponent,
                limit: limits.max_fields_per_component,
                actual: component.fields.len(),
            });
        }
        let (component_changed, field_mask) =
            scratch.component_and_field_masks_mut(component_count, component.fields.len());
        if component_changed[idx] {
            let field_mask = compute_field_mask_into(component, base, curr, field_mask)?;
            if component.fields.len() > limits.max_fields_per_component {
                return Err(CodecError::LimitsExceeded {
                    kind: LimitKind::FieldsPerComponent,
                    limit: limits.max_fields_per_component,
                    actual: component.fields.len(),
                });
            }

            for bit in field_mask {
                writer.write_bit(*bit)?;
            }
            for (((field, _base_val), curr_val), changed) in component
                .fields
                .iter()
                .zip(base.fields.iter())
                .zip(curr.fields.iter())
                .zip(field_mask.iter())
            {
                if *changed {
                    write_field_value(component.id, *field, *curr_val, writer)?;
                }
            }
        }
    }
    Ok(())
}

fn decode_update_component(
    component: &ComponentDef,
    reader: &mut BitReader<'_>,
    limits: &CodecLimits,
) -> CodecResult<Vec<(usize, FieldValue)>> {
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
    if !mask.iter().any(|b| *b) {
        return Err(CodecError::InvalidMask {
            kind: MaskKind::FieldMask {
                component: component.id,
            },
            reason: MaskReason::EmptyFieldMask {
                component: component.id,
            },
        });
    }
    let mut fields = Vec::new();
    for (idx, field) in component.fields.iter().enumerate() {
        if mask[idx] {
            let value = read_field_value(component.id, *field, reader)?;
            fields.push((idx, value));
        }
    }
    Ok(fields)
}

fn compute_field_mask_into<'a>(
    component: &ComponentDef,
    baseline: &ComponentSnapshot,
    current: &ComponentSnapshot,
    field_mask: &'a mut [bool],
) -> CodecResult<&'a [bool]> {
    for (((field, base_val), curr_val), slot) in component
        .fields
        .iter()
        .zip(baseline.fields.iter())
        .zip(current.fields.iter())
        .zip(field_mask.iter_mut())
    {
        *slot = field_changed(component.id, *field, *base_val, *curr_val)?;
    }
    Ok(field_mask)
}

fn field_changed(
    component_id: ComponentId,
    field: FieldDef,
    baseline: FieldValue,
    current: FieldValue,
) -> CodecResult<bool> {
    match field.change {
        ChangePolicy::Always => field_differs(component_id, field, baseline, current),
        ChangePolicy::Threshold { threshold_q } => {
            field_exceeds_threshold(component_id, field, baseline, current, threshold_q)
        }
    }
}

fn field_differs(
    component_id: ComponentId,
    field: FieldDef,
    baseline: FieldValue,
    current: FieldValue,
) -> CodecResult<bool> {
    match (baseline, current) {
        (FieldValue::Bool(a), FieldValue::Bool(b)) => Ok(a != b),
        (FieldValue::UInt(a), FieldValue::UInt(b)) => Ok(a != b),
        (FieldValue::SInt(a), FieldValue::SInt(b)) => Ok(a != b),
        (FieldValue::VarUInt(a), FieldValue::VarUInt(b)) => Ok(a != b),
        (FieldValue::VarSInt(a), FieldValue::VarSInt(b)) => Ok(a != b),
        (FieldValue::FixedPoint(a), FieldValue::FixedPoint(b)) => Ok(a != b),
        _ => Err(CodecError::InvalidValue {
            component: component_id,
            field: field.id,
            reason: ValueReason::TypeMismatch {
                expected: codec_name(field.codec),
                found: value_name(current),
            },
        }),
    }
}

fn field_exceeds_threshold(
    component_id: ComponentId,
    field: FieldDef,
    baseline: FieldValue,
    current: FieldValue,
    threshold_q: u32,
) -> CodecResult<bool> {
    let threshold_q = threshold_q as u64;
    match (baseline, current) {
        (FieldValue::FixedPoint(a), FieldValue::FixedPoint(b)) => {
            Ok((a - b).unsigned_abs() > threshold_q)
        }
        (FieldValue::UInt(a), FieldValue::UInt(b)) => Ok(a.abs_diff(b) > threshold_q),
        (FieldValue::SInt(a), FieldValue::SInt(b)) => Ok((a - b).unsigned_abs() > threshold_q),
        (FieldValue::VarUInt(a), FieldValue::VarUInt(b)) => Ok(a.abs_diff(b) > threshold_q),
        (FieldValue::VarSInt(a), FieldValue::VarSInt(b)) => {
            Ok((a - b).unsigned_abs() > threshold_q)
        }
        (FieldValue::Bool(a), FieldValue::Bool(b)) => Ok(a != b),
        _ => Err(CodecError::InvalidValue {
            component: component_id,
            field: field.id,
            reason: ValueReason::TypeMismatch {
                expected: codec_name(field.codec),
                found: value_name(current),
            },
        }),
    }
}

fn entity_has_updates(
    schema: &schema::Schema,
    baseline: &EntitySnapshot,
    current: &EntitySnapshot,
    limits: &CodecLimits,
) -> CodecResult<bool> {
    ensure_component_presence_matches(schema, baseline, current)?;
    for component in &schema.components {
        let base = find_component(baseline, component.id);
        let curr = find_component(current, component.id);
        if let (Some(base), Some(curr)) = (base, curr) {
            if base.fields.len() != component.fields.len()
                || curr.fields.len() != component.fields.len()
            {
                return Err(CodecError::InvalidMask {
                    kind: MaskKind::FieldMask {
                        component: component.id,
                    },
                    reason: MaskReason::FieldCountMismatch {
                        expected: component.fields.len(),
                        actual: base.fields.len().max(curr.fields.len()),
                    },
                });
            }
            if component.fields.len() > limits.max_fields_per_component {
                return Err(CodecError::LimitsExceeded {
                    kind: LimitKind::FieldsPerComponent,
                    limit: limits.max_fields_per_component,
                    actual: component.fields.len(),
                });
            }
            for ((field, base_val), curr_val) in component
                .fields
                .iter()
                .zip(base.fields.iter())
                .zip(curr.fields.iter())
            {
                if field_changed(component.id, *field, *base_val, *curr_val)? {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

fn ensure_component_presence_matches(
    schema: &schema::Schema,
    baseline: &EntitySnapshot,
    current: &EntitySnapshot,
) -> CodecResult<()> {
    // In this version, component presence is stable across an entity's lifetime.
    for component in &schema.components {
        let base = find_component(baseline, component.id).is_some();
        let curr = find_component(current, component.id).is_some();
        if base != curr {
            return Err(CodecError::InvalidMask {
                kind: MaskKind::ComponentMask,
                reason: MaskReason::ComponentPresenceMismatch {
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

fn codec_name(codec: schema::FieldCodec) -> &'static str {
    match codec {
        schema::FieldCodec::Bool => "bool",
        schema::FieldCodec::UInt { .. } => "uint",
        schema::FieldCodec::SInt { .. } => "sint",
        schema::FieldCodec::VarUInt => "varuint",
        schema::FieldCodec::VarSInt => "varsint",
        schema::FieldCodec::FixedPoint(_) => "fixed-point",
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

#[cfg(test)]
mod tests {
    use super::*;
    use schema::{ComponentDef, FieldCodec, FieldDef, FieldId, Schema};

    fn schema_one_bool() -> Schema {
        let component = ComponentDef::new(ComponentId::new(1).unwrap())
            .field(FieldDef::new(FieldId::new(1).unwrap(), FieldCodec::bool()));
        Schema::new(vec![component]).unwrap()
    }

    fn schema_uint_threshold(threshold_q: u32) -> Schema {
        let field = FieldDef::new(FieldId::new(1).unwrap(), FieldCodec::uint(8))
            .change(ChangePolicy::Threshold { threshold_q });
        let component = ComponentDef::new(ComponentId::new(1).unwrap()).field(field);
        Schema::new(vec![component]).unwrap()
    }

    fn schema_two_components() -> Schema {
        let c1 = ComponentDef::new(ComponentId::new(1).unwrap())
            .field(FieldDef::new(FieldId::new(1).unwrap(), FieldCodec::bool()));
        let c2 = ComponentDef::new(ComponentId::new(2).unwrap())
            .field(FieldDef::new(FieldId::new(1).unwrap(), FieldCodec::bool()));
        Schema::new(vec![c1, c2]).unwrap()
    }

    fn baseline_snapshot() -> Snapshot {
        Snapshot {
            tick: SnapshotTick::new(10),
            entities: vec![EntitySnapshot {
                id: EntityId::new(1),
                components: vec![ComponentSnapshot {
                    id: ComponentId::new(1).unwrap(),
                    fields: vec![FieldValue::Bool(false)],
                }],
            }],
        }
    }

    #[test]
    fn no_op_delta_is_empty() {
        let schema = schema_one_bool();
        let baseline = baseline_snapshot();
        let current = baseline.clone();
        let mut buf = [0u8; 128];
        let bytes = encode_delta_snapshot(
            &schema,
            SnapshotTick::new(11),
            baseline.tick,
            &baseline,
            &current,
            &CodecLimits::for_testing(),
            &mut buf,
        )
        .unwrap();
        let header =
            wire::PacketHeader::delta_snapshot(schema_hash(&schema), 11, baseline.tick.raw(), 0);
        let mut expected = [0u8; wire::HEADER_SIZE];
        encode_header(&header, &mut expected).unwrap();
        assert_eq!(&buf[..bytes], expected.as_slice());
    }

    #[test]
    fn delta_roundtrip_single_update() {
        let schema = schema_one_bool();
        let baseline = baseline_snapshot();
        let current = Snapshot {
            tick: SnapshotTick::new(11),
            entities: vec![EntitySnapshot {
                id: EntityId::new(1),
                components: vec![ComponentSnapshot {
                    id: ComponentId::new(1).unwrap(),
                    fields: vec![FieldValue::Bool(true)],
                }],
            }],
        };

        let mut buf = [0u8; 128];
        let bytes = encode_delta_snapshot(
            &schema,
            current.tick,
            baseline.tick,
            &baseline,
            &current,
            &CodecLimits::for_testing(),
            &mut buf,
        )
        .unwrap();
        let applied = apply_delta_snapshot(
            &schema,
            &baseline,
            &buf[..bytes],
            &wire::Limits::for_testing(),
            &CodecLimits::for_testing(),
        )
        .unwrap();
        assert_eq!(applied.entities, current.entities);
    }

    #[test]
    fn delta_roundtrip_reuse_scratch() {
        let schema = schema_one_bool();
        let baseline = baseline_snapshot();
        let current_one = Snapshot {
            tick: SnapshotTick::new(11),
            entities: vec![EntitySnapshot {
                id: EntityId::new(1),
                components: vec![ComponentSnapshot {
                    id: ComponentId::new(1).unwrap(),
                    fields: vec![FieldValue::Bool(true)],
                }],
            }],
        };
        let current_two = Snapshot {
            tick: SnapshotTick::new(12),
            entities: vec![EntitySnapshot {
                id: EntityId::new(1),
                components: vec![ComponentSnapshot {
                    id: ComponentId::new(1).unwrap(),
                    fields: vec![FieldValue::Bool(false)],
                }],
            }],
        };

        let mut scratch = CodecScratch::default();
        let mut buf_one = [0u8; 128];
        let mut buf_two = [0u8; 128];

        let bytes_one = encode_delta_snapshot_with_scratch(
            &schema,
            current_one.tick,
            baseline.tick,
            &baseline,
            &current_one,
            &CodecLimits::for_testing(),
            &mut scratch,
            &mut buf_one,
        )
        .unwrap();
        let applied_one = apply_delta_snapshot(
            &schema,
            &baseline,
            &buf_one[..bytes_one],
            &wire::Limits::for_testing(),
            &CodecLimits::for_testing(),
        )
        .unwrap();
        assert_eq!(applied_one.entities, current_one.entities);

        let bytes_two = encode_delta_snapshot_with_scratch(
            &schema,
            current_two.tick,
            baseline.tick,
            &baseline,
            &current_two,
            &CodecLimits::for_testing(),
            &mut scratch,
            &mut buf_two,
        )
        .unwrap();
        let applied_two = apply_delta_snapshot(
            &schema,
            &baseline,
            &buf_two[..bytes_two],
            &wire::Limits::for_testing(),
            &CodecLimits::for_testing(),
        )
        .unwrap();
        assert_eq!(applied_two.entities, current_two.entities);
    }

    #[test]
    fn delta_roundtrip_create_destroy_update() {
        let schema = schema_one_bool();
        let baseline = Snapshot {
            tick: SnapshotTick::new(10),
            entities: vec![
                EntitySnapshot {
                    id: EntityId::new(1),
                    components: vec![ComponentSnapshot {
                        id: ComponentId::new(1).unwrap(),
                        fields: vec![FieldValue::Bool(false)],
                    }],
                },
                EntitySnapshot {
                    id: EntityId::new(2),
                    components: vec![ComponentSnapshot {
                        id: ComponentId::new(1).unwrap(),
                        fields: vec![FieldValue::Bool(false)],
                    }],
                },
            ],
        };
        let current = Snapshot {
            tick: SnapshotTick::new(11),
            entities: vec![
                EntitySnapshot {
                    id: EntityId::new(2),
                    components: vec![ComponentSnapshot {
                        id: ComponentId::new(1).unwrap(),
                        fields: vec![FieldValue::Bool(true)],
                    }],
                },
                EntitySnapshot {
                    id: EntityId::new(3),
                    components: vec![ComponentSnapshot {
                        id: ComponentId::new(1).unwrap(),
                        fields: vec![FieldValue::Bool(true)],
                    }],
                },
            ],
        };

        let mut buf = [0u8; 256];
        let bytes = encode_delta_snapshot(
            &schema,
            current.tick,
            baseline.tick,
            &baseline,
            &current,
            &CodecLimits::for_testing(),
            &mut buf,
        )
        .unwrap();
        let applied = apply_delta_snapshot(
            &schema,
            &baseline,
            &buf[..bytes],
            &wire::Limits::for_testing(),
            &CodecLimits::for_testing(),
        )
        .unwrap();
        assert_eq!(applied.entities, current.entities);
    }

    #[test]
    fn delta_roundtrip_single_component_change() {
        let schema = schema_two_components();
        let baseline = Snapshot {
            tick: SnapshotTick::new(10),
            entities: vec![EntitySnapshot {
                id: EntityId::new(1),
                components: vec![
                    ComponentSnapshot {
                        id: ComponentId::new(1).unwrap(),
                        fields: vec![FieldValue::Bool(false)],
                    },
                    ComponentSnapshot {
                        id: ComponentId::new(2).unwrap(),
                        fields: vec![FieldValue::Bool(false)],
                    },
                ],
            }],
        };
        let current = Snapshot {
            tick: SnapshotTick::new(11),
            entities: vec![EntitySnapshot {
                id: EntityId::new(1),
                components: vec![
                    ComponentSnapshot {
                        id: ComponentId::new(1).unwrap(),
                        fields: vec![FieldValue::Bool(true)],
                    },
                    ComponentSnapshot {
                        id: ComponentId::new(2).unwrap(),
                        fields: vec![FieldValue::Bool(false)],
                    },
                ],
            }],
        };

        let mut buf = [0u8; 256];
        let bytes = encode_delta_snapshot(
            &schema,
            current.tick,
            baseline.tick,
            &baseline,
            &current,
            &CodecLimits::for_testing(),
            &mut buf,
        )
        .unwrap();
        let applied = apply_delta_snapshot(
            &schema,
            &baseline,
            &buf[..bytes],
            &wire::Limits::for_testing(),
            &CodecLimits::for_testing(),
        )
        .unwrap();
        assert_eq!(applied.entities, current.entities);
    }

    #[test]
    fn baseline_tick_mismatch_is_error() {
        let schema = schema_one_bool();
        let baseline = baseline_snapshot();
        let current = baseline.clone();
        let mut buf = [0u8; 128];
        let bytes = encode_delta_snapshot(
            &schema,
            SnapshotTick::new(11),
            baseline.tick,
            &baseline,
            &current,
            &CodecLimits::for_testing(),
            &mut buf,
        )
        .unwrap();
        let mut packet = wire::decode_packet(&buf[..bytes], &wire::Limits::for_testing()).unwrap();
        packet.header.baseline_tick = 999;
        wire::encode_header(&packet.header, &mut buf[..wire::HEADER_SIZE]).unwrap();
        let err = apply_delta_snapshot(
            &schema,
            &baseline,
            &buf[..bytes],
            &wire::Limits::for_testing(),
            &CodecLimits::for_testing(),
        )
        .unwrap_err();
        assert!(matches!(err, CodecError::BaselineTickMismatch { .. }));
    }

    #[test]
    fn threshold_suppresses_small_change() {
        let schema = schema_uint_threshold(5);
        let baseline = Snapshot {
            tick: SnapshotTick::new(10),
            entities: vec![EntitySnapshot {
                id: EntityId::new(1),
                components: vec![ComponentSnapshot {
                    id: ComponentId::new(1).unwrap(),
                    fields: vec![FieldValue::UInt(10)],
                }],
            }],
        };
        let current = Snapshot {
            tick: SnapshotTick::new(11),
            entities: vec![EntitySnapshot {
                id: EntityId::new(1),
                components: vec![ComponentSnapshot {
                    id: ComponentId::new(1).unwrap(),
                    fields: vec![FieldValue::UInt(12)],
                }],
            }],
        };

        let mut buf = [0u8; 128];
        let bytes = encode_delta_snapshot(
            &schema,
            current.tick,
            baseline.tick,
            &baseline,
            &current,
            &CodecLimits::for_testing(),
            &mut buf,
        )
        .unwrap();

        let packet = wire::decode_packet(&buf[..bytes], &wire::Limits::for_testing()).unwrap();
        assert_eq!(packet.sections.len(), 0);

        let applied = apply_delta_snapshot(
            &schema,
            &baseline,
            &buf[..bytes],
            &wire::Limits::for_testing(),
            &CodecLimits::for_testing(),
        )
        .unwrap();
        assert_eq!(applied.entities, baseline.entities);
    }
}
