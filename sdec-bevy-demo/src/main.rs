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
    /// Replication mode (sdec, naive, or lightyear).
    #[arg(long, value_enum, default_value_t = Mode::Sdec)]
    mode: Mode,
    /// Output directory for summary.json.
    #[arg(long, default_value = "target/sdec-bevy-demo")]
    out_dir: PathBuf,
    /// Simulated packet drop rate (0.0 - 1.0).
    #[arg(long, default_value_t = 0.0)]
    drop_rate: f32,
}

#[derive(Clone, Copy, Debug, ValueEnum, Serialize)]
enum Mode {
    Sdec,
    Naive,
    Lightyear,
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
    ticks: u32,
    entities: u32,
    drop_rate: f32,
    bytes_avg: u64,
    bytes_p95: u64,
    encode_us_avg: u64,
    encode_us_p95: u64,
    apply_us_avg: u64,
    apply_us_p95: u64,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    fs::create_dir_all(&cli.out_dir)
        .with_context(|| format!("create {}", cli.out_dir.display()))?;

    let mut schema_builder = BevySchemaBuilder::new();
    schema_builder.component::<PositionYaw>();
    let schema = schema_builder.build().context("build schema")?;

    let mut server_world = World::new();
    let mut client_world = World::new();
    let mut server_entities = EntityMap::new();
    let mut client_entities = EntityMap::new();
    let mut client_entity_list = Vec::new();

    let mut rng = Rng::new(cli.seed);
    for _ in 0..cli.entities {
        let position = PositionYaw {
            x_q: rng.range_i64(-50_000, 50_000),
            y_q: rng.range_i64(-50_000, 50_000),
            yaw: (rng.next_u32() % 4096) as u16,
        };
        let server = server_world.spawn(position).id();
        let client = client_world.spawn(position).id();
        let _ = server_entities.entity_id(server);
        let _ = client_entities.entity_id(client);
        client_entity_list.push(client);
    }

    let mut bytes = Vec::new();
    let mut enc_times = Vec::new();
    let mut apply_times = Vec::new();

    for tick in 1..=cli.ticks {
        step_positions(&mut server_world, &mut rng);

        let start = Instant::now();
        let payload = match cli.mode {
            Mode::Sdec => {
                let changes = extract_changes(&schema, &mut server_world, &mut server_entities);
                let mut buf = vec![0u8; 16 * 1024];
                let len = codec::encode_delta_from_changes(
                    &mut codec::SessionEncoder::new(
                        schema.schema(),
                        &codec::CodecLimits::default(),
                    ),
                    SnapshotTick::new(tick),
                    SnapshotTick::new(tick.saturating_sub(1)),
                    &changes.creates,
                    &changes.destroys,
                    &changes.updates,
                    &mut buf,
                )?;
                buf.truncate(len);
                buf
            }
            Mode::Naive => {
                let snapshot = build_snapshot(&mut server_world);
                bincode::serialize(&snapshot).context("serialize naive snapshot")?
            }
            Mode::Lightyear => {
                let snapshot = build_snapshot(&mut server_world);
                bitcode::encode(&snapshot)
            }
        };
        let encode_elapsed = start.elapsed();

        if rng.chance() >= cli.drop_rate {
            let apply_start = Instant::now();
            match cli.mode {
                Mode::Sdec => {
                    let packet = wire::decode_packet(&payload, &wire::Limits::default())
                        .context("decode")?;
                    let decoded = codec::decode_delta_packet(
                        schema.schema(),
                        &packet,
                        &codec::CodecLimits::default(),
                    )?;
                    apply_changes(
                        &schema,
                        &mut client_world,
                        &mut client_entities,
                        &decoded.creates,
                        &decoded.destroys,
                        &decoded.updates,
                    )?;
                }
                Mode::Naive => {
                    let snapshot: SnapshotData =
                        bincode::deserialize(&payload).context("deserialize naive snapshot")?;
                    apply_snapshot(&mut client_world, &client_entity_list, &snapshot);
                }
                Mode::Lightyear => {
                    let snapshot: SnapshotData =
                        bitcode::decode(&payload).context("bitcode decode")?;
                    apply_snapshot(&mut client_world, &client_entity_list, &snapshot);
                }
            }
            apply_times.push(apply_start.elapsed());
        }

        bytes.push(payload.len() as u64);
        enc_times.push(encode_elapsed);
    }

    let summary = Summary {
        mode: cli.mode,
        ticks: cli.ticks,
        entities: cli.entities,
        drop_rate: cli.drop_rate,
        bytes_avg: avg_u64(&bytes),
        bytes_p95: p95_u64(&mut bytes.clone()),
        encode_us_avg: avg_duration_us(&enc_times),
        encode_us_p95: p95_duration_us(&mut enc_times.clone()),
        apply_us_avg: avg_duration_us(&apply_times),
        apply_us_p95: p95_duration_us(&mut apply_times.clone()),
    };

    let out_path = cli.out_dir.join("summary.json");
    fs::write(&out_path, serde_json::to_string_pretty(&summary)?)
        .with_context(|| format!("write {}", out_path.display()))?;

    println!("summary: {}", out_path.display());
    Ok(())
}

fn step_positions(world: &mut World, rng: &mut Rng) {
    let mut query = world.query::<&mut PositionYaw>();
    for mut pos in query.iter_mut(world) {
        pos.x_q = (pos.x_q + rng.range_i64(-500, 500)).clamp(-100_000, 100_000);
        pos.y_q = (pos.y_q + rng.range_i64(-500, 500)).clamp(-100_000, 100_000);
        pos.yaw = ((pos.yaw as u32 + (rng.next_u32() % 13)) % 4096) as u16;
    }
}

#[derive(Debug, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
struct SnapshotData {
    entities: Vec<PositionYaw>,
}

fn build_snapshot(world: &mut World) -> SnapshotData {
    let mut query = world.query::<&PositionYaw>();
    let entities = query.iter(world).cloned().collect();
    SnapshotData { entities }
}

fn apply_snapshot(world: &mut World, entities: &[Entity], snapshot: &SnapshotData) {
    let mut query = world.query::<&mut PositionYaw>();
    for (entity, next) in entities
        .iter()
        .copied()
        .zip(snapshot.entities.iter().copied())
    {
        if let Ok(mut pos) = query.get_mut(world, entity) {
            *pos = next;
        }
    }
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
