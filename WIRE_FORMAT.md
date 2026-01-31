# Wire Format

This document specifies the on-the-wire packet format for the snapshot/delta codec.

Design goals:
- Minimal required fields for snapshot/delta replication.
- Safe decoding with explicit bounds and limits.
- Deterministic parsing.
- Versioned and evolvable.

This format is transport-agnostic (works over UDP, reliable channels, files, etc.).

---

## Definitions

- **tick**: The simulation tick the snapshot represents (u32).
- **baseline_tick**: The tick of the baseline snapshot used to encode a delta (u32).
- **schema_hash**: A deterministic 64-bit identifier of the replication schema.
- **entity**: An object with a stable `EntityId` within its lifetime.
- **component**: A group of fields replicated together under a `ComponentId`.

All integers are encoded in little-endian when byte-aligned.
Bit-level encoding is defined by the `bitstream` layer.

---

## Packet Overview

A packet is:

1) `Header` (byte-aligned)
2) `Payload` (bit-packed sections)

Packets are self-contained: they include enough metadata to validate schema compatibility and decode safely.

---

## Header (version 0)

The header is minimal and only includes fields we know we need now.

| Field          | Type  | Required | Description |
|----------------|-------|----------|-------------|
| `magic`        | u32   | yes      | Fixed constant to identify this protocol. |
| `version`      | u16   | yes      | Wire version. Version 0 uses `0`. |
| `flags`        | u16   | yes      | Packet kind flags (see below). |
| `schema_hash`  | u64   | yes      | Reject packet if mismatched. |
| `tick`         | u32   | yes      | Snapshot tick. |
| `baseline_tick`| u32   | yes      | For delta packets: baseline tick. For full snapshots: `0`. |
| `payload_len`  | u32   | yes      | Payload length in bytes following the header. |

### Header constants
- `magic`: chosen constant (set once; never change in versioned releases).
- `version`: starts at `0`.

### Flags (version 0)
Flags are a bitset:

- `FULL_SNAPSHOT` (bit 0): payload contains a full snapshot.
- `DELTA_SNAPSHOT` (bit 1): payload contains a delta snapshot.

Exactly one of `FULL_SNAPSHOT` or `DELTA_SNAPSHOT` MUST be set in version 0.

Reserved bits:
- bits 2..15 reserved for future use; MUST be zero in version 0.

### Payload length validation
`payload_len` MUST match the number of bytes following the header. Packets with
extra or missing payload bytes are invalid in version 0.

---

## Payload Structure

The payload is a sequence of **sections**. Each section is:

- `section_tag` (u8, byte-aligned)
- `section_len` (varuint, byte-aligned)
- `section_body` (`section_len` bytes)

Rationale:
- Byte-aligned section framing simplifies skipping unknown sections in future versions.
- Section bodies may contain bit-packed structures internally.
- `section_len` is a varuint capped at u32 (max 5 bytes). Overflow is invalid.

### Section tags (version 0)
Only the essential sections are defined.

| Tag | Name              | Present in FULL | Present in DELTA | Purpose |
|-----|-------------------|-----------------|------------------|---------|
| 1   | `ENTITY_CREATE`   | optional        | optional         | Spawn new entities with initial component state. |
| 2   | `ENTITY_DESTROY`  | optional        | optional         | Despawn entities. |
| 3   | `ENTITY_UPDATE`   | optional        | optional         | Update existing entities (delta within this packet). |

Notes:
- FULL snapshot can be represented as a set of creates + updates; however in the initial version we keep semantics simple:
  - FULL packets SHOULD include all entities either as creates or updates.
- DELTA packets include only changes since baseline (creates/destroys/updates).

Unknown section tags:
- In the initial version: decoder MAY reject unknown tags.
- In later versions: decoders SHOULD skip unknown tags using `section_len` for forward compatibility.

---

## Shared Types

### EntityId (version 0)
- `EntityId`: u32

### ComponentId (version 0)
- `ComponentId`: u16 (small integer id from schema)

### Field encoding
Fields are encoded according to the schema.
In the initial version, `schema` describes each field codec as:
- primitive kind (bool, int, fixed-point)
- bit width (if fixed)
- bounds + precision (if fixed-point)
- optional threshold for change emission (delta encoder only)

The wire does not embed field types; it relies on `schema_hash` and schema agreement.

---

## `ENTITY_CREATE` section (tag = 1)

Body:

- `count` (varuint)
- repeated `count` times:
  - `entity_id` (u32)
  - `type_id` (u16)  // optional in the initial version if single entity type; keep if needed now
  - `component_mask` (bitset, size = num_components in schema)
  - for each component present in `component_mask`:
    - `field_mask` (bitset, size = num_fields in component)
    - encoded field values for bits set in `field_mask`

Notes:
- For creates, `field_mask` SHOULD typically include all fields needed to initialize the entity.
- `type_id` is only included if the schema supports multiple entity types; if not needed initially, omit `type_id` entirely in v0 and treat all entities as a single type.

**Flexibility rule:** only include `type_id` if you truly need multi-type entities in the initial version.
If you don’t, drop it from the initial version to keep the format minimal.

---

## `ENTITY_DESTROY` section (tag = 2)

Body:
- `count` (varuint)
- repeated `count` times:
  - `entity_id` (u32)

Encoding:
- entity IDs MUST be unique within the section.

---

## `ENTITY_UPDATE` section (tag = 3)

Body:
- `count` (varuint)
- repeated `count` times:
  - `entity_id` (u32)
  - `component_mask` (bitset)
  - for each component present:
    - `field_mask` (bitset)
    - encoded values for fields set in `field_mask`

Notes:
- Updates are relative to the **reconstructed state** at the receiver for `baseline_tick` (delta) or the receiver’s cleared state (full snapshot semantics).
- Field masks enable sparse changes and avoid sending unchanged fields.

---

## Delta Semantics

For packets with `DELTA_SNAPSHOT` set:
- `baseline_tick` MUST be non-zero.
- Receiver must have a baseline snapshot for `baseline_tick`.
- If missing:
  - receiver should signal the caller (codec returns a typed error)
  - caller typically triggers a resync (full snapshot)

The initial version does not define ACK messages; ACKs are out-of-band and transport-specific.
The codec only consumes `baseline_tick` provided by the caller.

---

## Limits (Safety)

Decoders MUST enforce limits before iterating or allocating.

Recommended `CodecLimits` (initial defaults; tune per game):
- `max_packet_bytes`
- `max_sections`
- `max_entities_create`
- `max_entities_update`
- `max_entities_destroy`
- `max_components_per_entity`
- `max_fields_per_component`
- `max_bitset_bits` (derived from schema; still enforce)

If any limit is exceeded:
- decoding MUST fail with a structured error
- no partial state should be applied (atomic apply)

---

## Compatibility & Evolution

### Versioning
- `version` is the wire compatibility knob.
- Version 0 decoders reject packets where `version != 0`.

### Additive evolution strategy (preferred)
In v1+:
- allow unknown sections to be skipped (using `section_len`)
- add optional sections without breaking older decoders

### Breaking changes
- bump `version`
- keep `magic` stable

---

## Changelog

### Version 0
- Minimal header: magic, version, flags, schema_hash, tick, baseline_tick, payload_len
- Sectioned payload with create/destroy/update
- Schema-driven field encoding
- Required limits for bounded decode
