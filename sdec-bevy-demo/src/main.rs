use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bevy_ecs::prelude::*;
use clap::{Parser, ValueEnum};
use codec::{FieldValue, SnapshotTick};
use repgraph::{ClientId, ClientView, ReplicationConfig, ReplicationGraph, Vec3, WorldView};
use sdec_bevy::{
    apply_changes, extract_changes, BevySchemaBuilder, EntityMap, ReplicatedComponent,
    ReplicatedField,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Parser)]
#[command(name = "sdec-bevy-demo", version, about = "sdec Bevy demo harness")]
struct Cli {
    /// Number of entities to simulate.
    #[arg(long, default_value_t = 16)]
    entities: u32,
    /// Number of ticks to simulate.
    #[arg(long, default_value_t = 300)]
    ticks: u32,
    /// RNG seed for deterministic results.
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// Replication mode (sdec, naive, or lightyear bitcode snapshot baseline).
    #[arg(long, value_enum, default_value_t = Mode::Sdec)]
    mode: Mode,
    /// Benchmark scenario preset.
    #[arg(long, value_enum, default_value_t = Scenario::Dense)]
    scenario: Scenario,
    /// Number of clients to simulate (scenario overrides if unset).
    #[arg(long)]
    clients: Option<u32>,
    /// Dirty percent (0.0 - 1.0, scenario overrides if unset).
    #[arg(long)]
    dirty_pct: Option<f32>,
    /// Visibility radius in position units (scenario overrides if unset).
    #[arg(long)]
    visibility_radius: Option<i64>,
    /// Output directory for summary.json.
    #[arg(long, default_value = "target/sdec-bevy-demo")]
    out_dir: PathBuf,
    /// Simulated packet drop rate (0.0 - 1.0).
    #[arg(long, default_value_t = 0.0)]
    drop_rate: f32,
    /// Simulated reorder rate (0.0 - 1.0) for scenario=loss.
    #[arg(long, default_value_t = 0.0)]
    reorder_rate: f32,
    /// Validate client state against server state each tick.
    #[arg(long, default_value_t = false)]
    validate: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum, Serialize)]
enum Mode {
    Sdec,
    Naive,
    Lightyear,
}

#[derive(Clone, Copy, Debug, ValueEnum, Serialize)]
enum Scenario {
    Dense,
    Relevancy,
    Loss,
}

#[derive(
    Component, Debug, Clone, Copy, Serialize, Deserialize, bitcode::Encode, bitcode::Decode,
)]
struct PositionYaw {
    x_q: i64,
    y_q: i64,
    yaw: u16,
}

impl ReplicatedComponent for PositionYaw {
    const COMPONENT_ID: u16 = 1;

    fn fields() -> Vec<ReplicatedField> {
        vec![
            ReplicatedField {
                id: 1,
                codec: schema::FieldCodec::fixed_point(-100_000, 100_000, 100),
                change: None,
            },
            ReplicatedField {
                id: 2,
                codec: schema::FieldCodec::fixed_point(-100_000, 100_000, 100),
                change: None,
            },
            ReplicatedField {
                id: 3,
                codec: schema::FieldCodec::uint(12),
                change: None,
            },
        ]
    }

    fn read_fields(&self) -> Vec<FieldValue> {
        vec![
            FieldValue::FixedPoint(self.x_q),
            FieldValue::FixedPoint(self.y_q),
            FieldValue::UInt(self.yaw as u64),
        ]
    }

    fn apply_field(&mut self, index: usize, value: FieldValue) -> Result<()> {
        match (index, value) {
            (0, FieldValue::FixedPoint(v)) => self.x_q = v,
            (1, FieldValue::FixedPoint(v)) => self.y_q = v,
            (2, FieldValue::UInt(v)) => self.yaw = v as u16,
            _ => anyhow::bail!("invalid field index/value"),
        }
        Ok(())
    }

    fn from_fields(fields: &[FieldValue]) -> Result<Self> {
        if fields.len() != 3 {
            anyhow::bail!("expected 3 fields");
        }
        let x_q = match fields[0] {
            FieldValue::FixedPoint(v) => v,
            _ => anyhow::bail!("invalid field 0"),
        };
        let y_q = match fields[1] {
            FieldValue::FixedPoint(v) => v,
            _ => anyhow::bail!("invalid field 1"),
        };
        let yaw = match fields[2] {
            FieldValue::UInt(v) => v as u16,
            _ => anyhow::bail!("invalid field 2"),
        };
        Ok(Self { x_q, y_q, yaw })
    }
}

#[derive(Default)]
struct Rng {
    state: u64,
}

impl Rng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u32(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1);
        (self.state >> 32) as u32
    }

    fn range_i64(&mut self, min: i64, max: i64) -> i64 {
        let span = (max - min).unsigned_abs().max(1) + 1;
        let value = (self.next_u32() as u64) % span;
        min + value as i64
    }

    fn chance(&mut self) -> f32 {
        (self.next_u32() as f32) / (u32::MAX as f32)
    }
}

