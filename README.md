# sdec

**S**napshot **D**elta **E**ncoding **C**odec — A transport-agnostic, bit-packed snapshot + delta codec for realtime state synchronization.

[![CI](https://github.com/kplane-dev/sdec/actions/workflows/ci.yml/badge.svg)](https://github.com/kplane-dev/sdec/actions/workflows/ci.yml)

## Overview

`sdec` provides a high-performance codec for replicating game state over the network. It focuses on:

- **Bit-packed encoding** — Minimize bandwidth with bit-level precision
- **Delta compression** — Send only what changed since the last acknowledged state
- **Quantization** — Configurable precision for position, rotation, and other numeric fields
- **Correctness first** — Bounded decoding, no panics, explicit error handling
- **Engine agnostic** — Bytes in, bytes out. No assumptions about ECS or networking stack

For interest management and per-client change lists, use the `sdec-repgraph` crate. It decides
what gets encoded and feeds directly into `codec::encode_delta_from_changes`.

## What We Solve

Real-time state replication needs predictable bandwidth and deterministic behavior, not just fast
serialization. `sdec` is built to encode *snapshots and deltas* efficiently and safely, explicitly
prioritizing network bytes over CPU when the two are in tension, with control over change
detection, quantization, and wire budgets.

## Philosophy

- **Determinism over heuristics** — Same inputs produce the same bytes across platforms.
- **Bandwidth first** — Bit-packed fields and delta semantics keep packets small.
- **Explicit control** — Schemas define codecs, change policies, and thresholds.
- **Composable layers** — Codec, schema, wire framing, and relevancy are separate concerns.

## Status

🚧 **Work in Progress** — Core protocol is stable; sessions/compact headers and repgraph
integration are active, and public APIs are still evolving.

## Initial Results (Simbench)

Dense, 16 players, 300 ticks (delta encoding):

Commands:

```bash
cargo run -p simbench --release
```

| Metric | SDEC delta | Bincode delta |
| --- | --- | --- |
| Avg bytes | 259B | 1114B |
| P95 bytes | 266B | 1159B |
| Encode avg | ~10us | ~2us |

In this codec-only harness, SDEC produces significantly smaller deltas than a
generic bincode delta payload at the cost of higher CPU per encode. This matters
in realtime replication where bandwidth (not CPU) is often the limiting factor.

## Initial Results (sdec-bevy-demo)

Dense, 64 entities, 300 ticks, 100% dirty (end-to-end Bevy path):

Commands:

```bash
cargo run -p sdec-bevy-demo --release --bin sdec-bevy-demo -- --entities 64 --ticks 300 --dirty-pct 1.0
cargo run -p sdec-bevy-demo --release --bin sdec-bevy-demo -- --mode naive --entities 64 --ticks 300 --dirty-pct 1.0
```

| Metric | SDEC | Naive (bincode snapshot) |
| --- | --- | --- |
| Avg bytes | 649B | 1416B |
| P95 bytes | 649B | 1416B |
| Encode avg | ~26us | ~2us |
| Apply avg | ~13us | ~3us |

This path includes change extraction, delta building, encoding, and apply logic.
The naive/bincode baseline is faster but more than 2x larger on the wire. SDEC
trades CPU for bandwidth, which is usually the binding constraint in realtime
replication.

See [ARCHITECTURE.md](ARCHITECTURE.md) for design details and [WIRE_FORMAT.md](WIRE_FORMAT.md) for the binary protocol specification.

## Docs

- [Getting Started](docs/getting-started.md)
- [Protocol: Sessions and Compact Frames](docs/protocol.md)
- [Recovery and Failure Modes](docs/recovery.md)

## Workspace Structure

| Crate | Description |
|-------|-------------|
| `bitstream` | Low-level bit packing primitives (BitWriter, BitReader) |
| `wire` | Wire format: packet headers, section framing, limits |
| `schema` | Replication schema model and field codecs |
| `codec` | Snapshot/delta encoding and decoding |
| `repgraph` | Replication graph + interest management |
| `sdec-bevy` | Bevy ECS adapter for schema/extract/apply |
| `tools` | Introspection and debugging utilities |
| `simbench` | Scenario generation and benchmarking |

## Quick Start

```rust
use codec::{
    apply_delta_snapshot, encode_delta_snapshot, encode_full_snapshot, CodecLimits, Snapshot,
    SnapshotTick,
};
use schema::{ComponentDef, FieldCodec, FieldDef, FieldId, Schema};

let component = ComponentDef::new(schema::ComponentId::new(1).unwrap())
    .field(FieldDef::new(FieldId::new(1).unwrap(), FieldCodec::bool()));
let schema = Schema::new(vec![component]).unwrap();

let baseline = Snapshot {
    tick: SnapshotTick::new(10),
    entities: vec![],
};

let mut buf = [0u8; 256];
let _full_len = encode_full_snapshot(
    &schema,
    baseline.tick,
    &baseline.entities,
    &CodecLimits::for_testing(),
    &mut buf,
)
.unwrap();

let delta_len = encode_delta_snapshot(
    &schema,
    SnapshotTick::new(11),
    baseline.tick,
    &baseline,
    &baseline,
    &CodecLimits::for_testing(),
    &mut buf,
)
.unwrap();

let applied = apply_delta_snapshot(
    &schema,
    &baseline,
    &buf[..delta_len],
    &wire::Limits::for_testing(),
    &CodecLimits::for_testing(),
)
.unwrap();
```

## Tools

The `sdec-tools` CLI provides packet inspection and decoding:

```bash
cargo run -p tools -- inspect packet.bin --schema schema.json
cargo run -p tools -- inspect captures/ --schema schema.json --glob "delta_*.bin" --sort size --limit 10
cargo run -p tools -- decode packet.bin --schema schema.json
cargo run -p tools -- decode packet.bin --schema schema.json --format pretty
```

Schema JSON is available via the optional `serde` feature on the `schema` crate.

## Demo Simulation

Generate deterministic captures and a summary report:

```bash
cargo run -p demo-sim -- --players 16 --ticks 300 --seed 1 --out-dir captures
```

This writes `schema.json`, `full_*.bin`, `delta_*.bin`, and `summary.json` to the output directory.

## Simbench Harness

Run a deterministic benchmark and emit `summary.json`:

```bash
cargo run -p simbench -- --players 16 --ticks 300 --seed 1 --out-dir target/simbench
```
## Building

```bash
# Build all crates
cargo build --workspace

# Run tests
cargo test --workspace

# Run clippy
cargo clippy --workspace --all-targets -- -D warnings

# Format code
cargo fmt --all
```

## Design Goals

1. **Correctness and safety** — Bounded decoding, no panics, no OOM amplification
2. **Engine agnostic** — No dependency on specific game engines or networking stacks
3. **Pragmatic performance** — Zero steady-state allocations, competitive wire efficiency
4. **Evolvable format** — Versioned wire protocol with room for extensions
5. **First-class tooling** — Inspection and debugging tools are part of the product

## Non-Goals (for now)

- Transport layer (UDP, QUIC, etc.)
- Client prediction / server reconciliation
- Encryption / authentication

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

## Contributing

Contributions are welcome! Please read the architecture docs before submitting PRs.

Every PR should:
- Add/extend tests proportional to changes
- Pass `cargo clippy` with no warnings
- Pass `cargo fmt --check`
- Maintain the correctness invariants documented in ARCHITECTURE.md
