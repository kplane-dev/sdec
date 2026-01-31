use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use bitstream::BitVecWriter;
use clap::{Parser, ValueEnum};
use codec::{
    encode_delta_snapshot_for_client_with_scratch, encode_delta_snapshot_with_scratch,
    encode_full_snapshot, CodecLimits, CodecScratch,
};
use serde::Serialize;
use wire::Limits as WireLimits;

#[derive(Parser)]
#[command(
    name = "simbench",
    version,
    about = "sdec simulation benchmark harness"
)]
struct Cli {
    /// Scenario to run (dense, idle, burst, visibility).
    #[arg(long, value_enum, default_value_t = Scenario::Dense)]
    scenario: Scenario,
    /// Number of simulated players/entities.
    #[arg(long, default_value_t = 16)]
    players: u32,
    /// Number of ticks to simulate.
    #[arg(long, default_value_t = 300)]
    ticks: u32,
    /// RNG seed for deterministic results.
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// Probability an entity is idle this tick (idle scenario).
    #[arg(long, default_value_t = 0.8)]
    idle_ratio: f32,
    /// Max jitter amplitude in quantized units (idle scenario).
    #[arg(long, default_value_t = 2)]
    jitter_amplitude_q: i64,
    /// Change threshold in quantized units.
    #[arg(long, default_value_t = 0)]
    threshold_q: u32,
    /// Optional burst event cadence (burst scenario).
    #[arg(long)]
    burst_every: Option<u32>,
    /// Fraction of entities affected by burst.
    #[arg(long, default_value_t = 0.25)]
    burst_fraction: f32,
    /// Burst amplitude in quantized units.
    #[arg(long, default_value_t = 1000)]
    burst_amplitude_q: i64,
    /// Number of clients to evaluate (visibility scenario).
    #[arg(long, default_value_t = 4)]
    clients: u32,
    /// Visibility radius in quantized units (visibility scenario).
    #[arg(long, default_value_t = 200)]
    visibility_radius_q: i64,
    /// World size in quantized units (visibility scenario).
    #[arg(long, default_value_t = 2000)]
    world_size_q: i64,
    /// Output directory for summary.json.
    #[arg(long, default_value = "target/simbench")]
    out_dir: PathBuf,
    /// Emit per-client breakdown details to stdout (visibility scenario only).
    #[arg(long, default_value_t = false)]
    debug_client_breakdown: bool,
    /// Fail if p95 delta packet size exceeds this value.
    #[arg(long)]
    max_p95_delta_bytes: Option<u64>,
    /// Fail if average delta packet size exceeds this value.
    #[arg(long)]
    max_avg_delta_bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, ValueEnum, Serialize, PartialEq, Eq)]
enum Scenario {
    Dense,
    Idle,
    Burst,
    Visibility,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let schema = demo_schema(cli.threshold_q);
    let limits = CodecLimits::default();
    let wire_limits = WireLimits::default();

    fs::create_dir_all(&cli.out_dir)
        .with_context(|| format!("create output dir {}", cli.out_dir.display()))?;

    let mut rng = Rng::new(cli.seed);
    let mut states = init_states(cli.players, &mut rng, cli.world_size_q);

    let mut baseline_snapshot = codec::Snapshot {
        tick: codec::SnapshotTick::new(0),
        entities: Vec::new(),
    };
    let mut scratch = CodecScratch::default();

    let mut sdec = EncoderStats::default();
    let mut naive = EncoderStats::default();
    let mut full_bincode_bytes_total = 0u64;
    let mut full_bytes_total = 0u64;
    let mut full_count = 0u32;

    let mut per_client_stats = if cli.scenario == Scenario::Visibility {
        Some(PerClientStats::new(cli.clients))
    } else {
        None
    };
    let mut per_client_breakdown =
        if cli.scenario == Scenario::Visibility && cli.debug_client_breakdown {
            Some(ClientBreakdown::default())
        } else {
            None
        };
    let mut client_baselines: Vec<codec::Snapshot> = Vec::new();