#[derive(Debug, Serialize)]
struct Summary {
    mode: Mode,
    scenario: Scenario,
    ticks: u32,
    entities: u32,
    clients: u32,
    dirty_pct: f32,
    visibility_radius: i64,
    drop_rate: f32,
    reorder_rate: f32,
    session_init_bytes: u64,
    bytes_avg: u64,
    bytes_p95: u64,
    encode_us_avg: u64,
    encode_us_p95: u64,
    replication_us_avg: u64,
    replication_us_p95: u64,
    codec_us_avg: u64,
    codec_us_p95: u64,
    header_us_avg: u64,
    header_us_p95: u64,
    apply_us_avg: u64,
    apply_us_p95: u64,
    errors: u64,
    resyncs: u64,
    validation_errors: u64,
    total_create_entities: u64,
    total_create_components: u64,
    total_create_fields: u64,
    total_update_entities: u64,
    total_update_components: u64,
    total_update_fields: u64,
    total_destroy_entities: u64,
    avg_create_entities_per_client_tick: f64,
    avg_create_components_per_client_tick: f64,
    avg_create_fields_per_client_tick: f64,
    avg_update_entities_per_client_tick: f64,
    avg_update_components_per_client_tick: f64,
    avg_update_fields_per_client_tick: f64,
    avg_destroy_entities_per_client_tick: f64,
}

struct ClientState<'a> {
    client_id: ClientId,
    world: World,
    entities: EntityMap,
    visible: HashSet<codec::EntityId>,
    session: Option<codec::SessionState>,
    last_tick: SnapshotTick,
    last_applied_tick: SnapshotTick,
    last_applied_full: bool,
    queue: VecDeque<Vec<u8>>,
    errors: u64,
    encoder: codec::SessionEncoder<'a>,
    delta_buf: Vec<u8>,
    send_buf: Vec<u8>,
}

struct ServerSnapshot {
    tick: u32,
    entities: Vec<(codec::EntityId, PositionYaw)>,
}

