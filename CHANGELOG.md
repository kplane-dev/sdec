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

