use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use codec::{
    apply_delta_snapshot_from_packet, decode_full_snapshot_from_packet, encode_delta_snapshot,
    encode_full_snapshot, CodecLimits, Snapshot, SnapshotTick, WireLimits,
};
use demo_schema::{
    build_snapshot, demo_schema, DemoEntityState, POS_MAX, POS_MIN, VEL_MAX, VEL_MIN,
};
use serde::Serialize;
use tools::decode_packet_json;
use wire::decode_packet;

#[derive(Parser)]
#[command(
    name = "demo-sim",
    version,
    about = "Deterministic demo capture generator"
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
    /// Output directory for captures.
    #[arg(long, default_value = "captures")]
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
    write_schema_json(&cli.out_dir, &schema)?;

    let mut rng = Rng::new(cli.seed);
    let mut states = init_states(cli.players, &mut rng);

    let mut summary = Summary::new(cli.players, cli.ticks, cli.seed, cli.burst_every);
    let mut baseline = Snapshot {
        tick: SnapshotTick::new(0),
        entities: Vec::new(),
    };

    for tick in 1..=cli.ticks {
        step_states(&mut states, &mut rng, tick, cli.burst_every);
        let snapshot = build_snapshot(SnapshotTick::new(tick), &states);
        if tick == 1 {
            let bytes = encode_full(&schema, &snapshot, &limits)?;
            let path = cli.out_dir.join(format!("full_{tick:06}.bin"));
            write_packet(&path, &bytes)?;
            validate_packet(&schema, &wire_limits, &limits, &bytes, &snapshot, None)?;
            summary.push_full(bytes.len() as u64);
        } else {
            let bytes = encode_delta(&schema, &baseline, &snapshot, &limits)?;
            let path = cli.out_dir.join(format!(
                "delta_{tick:06}_base_{:06}.bin",
                baseline.tick.raw()
            ));
            write_packet(&path, &bytes)?;
            validate_packet(
                &schema,
                &wire_limits,
                &limits,
                &bytes,
                &snapshot,
                Some(&baseline),
            )?;
            summary.push_delta(bytes.len() as u64);
        }
        baseline = snapshot;
    }

    summary.finalize();
    summary.assert_budgets(cli.max_p95_delta_bytes, cli.max_avg_delta_bytes)?;
    write_summary_json(&cli.out_dir, &summary)?;

    Ok(())
}

fn write_schema_json(out_dir: &Path, schema: &schema::Schema) -> Result<()> {
    let path = out_dir.join("schema.json");
    let contents = serde_json::to_string_pretty(schema).context("serialize schema")?;
    fs::write(&path, contents).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn write_packet(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes).with_context(|| format!("write {}", path.display()))
}

fn write_summary_json(out_dir: &Path, summary: &Summary) -> Result<()> {
    let path = out_dir.join("summary.json");
    let contents = serde_json::to_string_pretty(summary).context("serialize summary")?;
    fs::write(&path, contents).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn encode_full(
    schema: &schema::Schema,
    snapshot: &Snapshot,
    limits: &CodecLimits,
) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; limits.max_section_bytes.max(wire::HEADER_SIZE) * 4];
    let bytes = encode_full_snapshot(schema, snapshot.tick, &snapshot.entities, limits, &mut buf)
        .context("encode full snapshot")?;
    buf.truncate(bytes);
    Ok(buf)
}

fn encode_delta(
    schema: &schema::Schema,
    baseline: &Snapshot,
    current: &Snapshot,
    limits: &CodecLimits,
) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; limits.max_section_bytes.max(wire::HEADER_SIZE) * 4];
    let bytes = encode_delta_snapshot(
        schema,
        current.tick,
        baseline.tick,
        baseline,
        current,
        limits,
        &mut buf,
    )
    .context("encode delta snapshot")?;
    buf.truncate(bytes);
    Ok(buf)
}

fn validate_packet(
    schema: &schema::Schema,
    wire_limits: &WireLimits,
    limits: &CodecLimits,
    bytes: &[u8],
    expected: &Snapshot,
    baseline: Option<&Snapshot>,
) -> Result<()> {
    let packet = decode_packet(bytes, wire_limits).context("decode packet")?;
    let _ = decode_packet_json(bytes, schema, wire_limits, limits).context("tools decode")?;
    if packet.header.flags.is_full_snapshot() {
        let decoded = decode_full_snapshot_from_packet(schema, &packet, limits)
            .context("decode full snapshot")?;
        if decoded.entities != expected.entities {
            anyhow::bail!("full snapshot decode mismatch");
        }
    } else {
        let baseline = baseline.context("missing baseline for delta validation")?;
        let applied = apply_delta_snapshot_from_packet(schema, baseline, &packet, limits)
            .context("apply delta snapshot")?;
        if applied.entities != expected.entities {
            anyhow::bail!("delta apply produced mismatched snapshot");
        }
    }
    Ok(())
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
    avg_bytes_per_tick: u64,
    avg_delta_bytes: u64,
    p95_delta_bytes: u64,
    #[serde(skip)]
    delta_sizes: Vec<u64>,
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
            avg_bytes_per_tick: 0,
            avg_delta_bytes: 0,
            p95_delta_bytes: 0,
            delta_sizes: Vec::new(),
        }
    }

    fn push_full(&mut self, bytes: u64) {
        self.full_count += 1;
        self.full_bytes_total += bytes;
    }

    fn push_delta(&mut self, bytes: u64) {
        self.delta_count += 1;
        self.delta_bytes_total += bytes;
        self.delta_sizes.push(bytes);
    }

    fn finalize(&mut self) {
        if self.ticks > 0 {
            self.avg_bytes_per_tick =
                (self.full_bytes_total + self.delta_bytes_total) / self.ticks as u64;
        }
        if self.delta_count > 0 {
            self.avg_delta_bytes = self.delta_bytes_total / self.delta_count as u64;
            self.delta_sizes.sort_unstable();
            let idx = ((self.delta_sizes.len() as f64) * 0.95).ceil() as usize;
            let idx = idx.saturating_sub(1).min(self.delta_sizes.len() - 1);
            self.p95_delta_bytes = self.delta_sizes[idx];
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