impl<'a> ClientState<'a> {
    fn new(
        client_id: ClientId,
        schema: &'a schema::Schema,
        limits: &'a codec::CodecLimits,
        delta_buf_size: usize,
        send_buf_size: usize,
    ) -> Self {
        Self {
            client_id,
            world: World::new(),
            entities: EntityMap::new(),
            visible: HashSet::new(),
            session: None,
            last_tick: SnapshotTick::new(0),
            last_applied_tick: SnapshotTick::new(0),
            last_applied_full: false,
            queue: VecDeque::new(),
            errors: 0,
            encoder: codec::SessionEncoder::new(schema, limits),
            delta_buf: vec![0u8; delta_buf_size],
            send_buf: vec![0u8; send_buf_size],
        }
    }
}
fn main() -> Result<()> {
    let cli = Cli::parse();
    fs::create_dir_all(&cli.out_dir)
        .with_context(|| format!("create {}", cli.out_dir.display()))?;

    let (clients, dirty_pct, visibility_radius) = scenario_config(&cli);

    let mut schema_builder = BevySchemaBuilder::new();
    schema_builder.component::<PositionYaw>();
    let schema = schema_builder.build().context("build schema")?;

    let mut server_world = World::new();
    let mut server_entities = EntityMap::new();

    let mut rng = Rng::new(cli.seed);
    for _ in 0..cli.entities {
        let position = PositionYaw {
            x_q: rng.range_i64(-50_000, 50_000),
            y_q: rng.range_i64(-50_000, 50_000),
            yaw: (rng.next_u32() % 4096) as u16,
        };
        let server = server_world.spawn(position).id();
        let _ = server_entities.entity_id(server);
    }

    let mut bytes = Vec::new();
    let mut enc_times = Vec::new();
    let mut replication_times = Vec::new();
    let mut codec_times = Vec::new();
    let mut header_times = Vec::new();
    let mut apply_times = Vec::new();
    let mut errors = 0u64;
    let mut resyncs = 0u64;
    let mut validation_errors = 0u64;
    let mut server_snapshots: VecDeque<ServerSnapshot> = VecDeque::new();

    let mut sdec_limits = codec::CodecLimits::default();
    let entity_count = cli.entities as usize;
    sdec_limits.max_section_bytes = 16 * 1024 * 1024;
    sdec_limits.max_entities_create = sdec_limits.max_entities_create.max(entity_count);
    sdec_limits.max_entities_destroy = sdec_limits.max_entities_destroy.max(entity_count);
    sdec_limits.max_entities_update = sdec_limits.max_entities_update.max(entity_count * 2);
    sdec_limits.max_total_entities_after_apply = sdec_limits
        .max_total_entities_after_apply
        .max(entity_count * 2);
    let full_buf_size = (entity_count.saturating_mul(1024)).max(16 * 1024 * 1024);
    let delta_buf_size = (entity_count.saturating_mul(512)).max(4 * 1024 * 1024);
    let send_buf_size = full_buf_size.max(delta_buf_size);
    let mut sdec_session_init_bytes = 0u64;
    let mut total_create_entities = 0u64;
    let mut total_create_components = 0u64;
    let mut total_create_fields = 0u64;
    let mut total_update_entities = 0u64;
    let mut total_update_components = 0u64;
    let mut total_update_fields = 0u64;
    let mut total_destroy_entities = 0u64;

    let client_positions = build_client_positions(clients, &mut rng);
    let view_radius = if visibility_radius <= 0 {
        f32::MAX
    } else {
        visibility_radius as f32
    };
    let mut graph = ReplicationGraph::new(ReplicationConfig::default_limits());
    let mut clients_state: Vec<ClientState<'_>> = (0..clients as usize)
        .map(|idx| {
            let client_id = ClientId((idx as u32) + 1);
            let (x, y) = client_positions[idx];
            graph.upsert_client(
                client_id,
                ClientView::new(
                    Vec3 {
                        x: x as f32,
                        y: y as f32,
                        z: 0.0,
                    },
                    view_radius,
                ),
            );
            ClientState::new(
                client_id,
                schema.schema(),
                &sdec_limits,
                delta_buf_size,
                send_buf_size,
            )
        })
        .collect();

    if matches!(cli.mode, Mode::Sdec) {
        let mut init_buf = vec![0u8; 128];
        for (idx, client_state) in clients_state.iter_mut().enumerate() {
            init_buf.resize(128, 0);
            let init_len = codec::encode_session_init_packet(
                schema.schema(),
                SnapshotTick::new(0),
                Some((idx + 1) as u64),
                codec::CompactHeaderMode::SessionV1,
                &sdec_limits,
                &mut init_buf,
            )?;
            sdec_session_init_bytes += init_len as u64;

            let init_packet = wire::decode_packet(&init_buf[..init_len], &wire::Limits::default())
                .context("decode session init")?;
            let session =
                codec::decode_session_init_packet(schema.schema(), &init_packet, &sdec_limits)
                    .context("decode session init packet")?;
            client_state.session = Some(session);
        }
    }

    for tick in 1..=cli.ticks {
        step_positions(&mut server_world, &mut rng, dirty_pct);
        let positions = collect_positions(&mut server_world);
        let all_visible = visibility_radius <= 0;
        let all_ids: HashSet<codec::EntityId> = positions
            .iter()
            .map(|(entity, _)| server_entities.entity_id(*entity))
            .collect();
        let changes = extract_changes(&schema, &mut server_world, &mut server_entities);

        if cli.validate {
            let snapshot = build_server_snapshot(&mut server_world, &mut server_entities);
            server_snapshots.push_back(ServerSnapshot {
                tick,
                entities: snapshot,
            });
            while server_snapshots.len() > 256 {
                server_snapshots.pop_front();
            }
        }

        let mut dirty_map: HashMap<codec::EntityId, Vec<schema::ComponentId>> = HashMap::new();
        for update in &changes.updates {
            let entry = dirty_map.entry(update.id).or_default();
            for component in &update.components {
                if !entry.contains(&component.id) {
                    entry.push(component.id);
                }
            }
        }
        for (entity, pos) in &positions {
            let id = server_entities.entity_id(*entity);
            let dirty = dirty_map.remove(&id).unwrap_or_default();
            graph.update_entity(
                id,
                Vec3 {
                    x: pos.x_q as f32,
                    y: pos.y_q as f32,
                    z: 0.0,
                },
                &dirty,
            );
        }
        for destroy in &changes.destroys {
            graph.remove_entity(*destroy);
        }
        for (client_idx, client_state) in clients_state.iter_mut().enumerate() {
            if all_visible {
                client_state.visible.clear();
                client_state.visible.extend(all_ids.iter().copied());
            } else {
                client_state.visible = visible_entity_ids(
                    &positions,
                    &mut server_entities,
                    client_positions[client_idx],
                    visibility_radius,
                );
            }

            let start = Instant::now();
            let (payload, replication_elapsed, codec_elapsed, header_elapsed) = match cli.mode {
                Mode::Sdec => {
                    let replication_start = Instant::now();
                    let world_view = DemoWorldView {
                        schema: &schema,
                        world: &server_world,
                        entities: &server_entities,
                    };
                    let delta = graph.build_client_delta(client_state.client_id, &world_view);
                    let replication_elapsed = replication_start.elapsed();
                    total_create_entities += delta.creates.len() as u64;
                    total_destroy_entities += delta.destroys.len() as u64;
                    total_update_entities += delta.updates.len() as u64;
                    for create in &delta.creates {
                        total_create_components += create.components.len() as u64;
                        for component in &create.components {
                            total_create_fields += component.fields.len() as u64;
                        }
                    }
                    for update in &delta.updates {
                        total_update_components += update.components.len() as u64;
                        for component in &update.components {
                            total_update_fields += component.fields.len() as u64;
                        }
                    }
                    if tick == 1 {
                        let codec_start = Instant::now();
                        let len = encode_full_snapshot_retry(
                            schema.schema(),
                            SnapshotTick::new(tick),
                            &delta.creates,
                            &sdec_limits,
                            &mut client_state.send_buf,
                        )?;
                        let codec_elapsed = codec_start.elapsed();
                        client_state.send_buf.truncate(len);
                        client_state.last_tick = SnapshotTick::new(tick);
                        let payload = std::mem::take(&mut client_state.send_buf);
                        (payload, replication_elapsed, codec_elapsed, Duration::ZERO)
                    } else {
                        let codec_start = Instant::now();
                        let len = encode_delta_retry(
                            &mut client_state.encoder,
                            SnapshotTick::new(tick),
                            SnapshotTick::new(tick.saturating_sub(1)),
                            &delta.creates,
                            &delta.destroys,
                            &delta.updates,
                            &mut client_state.delta_buf,
                        )?;
                        let codec_elapsed = codec_start.elapsed();
                        let payload = &client_state.delta_buf[wire::HEADER_SIZE..len];
                        let header_start = Instant::now();
                        let compact_len = build_compact_packet(
                            &mut client_state.send_buf,
                            wire::SessionFlags::delta_snapshot(),
                            client_state.last_tick,
                            SnapshotTick::new(tick),
                            SnapshotTick::new(tick.saturating_sub(1)),
                            payload,
                        )?;
                        let header_elapsed = header_start.elapsed();
                        client_state.send_buf.truncate(compact_len);
                        client_state.last_tick = SnapshotTick::new(tick);
                        let packet = std::mem::take(&mut client_state.send_buf);
                        (packet, replication_elapsed, codec_elapsed, header_elapsed)
                    }
                }
                Mode::Naive => {
                    let replication_start = Instant::now();
                    let snapshot = build_snapshot_for_ids(
                        &mut server_world,
                        &mut server_entities,
                        &client_state.visible,
                    );
                    let replication_elapsed = replication_start.elapsed();
                    let codec_start = Instant::now();
                    let payload = bincode::serialize(&snapshot).context("serialize naive snapshot")?;
                    (payload, replication_elapsed, codec_start.elapsed(), Duration::ZERO)
                }
                Mode::Lightyear => {
                    let replication_start = Instant::now();
                    let snapshot = build_snapshot_for_ids(
                        &mut server_world,
                        &mut server_entities,
                        &client_state.visible,
                    );
                    let replication_elapsed = replication_start.elapsed();
                    let codec_start = Instant::now();
                    let payload = bitcode::encode(&snapshot);
                    (payload, replication_elapsed, codec_start.elapsed(), Duration::ZERO)
                }
            };
            enc_times.push(start.elapsed());
            replication_times.push(replication_elapsed);
            codec_times.push(codec_elapsed);
            if header_elapsed > Duration::ZERO {
                header_times.push(header_elapsed);
            }
            bytes.push(payload.len() as u64);

            if rng.chance() >= cli.drop_rate {
                client_state.queue.push_back(payload);
                if matches!(cli.scenario, Scenario::Loss)
                    && rng.chance() < cli.reorder_rate
                    && client_state.queue.len() >= 2
                {
                    let len = client_state.queue.len();
                    client_state.queue.swap(len - 1, len - 2);
                }
            } else if matches!(cli.mode, Mode::Sdec) {
                client_state.send_buf = payload;
                client_state.send_buf.clear();
            }

            if let Some(delivered) = client_state.queue.pop_front() {
                let apply_start = Instant::now();
                let apply_result = match cli.mode {
                    Mode::Sdec => apply_sdec_packet(
                        &schema,
                        &mut client_state.world,
                        &mut client_state.entities,
                        &mut client_state.session,
                        &sdec_limits,
                        &delivered,
                    ),
                    Mode::Naive => {
                        let snapshot: SnapshotData = bincode::deserialize(&delivered)
                            .context("deserialize naive snapshot")?;
                        apply_snapshot(
                            &mut client_state.world,
                            &mut client_state.entities,
                            &snapshot,
                        );
                        client_state.last_applied_tick = SnapshotTick::new(tick);
                        client_state.last_applied_full = true;
                        Ok(AppliedPacket::Full(SnapshotTick::new(tick)))
                    }
                    Mode::Lightyear => {
                        let snapshot: SnapshotData =
                            bitcode::decode(&delivered).context("bitcode decode")?;
                        apply_snapshot(
                            &mut client_state.world,
                            &mut client_state.entities,
                            &snapshot,
                        );
                        client_state.last_applied_tick = SnapshotTick::new(tick);
                        client_state.last_applied_full = true;
                        Ok(AppliedPacket::Full(SnapshotTick::new(tick)))
                    }
                };
                if matches!(cli.mode, Mode::Sdec) {
                    if let Ok(applied) = &apply_result {
                        match *applied {
                            AppliedPacket::Full(tick) => {
                                client_state.last_applied_tick = tick;
                                client_state.last_applied_full = true;
                            }
                            AppliedPacket::Delta(tick) => {
                                client_state.last_applied_tick = tick;
                                client_state.last_applied_full = false;
                            }
                        }
                    }
                }
                if apply_result.is_err() {
                    errors += 1;
                    client_state.errors += 1;
                    if matches!(cli.mode, Mode::Sdec) {
                        let resync_tick = SnapshotTick::new(
                            client_state
                                .session
                                .as_ref()
                                .map(|state| state.last_tick.raw())
                                .unwrap_or(0)
                                + 1,
                        );
                        if let Ok(resynced) = resync_client(
                            &schema,
                            &mut server_world,
                            &mut server_entities,
                            &mut client_state.world,
                            &mut client_state.entities,
                            &mut client_state.session,
                            &client_state.visible,
                            &sdec_limits,
                        ) {
                            if resynced {
                                resyncs += 1;
                                client_state.last_applied_tick = resync_tick;
                                client_state.last_applied_full = true;
                                if cli.validate {
                                    let snapshot = build_server_snapshot(
                                        &mut server_world,
                                        &mut server_entities,
                                    );
                                    server_snapshots.push_back(ServerSnapshot {
                                        tick: resync_tick.raw(),
                                        entities: snapshot,
                                    });
                                    while server_snapshots.len() > 256 {
                                        server_snapshots.pop_front();
                                    }
                                }
                                if cli.validate {
                                    if let Some(snapshot) = server_snapshots
                                        .iter()
                                        .find(|entry| entry.tick == resync_tick.raw())
                                    {
                                        let expected_ids = if matches!(cli.scenario, Scenario::Loss)
                                        {
                                            client_state.entities.ids().into_iter().collect()
                                        } else if visibility_radius <= 0 {
                                            snapshot.entities.iter().map(|(id, _)| *id).collect()
                                        } else {
                                            visible_entity_ids_snapshot(
                                                snapshot,
                                                client_positions[client_idx],
                                                visibility_radius,
                                            )
                                        };
                                        let (server_count, server_hash) =
                                            state_digest_snapshot(snapshot, &expected_ids);
                                        let (client_count, client_hash) = state_digest(
                                            &mut client_state.world,
                                            &client_state.entities,
                                            &expected_ids,
                                        );
                                        if client_count != server_count
                                            || client_hash != server_hash
                                        {
                                            validation_errors += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    apply_times.push(apply_start.elapsed());
                    if cli.validate {
                        if matches!(cli.scenario, Scenario::Loss) && !client_state.last_applied_full
                        {
                            continue;
                        }
                        let applied_tick = client_state.last_applied_tick.raw();
                        if applied_tick > 0 {
                            let snapshot = server_snapshots
                                .iter()
                                .find(|entry| entry.tick == applied_tick);
                            if let Some(snapshot) = snapshot {
                                let expected_ids = if matches!(cli.scenario, Scenario::Loss) {
                                    client_state.entities.ids().into_iter().collect()
                                } else if visibility_radius <= 0 {
                                    snapshot.entities.iter().map(|(id, _)| *id).collect()
                                } else {
                                    visible_entity_ids_snapshot(
                                        snapshot,
                                        client_positions[client_idx],
                                        visibility_radius,
                                    )
                                };
                                let (server_count, server_hash) =
                                    state_digest_snapshot(snapshot, &expected_ids);
                                let (client_count, client_hash) = state_digest(
                                    &mut client_state.world,
                                    &client_state.entities,
                                    &expected_ids,
                                );
                                if client_count != server_count || client_hash != server_hash {
                                    validation_errors += 1;
                                }
                            }
                        }
                    }
                }
                if matches!(cli.mode, Mode::Sdec) {
                    client_state.send_buf = delivered;
                    client_state.send_buf.clear();
                }
            }
        }

        graph.clear_dirty();
        graph.clear_removed();
        // Advance Bevy change detection so Changed/Added/Removed are per-tick.
        server_world.clear_trackers();
    }

    let denom = (cli.ticks as u64).saturating_mul(clients as u64);
    let denom_f64 = if denom == 0 { 1.0 } else { denom as f64 };
    let summary = Summary {
        mode: cli.mode,
        scenario: cli.scenario,
        ticks: cli.ticks,
        entities: cli.entities,
        clients,
        dirty_pct,
        visibility_radius,
        drop_rate: cli.drop_rate,
        reorder_rate: cli.reorder_rate,
        session_init_bytes: sdec_session_init_bytes,
        bytes_avg: avg_u64(&bytes),
        bytes_p95: p95_u64(&mut bytes.clone()),
        encode_us_avg: avg_duration_us(&enc_times),
        encode_us_p95: p95_duration_us(&mut enc_times.clone()),
        replication_us_avg: avg_duration_us(&replication_times),
        replication_us_p95: p95_duration_us(&mut replication_times.clone()),
        codec_us_avg: avg_duration_us(&codec_times),
        codec_us_p95: p95_duration_us(&mut codec_times.clone()),
        header_us_avg: avg_duration_us(&header_times),
        header_us_p95: p95_duration_us(&mut header_times.clone()),
        apply_us_avg: avg_duration_us(&apply_times),
        apply_us_p95: p95_duration_us(&mut apply_times.clone()),
        errors,
        resyncs,
        validation_errors,
        total_create_entities,
        total_create_components,
        total_create_fields,
        total_update_entities,
        total_update_components,
        total_update_fields,
        total_destroy_entities,
        avg_create_entities_per_client_tick: total_create_entities as f64 / denom_f64,
        avg_create_components_per_client_tick: total_create_components as f64 / denom_f64,
        avg_create_fields_per_client_tick: total_create_fields as f64 / denom_f64,
        avg_update_entities_per_client_tick: total_update_entities as f64 / denom_f64,
        avg_update_components_per_client_tick: total_update_components as f64 / denom_f64,
        avg_update_fields_per_client_tick: total_update_fields as f64 / denom_f64,
        avg_destroy_entities_per_client_tick: total_destroy_entities as f64 / denom_f64,
    };

    let out_path = cli.out_dir.join("summary.json");
    fs::write(&out_path, serde_json::to_string_pretty(&summary)?)
        .with_context(|| format!("write {}", out_path.display()))?;

    println!("summary: {}", out_path.display());
    Ok(())
}

fn scenario_config(cli: &Cli) -> (u32, f32, i64) {
    let (default_clients, default_dirty, default_radius) = match cli.scenario {
        Scenario::Dense => (1, 1.0, 0),
        Scenario::Relevancy => (64, 0.1, 30_000),
        Scenario::Loss => (64, 0.1, 30_000),
    };
    let clients = cli.clients.unwrap_or(default_clients).max(1);
    let dirty_pct = cli.dirty_pct.unwrap_or(default_dirty).clamp(0.0, 1.0);
    let visibility_radius = cli.visibility_radius.unwrap_or(default_radius);
    (clients, dirty_pct, visibility_radius)
}

fn build_client_positions(clients: u32, rng: &mut Rng) -> Vec<(i64, i64)> {
    let radius = 40_000.0;
    let mut positions = Vec::new();
    let total = clients.max(1) as f64;
    for idx in 0..clients {
        let angle = (idx as f64 / total) * std::f64::consts::TAU;
        let jitter_x = rng.range_i64(-1_000, 1_000) as f64;
        let jitter_y = rng.range_i64(-1_000, 1_000) as f64;
        let x = (radius * angle.cos() + jitter_x) as i64;
        let y = (radius * angle.sin() + jitter_y) as i64;
        positions.push((x, y));
    }
    positions
}

fn collect_positions(world: &mut World) -> Vec<(Entity, PositionYaw)> {
    let mut query = world.query::<(Entity, &PositionYaw)>();
    query
        .iter(world)
        .map(|(entity, pos)| (entity, *pos))
        .collect()
}

fn visible_entity_ids(
    positions: &[(Entity, PositionYaw)],
    entities: &mut EntityMap,
    client_pos: (i64, i64),
    radius: i64,
) -> HashSet<codec::EntityId> {
    let mut visible = HashSet::new();
    let radius_sq = radius.saturating_mul(radius);
    for (entity, pos) in positions {
        let dx = pos.x_q - client_pos.0;
        let dy = pos.y_q - client_pos.1;
        let dist_sq = dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy));
        if dist_sq <= radius_sq {
            let id = entities.entity_id(*entity);
            visible.insert(id);
        }
    }
    visible
}

fn visible_entity_ids_snapshot(
    snapshot: &ServerSnapshot,
    client_pos: (i64, i64),
    radius: i64,
) -> HashSet<codec::EntityId> {
    let mut visible = HashSet::new();
    let radius_sq = radius.saturating_mul(radius);
    for (id, pos) in &snapshot.entities {
        let dx = pos.x_q - client_pos.0;
        let dy = pos.y_q - client_pos.1;
        let dist_sq = dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy));
        if dist_sq <= radius_sq {
            visible.insert(*id);
        }
    }
    visible
}

struct DemoWorldView<'a> {
    schema: &'a sdec_bevy::BevySchema,
    world: &'a World,
    entities: &'a EntityMap,
}

impl<'a> WorldView for DemoWorldView<'a> {
    fn snapshot(&self, entity: codec::EntityId) -> codec::EntitySnapshot {
        let Some(bevy_entity) = self.entities.entity(entity) else {
            return codec::EntitySnapshot {
                id: entity,
                components: Vec::new(),
            };
        };
        let components = self.schema.snapshot_entity(self.world, bevy_entity);
        codec::EntitySnapshot {
            id: entity,
            components,
        }
    }

    fn update(
        &self,
        entity: codec::EntityId,
        dirty_components: &[schema::ComponentId],
    ) -> Option<codec::DeltaUpdateEntity> {
        let bevy_entity = self.entities.entity(entity)?;
        self.schema
            .build_delta_update(self.world, bevy_entity, entity, dirty_components)
    }
}

fn build_sdec_snapshot_for_ids(
    schema: &sdec_bevy::BevySchema,
    world: &World,
    entities: &mut EntityMap,
    ids: &HashSet<codec::EntityId>,
) -> Vec<codec::EntitySnapshot> {
    let mut snapshots = Vec::new();
    for id in ids {
        let Some(entity) = entities.entity(*id) else {
            continue;
        };
        let components = schema.snapshot_entity(world, entity);
        if components.is_empty() {
            continue;
        }
        snapshots.push(codec::EntitySnapshot {
            id: *id,
            components,
        });
    }
    snapshots.sort_by_key(|entity| entity.id.raw());
    snapshots
}

#[derive(Clone, Copy)]
enum AppliedPacket {
    Full(SnapshotTick),
    Delta(SnapshotTick),
}

fn apply_sdec_packet(
    schema: &sdec_bevy::BevySchema,
    world: &mut World,
    entities: &mut EntityMap,
    session: &mut Option<codec::SessionState>,
    limits: &codec::CodecLimits,
    bytes: &[u8],
) -> Result<AppliedPacket> {
    if bytes.len() >= 4 {
        let magic = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        if magic == wire::MAGIC {
            let packet = wire::decode_packet(bytes, &wire::Limits::default())
                .context("decode full snapshot")?;
            let snapshot =
                codec::decode_full_snapshot_from_packet(schema.schema(), &packet, limits)?;
            apply_full_snapshot(schema, world, entities, &snapshot)?;
            if let Some(state) = session.as_mut() {
                state.last_tick = snapshot.tick;
            }
            return Ok(AppliedPacket::Full(snapshot.tick));
        }
    }

    let session = session
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("session init missing"))?;
    let packet =
        codec::decode_session_packet(schema.schema(), session, bytes, &wire::Limits::default())?;
    let decoded = codec::decode_delta_packet(schema.schema(), &packet, limits)?;
    apply_changes(
        schema,
        world,
        entities,
        &decoded.creates,
        &decoded.destroys,
        &decoded.updates,
    )?;
    Ok(AppliedPacket::Delta(session.last_tick))
}

