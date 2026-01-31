use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use bitstream::BitVecWriter;
use clap::Parser;
use codec::{encode_delta_snapshot_with_scratch, encode_full_snapshot, CodecLimits, CodecScratch};
use serde::Serialize;
use wire::Limits as WireLimits;

#[derive(Parser)]
#[command(
    name = "simbench",
    version,
    about = "sdec simulation benchmark harness"
)]
struct Cli {
    /// Number of simulated players/entities.
    #[arg(long, default_value_t = 16)]
    players: u32,
    /// Number of ticks to simulate.
    #[arg(long, default_value_t = 300)]
    ticks: u32,
    /// RNG seed for deterministic results.
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// Optional burst event cadence.
    #[arg(long)]
    burst_every: Option<u32>,
    /// Output directory for summary.json.
    #[arg(long, default_value = "target/simbench")]
    out_dir: PathBuf,
    /// Fail if p95 delta packet size exceeds this value.
    #[arg(long)]
    max_p95_delta_bytes: Option<u64>,
    /// Fail if average delta packet size exceeds this value.
    #[arg(long)]
    max_avg_delta_bytes: Option<u64>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let schema = demo_schema();
    let limits = CodecLimits::default();
    let wire_limits = WireLimits::default();

    fs::create_dir_all(&cli.out_dir)
        .with_context(|| format!("create output dir {}", cli.out_dir.display()))?;

    let mut rng = Rng::new(cli.seed);
    let mut states = init_states(cli.players, &mut rng);

    let mut summary = Summary::new(cli.players, cli.ticks, cli.seed, cli.burst_every);
    let mut baseline_snapshot = codec::Snapshot {
        tick: codec::SnapshotTick::new(0),
        entities: Vec::new(),
    };
    let mut scratch = CodecScratch::default();

    for tick in 1..=cli.ticks {
        step_states(&mut states, &mut rng, tick, cli.burst_every);
        let snapshot = build_snapshot(codec::SnapshotTick::new(tick), &states);

        summary.full_bincode_bytes_total += encode_bincode_snapshot(&states)? as u64;

        if tick == 1 {
            let full_bytes = encode_full(&schema, &snapshot, &limits)?;
            summary.full_bytes_total += full_bytes.len() as u64;
            summary.full_count += 1;
        } else {
            let start = Instant::now();
            let delta_bytes = encode_delta_with_scratch(
                &schema,
                &baseline_snapshot,
                &snapshot,
                &limits,
                &mut scratch,
            )?;
            let elapsed = start.elapsed();
            summary.delta_bytes_total += delta_bytes.len() as u64;
            summary.delta_count += 1;
            summary.delta_sizes.push(delta_bytes.len() as u64);
            summary.encode_us.push(elapsed.as_micros() as u64);

            summary.delta_naive_bytes_total +=
                encode_naive_delta(&baseline_snapshot, &snapshot)? as u64;
        }

        baseline_snapshot = snapshot;
    }

    summary.finalize();
    summary.assert_budgets(cli.max_p95_delta_bytes, cli.max_avg_delta_bytes)?;
    write_summary_json(&cli.out_dir, &summary)?;

    // Validate that encoded payloads would be accepted by the wire limits.
    if summary.p95_delta_bytes > wire_limits.max_packet_bytes as u64 {
        anyhow::bail!(
            "p95 delta bytes {} exceeds wire packet limit {}",
            summary.p95_delta_bytes,
            wire_limits.max_packet_bytes
        );
    }

    Ok(())
}

fn write_summary_json(out_dir: &Path, summary: &Summary) -> Result<()> {
    let path = out_dir.join("summary.json");
    let contents = serde_json::to_string_pretty(summary).context("serialize summary")?;
    fs::write(&path, contents).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn encode_full(
    schema: &schema::Schema,
    snapshot: &codec::Snapshot,
    limits: &CodecLimits,
) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; limits.max_section_bytes.max(wire::HEADER_SIZE) * 4];
    let bytes = encode_full_snapshot(schema, snapshot.tick, &snapshot.entities, limits, &mut buf)
        .context("encode full snapshot")?;
    buf.truncate(bytes);
    Ok(buf)
}

