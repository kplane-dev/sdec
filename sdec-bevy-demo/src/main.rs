use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bevy_ecs::prelude::*;
use clap::{Parser, ValueEnum};
use codec::{FieldValue, SnapshotTick};
use sdec_bevy::{
    apply_changes, extract_changes, BevySchemaBuilder, EntityMap, ReplicatedComponent,
    ReplicatedField,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};

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
}

struct ClientState {
    world: World,
    entities: EntityMap,
    known: HashSet<codec::EntityId>,
    visible: HashSet<codec::EntityId>,
    change_set: ClientChangeSet,
    delta_buf: Vec<u8>,
    send_buf: Vec<u8>,
    session: Option<codec::SessionState>,
    last_tick: SnapshotTick,
    queue: VecDeque<Vec<u8>>,
    errors: u64,
}

impl ClientState {
    fn new() -> Self {
        Self {
            world: World::new(),
            entities: EntityMap::new(),
            known: HashSet::new(),
            visible: HashSet::new(),
            change_set: ClientChangeSet::default(),
            delta_buf: Vec::new(),
            send_buf: Vec::new(),
            session: None,
            last_tick: SnapshotTick::new(0),
            queue: VecDeque::new(),
            errors: 0,
        }
    }
}

#[derive(Default)]
struct ClientChangeSet {
    creates: Vec<codec::EntitySnapshot>,
    destroys: Vec<codec::EntityId>,
    updates: Vec<codec::DeltaUpdateEntity>,
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

    let client_positions = build_client_positions(clients, &mut rng);
    let mut clients_state: Vec<ClientState> =
        (0..clients as usize).map(|_| ClientState::new()).collect();

    let mut bytes = Vec::new();
    let mut enc_times = Vec::new();
    let mut replication_times = Vec::new();
    let mut codec_times = Vec::new();
    let mut header_times = Vec::new();
    let mut apply_times = Vec::new();
    let mut errors = 0u64;
    let mut resyncs = 0u64;

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
    let mut sdec_encoder = codec::SessionEncoder::new(schema.schema(), &sdec_limits);
    let mut sdec_session_init_bytes = 0u64;

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
                    build_sdec_client_changes(
                        &schema,
                        &server_world,
                        &mut server_entities,
                        &changes.updates,
                        &client_state.visible,
                        &mut client_state.known,
                        &mut client_state.change_set,
                    );
                    if tick == 1 {
                        let snapshot = build_sdec_snapshot_for_ids(
                            &schema,
                            &server_world,
                            &mut server_entities,
                            &client_state.visible,
                        );
                        let replication_elapsed = replication_start.elapsed();
                        let codec_start = Instant::now();
                        let _len = encode_full_snapshot_retry(
                            schema.schema(),
                            SnapshotTick::new(tick),
                            &snapshot,
                            &sdec_limits,
                            full_buf_size,
                            &mut client_state.send_buf,
                        )?;
                        client_state.last_tick = SnapshotTick::new(tick);
                        let payload = std::mem::take(&mut client_state.send_buf);
                        (payload, replication_elapsed, codec_start.elapsed(), Duration::ZERO)
                    } else {
                        let replication_elapsed = replication_start.elapsed();
                        let codec_start = Instant::now();
                        let delta_len = encode_delta_retry(
                            &mut sdec_encoder,
                            SnapshotTick::new(tick),
                            SnapshotTick::new(tick.saturating_sub(1)),
                            &client_state.change_set.creates,
                            &client_state.change_set.destroys,
                            &client_state.change_set.updates,
                            delta_buf_size,
                            &mut client_state.delta_buf,
                        )?;
                        let payload = &client_state.delta_buf[wire::HEADER_SIZE..delta_len];
                        let header_start = Instant::now();
                        let new_last_tick = build_compact_packet_into(
                            &mut client_state.send_buf,
                            wire::SessionFlags::delta_snapshot(),
                            client_state.last_tick,
                            SnapshotTick::new(tick),
                            SnapshotTick::new(tick.saturating_sub(1)),
                            payload,
                        )?;
                        client_state.last_tick = new_last_tick;
                        let packet = std::mem::take(&mut client_state.send_buf);
                        (packet, replication_elapsed, codec_start.elapsed(), header_start.elapsed())
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
                        Ok(())
                    }
                    Mode::Lightyear => {
                        let snapshot: SnapshotData =
                            bitcode::decode(&delivered).context("bitcode decode")?;
                        apply_snapshot(
                            &mut client_state.world,
                            &mut client_state.entities,
                            &snapshot,
                        );
                        Ok(())
                    }
                };
                if apply_result.is_err() {
                    errors += 1;
                    client_state.errors += 1;
                    if matches!(cli.mode, Mode::Sdec) {
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
                            }
                        }
                    }
                } else {
                    apply_times.push(apply_start.elapsed());
                }
                if matches!(cli.mode, Mode::Sdec) {
                    client_state.send_buf = delivered;
                    client_state.send_buf.clear();
                }
            }
        }

        // Advance Bevy change detection so Changed/Added/Removed are per-tick.
        server_world.clear_trackers();
    }

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