    for tick in 1..=cli.ticks {
        step_states(&mut states, &mut rng, tick, &cli);
        let snapshot = build_snapshot(codec::SnapshotTick::new(tick), &states);

        full_bincode_bytes_total += encode_bincode_snapshot(&states)? as u64;

        if tick == 1 {
            let full_bytes = encode_full(&schema, &snapshot, &limits)?;
            full_bytes_total += full_bytes.len() as u64;
            full_count += 1;
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
            sdec.add(delta_bytes.len() as u64, elapsed.as_micros() as u64);
            let naive_start = Instant::now();
            let naive_bytes = encode_naive_delta(&schema, &baseline_snapshot, &snapshot)?;
            let naive_elapsed = naive_start.elapsed();
            naive.add(naive_bytes as u64, naive_elapsed.as_micros() as u64);
        }

        if cli.scenario == Scenario::Visibility {
            if client_baselines.is_empty() {
                client_baselines = (0..cli.clients)
                    .map(|_| codec::Snapshot {
                        tick: codec::SnapshotTick::new(0),
                        entities: Vec::new(),
                    })
                    .collect();
            }
            run_visibility(
                &schema,
                &states,
                tick,
                &mut client_baselines,
                &mut scratch,
                cli.visibility_radius_q,
                per_client_stats.as_mut().expect("per-client stats"),
                per_client_breakdown.as_mut(),
            )?;
        }

        baseline_snapshot = snapshot;
    }

    let summary = Summary::new(
        &cli,
        full_count,
        full_bytes_total,
        full_bincode_bytes_total,
        sdec,
        naive,
        per_client_stats,
    );

    summary.assert_budgets(cli.max_p95_delta_bytes, cli.max_avg_delta_bytes)?;
    write_summary_json(&cli.out_dir, &summary)?;
    if let Some(breakdown) = per_client_breakdown {
        breakdown.print();
    }