fn encode_delta_with_scratch(
    schema: &schema::Schema,
    baseline: &codec::Snapshot,
    current: &codec::Snapshot,
    limits: &CodecLimits,
    scratch: &mut CodecScratch,
) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; limits.max_section_bytes.max(wire::HEADER_SIZE) * 4];
    let bytes = encode_delta_snapshot_with_scratch(
        schema,
        current.tick,
        baseline.tick,
        baseline,
        current,
        limits,
        scratch,
        &mut buf,
    )
    .context("encode delta snapshot")?;
    buf.truncate(bytes);
    Ok(buf)
}

fn encode_bincode_snapshot(states: &[DemoEntityState]) -> Result<usize> {
    let snapshot = SerdeSnapshot {
        entities: states
            .iter()
            .map(|state| SerdeEntity {
                id: state.id.raw(),
                pos_q: state.pos_q,
                vel_q: state.vel_q,
                yaw: state.yaw,
                flags: state.flags,
            })
            .collect(),
    };
    let bytes = bincode::serialize(&snapshot).context("bincode snapshot")?;
    Ok(bytes.len())
}

fn encode_naive_delta(baseline: &codec::Snapshot, current: &codec::Snapshot) -> Result<usize> {
    let mut writer = BitVecWriter::new();
    writer.align_to_byte();
    let mut changed_entities = 0u32;
    let mut entity_offsets = Vec::new();

    for entity in &current.entities {
        let base = baseline.entities.iter().find(|e| e.id == entity.id);
        if let Some(base) = base {
            let changed_fields = diff_entity_fields(base, entity);
            if !changed_fields.is_empty() {
                changed_entities += 1;
                entity_offsets.push((entity.id.raw(), changed_fields));
            }
        }
    }

    writer.write_varu32(changed_entities)?;
    for (entity_id, fields) in entity_offsets {
        writer.write_varu32(entity_id)?;
        writer.write_varu32(fields.len() as u32)?;
        for (field_idx, value) in fields {
            writer.write_varu32(field_idx as u32)?;
            write_field_value_naive(&mut writer, value)?;
        }
    }

    Ok(writer.finish().len())
}

fn diff_entity_fields(
    baseline: &codec::EntitySnapshot,
    current: &codec::EntitySnapshot,
) -> Vec<(usize, codec::FieldValue)> {
    let mut result = Vec::new();
    if baseline.components.is_empty() || current.components.is_empty() {
        return result;
    }
    let base_component = &baseline.components[0];
    let curr_component = &current.components[0];
    for (idx, (base, curr)) in base_component
        .fields
        .iter()
        .zip(curr_component.fields.iter())
        .enumerate()
    {
        if base != curr {
            result.push((idx, *curr));
        }
    }
    result
}

fn write_field_value_naive(writer: &mut BitVecWriter, value: codec::FieldValue) -> Result<()> {
    match value {
        codec::FieldValue::Bool(value) => {
            writer.write_varu32(u32::from(value))?;
        }
        codec::FieldValue::UInt(value) | codec::FieldValue::VarUInt(value) => {
            writer.write_varu32(value as u32)?;
        }
        codec::FieldValue::SInt(value) | codec::FieldValue::VarSInt(value) => {
            writer.write_vars32(value as i32)?;
        }
        codec::FieldValue::FixedPoint(value) => {
            writer.write_vars32(value as i32)?;
        }
    }
    Ok(())
}

fn build_snapshot(tick: codec::SnapshotTick, states: &[DemoEntityState]) -> codec::Snapshot {
    let mut entities: Vec<codec::EntitySnapshot> =
        states.iter().map(DemoEntityState::to_snapshot).collect();
    entities.sort_by_key(|entity| entity.id.raw());
    codec::Snapshot { tick, entities }
}

