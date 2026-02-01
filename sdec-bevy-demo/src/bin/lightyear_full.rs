use std::net::SocketAddr;
use std::time::{Duration, Instant};

use anyhow::Result;
use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use clap::Parser;
use crossbeam_channel::{unbounded, Receiver, Sender};
use lightyear::prelude::client::ClientCommands;
use lightyear::prelude::server::ServerCommands;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Parser)]
#[command(
    name = "sdec-bevy-demo-lightyear-full",
    version,
    about = "Lightyear full stack demo"
)]
struct Cli {
    /// Number of entities to simulate.
    #[arg(long, default_value_t = 512)]
    entities: u32,
    /// Number of clients to simulate.
    #[arg(long, default_value_t = 64)]
    clients: u32,
    /// Number of ticks to simulate.
    #[arg(long, default_value_t = 300)]
    ticks: u32,
    /// RNG seed for deterministic results.
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// Dirty percent (0.0 - 1.0).
    #[arg(long, default_value_t = 0.1)]
    dirty_pct: f32,
    /// Validate client state against server state each tick.
    #[arg(long, default_value_t = false)]
    validate: bool,
}

#[derive(Component, Clone, Copy, Serialize, Deserialize, Reflect)]
#[reflect(Component)]
struct PositionYaw {
    x_q: i64,
    y_q: i64,
    yaw: u16,
}

#[derive(Channel, Reflect)]
struct ReplicationChannel;

struct ProtocolPlugin;

impl Plugin for ProtocolPlugin {
    fn build(&self, app: &mut App) {
        app.register_component::<PositionYaw>(ChannelDirection::ServerToClient);
        app.add_channel::<ReplicationChannel>(ChannelSettings {
            mode: ChannelMode::UnorderedUnreliable,
            ..default()
        });
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
    ticks: u32,
    entities: u32,
    clients: u32,
    dirty_pct: f32,
    bytes_avg: u64,
    bytes_p95: u64,
    server_update_us_avg: u64,
    server_update_us_p95: u64,
    client_update_us_avg: u64,
    client_update_us_p95: u64,
    validation_errors: u64,
}

struct Forwarder {
    s2c_raw_rx: Receiver<Vec<u8>>,
    s2c_tx: Sender<Vec<u8>>,
    c2s_raw_rx: Receiver<Vec<u8>>,
    c2s_tx: Sender<Vec<u8>>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut rng = Rng::new(cli.seed);

    let protocol_id = 42u64;
    let private_key = generate_key();
    let server_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();

    let mut channels = Vec::new();
    let mut client_ios = Vec::new();
    let mut forwarders = Vec::new();
    for idx in 0..cli.clients {
        let (s2c_raw_tx, s2c_raw_rx) = unbounded::<Vec<u8>>();
        let (s2c_tx, s2c_rx) = unbounded::<Vec<u8>>();
        let (c2s_raw_tx, c2s_raw_rx) = unbounded::<Vec<u8>>();
        let (c2s_tx, c2s_rx) = unbounded::<Vec<u8>>();
        let client_addr: SocketAddr = format!("127.0.0.1:{}", 20000 + idx).parse().unwrap();
        channels.push((client_addr, c2s_rx, s2c_raw_tx));
        client_ios.push((s2c_rx, c2s_raw_tx));
        forwarders.push(Forwarder {
            s2c_raw_rx,
            s2c_tx,
            c2s_raw_rx,
            c2s_tx,
        });
    }

    let shared = SharedConfig::default();
    let server_config = server::ServerConfig {
        shared: shared.clone(),
        net: vec![server::NetConfig::Netcode {
            config: server::NetcodeConfig::default()
                .with_protocol_id(protocol_id)
                .with_key(private_key),
            io: IoConfig {
                transport: TransportConfig::Channels { channels },
                conditioner: None,
            },
        }],
        ..Default::default()
    };

    let mut server_app = App::new();
    server_app.add_plugins(MinimalPlugins.build());
    server_app.add_plugins((server::ServerPlugin::new(server_config), ProtocolPlugin));
    server_app.finish();
    server_app
        .world
        .run_system_once(|mut commands: Commands| commands.start_server());

    let now = bevy::utils::Instant::now();
    server_app.insert_resource(TimeUpdateStrategy::ManualInstant(now));

    let mut client_apps = Vec::new();
    for (idx, (recv, send)) in client_ios.into_iter().enumerate() {
        let net_config = client::NetConfig::Netcode {
            auth: client::Authentication::Manual {
                server_addr,
                client_id: (idx as u64) + 1,
                private_key,
                protocol_id,
            },
            config: Default::default(),
            io: IoConfig {
                transport: TransportConfig::LocalChannel { recv, send },
                conditioner: None,
            },
        };
        let config = client::ClientConfig {
            shared: shared.clone(),
            net: net_config,
            ..default()
        };
        let mut client_app = App::new();
        client_app.add_plugins(MinimalPlugins.build());
        client_app.add_plugins((client::ClientPlugin::new(config), ProtocolPlugin));
        client_app.finish();
        client_app
            .world
            .run_system_once(|mut commands: Commands| commands.connect_client());
        client_app.insert_resource(TimeUpdateStrategy::ManualInstant(now));
        client_apps.push(client_app);
    }