    if summary.sdec.delta_p95 > wire_limits.max_packet_bytes as u64 {
        anyhow::bail!(
            "p95 delta bytes {} exceeds wire packet limit {}",
            summary.sdec.delta_p95,
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

fn encode_delta_for_client_with_scratch(
    schema: &schema::Schema,
    baseline: &codec::Snapshot,
    current: &codec::Snapshot,
    limits: &CodecLimits,
    scratch: &mut CodecScratch,
) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; limits.max_section_bytes.max(wire::HEADER_SIZE) * 4];
    let bytes = encode_delta_snapshot_for_client_with_scratch(
        schema,
        current.tick,
        baseline.tick,
        baseline,
        current,
        limits,
        scratch,
        &mut buf,
    )
    .context("encode delta snapshot (client)")?;
    buf.truncate(bytes);
    Ok(buf)
}

#[derive(Default, Debug)]
struct ClientBreakdown {
    packets: u64,
    packet_bytes: u64,
    destroy_bytes: u64,
    create_bytes: u64,
    update_masked_bytes: u64,
    update_sparse_bytes: u64,
    update_entities: u64,
    update_components: u64,
    update_fields: u64,
}

impl ClientBreakdown {
    fn print(&self) {
        println!("client breakdown (visibility):");
        println!("  packets: {}", self.packets);
        println!(
            "  section bytes: destroy={} create={} update_masked={} update_sparse={}",
            self.destroy_bytes,
            self.create_bytes,
            self.update_masked_bytes,
            self.update_sparse_bytes
        );
        if self.packets > 0 {
            let avg_packet = self.packet_bytes as f64 / self.packets as f64;
            let avg_update_bytes =
                (self.update_masked_bytes + self.update_sparse_bytes) as f64 / self.packets as f64;
            println!("  avg packet bytes: {:.1}", avg_packet);
            println!("  avg update section bytes: {:.1}", avg_update_bytes);
            println!("  avg overhead bytes: {:.1}", avg_packet - avg_update_bytes);
        }
        println!(
            "  updates: entities={} components={} fields={}",
            self.update_entities, self.update_components, self.update_fields
        );
    }
}

fn record_client_breakdown(
    breakdown: &mut ClientBreakdown,
    schema: &schema::Schema,
    bytes: &[u8],
) -> Result<()> {
    let packet = wire::decode_packet(bytes, &WireLimits::default()).context("decode packet")?;
    breakdown.packets += 1;
    breakdown.packet_bytes += bytes.len() as u64;
    for section in &packet.sections {
        match section.tag {
            wire::SectionTag::EntityDestroy => breakdown.destroy_bytes += section.body.len() as u64,
            wire::SectionTag::EntityCreate => breakdown.create_bytes += section.body.len() as u64,
            wire::SectionTag::EntityUpdate => {
                breakdown.update_masked_bytes += section.body.len() as u64
            }
            wire::SectionTag::EntityUpdateSparse | wire::SectionTag::EntityUpdateSparsePacked => {
                breakdown.update_sparse_bytes += section.body.len() as u64
            }
            _ => {}
        }
    }

    if packet.header.flags.is_delta_snapshot() {
        let decoded = codec::decode_delta_packet(schema, &packet, &CodecLimits::default())
            .context("decode delta packet")?;
        breakdown.update_entities += decoded.updates.len() as u64;
        for entity in decoded.updates {
            breakdown.update_components += entity.components.len() as u64;
            for component in entity.components {
                breakdown.update_fields += component.fields.len() as u64;
            }
        }
    }

    Ok(())
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

fn encode_naive_delta(
    schema: &schema::Schema,
    baseline: &codec::Snapshot,
    current: &codec::Snapshot,
) -> Result<usize> {
    let mut writer = BitVecWriter::new();
    writer.align_to_byte();
    let mut changed_entities = 0u32;
    let mut entity_offsets = Vec::new();

    for entity in &current.entities {
        let base = baseline.entities.iter().find(|e| e.id == entity.id);
        if let Some(base) = base {
            let changed_fields = diff_entity_fields(schema, base, entity)?;
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
    schema: &schema::Schema,
    baseline: &codec::EntitySnapshot,
    current: &codec::EntitySnapshot,
) -> Result<Vec<(usize, codec::FieldValue)>> {
    let mut result = Vec::new();
    if baseline.components.is_empty() || current.components.is_empty() {
        return Ok(result);
    }
    let base_component = &baseline.components[0];
    let curr_component = &current.components[0];
    let component = schema
        .components
        .first()
        .context("missing component definition")?;

    for (idx, (field, (base, curr))) in component
        .fields
        .iter()
        .zip(
            base_component
                .fields
                .iter()
                .zip(curr_component.fields.iter()),
        )
        .enumerate()
    {
        if field_changed(field, *base, *curr)? {
            result.push((idx, *curr));
        }
    }
    Ok(result)
}

fn field_changed(
    field: &schema::FieldDef,
    baseline: codec::FieldValue,
    current: codec::FieldValue,
) -> Result<bool> {
    match field.change {
        schema::ChangePolicy::Always => field_differs(field, baseline, current),
        schema::ChangePolicy::Threshold { threshold_q } => {
            field_exceeds_threshold(field, baseline, current, threshold_q)
        }
    }
}

fn field_differs(
    field: &schema::FieldDef,
    baseline: codec::FieldValue,
    current: codec::FieldValue,
) -> Result<bool> {
    Ok(match (baseline, current) {
        (codec::FieldValue::Bool(a), codec::FieldValue::Bool(b)) => a != b,
        (codec::FieldValue::UInt(a), codec::FieldValue::UInt(b)) => a != b,
        (codec::FieldValue::SInt(a), codec::FieldValue::SInt(b)) => a != b,
        (codec::FieldValue::VarUInt(a), codec::FieldValue::VarUInt(b)) => a != b,
        (codec::FieldValue::VarSInt(a), codec::FieldValue::VarSInt(b)) => a != b,
        (codec::FieldValue::FixedPoint(a), codec::FieldValue::FixedPoint(b)) => a != b,
        _ => {
            anyhow::bail!(
                "field type mismatch for {:?} ({:?} vs {:?})",
                field.id,
                baseline,
                current
            )
        }
    })
}

fn field_exceeds_threshold(
    field: &schema::FieldDef,
    baseline: codec::FieldValue,
    current: codec::FieldValue,
    threshold_q: u32,
) -> Result<bool> {
    let threshold_q = threshold_q as u64;
    Ok(match (baseline, current) {
        (codec::FieldValue::FixedPoint(a), codec::FieldValue::FixedPoint(b)) => {
            (a - b).unsigned_abs() > threshold_q
        }
        (codec::FieldValue::UInt(a), codec::FieldValue::UInt(b)) => a.abs_diff(b) > threshold_q,
        (codec::FieldValue::SInt(a), codec::FieldValue::SInt(b)) => {
            (a - b).unsigned_abs() > threshold_q
        }
        (codec::FieldValue::VarUInt(a), codec::FieldValue::VarUInt(b)) => {
            a.abs_diff(b) > threshold_q
        }
        (codec::FieldValue::VarSInt(a), codec::FieldValue::VarSInt(b)) => {
            (a - b).unsigned_abs() > threshold_q
        }
        (codec::FieldValue::Bool(a), codec::FieldValue::Bool(b)) => a != b,
        _ => {
            anyhow::bail!(
                "field type mismatch for {:?} ({:?} vs {:?})",
                field.id,
                baseline,
                current
            )
        }
    })
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

fn demo_schema(threshold_q: u32) -> schema::Schema {
    let threshold = schema::ChangePolicy::Threshold { threshold_q };
    let component = schema::ComponentDef::new(component_id())
        .field(
            schema::FieldDef::new(
                field_id(1),
                schema::FieldCodec::fixed_point(POS_MIN, POS_MAX, POS_SCALE),
            )
            .change(threshold),
        )
        .field(
            schema::FieldDef::new(
                field_id(2),
                schema::FieldCodec::fixed_point(POS_MIN, POS_MAX, POS_SCALE),
            )
            .change(threshold),
        )
        .field(
            schema::FieldDef::new(
                field_id(3),
                schema::FieldCodec::fixed_point(POS_MIN, POS_MAX, POS_SCALE),
            )
            .change(threshold),
        )
        .field(
            schema::FieldDef::new(
                field_id(4),
                schema::FieldCodec::fixed_point(VEL_MIN, VEL_MAX, VEL_SCALE),
            )
            .change(threshold),
        )
        .field(
            schema::FieldDef::new(
                field_id(5),
                schema::FieldCodec::fixed_point(VEL_MIN, VEL_MAX, VEL_SCALE),
            )
            .change(threshold),
        )
        .field(
            schema::FieldDef::new(
                field_id(6),
                schema::FieldCodec::fixed_point(VEL_MIN, VEL_MAX, VEL_SCALE),
            )
            .change(threshold),
        )
        .field(schema::FieldDef::new(field_id(7), schema::FieldCodec::uint(12)).change(threshold))
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

fn init_states(players: u32, rng: &mut Rng, world_size_q: i64) -> Vec<DemoEntityState> {
    let mut states = Vec::with_capacity(players as usize);
    let grid = (players as f64).sqrt().ceil() as u32;
    let spacing = (world_size_q / grid.max(1) as i64).max(1);
    for idx in 0..players {
        let id = codec::EntityId::new(idx + 1);
        let row = idx / grid;
        let col = idx % grid;
        let base_x = (col as i64 * spacing) - world_size_q / 2;
        let base_y = (row as i64 * spacing) - world_size_q / 2;
        let pos_q = [
            clamp(base_x + rng.range_i64(-50, 50), POS_MIN, POS_MAX),
            clamp(base_y + rng.range_i64(-50, 50), POS_MIN, POS_MAX),
            rng.range_i64(POS_MIN / 10, POS_MAX / 10),
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

fn step_states(states: &mut [DemoEntityState], rng: &mut Rng, tick: u32, cli: &Cli) {
    match cli.scenario {
        Scenario::Dense | Scenario::Visibility => step_dense(states, rng, tick, cli.burst_every),
        Scenario::Idle => step_idle(states, rng, cli.idle_ratio, cli.jitter_amplitude_q),
        Scenario::Burst => step_burst(
            states,
            rng,
            tick,
            cli.burst_every,
            cli.burst_fraction,
            cli.burst_amplitude_q,
        ),
    }
}

fn step_dense(states: &mut [DemoEntityState], rng: &mut Rng, tick: u32, burst: Option<u32>) {
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

fn step_idle(states: &mut [DemoEntityState], rng: &mut Rng, idle_ratio: f32, jitter_q: i64) {
    for state in states {
        let idle = (rng.next_u32() as f32 / u32::MAX as f32) < idle_ratio;
        if idle {
            for axis in 0..3 {
                let delta = rng.range_i64(-jitter_q, jitter_q);
                state.pos_q[axis] = clamp(state.pos_q[axis] + delta, POS_MIN, POS_MAX);
                let vel_delta = rng.range_i64(-jitter_q, jitter_q);
                state.vel_q[axis] = clamp(state.vel_q[axis] + vel_delta, VEL_MIN, VEL_MAX);
            }
            let yaw_delta = rng.range_i64(-jitter_q, jitter_q);
            state.yaw = ((state.yaw as i64 + yaw_delta).rem_euclid(4096)) as u16;
        } else {
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
        }
    }
}

fn step_burst(
    states: &mut [DemoEntityState],
    rng: &mut Rng,
    tick: u32,
    burst_every: Option<u32>,
    burst_fraction: f32,
    burst_amplitude_q: i64,
) {
    step_dense(states, rng, tick, None);
    let burst_now = burst_every.is_some_and(|every| every > 0 && tick % every == 0);
    if !burst_now {
        return;
    }
    for state in states {
        let burst = (rng.next_u32() as f32 / u32::MAX as f32) < burst_fraction;
        if burst {
            for axis in 0..3 {
                let delta = if rng.next_u32() % 2 == 0 {
                    burst_amplitude_q
                } else {
                    -burst_amplitude_q
                };
                state.pos_q[axis] = clamp(state.pos_q[axis] + delta, POS_MIN, POS_MAX);
            }
            state.yaw = ((state.yaw as i64 + burst_amplitude_q).rem_euclid(4096)) as u16;
            state.flags[0] = !state.flags[0];
        }
    }
}

fn run_visibility(
    schema: &schema::Schema,
    states: &[DemoEntityState],
    tick: u32,
    baselines: &mut [codec::Snapshot],
    scratch: &mut CodecScratch,
    radius_q: i64,
    stats: &mut PerClientStats,
    mut breakdown: Option<&mut ClientBreakdown>,
) -> Result<()> {
    let radius_sq = radius_q * radius_q;
    for (idx, baseline) in baselines.iter_mut().enumerate() {
        let observer = &states[idx % states.len()];
        let relevant: Vec<DemoEntityState> = states
            .iter()
            .filter(|state| {
                let dx = state.pos_q[0] - observer.pos_q[0];
                let dy = state.pos_q[1] - observer.pos_q[1];
                dx * dx + dy * dy <= radius_sq
            })
            .cloned()
            .collect();
        let snapshot = build_snapshot(codec::SnapshotTick::new(tick), &relevant);
        stats.full_bincode_total += encode_bincode_snapshot(&relevant)? as u64;

        if tick > 1 {
            let start = Instant::now();
            let delta_bytes = encode_delta_for_client_with_scratch(
                schema,
                baseline,
                &snapshot,
                &CodecLimits::default(),
                scratch,
            )?;
            if let Some(breakdown) = breakdown.as_mut() {
                record_client_breakdown(breakdown, schema, &delta_bytes)?;
            }
            let elapsed = start.elapsed();
            stats
                .sdec
                .add(delta_bytes.len() as u64, elapsed.as_micros() as u64);
            let naive_start = Instant::now();
            let naive_bytes = encode_naive_delta(schema, baseline, &snapshot)?;
            let naive_elapsed = naive_start.elapsed();
            stats
                .naive
                .add(naive_bytes as u64, naive_elapsed.as_micros() as u64);
        }

        *baseline = snapshot;
    }
    Ok(())
}

fn clamp(value: i64, min: i64, max: i64) -> i64 {
    value.min(max).max(min)
}

#[derive(Default)]
struct EncoderStats {
    sizes: Vec<u64>,
    encode_us: Vec<u64>,
    total_bytes: u64,
    count: u32,
}

impl EncoderStats {
    fn add(&mut self, bytes: u64, encode_us: u64) {
        self.total_bytes += bytes;
        self.count += 1;
        self.sizes.push(bytes);
        if encode_us > 0 {
            self.encode_us.push(encode_us);
        }
    }

    fn finalize(&mut self) -> EncoderSummary {
        let avg = if self.count > 0 {
            self.total_bytes / self.count as u64
        } else {
            0
        };
        let p95_value = if self.sizes.is_empty() {
            0
        } else {
            p95(&mut self.sizes)
        };
        let avg_encode = if self.encode_us.is_empty() {
            0
        } else {
            self.encode_us.iter().sum::<u64>() / self.encode_us.len() as u64
        };
        let p95_encode = if self.encode_us.is_empty() {
            0
        } else {
            p95(&mut self.encode_us)
        };
        EncoderSummary {
            delta_count: self.count,
            delta_bytes_total: self.total_bytes,
            delta_avg: avg,
            delta_p95: p95_value,
            encode_us_avg: avg_encode,
            encode_us_p95: p95_encode,
        }
    }
}

struct PerClientStats {
    clients: u32,
    sdec: EncoderStats,
    naive: EncoderStats,
    full_bincode_total: u64,
}

impl PerClientStats {
    fn new(clients: u32) -> Self {
        Self {
            clients,
            sdec: EncoderStats::default(),
            naive: EncoderStats::default(),
            full_bincode_total: 0,
        }
    }
}

#[derive(Debug, Serialize)]
struct Summary {
    scenario: ScenarioConfig,
    sdec: EncoderSummary,
    delta_naive: EncoderSummary,
    full_bincode: FullSummary,
    per_client: Option<PerClientSummary>,
}

impl Summary {
    fn new(
        cli: &Cli,
        full_count: u32,
        full_bytes_total: u64,
        full_bincode_bytes_total: u64,
        mut sdec: EncoderStats,
        mut naive: EncoderStats,
        per_client_stats: Option<PerClientStats>,
    ) -> Self {
        let sdec_summary = sdec.finalize();
        let naive_summary = naive.finalize();
        let avg_full = if cli.ticks > 0 {
            full_bytes_total / cli.ticks as u64
        } else {
            0
        };
        let avg_bincode = if cli.ticks > 0 {
            full_bincode_bytes_total / cli.ticks as u64
        } else {
            0
        };
        let per_client = per_client_stats.map(|mut stats| {
            let sdec_summary = stats.sdec.finalize();
            let naive_summary = stats.naive.finalize();
            let denom = stats.clients.max(1) as u64 * cli.ticks as u64;
            PerClientSummary {
                clients: stats.clients,
                sdec_avg_per_client: sdec_summary.delta_avg,
                naive_avg_per_client: naive_summary.delta_avg,
                full_bincode_avg_per_client: stats.full_bincode_total / denom,
            }
        });

        Summary {
            scenario: ScenarioConfig::from(cli),
            sdec: sdec_summary,
            delta_naive: naive_summary,
            full_bincode: FullSummary {
                full_count,
                full_bytes_total,
                full_bincode_bytes_total,
                avg_full_bytes: avg_full,
                avg_full_bincode_bytes: avg_bincode,
            },
            per_client,
        }
    }

    fn assert_budgets(&self, max_p95: Option<u64>, max_avg: Option<u64>) -> Result<()> {
        if let Some(max_p95) = max_p95 {
            if self.sdec.delta_p95 > max_p95 {
                anyhow::bail!(
                    "p95 delta bytes {} exceeds budget {}",
                    self.sdec.delta_p95,
                    max_p95
                );
            }
        }
        if let Some(max_avg) = max_avg {
            if self.sdec.delta_avg > max_avg {
                anyhow::bail!(
                    "avg delta bytes {} exceeds budget {}",
                    self.sdec.delta_avg,
                    max_avg
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
struct ScenarioConfig {
    name: Scenario,
    players: u32,
    ticks: u32,
    seed: u64,
    idle_ratio: f32,
    jitter_amplitude_q: i64,
    threshold_q: u32,
    burst_every: Option<u32>,
    burst_fraction: f32,
    burst_amplitude_q: i64,
    clients: u32,
    visibility_radius_q: i64,
    world_size_q: i64,
}

impl From<&Cli> for ScenarioConfig {
    fn from(cli: &Cli) -> Self {
        Self {
            name: cli.scenario,
            players: cli.players,
            ticks: cli.ticks,
            seed: cli.seed,
            idle_ratio: cli.idle_ratio,
            jitter_amplitude_q: cli.jitter_amplitude_q,
            threshold_q: cli.threshold_q,
            burst_every: cli.burst_every,
            burst_fraction: cli.burst_fraction,
            burst_amplitude_q: cli.burst_amplitude_q,
            clients: cli.clients,
            visibility_radius_q: cli.visibility_radius_q,
            world_size_q: cli.world_size_q,
        }
    }
}

#[derive(Debug, Serialize)]
struct EncoderSummary {
    delta_count: u32,
    delta_bytes_total: u64,
    delta_avg: u64,
    delta_p95: u64,
    encode_us_avg: u64,
    encode_us_p95: u64,
}

#[derive(Debug, Serialize)]
struct FullSummary {
    full_count: u32,
    full_bytes_total: u64,
    full_bincode_bytes_total: u64,
    avg_full_bytes: u64,
    avg_full_bincode_bytes: u64,
}

#[derive(Debug, Serialize)]
struct PerClientSummary {
    clients: u32,
    sdec_avg_per_client: u64,
    naive_avg_per_client: u64,
    full_bincode_avg_per_client: u64,
}

fn p95(values: &mut [u64]) -> u64 {
    values.sort_unstable();
    let idx = ((values.len() as f64) * 0.95).ceil() as usize;
    let idx = idx.saturating_sub(1).min(values.len() - 1);
    values[idx]
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