#[allow(clippy::too_many_arguments)]
fn resync_client(
    schema: &sdec_bevy::BevySchema,
    server_world: &mut World,
    server_entities: &mut EntityMap,
    client_world: &mut World,
    client_entities: &mut EntityMap,
    session: &mut Option<codec::SessionState>,
    visible_ids: &HashSet<codec::EntityId>,
    limits: &codec::CodecLimits,
) -> Result<bool> {
    let snapshot = build_sdec_snapshot_for_ids(schema, server_world, server_entities, visible_ids);
    if snapshot.is_empty() {
        return Ok(false);
    }
    let mut buf = vec![0u8; 256 * 1024];
    let len = encode_full_snapshot_retry(
        schema.schema(),
        SnapshotTick::new(session.as_ref().map(|s| s.last_tick.raw()).unwrap_or(0) + 1),
        &snapshot,
        limits,
        &mut buf,
    )?;
    let _ = apply_sdec_packet(
        schema,
        client_world,
        client_entities,
        session,
        limits,
        &buf[..len],
    )?;
    Ok(true)
}

fn encode_full_snapshot_retry(
    schema: &schema::Schema,
    tick: SnapshotTick,
    snapshot: &[codec::EntitySnapshot],
    limits: &codec::CodecLimits,
    buf: &mut Vec<u8>,
) -> Result<usize> {
    let mut size = buf.len().max(1024);
    for _ in 0..5 {
        if buf.len() < size {
            buf.resize(size, 0);
        }
        match codec::encode_full_snapshot(schema, tick, snapshot, limits, buf) {
            Ok(len) => return Ok(len),
            Err(codec::CodecError::OutputTooSmall { needed, .. }) => {
                size = size.max(needed).saturating_mul(2);
            }
            Err(codec::CodecError::Bitstream(_)) => size = size.saturating_mul(2),
            Err(err) => return Err(err.into()),
        }
    }
    Err(anyhow::anyhow!("snapshot encode retry overflow"))
}