fn demo_schema() -> schema::Schema {
    let component = schema::ComponentDef::new(component_id())
        .field(schema::FieldDef::new(
            field_id(1),
            schema::FieldCodec::fixed_point(POS_MIN, POS_MAX, POS_SCALE),
        ))
        .field(schema::FieldDef::new(
            field_id(2),
            schema::FieldCodec::fixed_point(POS_MIN, POS_MAX, POS_SCALE),
        ))
        .field(schema::FieldDef::new(
            field_id(3),
            schema::FieldCodec::fixed_point(POS_MIN, POS_MAX, POS_SCALE),
        ))
        .field(schema::FieldDef::new(
            field_id(4),
            schema::FieldCodec::fixed_point(VEL_MIN, VEL_MAX, VEL_SCALE),
        ))
        .field(schema::FieldDef::new(
            field_id(5),
            schema::FieldCodec::fixed_point(VEL_MIN, VEL_MAX, VEL_SCALE),
        ))
        .field(schema::FieldDef::new(
            field_id(6),
            schema::FieldCodec::fixed_point(VEL_MIN, VEL_MAX, VEL_SCALE),
        ))
        .field(schema::FieldDef::new(
            field_id(7),
            schema::FieldCodec::uint(12),
        ))
        .field(schema::FieldDef::new(
            field_id(8),
            schema::FieldCodec::bool(),
        ))
        .field(schema::FieldDef::new(
            field_id(9),
            schema::FieldCodec::bool(),
        ))
        .field(schema::FieldDef::new(
            field_id(10),
            schema::FieldCodec::bool(),
        ));
    schema::Schema::new(vec![component]).expect("demo schema must be valid")
}

fn component_id() -> schema::ComponentId {
    schema::ComponentId::new(1).expect("component id must be non-zero")
}

fn field_id(value: u16) -> schema::FieldId {
    schema::FieldId::new(value).expect("field id must be non-zero")
}

const POS_SCALE: u32 = 100;
const POS_MIN: i64 = -100_000;
const POS_MAX: i64 = 100_000;
const VEL_SCALE: u32 = 100;
const VEL_MIN: i64 = -10_000;
const VEL_MAX: i64 = 10_000;

#[derive(Debug, Clone)]
struct DemoEntityState {
    id: codec::EntityId,
    pos_q: [i64; 3],
    vel_q: [i64; 3],
    yaw: u16,
    flags: [bool; 3],
}

impl DemoEntityState {
    fn to_snapshot(&self) -> codec::EntitySnapshot {
        codec::EntitySnapshot {
            id: self.id,
            components: vec![codec::ComponentSnapshot {
                id: component_id(),
                fields: vec![
                    codec::FieldValue::FixedPoint(self.pos_q[0]),
                    codec::FieldValue::FixedPoint(self.pos_q[1]),
                    codec::FieldValue::FixedPoint(self.pos_q[2]),
                    codec::FieldValue::FixedPoint(self.vel_q[0]),
                    codec::FieldValue::FixedPoint(self.vel_q[1]),
                    codec::FieldValue::FixedPoint(self.vel_q[2]),
                    codec::FieldValue::UInt(self.yaw as u64),
                    codec::FieldValue::Bool(self.flags[0]),
                    codec::FieldValue::Bool(self.flags[1]),
                    codec::FieldValue::Bool(self.flags[2]),
                ],
            }],
        }
    }
}

fn init_states(players: u32, rng: &mut Rng) -> Vec<DemoEntityState> {
    let mut states = Vec::with_capacity(players as usize);
    for idx in 0..players {
        let id = codec::EntityId::new(idx + 1);
        let pos_q = [
            rng.range_i64(POS_MIN / 2, POS_MAX / 2),
            rng.range_i64(POS_MIN / 2, POS_MAX / 2),
            rng.range_i64(POS_MIN / 2, POS_MAX / 2),
        ];
        let vel_q = [
            rng.range_i64(VEL_MIN / 10, VEL_MAX / 10),
            rng.range_i64(VEL_MIN / 10, VEL_MAX / 10),
            rng.range_i64(VEL_MIN / 10, VEL_MAX / 10),
        ];
        let yaw = (rng.next_u32() % 4096) as u16;
        states.push(DemoEntityState {
            id,
            pos_q,
            vel_q,
            yaw,
            flags: [false, false, false],
        });
    }
    states
}

fn step_states(states: &mut [DemoEntityState], rng: &mut Rng, tick: u32, burst: Option<u32>) {
    let burst_now = burst.is_some_and(|every| every > 0 && tick % every == 0);
    for state in states {
        for axis in 0..3 {
            if rng.next_u32() % 20 == 0 {
                let delta = rng.range_i64(-50, 50);
                state.vel_q[axis] = clamp(state.vel_q[axis] + delta, VEL_MIN, VEL_MAX);
            }
            state.pos_q[axis] = clamp(state.pos_q[axis] + state.vel_q[axis], POS_MIN, POS_MAX);
            if state.pos_q[axis] == POS_MIN || state.pos_q[axis] == POS_MAX {
                state.vel_q[axis] = -state.vel_q[axis];
            }
        }
        state.yaw = ((state.yaw as u32 + (rng.next_u32() % 13)) % 4096) as u16;
        if burst_now {
            state.flags[0] = !state.flags[0];
            state.flags[1] = !state.flags[1];
            state.yaw = ((state.yaw as u32 + 97) % 4096) as u16;
        }
        if rng.next_u32() % 50 == 0 {
            state.flags[2] = !state.flags[2];
        }
    }
}

