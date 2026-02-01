use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use codec::{
    apply_delta_snapshot_from_packet, decode_full_snapshot_from_packet, decode_session_init_packet,
    decode_session_packet, encode_delta_snapshot_for_client_session, encode_full_snapshot,
    encode_session_init_packet, CodecLimits, CompactHeaderMode, SessionState, Snapshot,
    SnapshotTick, WireLimits,
};
use demo_schema::{demo_schema, DemoEntityState, POS_MAX, POS_MIN, VEL_MAX, VEL_MIN};
use serde::Serialize;
use tools::{build_decode_output, decode_packet_json, inspect_packet};
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
    /// Number of simulated clients (per-client captures).
    #[arg(long, default_value_t = 4)]
    clients: u32,
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
    if cli.players == 0 {
        anyhow::bail!("players must be > 0");
    }
    let schema = demo_schema();
    let limits = CodecLimits::default();
    let wire_limits = WireLimits::default();
    let clients = cli.clients.min(cli.players).max(1);

    fs::create_dir_all(&cli.out_dir)
        .with_context(|| format!("create output dir {}", cli.out_dir.display()))?;
    write_schema_json(&cli.out_dir, &schema)?;

    let session_tick = SnapshotTick::new(1);
    let mut session_buf = vec![0u8; wire::HEADER_SIZE + limits.max_section_bytes.max(32)];
    let session_len = encode_session_init_packet(
        &schema,
        session_tick,
        Some(cli.seed),
        CompactHeaderMode::SessionV1,
        &limits,
        &mut session_buf,
    )
    .context("encode session init")?;
    session_buf.truncate(session_len);
    let session_path = cli.out_dir.join("session_init.bin");
    write_packet(&session_path, &session_buf)?;
    let session_packet =
        decode_packet(&session_buf, &wire_limits).context("decode session init")?;
    let session_state = decode_session_init_packet(&schema, &session_packet, &limits)
        .context("decode session init")?;
    let _ = inspect_packet(&session_buf, None, &wire_limits, &limits)
        .context("tools inspect session init")?;

    let mut rng = Rng::new(cli.seed);
    let mut states = init_states(cli.players, &mut rng);

    let mut summary = Summary::new(cli.players, clients, cli.ticks, cli.seed, cli.burst_every);
    summary.set_session_init_bytes(session_len as u64);
    let mut baselines: Vec<Snapshot> = Vec::with_capacity(clients as usize);
    let mut session_states: Vec<SessionState> = vec![session_state; clients as usize];
    let mut session_last_ticks: Vec<SnapshotTick> = vec![session_tick; clients as usize];

    for tick in 1..=cli.ticks {
        step_states(&mut states, &mut rng, tick, cli.burst_every);
        if tick == 1 {
            baselines.clear();
            for client_idx in 0..clients {
                let snapshot =
                    build_visible_snapshot(SnapshotTick::new(tick), &states, client_idx as usize);
                let bytes = encode_full(&schema, &snapshot, &limits)?;
                let path = cli.out_dir.join(format!(
                    "full_client_{client:03}_tick_{tick:06}.bin",
                    client = client_idx + 1
                ));
                write_packet(&path, &bytes)?;
                validate_packet(&schema, &wire_limits, &limits, &bytes, &snapshot, None)?;
                summary.push_full(bytes.len() as u64);
                baselines.push(snapshot);
            }
        } else {
            for client_idx in 0..clients as usize {
                let baseline = &baselines[client_idx];
                let snapshot = build_visible_snapshot(SnapshotTick::new(tick), &states, client_idx);
                let mut buf = vec![0u8; limits.max_section_bytes.max(wire::HEADER_SIZE) * 4];
                let bytes = encode_delta_snapshot_for_client_session(
                    &schema,
                    snapshot.tick,
                    baseline.tick,
                    baseline,
                    &snapshot,
                    &limits,
                    &mut session_last_ticks[client_idx],
                    &mut buf,
                )
                .context("encode session delta")?;
                buf.truncate(bytes);
                let path = cli.out_dir.join(format!(
                    "delta_client_{client:03}_tick_{tick:06}_base_{base:06}.bin",
                    client = client_idx + 1,
                    base = baseline.tick.raw()
                ));
                write_packet(&path, &buf)?;
                validate_session_delta(
                    &schema,
                    &wire_limits,
                    &limits,
                    &buf,
                    &snapshot,
                    baseline,
                    &mut session_states[client_idx],
                )?;
                summary.push_delta(bytes as u64);
                baselines[client_idx] = snapshot;
            }
        }
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

fn validate_session_delta(
    schema: &schema::Schema,
    wire_limits: &WireLimits,
    limits: &CodecLimits,
    bytes: &[u8],
    expected: &Snapshot,
    baseline: &Snapshot,
    session: &mut SessionState,
) -> Result<()> {
    let packet =
        decode_session_packet(schema, session, bytes, wire_limits).context("decode session")?;
    let _ = build_decode_output(schema, &packet, limits).context("tools decode session")?;
    let applied = apply_delta_snapshot_from_packet(schema, baseline, &packet, limits)
        .context("apply delta snapshot")?;
    if applied.entities != expected.entities {
        anyhow::bail!("session delta apply produced mismatched snapshot");
    }
    Ok(())
}

const VIS_RADIUS: i64 = 30_000;

fn build_visible_snapshot(
    tick: SnapshotTick,
    states: &[DemoEntityState],
    client_idx: usize,
) -> Snapshot {
    let client_state = &states[client_idx];
    let center = client_state.pos_q;
    let center_id = client_state.id;
    let radius_sq = (VIS_RADIUS as i128) * (VIS_RADIUS as i128);
    let mut entities = Vec::new();
    for state in states {
        if state.id == center_id {
            entities.push(state.to_snapshot());
            continue;
        }
        let dx = state.pos_q[0] as i128 - center[0] as i128;
        let dy = state.pos_q[1] as i128 - center[1] as i128;
        let dz = state.pos_q[2] as i128 - center[2] as i128;
        let dist_sq = dx * dx + dy * dy + dz * dz;
        if dist_sq <= radius_sq {
            entities.push(state.to_snapshot());
        }
    }
    entities.sort_by_key(|entity| entity.id.raw());
    Snapshot { tick, entities }
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
    clients: u32,
    ticks: u32,
    seed: u64,
    burst_every: Option<u32>,
    session_init_bytes: u64,
    full_count: u32,
    delta_count: u32,
    full_bytes_total: u64,
    delta_bytes_total: u64,
    avg_bytes_per_client_tick: u64,
    avg_delta_bytes: u64,
    p95_delta_bytes: u64,
    #[serde(skip)]
    delta_sizes: Vec<u64>,
}

impl Summary {
    fn new(players: u32, clients: u32, ticks: u32, seed: u64, burst_every: Option<u32>) -> Self {
        Self {
            players,
            clients,
            ticks,
            seed,
            burst_every,
            session_init_bytes: 0,
            full_count: 0,
            delta_count: 0,
            full_bytes_total: 0,
            delta_bytes_total: 0,
            avg_bytes_per_client_tick: 0,
            avg_delta_bytes: 0,
            p95_delta_bytes: 0,
            delta_sizes: Vec::new(),
        }
    }

    fn set_session_init_bytes(&mut self, bytes: u64) {
        self.session_init_bytes = bytes;
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
        if self.ticks > 0 && self.clients > 0 {
            let denom = (self.ticks as u64).saturating_mul(self.clients as u64);
            self.avg_bytes_per_client_tick =
                (self.full_bytes_total + self.delta_bytes_total) / denom;
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