fn encode_delta_retry(
    encoder: &mut codec::SessionEncoder<'_>,
    tick: SnapshotTick,
    baseline_tick: SnapshotTick,
    creates: &[codec::EntitySnapshot],
    destroys: &[codec::EntityId],
    updates: &[codec::DeltaUpdateEntity],
    buf: &mut Vec<u8>,
) -> Result<usize> {
    let mut size = buf.len().max(1024);
    for _ in 0..5 {
        if buf.len() < size {
            buf.resize(size, 0);
        }
        match codec::encode_delta_from_changes(
            encoder,
            tick,
            baseline_tick,
            creates,
            destroys,
            updates,
            buf,
        ) {
            Ok(len) => return Ok(len),
            Err(codec::CodecError::OutputTooSmall { needed, .. }) => {
                size = size.max(needed).saturating_mul(2);
            }
            Err(codec::CodecError::Bitstream(_)) => size = size.saturating_mul(2),
            Err(err) => return Err(err.into()),
        }
    }
    Err(anyhow::anyhow!("delta encode retry overflow"))
}

fn step_positions(world: &mut World, rng: &mut Rng, dirty_pct: f32) {
    let mut query = world.query::<&mut PositionYaw>();
    for mut pos in query.iter_mut(world) {
        if rng.chance() > dirty_pct {
            continue;
        }
        pos.x_q = (pos.x_q + rng.range_i64(-500, 500)).clamp(-100_000, 100_000);
        pos.y_q = (pos.y_q + rng.range_i64(-500, 500)).clamp(-100_000, 100_000);
        pos.yaw = ((pos.yaw as u32 + (rng.next_u32() % 13)) % 4096) as u16;
    }
}

