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

## Status

🚧 **Work in Progress** — Core protocol is stable; sessions/compact headers and repgraph
integration are active, and public APIs are still evolving.

Full snapshots are for **initial sync and recovery**. Compact deltas are for
**steady-state replication**.

## Initial Results (Simbench)

- Global delta size (dense): 259B avg, 266B p95 (vs 268B/282B naive).
- Per-client visibility: ~21B avg per client (naive list ~17B, full bincode ~65B).
- Dirty-list encode (dense): ~2x faster (97us → 47us avg).

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