    // Spawn server entities.
    for _ in 0..cli.entities {
        let position = PositionYaw {
            x_q: 0,
            y_q: 0,
            yaw: 0,
        };
        server_app.world.spawn((position, Replicate::default()));
    }

    let mut bytes = Vec::new();
    let mut server_times = Vec::new();
    let mut client_times = Vec::new();
    let mut per_tick_bytes: Vec<u64> = vec![0; cli.clients as usize];

    let mut current_time = now;
    let tick_duration = shared.tick.tick_duration;

    // Let connections handshake and sync before we start measuring.
    let mut synced = false;
    for _ in 0..200 {
        current_time += tick_duration;
        server_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
        for client_app in &mut client_apps {
            client_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
        }
        forward_client_to_server(&mut forwarders);
        server_app.update();
        per_tick_bytes.fill(0);
        forward_server_to_client(&mut forwarders, &mut per_tick_bytes);
        for client_app in &mut client_apps {
            client_app.update();
        }

        let all_synced = client_apps.iter().all(|client_app| {
            client_app
                .world
                .get_resource::<client::ConnectionManager>()
                .is_some_and(|manager| manager.is_synced())
        });
        if all_synced {
            synced = true;
            break;
        }
    }
    if !synced {
        return Err(anyhow::anyhow!(
            "lightyear clients failed to sync within warmup window"
        ));
    }

    let mut validation_errors = 0u64;
    for _ in 0..cli.ticks {
        current_time += tick_duration;
        server_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
        for client_app in &mut client_apps {
            client_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
        }

        // mutate a subset of entities
        {
            let mut query = server_app.world.query::<&mut PositionYaw>();
            for mut pos in query.iter_mut(&mut server_app.world) {
                if rng.chance() > cli.dirty_pct {
                    continue;
                }
                pos.x_q = (pos.x_q + rng.range_i64(-500, 500)).clamp(-100_000, 100_000);
                pos.y_q = (pos.y_q + rng.range_i64(-500, 500)).clamp(-100_000, 100_000);
                pos.yaw = ((pos.yaw as u32 + (rng.next_u32() % 13)) % 4096) as u16;
            }
        }

        forward_client_to_server(&mut forwarders);
        let server_start = Instant::now();
        server_app.update();
        server_times.push(server_start.elapsed());
        let server_digest = if cli.validate {
            Some(state_digest_server(&mut server_app.world))
        } else {
            None
        };
        per_tick_bytes.fill(0);
        forward_server_to_client(&mut forwarders, &mut per_tick_bytes);

        let mut per_tick_client_time = Duration::default();
        for client_app in &mut client_apps {
            let start = Instant::now();
            client_app.update();
            per_tick_client_time += start.elapsed();
            if let Some((server_count, server_hash)) = server_digest {
                let (client_count, client_hash) = state_digest_client(&mut client_app.world);
                if client_count != server_count || client_hash != server_hash {
                    validation_errors += 1;
                }
            }
        }
        bytes.extend(per_tick_bytes.iter().copied());
        let avg_client_time = per_tick_client_time.as_micros() as u64 / cli.clients.max(1) as u64;
        client_times.push(Duration::from_micros(avg_client_time));
    }

    let summary = Summary {
        ticks: cli.ticks,
        entities: cli.entities,
        clients: cli.clients,
        dirty_pct: cli.dirty_pct,
        bytes_avg: avg_u64(&bytes),
        bytes_p95: p95_u64(&mut bytes.clone()),
        server_update_us_avg: avg_duration_us(&server_times),
        server_update_us_p95: p95_duration_us(&mut server_times.clone()),
        client_update_us_avg: avg_duration_us(&client_times),
        client_update_us_p95: p95_duration_us(&mut client_times.clone()),
        validation_errors,
    };

    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

fn forward_client_to_server(forwarders: &mut [Forwarder]) {
    for forwarder in forwarders {
        while let Ok(packet) = forwarder.c2s_raw_rx.try_recv() {
            let _ = forwarder.c2s_tx.send(packet);
        }
    }
}

fn forward_server_to_client(forwarders: &mut [Forwarder], per_tick_bytes: &mut [u64]) {
    for (idx, forwarder) in forwarders.iter_mut().enumerate() {
        let mut bytes = 0u64;
        while let Ok(packet) = forwarder.s2c_raw_rx.try_recv() {
            bytes += packet.len() as u64;
            let _ = forwarder.s2c_tx.send(packet);
        }
        if let Some(slot) = per_tick_bytes.get_mut(idx) {
            *slot += bytes;
        }
    }
}

fn state_digest_server(world: &mut World) -> (u64, u64) {
    let mut count = 0u64;
    let mut hash = 0u64;
    let mut query = world.query::<&PositionYaw>();
    for position in query.iter(world) {
        count += 1;
        hash = mix_hash(hash, position);
    }
    (count, hash)
}

fn state_digest_client(world: &mut World) -> (u64, u64) {
    let mut count = 0u64;
    let mut hash = 0u64;
    let mut query = world.query_filtered::<&PositionYaw, With<Replicated>>();
    for position in query.iter(world) {
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