#[derive(Debug, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
struct SnapshotEntity {
    id: u32,
    position: PositionYaw,
}

#[derive(Debug, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
struct SnapshotData {
    entities: Vec<SnapshotEntity>,
}

fn build_snapshot_for_ids(
    world: &mut World,
    entities: &mut EntityMap,
    ids: &HashSet<codec::EntityId>,
) -> SnapshotData {
    let mut query = world.query::<(Entity, &PositionYaw)>();
    let mut snapshots = Vec::new();
    for (entity, position) in query.iter(world) {
        let id = entities.entity_id(entity);
        if !ids.contains(&id) {
            continue;
        }
        snapshots.push(SnapshotEntity {
            id: id.raw(),
            position: *position,
        });
    }
    snapshots.sort_by_key(|entry| entry.id);
    SnapshotData {
        entities: snapshots,
    }
}

fn apply_snapshot(world: &mut World, entities: &mut EntityMap, snapshot: &SnapshotData) {
    let mut query = world.query::<&mut PositionYaw>();
    let mut seen = HashSet::new();
    for entry in &snapshot.entities {
        let id = codec::EntityId::new(entry.id);
        let entity = entities.entity(id).unwrap_or_else(|| {
            let new_entity = world.spawn(entry.position).id();
            entities.register(id, new_entity);
            new_entity
        });
        if let Ok(mut pos) = query.get_mut(world, entity) {
            *pos = entry.position;
        }
        seen.insert(id);
    }
    for id in entities.ids() {
        if seen.contains(&id) {
            continue;
        }
        if let Some(entity) = entities.entity(id) {
            world.despawn(entity);
            entities.unregister(id);
        }
    }
}