fn clamp(value: i64, min: i64, max: i64) -> i64 {
    value.min(max).max(min)
}

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
}

#[derive(Debug, Serialize)]
struct Summary {
    players: u32,
    ticks: u32,
    seed: u64,
    burst_every: Option<u32>,
    full_count: u32,
    delta_count: u32,
    full_bytes_total: u64,
    delta_bytes_total: u64,
    full_bincode_bytes_total: u64,
    delta_naive_bytes_total: u64,
    avg_bytes_per_tick: u64,
    avg_delta_bytes: u64,
    p95_delta_bytes: u64,
    avg_full_bincode_bytes: u64,
    avg_delta_naive_bytes: u64,
    avg_encode_us: u64,
    p95_encode_us: u64,
    #[serde(skip)]
    delta_sizes: Vec<u64>,
    #[serde(skip)]
    encode_us: Vec<u64>,
}

impl Summary {
    fn new(players: u32, ticks: u32, seed: u64, burst_every: Option<u32>) -> Self {
        Self {
            players,
            ticks,
            seed,
            burst_every,
            full_count: 0,
            delta_count: 0,
            full_bytes_total: 0,
            delta_bytes_total: 0,
            full_bincode_bytes_total: 0,
            delta_naive_bytes_total: 0,
            avg_bytes_per_tick: 0,
            avg_delta_bytes: 0,
            p95_delta_bytes: 0,
            avg_full_bincode_bytes: 0,
            avg_delta_naive_bytes: 0,
            avg_encode_us: 0,
            p95_encode_us: 0,
            delta_sizes: Vec::new(),
            encode_us: Vec::new(),
        }
    }

    fn finalize(&mut self) {
        if self.ticks > 0 {
            self.avg_bytes_per_tick =
                (self.full_bytes_total + self.delta_bytes_total) / self.ticks as u64;
            self.avg_full_bincode_bytes = self.full_bincode_bytes_total / self.ticks as u64;
        }
        if self.delta_count > 0 {
            self.avg_delta_bytes = self.delta_bytes_total / self.delta_count as u64;
            self.p95_delta_bytes = p95(&mut self.delta_sizes);
            self.avg_delta_naive_bytes = self.delta_naive_bytes_total / self.delta_count as u64;
        }
        if !self.encode_us.is_empty() {
            let total: u64 = self.encode_us.iter().sum();
            self.avg_encode_us = total / self.encode_us.len() as u64;
            self.p95_encode_us = p95(&mut self.encode_us);
        }
    }

    fn assert_budgets(&self, max_p95: Option<u64>, max_avg: Option<u64>) -> Result<()> {
        if let Some(max_p95) = max_p95 {
            if self.p95_delta_bytes > max_p95 {
                anyhow::bail!(
                    "p95 delta bytes {} exceeds budget {}",
                    self.p95_delta_bytes,
                    max_p95
                );
            }
        }
        if let Some(max_avg) = max_avg {
            if self.avg_delta_bytes > max_avg {
                anyhow::bail!(
                    "avg delta bytes {} exceeds budget {}",
                    self.avg_delta_bytes,
                    max_avg
                );
            }
        }
        Ok(())
    }
}

fn p95(values: &mut [u64]) -> u64 {
    values.sort_unstable();
    let idx = ((values.len() as f64) * 0.95).ceil() as usize;
    let idx = idx.saturating_sub(1).min(values.len() - 1);
    values[idx]
}

#[derive(Debug, Serialize)]
struct SerdeSnapshot {
    entities: Vec<SerdeEntity>,
}

#[derive(Debug, Serialize)]
struct SerdeEntity {
    id: u32,
    pos_q: [i64; 3],
    vel_q: [i64; 3],
    yaw: u16,
    flags: [bool; 3],
}
