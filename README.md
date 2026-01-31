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

## Status

🚧 **Work in Progress** — Currently implementing the initial core release (codec + stable wire format).

See [ARCHITECTURE.md](ARCHITECTURE.md) for design details and [WIRE_FORMAT.md](WIRE_FORMAT.md) for the binary protocol specification.

## Workspace Structure

| Crate | Description |
|-------|-------------|
| `bitstream` | Low-level bit packing primitives (BitWriter, BitReader) |
| `wire` | Wire format: packet headers, section framing, limits |
| `schema` | Replication schema model and field codecs |
| `codec` | Snapshot/delta encoding and decoding |
| `tools` | Introspection and debugging utilities |
| `simbench` | Scenario generation and benchmarking |

## Quick Start

```rust
// Example usage (coming in the initial release)
use codec::{Encoder, Decoder, Schema};

// Define your schema
let schema = Schema::builder()
    .component("Transform")
        .field("x", FieldCodec::fixed_point(-1000.0, 1000.0, 16))
        .field("y", FieldCodec::fixed_point(-1000.0, 1000.0, 16))
        .field("z", FieldCodec::fixed_point(-1000.0, 1000.0, 16))
    .build();

// Encode a snapshot
let mut encoder = Encoder::new(&schema);
// ... add entities ...
let packet = encoder.encode_full_snapshot(tick);

// Decode on the client
let mut decoder = Decoder::new(&schema);
let snapshot = decoder.decode(&packet)?;
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
- Interest management / relevancy filtering
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