fn apply_full_snapshot(
    schema: &sdec_bevy::BevySchema,
    world: &mut World,
    entities: &mut EntityMap,
    snapshot: &codec::Snapshot,
) -> Result<()> {
    let mut seen = HashSet::new();
    for entity_snapshot in &snapshot.entities {
        let entity = entities.entity(entity_snapshot.id).unwrap_or_else(|| {
            let new_entity = world.spawn_empty().id();
            entities.register(entity_snapshot.id, new_entity);
            new_entity
        });
        for component in &entity_snapshot.components {
            let fields: Vec<(usize, FieldValue)> =
                component.fields.iter().copied().enumerate().collect();
            if schema
                .apply_component_fields(world, entity, component.id, &fields)
                .is_err()
            {
                schema.insert_component_fields(world, entity, component.id, &component.fields)?;
            }
        }
        seen.insert(entity_snapshot.id);
    }
    for id in entities.ids() {
        if seen.contains(&id) {
            continue;
        }
        if let Some(entity) = entities.entity(id) {
            world.despawn(entity);
            entities.unregister(id);
        }
    }
    Ok(())
}

fn state_digest(
    world: &mut World,
    entities: &EntityMap,
    expected_ids: &HashSet<codec::EntityId>,
) -> (u64, u64) {
    let mut count = 0u64;
    let mut hash = 0u64;
    let mut query = world.query::<&PositionYaw>();
    let mut ids: Vec<codec::EntityId> = expected_ids.iter().copied().collect();
    ids.sort_by_key(|id| id.raw());
    for id in ids {
        let Some(entity) = entities.entity(id) else {
            continue;
        };
        if let Ok(position) = query.get(world, entity) {
            count += 1;
            hash = mix_hash(hash, position);
        }
    }
    (count, hash)
}