fn build_sdec_client_changes(
    schema: &sdec_bevy::BevySchema,
    world: &World,
    entities: &mut EntityMap,
    updates: &[codec::DeltaUpdateEntity],
    visible_ids: &HashSet<codec::EntityId>,
    known: &mut HashSet<codec::EntityId>,
    change_set: &mut ClientChangeSet,
) {
    change_set.creates.clear();
    change_set.destroys.clear();
    change_set.updates.clear();

    for id in visible_ids.iter().copied() {
        if known.contains(&id) {
            continue;
        }
        let Some(entity) = entities.entity(id) else {
            continue;
        };
        let components = schema.snapshot_entity(world, entity);
        if components.is_empty() {
            continue;
        }
        change_set.creates.push(codec::EntitySnapshot { id, components });
    }
    change_set.creates.sort_by_key(|entity| entity.id.raw());

    for id in known.iter().copied() {
        if !visible_ids.contains(&id) {
            change_set.destroys.push(id);
        }
    }
    change_set.destroys.sort_by_key(|id| id.raw());

    for update in updates {
        if visible_ids.contains(&update.id) && known.contains(&update.id) {
            change_set.updates.push(update.clone());
        }
    }

    known.clear();
    known.extend(visible_ids.iter().copied());

}

fn apply_sdec_packet(
    schema: &sdec_bevy::BevySchema,
    world: &mut World,
    entities: &mut EntityMap,
    session: &mut Option<codec::SessionState>,
    limits: &codec::CodecLimits,
    bytes: &[u8],
) -> Result<()> {
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
            return Ok(());
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
    Ok(())
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
    let mut buf = Vec::new();
    let len = encode_full_snapshot_retry(
        schema.schema(),
        SnapshotTick::new(session.as_ref().map(|s| s.last_tick.raw()).unwrap_or(0) + 1),
        &snapshot,
        limits,
        256 * 1024,
        &mut buf,
    )?;
    apply_sdec_packet(schema, client_world, client_entities, session, limits, &buf[..len])?;
    Ok(true)
}

fn encode_full_snapshot_retry(
    schema: &schema::Schema,
    tick: SnapshotTick,
    snapshot: &[codec::EntitySnapshot],
    limits: &codec::CodecLimits,
    start_size: usize,
    buf: &mut Vec<u8>,
) -> Result<usize> {
    let mut size = start_size.max(1024);
    for _ in 0..5 {
        if buf.len() < size {
            buf.resize(size, 0);
        }
        match codec::encode_full_snapshot(schema, tick, snapshot, limits, buf) {
            Ok(len) => {
                buf.truncate(len);
                return Ok(len);
            }
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
    start_size: usize,
    buf: &mut Vec<u8>,
) -> Result<usize> {
    let mut size = start_size.max(1024);
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
            Ok(len) => {
                buf.truncate(len);
                return Ok(len);
            }
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

fn build_compact_packet_into(
    buf: &mut Vec<u8>,
    flags: wire::SessionFlags,
    last_tick: SnapshotTick,
    tick: SnapshotTick,
    baseline_tick: SnapshotTick,
    payload: &[u8],
) -> Result<SnapshotTick> {
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
    if buf.len() < needed {
        buf.resize(needed, 0);
    }
    let header_len = wire::encode_session_header(
        buf,
        flags,
        tick_delta,
        baseline_delta,
        payload.len() as u32,
    )
    .map_err(|err| anyhow::anyhow!("encode session header: {err:?}"))?;
    let end = header_len + payload.len();
    buf[header_len..end].copy_from_slice(payload);
    buf.truncate(end);
    Ok(tick)
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
