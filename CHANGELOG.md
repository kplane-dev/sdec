# Changelog

All notable changes to this project will be documented in this file.
This project follows Semantic Versioning for pre-1.0 releases.

## [0.1.0] - 2026-01-31

### Added
- Workspace layout with `bitstream`, `wire`, `schema`, `codec`, `tools`, and `simbench`.
- CI checks for formatting, clippy, tests, docs, and publish dry-run.
- Release-gated publishing workflow for crates.io.
- Initial docs: architecture and wire format specification.

## [0.2.0] - 2026-01-31

### Added
- Bitstream bounded writer/reader, varints, alignment helpers, and tests.
- Wire framing with section slicing, limits enforcement, and header validation.
- Schema runtime model with integer-based codecs, validation, and deterministic hashing.
- Full snapshot encode/decode with strict section framing and golden fixtures.
- Baseline ring buffer keyed by tick with eviction behavior and lookup helpers.

## [0.3.0] - 2026-01-31

### Added
- Delta snapshot encoding/decoding with create/destroy/update sections and masks.
- Baseline tick validation and strict section ordering for delta apply.
- Codec scratch buffers for allocation-free hot paths and scratch reuse tests.

### Changed
- Delta encoding now streams entity ops without temporary allocations.
- Update component diffing uses bounded scratch masks and limit checks before growth.

## [0.4.0] - 2026-01-31

### Added
- `sdec-tools` CLI with `inspect` and `decode` commands and JSON decode output.
- Schema JSON support via optional serde feature.
- Delta packet decode helper for tools (`decode_delta_packet`).

### Changed
- Tools crate now includes structured inspect/decode report builders and tests.

## [0.6.1] - 2026-01-31

### Added
- Session fuzz target and chaos tests for session ordering/baseline handling.
- CI smoke fuzz run for `session_packet` on nightly.

### Changed
- Bump workspace and internal crate versions to 0.6.1.

## [0.6.0] - 2026-01-31

### Added
- On-wire session init packets for schema/hash negotiation and compact header mode.
- Compact session header and per-client delta encoding/decoding path.
- Session state validation for schema/tick/baseline rules.

### Changed
- Bump workspace and internal crate versions to 0.6.0.

## [0.5.0] - 2026-01-31

### Added
- `demo-schema` and `demo-sim` reference crates for deterministic captures.
- Capture output (`schema.json`, `full_*.bin`, `delta_*.bin`, `summary.json`).
- CI demo simulation run with size budget checks.
- Simbench harness with baseline encoders and `summary.json` output.

