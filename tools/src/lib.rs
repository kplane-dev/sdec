//! Introspection and debugging tools for the sdec codec.
//!
//! This crate provides utilities for inspecting and understanding encoded packets.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use bitstream::BitReader;
use codec::{
    decode_delta_packet, decode_full_snapshot_from_packet, CodecLimits, DeltaDecoded,
    DeltaUpdateEntity, FieldValue, Snapshot,
};
use serde::Serialize;
use serde_json::{json, Value};
use wire::{decode_packet, PacketHeader, SectionTag, WirePacket};

#[derive(Debug, Clone)]
pub struct InspectReport {
    pub header: PacketHeader,
    pub sections: Vec<SectionReport>,
    pub update_summary: Option<UpdateSummary>,
}

#[derive(Debug, Clone)]
pub struct SectionReport {
    pub tag: SectionTag,
    pub byte_len: usize,
    pub entity_count: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct UpdateSummary {
    pub changed_components: usize,
    pub changed_fields: usize,
    pub by_component_fields: Vec<ComponentFieldCount>,
}

#[derive(Debug, Clone)]
pub struct ComponentFieldCount {
    pub component_id: u16,
    pub changed_fields: usize,
}

#[derive(Debug, Serialize)]
pub struct DecodeOutput {
    pub kind: String,
    pub header: HeaderOutput,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_snapshot: Option<FullSnapshotOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta_snapshot: Option<DeltaSnapshotOutput>,
}

#[derive(Debug, Serialize)]
pub struct HeaderOutput {
    pub version: u16,
    pub flags_raw: u16,
    pub is_full_snapshot: bool,
    pub is_delta_snapshot: bool,
    pub schema_hash: u64,
    pub tick: u32,
    pub baseline_tick: u32,
    pub payload_len: u32,
}

#[derive(Debug, Serialize)]
pub struct FullSnapshotOutput {
    pub entities: Vec<FullEntityOutput>,
}

#[derive(Debug, Serialize)]
pub struct FullEntityOutput {
    pub id: u32,
    pub components: Vec<FullComponentOutput>,
}

#[derive(Debug, Serialize)]
pub struct FullComponentOutput {
    pub id: u16,
    pub fields: Vec<FieldValueOutput>,
}

#[derive(Debug, Serialize)]
pub struct DeltaSnapshotOutput {
    pub destroys: Vec<u32>,
    pub creates: Vec<FullEntityOutput>,
    pub updates: Vec<DeltaUpdateEntityOutput>,
}

#[derive(Debug, Serialize)]
pub struct DeltaUpdateEntityOutput {
    pub id: u32,
    pub components: Vec<DeltaUpdateComponentOutput>,
}

#[derive(Debug, Serialize)]
pub struct DeltaUpdateComponentOutput {
    pub id: u16,
    pub fields: Vec<DeltaUpdateFieldOutput>,
}

#[derive(Debug, Serialize)]
pub struct DeltaUpdateFieldOutput {
    pub index: usize,
    pub value: FieldValueOutput,
}

#[derive(Debug, Serialize)]
pub struct FieldValueOutput {
    pub kind: String,
    pub value: Value,
}

pub fn format_decode_pretty(output: &DecodeOutput) -> String {
    let mut lines = Vec::new();
    lines.push(format!("kind: {}", output.kind));
    lines.push(format!(
        "version: {} flags: 0x{:04x} schema_hash: 0x{:016x}",
        output.header.version, output.header.flags_raw, output.header.schema_hash
    ));
    lines.push(format!(
        "tick: {} baseline_tick: {} payload_len: {}",
        output.header.tick, output.header.baseline_tick, output.header.payload_len
    ));

    if let Some(full) = &output.full_snapshot {
        lines.push(format!("entities: {}", full.entities.len()));
        for entity in &full.entities {
            lines.push(format!("  entity {}", entity.id));
            for component in &entity.components {
                lines.push(format!("    component {}", component.id));
                for field in &component.fields {
                    lines.push(format!("      {} = {}", field.kind, field.value));
                }
            }
        }
    }

    if let Some(delta) = &output.delta_snapshot {
        lines.push(format!("destroys: {}", delta.destroys.len()));
        if !delta.destroys.is_empty() {
            lines.push(format!("  ids: {:?}", delta.destroys));
        }
        lines.push(format!("creates: {}", delta.creates.len()));
        for entity in &delta.creates {
            lines.push(format!("  create {}", entity.id));
            for component in &entity.components {
                lines.push(format!("    component {}", component.id));
                for field in &component.fields {
                    lines.push(format!("      {} = {}", field.kind, field.value));
                }
            }
        }
        lines.push(format!("updates: {}", delta.updates.len()));
        for entity in &delta.updates {
            lines.push(format!("  update {}", entity.id));
            for component in &entity.components {
                lines.push(format!("    component {}", component.id));
                for field in &component.fields {
                    lines.push(format!("      field[{}] = {}", field.index, field.value.value));
                }
            }
        }
    }

    lines.join("\n")
}

pub fn inspect_packet(
    bytes: &[u8],
    schema: Option<&schema::Schema>,
    wire_limits: &wire::Limits,
    codec_limits: &CodecLimits,
) -> Result<InspectReport> {
    let packet = decode_packet(bytes, wire_limits).context("decode packet")?;
    let mut sections = Vec::new();

    for section in &packet.sections {
        let entity_count = match section.tag {
            SectionTag::EntityDestroy | SectionTag::EntityCreate | SectionTag::EntityUpdate => {
                Some(read_section_count(section.body).context("read section count")?)
            }
            _ => None,
        };
        sections.push(SectionReport {
            tag: section.tag,
            byte_len: section.body.len(),
            entity_count,
        });
    }

    let update_summary = match (schema, packet.header.flags.is_delta_snapshot()) {
        (Some(schema), true) => {
            let decoded = decode_delta_packet(schema, &packet, codec_limits)
                .context("decode delta packet")?;
            Some(summarize_updates(&decoded.updates))
        }
        _ => None,
    };

    Ok(InspectReport {
        header: packet.header,
        sections,
        update_summary,
    })
}

pub fn decode_packet_json(
    bytes: &[u8],
    schema: &schema::Schema,
    wire_limits: &wire::Limits,
    codec_limits: &CodecLimits,
) -> Result<DecodeOutput> {
    let packet = decode_packet(bytes, wire_limits).context("decode packet")?;
    build_decode_output(schema, &packet, codec_limits)
}

pub fn build_decode_output(
    schema: &schema::Schema,
    packet: &WirePacket<'_>,
    codec_limits: &CodecLimits,
) -> Result<DecodeOutput> {
    let header = packet.header;
    let header_out = HeaderOutput {
        version: header.version,
        flags_raw: header.flags.raw(),
        is_full_snapshot: header.flags.is_full_snapshot(),
        is_delta_snapshot: header.flags.is_delta_snapshot(),
        schema_hash: header.schema_hash,
        tick: header.tick,
        baseline_tick: header.baseline_tick,
        payload_len: header.payload_len,
    };

    if header.flags.is_full_snapshot() {
        let snapshot = decode_full_snapshot_from_packet(schema, packet, codec_limits)
            .context("decode full snapshot")?;
        Ok(DecodeOutput {
            kind: "full_snapshot".to_string(),
            header: header_out,
            full_snapshot: Some(full_snapshot_output(&snapshot)),
            delta_snapshot: None,
        })
    } else if header.flags.is_delta_snapshot() {
        let delta = decode_delta_packet(schema, packet, codec_limits).context("decode delta")?;
        Ok(DecodeOutput {
            kind: "delta_snapshot".to_string(),
            header: header_out,
            full_snapshot: None,
            delta_snapshot: Some(delta_snapshot_output(&delta)),
        })
    } else {
        Err(anyhow::anyhow!("packet flags do not indicate snapshot type"))
    }
}

fn read_section_count(body: &[u8]) -> Result<usize> {
    let mut reader = BitReader::new(body);
    reader.align_to_byte().context("align to byte")?;
    let count = reader.read_varu32().context("read varu32")? as usize;
    Ok(count)
}

fn summarize_updates(updates: &[DeltaUpdateEntity]) -> UpdateSummary {
    let mut changed_components = 0usize;
    let mut changed_fields = 0usize;
    let mut by_component: BTreeMap<u16, usize> = BTreeMap::new();

    for entity in updates {
        changed_components += entity.components.len();
        for component in &entity.components {
            let fields = component.fields.len();
            changed_fields += fields;
            *by_component.entry(component.id.get()).or_insert(0) += fields;
        }
    }

    let mut by_component_fields: Vec<ComponentFieldCount> = by_component
        .into_iter()
        .map(|(component_id, changed_fields)| ComponentFieldCount {
            component_id,
            changed_fields,
        })
        .collect();
    by_component_fields.sort_by(|a, b| {
        b.changed_fields
            .cmp(&a.changed_fields)
            .then_with(|| a.component_id.cmp(&b.component_id))
    });

    UpdateSummary {
        changed_components,
        changed_fields,
        by_component_fields,
    }
}

fn full_snapshot_output(snapshot: &Snapshot) -> FullSnapshotOutput {
    FullSnapshotOutput {
        entities: snapshot
            .entities
            .iter()
            .map(|entity| FullEntityOutput {
                id: entity.id.raw(),
                components: entity
                    .components
                    .iter()
                    .map(|component| FullComponentOutput {
                        id: component.id.get(),
                        fields: component
                            .fields
                            .iter()
                            .enumerate()
                            .map(|(index, value)| field_value_output(index, *value))
                            .collect(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn delta_snapshot_output(delta: &DeltaDecoded) -> DeltaSnapshotOutput {
    DeltaSnapshotOutput {
        destroys: delta.destroys.iter().map(|id| id.raw()).collect(),
        creates: delta
            .creates
            .iter()
            .map(|entity| FullEntityOutput {
                id: entity.id.raw(),
                components: entity
                    .components
                    .iter()
                    .map(|component| FullComponentOutput {
                        id: component.id.get(),
                        fields: component
                            .fields
                            .iter()
                            .enumerate()
                            .map(|(index, value)| field_value_output(index, *value))
                            .collect(),
                    })
                    .collect(),
            })
            .collect(),
        updates: delta
            .updates
            .iter()
            .map(|entity| DeltaUpdateEntityOutput {
                id: entity.id.raw(),
                components: entity
                    .components
                    .iter()
                    .map(|component| DeltaUpdateComponentOutput {
                        id: component.id.get(),
                        fields: component
                            .fields
                            .iter()
                            .map(|(index, value)| DeltaUpdateFieldOutput {
                                index: *index,
                                value: field_value_output(*index, *value),
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn field_value_output(index: usize, value: FieldValue) -> FieldValueOutput {
    let (kind, value) = match value {
        FieldValue::Bool(value) => ("bool", json!(value)),
        FieldValue::UInt(value) => ("uint", json!(value)),
        FieldValue::SInt(value) => ("sint", json!(value)),
        FieldValue::VarUInt(value) => ("varuint", json!(value)),
        FieldValue::VarSInt(value) => ("varsint", json!(value)),
        FieldValue::FixedPoint(value) => ("fixed-point-q", json!(value)),
    };

    FieldValueOutput {
        kind: format!("{}[{}]", kind, index),
        value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codec::{encode_delta_snapshot, encode_full_snapshot, ComponentSnapshot, EntitySnapshot};
    use schema::{ComponentDef, FieldCodec, FieldDef, FieldId, Schema};

    fn schema_one_bool() -> Schema {
        let component = ComponentDef::new(schema::ComponentId::new(1).unwrap())
            .field(FieldDef::new(FieldId::new(1).unwrap(), FieldCodec::bool()));
        Schema::new(vec![component]).unwrap()
    }

    fn baseline_snapshot() -> Snapshot {
        Snapshot {
            tick: codec::SnapshotTick::new(10),
            entities: vec![EntitySnapshot {
                id: codec::EntityId::new(1),
                components: vec![ComponentSnapshot {
                    id: schema::ComponentId::new(1).unwrap(),
                    fields: vec![FieldValue::Bool(false)],
                }],
            }],
        }
    }

    #[test]
    fn inspect_reports_update_summary() {
        let schema = schema_one_bool();
        let baseline = baseline_snapshot();
        let current = Snapshot {
            tick: codec::SnapshotTick::new(11),
            entities: vec![EntitySnapshot {
                id: codec::EntityId::new(1),
                components: vec![ComponentSnapshot {
                    id: schema::ComponentId::new(1).unwrap(),
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

        let report = inspect_packet(
            &buf[..bytes],
            Some(&schema),
            &wire::Limits::for_testing(),
            &CodecLimits::for_testing(),
        )
        .unwrap();

        let summary = report.update_summary.expect("update summary");
        assert_eq!(summary.changed_components, 1);
        assert_eq!(summary.changed_fields, 1);
    }

    #[test]
    fn decode_full_snapshot_json() {
        let schema = schema_one_bool();
        let baseline = baseline_snapshot();
        let mut buf = [0u8; 128];
        let bytes = encode_full_snapshot(
            &schema,
            baseline.tick,
            &baseline.entities,
            &CodecLimits::for_testing(),
            &mut buf,
        )
        .unwrap();

        let output = decode_packet_json(
            &buf[..bytes],
            &schema,
            &wire::Limits::for_testing(),
            &CodecLimits::for_testing(),
        )
        .unwrap();
        assert_eq!(output.kind, "full_snapshot");
        assert!(output.full_snapshot.is_some());
        let pretty = format_decode_pretty(&output);
        assert!(pretty.contains("kind: full_snapshot"));
    }
}
