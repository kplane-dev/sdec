# Architecture

This repo implements a transport-agnostic snapshot + delta codec for realtime state synchronization.
It focuses on: bit-packed encoding, quantization, baseline/ACK-driven delta compression, and correctness-first decoding.

The codec is **bytes in / bytes out**. It does not open sockets, manage connections, or assume any game engine/ECS.

---

## Goals

- **Correctness and safety by default**
  - Bounded decoding (no panics, no unbounded allocs, no OOM amplification).
  - Explicit limits and predictable memory usage.
  - Deterministic results for a given input ordering.

- **Engine and transport agnostic**
  - No dependency on UDP/ENet/QUIC.
  - No ECS types or engine-specific assumptions.
  - Caller supplies entity ordering and relevancy decisions.

- **Pragmatic performance**
  - Bit-packed payloads; quantization for common numeric fields.
  - No steady-state allocations in hot encode/decode paths (buffer reuse).
  - Optional tracing/introspection behind feature flags.

- **Evolvable wire format**
  - Versioned framing with room for additive extensions.
  - Schema identity to prevent silent decode mismatches.

---

## Non-Goals (for now)

- Interest management (relevancy sets). Caller provides the list of entities/components to replicate.
- Client prediction / server reconciliation / lag compensation.
- Encryption, authentication, NAT traversal, matchmaking.
- A full "network framework" (channels, reliability, resend). Transport layer is external.

---

## Workspace Layout

This is a single git repo using a Rust workspace. Split into crates to keep boundaries clean while iterating quickly.

### `bitstream/`
**Responsibility:** low-level bit packing primitives.

- `BitWriter`: write bits, aligned ints, varints.
- `BitReader`: bounded reads, exact error reporting.
- Utilities for fixed-point quantization encoding.
- **No domain knowledge** (no entities/components).

**Notes**
- Aim for `#![forbid(unsafe_code)]` in v0/v1. Revisit only with proof from profiling.
- Make bounds checks explicit and exhaustive.

### `wire/`
**Responsibility:** wire framing and canonical binary layout.

- Packet header encode/decode.
- Section tags and minimal section parsing.
- Limit checks that apply before allocating/iterating.

**Notes**
- `wire` does not know about the game state types—only the structure of the packet.
- Keep the format boring and stable.

### `schema/`
**Responsibility:** represent replication schemas and field codecs.

- Runtime schema model (v0).
- Deterministic `schema_hash`.
- Field codec descriptors:
  - `Bool`, `UInt`, `SInt`, `VarUInt`, `VarSInt`
  - `FixedPoint` (bounded, precision)
  - `Angle` (bounded, wrap-aware) — optional later
- Field policies:
  - quantization config
  - change threshold config (for delta emission)

**Notes**
- v0 starts runtime-first. Derive macros can come in v1.
- Avoid runtime reflection on arbitrary Rust types in v0; keep schema explicit.

### `codec/`
**Responsibility:** snapshot/delta logic.

- Build/encode full snapshots and deltas.
- Apply deltas to a baseline to reconstruct a new snapshot.
- Baseline history store (ring buffer) and baseline selection helpers.
- Change detection and per-field/per-component masks.

**Key types**
- `SnapshotTick` (u32)
- `EntityId` (u32 in v0; widenable later with a type alias)
- `Schema` (from `schema`)
- `CodecLimits` (hard bounds used by codec and wire)

**Notes**
- `codec` is where most invariants live. Keep them documented and tested.

### `tools/`
**Responsibility:** introspection / debugging tools.

- Decode a packet and print structure.
- Explain packet size by section/component/field (feature-gated tracing).
- Diff baseline vs current (uses decoded representations).

**Notes**
- Tooling is a major adoption lever—treat it as a first-class product.

### `simbench/`
**Responsibility:** reproducible scenarios for size/perf/robustness.

- Deterministic scenario generators (movement, bursts, spawns).
- Loss/reorder simulation to verify resync behavior.
- Benchmark output in JSON/CSV for CI regression checks.

---

## Public API Principles

### Bytes in / bytes out
- The codec interfaces should accept:
  - the schema,
  - a baseline reference (optional),
  - a “state view” provided by the caller (iterators/callbacks),
  - and output into caller-managed buffers.

### Explicit scratch/buffers
To avoid steady-state allocations:
- Provide `CodecScratch` that the caller owns and reuses.
- Encode functions accept `&mut Vec<u8>` or `&mut [u8]` and never allocate internally except in explicitly documented cold paths.

### Deterministic ordering
The codec should be deterministic given:
- stable entity ordering and stable component ordering.
Prefer:
- caller supplies stable ordering,
or
- codec sorts with caller-provided scratch (no heap).

---

## Core Invariants (Correctness)

These invariants must hold for all released versions:

1. **Decode never panics**
   - All parse errors are returned as `Result::Err` with structured errors.

2. **Decode is bounded**
   - Length prefixes are validated against limits *before* iteration and before any allocation.
   - All reads are bounds-checked against the input slice length.

3. **No amplification**
   - A small packet must not cause large allocations.
   - All allocations (if any) must be capped by `CodecLimits`.

4. **Schema mismatch is explicit**
   - Packets include `schema_hash`; mismatch fails fast.

5. **Wire format is versioned**
   - Backwards-incompatible changes require a version bump.
   - Additive changes should use extension mechanisms where possible.

---

## Testing Strategy (Gates)

Every PR must add tests proportional to the surface it changes.

### Required test layers
- Unit tests (`bitstream`, `wire`).
- Golden vectors for wire stability (`wire` + `codec` minimal fixtures).
- Property tests for round-trips:
  - full snapshot encode/decode round-trip
  - delta apply correctness: `apply_delta(baseline, encode_delta(baseline, current)) == current` (within quantization)
- Fuzzing targets for decoding:
  - packet framing decode
  - delta apply decode paths
- Chaos tests in `simbench` for loss/reorder recovery.

### “No compromise” checks
- `deny` unsafe in v0/v1.
- `clippy -D warnings`, `fmt`, `cargo test` in CI.
- Fuzz targets must compile in CI (running fuzz in CI can be periodic).

---

## Versioning

- `wire.version` is the on-the-wire compatibility contract.
- Crate semver is secondary to wire compatibility; still follow semver.
- Document wire version changes in `WIRE_FORMAT.md` changelog section.

---

## Extension Philosophy

We stay flexible by:
- keeping the header minimal,
- defining a small set of section types now,
- reserving extension points via optional sections and flags.

We do **not** include speculative fields “just in case.”
Everything in v0 is justified by immediate delta snapshot needs.
