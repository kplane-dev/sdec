use codec::{
    apply_delta_snapshot_from_packet, decode_session_init_packet, decode_session_packet,
    encode_delta_snapshot, encode_delta_snapshot_for_client_session_with_scratch,
    encode_session_init_packet, CodecError, CodecLimits, CodecScratch, CompactHeaderMode,
    ComponentSnapshot, EntityId, EntitySnapshot, FieldValue, Snapshot, SnapshotTick,
};
use schema::{ComponentDef, FieldCodec, FieldDef, FieldId, Schema};

fn schema_one_bool() -> Schema {
    let component = ComponentDef::new(schema::ComponentId::new(1).unwrap())
        .field(FieldDef::new(FieldId::new(1).unwrap(), FieldCodec::bool()));
    Schema::new(vec![component]).unwrap()
}

fn snapshot_with_bool(tick: u32, value: bool) -> Snapshot {
    Snapshot {
        tick: SnapshotTick::new(tick),
        entities: vec![EntitySnapshot {
            id: EntityId::new(1),
            components: vec![ComponentSnapshot {
                id: schema::ComponentId::new(1).unwrap(),
                fields: vec![FieldValue::Bool(value)],
            }],
        }],
    }
}

#[test]
fn session_sequence_valid() {
    let schema = schema_one_bool();
    let limits = CodecLimits::for_testing();
    let wire_limits = wire::Limits::for_testing();

    let mut init_buf = [0u8; 128];
    let init_len = encode_session_init_packet(
        &schema,
        SnapshotTick::new(1),
        Some(1),
        CompactHeaderMode::SessionV1,
        &limits,
        &mut init_buf,
    )
    .unwrap();
    let init_packet = wire::decode_packet(&init_buf[..init_len], &wire_limits).unwrap();
    let mut session = decode_session_init_packet(&schema, &init_packet, &limits).unwrap();

    let baseline = snapshot_with_bool(1, false);
    let current = snapshot_with_bool(2, true);
    let mut scratch = CodecScratch::default();
    let mut last_tick = baseline.tick;
    let mut delta_buf = vec![0u8; 256];
    let delta_len = encode_delta_snapshot_for_client_session_with_scratch(
        &schema,
        current.tick,
        baseline.tick,
        &baseline,
        &current,
        &limits,
        &mut scratch,
        &mut last_tick,
        &mut delta_buf,
    )
    .unwrap();
    let packet =
        decode_session_packet(&schema, &mut session, &delta_buf[..delta_len], &wire_limits)
            .unwrap();
    let applied = apply_delta_snapshot_from_packet(&schema, &baseline, &packet, &limits).unwrap();
    assert_eq!(applied.entities, current.entities);
}

#[test]
fn session_drop_hits_baseline_mismatch() {
    let schema = schema_one_bool();
    let limits = CodecLimits::for_testing();
    let wire_limits = wire::Limits::for_testing();

    let mut init_buf = [0u8; 128];
    let init_len = encode_session_init_packet(
        &schema,
        SnapshotTick::new(1),
        Some(1),
        CompactHeaderMode::SessionV1,
        &limits,
        &mut init_buf,
    )
    .unwrap();
    let init_packet = wire::decode_packet(&init_buf[..init_len], &wire_limits).unwrap();
    let mut session = decode_session_init_packet(&schema, &init_packet, &limits).unwrap();

    let snap1 = snapshot_with_bool(1, false);
    let snap2 = snapshot_with_bool(2, true);
    let snap3 = snapshot_with_bool(3, false);
    let mut scratch = CodecScratch::default();
    let mut last_tick = snap2.tick;
    let mut buf3 = vec![0u8; 256];
    let len3 = encode_delta_snapshot_for_client_session_with_scratch(
        &schema,
        snap3.tick,
        snap2.tick,
        &snap2,
        &snap3,
        &limits,
        &mut scratch,
        &mut last_tick,
        &mut buf3,
    )
    .unwrap();

    session.last_tick = snap2.tick;
    let packet = decode_session_packet(&schema, &mut session, &buf3[..len3], &wire_limits).unwrap();
    let err = apply_delta_snapshot_from_packet(&schema, &snap1, &packet, &limits).unwrap_err();
    assert!(matches!(err, CodecError::BaselineTickMismatch { .. }));
}

#[test]
fn session_init_required_for_init_decode() {
    let schema = schema_one_bool();
    let limits = CodecLimits::for_testing();
    let wire_limits = wire::Limits::for_testing();
    let baseline = snapshot_with_bool(1, false);
    let current = snapshot_with_bool(2, true);

    let mut buf = vec![0u8; 256];
    let len = encode_delta_snapshot(
        &schema,
        current.tick,
        baseline.tick,
        &baseline,
        &current,
        &limits,
        &mut buf,
    )
    .unwrap();
    let packet = wire::decode_packet(&buf[..len], &wire_limits).unwrap();
    let err = decode_session_init_packet(&schema, &packet, &limits).unwrap_err();
    assert!(matches!(err, CodecError::SessionMissing));
}

#[test]
fn baseline_mismatch_is_error() {
    let schema = schema_one_bool();
    let limits = CodecLimits::for_testing();
    let wire_limits = wire::Limits::for_testing();

    let mut init_buf = [0u8; 128];
    let init_len = encode_session_init_packet(
        &schema,
        SnapshotTick::new(1),
        Some(1),
        CompactHeaderMode::SessionV1,
        &limits,
        &mut init_buf,
    )
    .unwrap();
    let init_packet = wire::decode_packet(&init_buf[..init_len], &wire_limits).unwrap();
    let mut session = decode_session_init_packet(&schema, &init_packet, &limits).unwrap();

    let baseline = snapshot_with_bool(1, false);
    let current = snapshot_with_bool(2, true);
    let mut scratch = CodecScratch::default();
    let mut last_tick = baseline.tick;
    let mut delta_buf = vec![0u8; 256];
    let delta_len = encode_delta_snapshot_for_client_session_with_scratch(
        &schema,
        current.tick,
        baseline.tick,
        &baseline,
        &current,
        &limits,
        &mut scratch,
        &mut last_tick,
        &mut delta_buf,
    )
    .unwrap();

    let packet =
        decode_session_packet(&schema, &mut session, &delta_buf[..delta_len], &wire_limits)
            .unwrap();
    let wrong_baseline = snapshot_with_bool(0, false);
    let err =
        apply_delta_snapshot_from_packet(&schema, &wrong_baseline, &packet, &limits).unwrap_err();
    assert!(matches!(err, CodecError::BaselineTickMismatch { .. }));
}