fn build_server_snapshot(
    world: &mut World,
    entities: &mut EntityMap,
) -> Vec<(codec::EntityId, PositionYaw)> {
    let mut entries = Vec::new();
    let mut query = world.query::<(Entity, &PositionYaw)>();
    for (entity, position) in query.iter(world) {
        let id = entities.entity_id(entity);
        entries.push((id, *position));
    }
    entries.sort_by_key(|(id, _)| id.raw());
    entries
}

fn state_digest_snapshot(
    snapshot: &ServerSnapshot,
    expected_ids: &HashSet<codec::EntityId>,
) -> (u64, u64) {
    let mut count = 0u64;
    let mut hash = 0u64;
    for (id, position) in &snapshot.entities {
        if !expected_ids.contains(id) {
            continue;
        }
        count += 1;
        hash = mix_hash(hash, position);
    }
    (count, hash)
}

fn mix_hash(mut hash: u64, position: &PositionYaw) -> u64 {
    hash = hash
        .wrapping_mul(0x9E3779B97F4A7C15)
        .wrapping_add(position.x_q as u64);
    hash = hash
        .wrapping_mul(0x9E3779B97F4A7C15)
        .wrapping_add(position.y_q as u64);
    hash = hash
        .wrapping_mul(0x9E3779B97F4A7C15)
        .wrapping_add(position.yaw as u64);
    hash
}

fn build_compact_packet(
    out: &mut Vec<u8>,
    flags: wire::SessionFlags,
    last_tick: SnapshotTick,
    tick: SnapshotTick,
    baseline_tick: SnapshotTick,
    payload: &[u8],
) -> Result<usize> {
    let tick_raw = tick.raw();
    let last_raw = last_tick.raw();
    let tick_delta = tick_raw
        .checked_sub(last_raw)
        .ok_or_else(|| anyhow::anyhow!("tick went backwards"))?;
    if tick_delta == 0 {
        anyhow::bail!("tick delta must be non-zero for compact packets");
    }

    let baseline_delta = tick_raw
        .checked_sub(baseline_tick.raw())
        .ok_or_else(|| anyhow::anyhow!("baseline tick ahead of tick"))?;
    let needed = wire::SESSION_MAX_HEADER_SIZE + payload.len();
    if out.len() < needed {
        out.resize(needed, 0);
    }
    let header_len =
        wire::encode_session_header(out, flags, tick_delta, baseline_delta, payload.len() as u32)
            .map_err(|err| anyhow::anyhow!("encode session header: {err:?}"))?;
    out[header_len..header_len + payload.len()].copy_from_slice(payload);
    Ok(header_len + payload.len())
}

fn avg_u64(values: &[u64]) -> u64 {
    if values.is_empty() {
        return 0;
    }
    values.iter().sum::<u64>() / values.len() as u64
}

fn avg_duration_us(values: &[Duration]) -> u64 {
    if values.is_empty() {
        return 0;
    }
    values.iter().map(|v| v.as_micros() as u64).sum::<u64>() / values.len() as u64
}

fn p95_u64(values: &mut [u64]) -> u64 {
    values.sort_unstable();
    let idx = ((values.len() as f64) * 0.95).ceil() as usize;
    let idx = idx.saturating_sub(1).min(values.len().saturating_sub(1));
    values.get(idx).copied().unwrap_or(0)
}

fn p95_duration_us(values: &mut [Duration]) -> u64 {
    let mut micros: Vec<u64> = values.iter().map(|v| v.as_micros() as u64).collect();
    p95_u64(&mut micros)
}
