use codec::{
    apply_delta_snapshot_from_packet, encode_delta_snapshot, CodecLimits, ComponentSnapshot,
    EntitySnapshot, FieldValue, Snapshot,
};
use schema::{ComponentDef, ComponentId, FieldCodec, FieldDef, FieldId, Schema};
use wire::{decode_packet, Limits as WireLimits, SectionTag};

fn schema_with_uint_fields(field_count: usize) -> Schema {
    let mut component = ComponentDef::new(ComponentId::new(1).unwrap());
    for idx in 0..field_count {
        component = component.field(FieldDef::new(
            FieldId::new((idx + 1) as u16).unwrap(),
            FieldCodec::uint(10),
        ));
    }
    Schema::new(vec![component]).unwrap()
}

fn entity_with_uint_fields(id: u32, field_count: usize, value: u64) -> EntitySnapshot {
    EntitySnapshot {
        id: codec::EntityId::new(id),
        components: vec![ComponentSnapshot {
            id: ComponentId::new(1).unwrap(),
            fields: (0..field_count).map(|_| FieldValue::UInt(value)).collect(),
        }],
    }
}

#[test]
fn delta_selects_sparse_encoding_for_sparse_change() {
    let schema = schema_with_uint_fields(32);
    let limits = CodecLimits {
        max_fields_per_component: 32,
        ..CodecLimits::for_testing()
    };
    let baseline = Snapshot {
        tick: codec::SnapshotTick::new(1),
        entities: vec![entity_with_uint_fields(1, 32, 0)],
    };
    let mut current_entity = entity_with_uint_fields(1, 32, 0);
    current_entity.components[0].fields[3] = FieldValue::UInt(1);
    let current = Snapshot {
        tick: codec::SnapshotTick::new(2),
        entities: vec![current_entity.clone()],
    };

    let mut buf = [0u8; 512];
    let bytes = encode_delta_snapshot(
        &schema,
        current.tick,
        baseline.tick,
        &baseline,
        &current,
        &limits,
        &mut buf,
    )
    .unwrap();

    let packet = decode_packet(&buf[..bytes], &WireLimits::for_testing()).unwrap();
    assert!(packet
        .sections
        .iter()
        .any(|section| section.tag == SectionTag::EntityUpdateSparse));

    let applied = apply_delta_snapshot_from_packet(&schema, &baseline, &packet, &limits).unwrap();
    assert_eq!(applied.entities, current.entities);
}

#[test]
fn delta_selects_masked_encoding_for_dense_change() {
    let schema = schema_with_uint_fields(8);
    let baseline = Snapshot {
        tick: codec::SnapshotTick::new(1),
        entities: vec![entity_with_uint_fields(1, 8, 0)],
    };
    let current = Snapshot {
        tick: codec::SnapshotTick::new(2),
        entities: vec![entity_with_uint_fields(1, 8, 5)],
    };

    let mut buf = [0u8; 512];
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

    let packet = decode_packet(&buf[..bytes], &WireLimits::for_testing()).unwrap();
    assert!(packet
        .sections
        .iter()
        .any(|section| section.tag == SectionTag::EntityUpdate));
}
