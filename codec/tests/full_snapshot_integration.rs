use codec::{
    decode_full_snapshot, decode_full_snapshot_from_packet, encode_full_snapshot, CodecLimits,
    ComponentSnapshot, EntityId, EntitySnapshot, FieldValue, SnapshotTick,
};
use schema::{schema_hash, ComponentDef, ComponentId, FieldCodec, FieldDef, FieldId, Schema};
use wire::{decode_packet, encode_header, encode_section, PacketFlags, PacketHeader, SectionTag};

fn schema_one_bool_uint10() -> Schema {
    let c1 = ComponentId::new(1).unwrap();
    let f1 = FieldId::new(1).unwrap();
    let f2 = FieldId::new(2).unwrap();
    let component = ComponentDef::new(c1)
        .field(FieldDef::new(f1, FieldCodec::bool()))
        .field(FieldDef::new(f2, FieldCodec::uint(10)));
    Schema::new(vec![component]).unwrap()
}

#[test]
fn integration_encode_decode_via_wire_packet() {
    let schema = schema_one_bool_uint10();
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
        SnapshotTick::new(7),
        &entities,
        &CodecLimits::for_testing(),
        &mut buf,
    )
    .unwrap();

    let packet = decode_packet(&buf[..bytes], &wire::Limits::for_testing()).unwrap();
    let snapshot =
        decode_full_snapshot_from_packet(&schema, &packet, &CodecLimits::for_testing()).unwrap();

    assert_eq!(snapshot.tick, SnapshotTick::new(7));
    assert_eq!(snapshot.entities, entities);
}

#[test]
fn integration_decode_from_bytes_matches_wire_decode() {
    let schema = schema_one_bool_uint10();
    let entities = vec![EntitySnapshot {
        id: EntityId::new(1),
        components: vec![ComponentSnapshot {
            id: ComponentId::new(1).unwrap(),
            fields: vec![FieldValue::Bool(false), FieldValue::UInt(1)],
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

    let snapshot = decode_full_snapshot(
        &schema,
        &buf[..bytes],
        &wire::Limits::for_testing(),
        &CodecLimits::for_testing(),
    )
    .unwrap();

    assert_eq!(snapshot.entities, entities);
}

#[test]
fn integration_rejects_update_section_for_full_snapshot() {
    let schema = schema_one_bool_uint10();
    let body = [0u8]; // count = 0
    let mut section_buf = [0u8; 8];
    let section_len = encode_section(SectionTag::EntityUpdate, &body, &mut section_buf).unwrap();

    let payload_len = section_len as u32;
    let header = PacketHeader {
        version: wire::VERSION,
        flags: PacketFlags::full_snapshot(),
        schema_hash: schema_hash(&schema),
        tick: 1,
        baseline_tick: 0,
        payload_len,
    };

    let mut buf = [0u8; 128];
    encode_header(&header, &mut buf[..wire::HEADER_SIZE]).unwrap();
    buf[wire::HEADER_SIZE..wire::HEADER_SIZE + section_len]
        .copy_from_slice(&section_buf[..section_len]);

    let packet_bytes = &buf[..wire::HEADER_SIZE + section_len];
    let packet = decode_packet(packet_bytes, &wire::Limits::for_testing()).unwrap();
    let err = decode_full_snapshot_from_packet(&schema, &packet, &CodecLimits::for_testing())
        .unwrap_err();

    assert!(matches!(
        err,
        codec::CodecError::UnexpectedSection {
            section: SectionTag::EntityUpdate
        }
    ));
}
