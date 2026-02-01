#![no_main]

use codec::{decode_session_init_packet, decode_session_packet, CodecLimits, SessionState, SnapshotTick};
use libfuzzer_sys::fuzz_target;
use schema::{ComponentDef, FieldCodec, FieldDef, FieldId, Schema};

fn schema_one_bool() -> Schema {
    let component = ComponentDef::new(schema::ComponentId::new(1).unwrap())
        .field(FieldDef::new(FieldId::new(1).unwrap(), FieldCodec::bool()));
    Schema::new(vec![component]).unwrap()
}

fuzz_target!(|data: &[u8]| {
    let schema = schema_one_bool();
    let limits = CodecLimits::for_testing();
    let wire_limits = wire::Limits::for_testing();

    let mut session: Option<SessionState> = None;
    let mut idx = 0usize;
    while idx < data.len() && idx < 4096 {
        let len = (data[idx] as usize % 120).saturating_add(1);
        idx += 1;
        let end = (idx + len).min(data.len());
        let frame = &data[idx..end];
        idx = end;

        if let Ok(packet) = wire::decode_packet(frame, &wire_limits) {
            if packet.header.flags.is_session_init() {
                if let Ok(state) = decode_session_init_packet(&schema, &packet, &limits) {
                    session = Some(state);
                }
            }
        }

        if let Some(state) = session.as_mut() {
            let _ = decode_session_packet(&schema, state, frame, &wire_limits);
        } else if !frame.is_empty() {
            let _ = wire::decode_session_header(frame, 0);
        }
    }

    // Ensure we can decode a compact packet when session state is present.
    if let Some(state) = session.as_ref() {
        let _ = decode_session_packet(&schema, &mut state.clone(), data, &wire_limits);
    }

    // Exercise tick handling for zero-length input.
    let _ = SnapshotTick::new(0);
});
