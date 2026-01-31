# Protocol: Sessions and Compact Frames

This document summarizes the session handshake and compact header format.
For the full wire layout, see `WIRE_FORMAT.md`.

## Session init

Session init packets are regular sdec packets with `SESSION_INIT` set in the
v2 header flags. They carry the schema hash and header mode so both sides can
agree on how to decode subsequent snapshots.

Key properties:

- `SESSION_INIT` is exclusive (no full/delta flags set).
- Schema hash must match the local schema.
- The receiver must accept the session before processing compact deltas.

## Compact session header

Compact headers reduce per-packet overhead by using:

- 1 byte flags (full vs delta).
- Varint-encoded **tick delta** (relative to last tick).
- Varint-encoded **baseline delta** (relative to tick).
- Varint-encoded payload length.

This format is defined in `wire::session` and used by `codec::SessionState` to
enforce ordering and baseline constraints.

## Packed sparse indices

Packed sparse updates are used for per-client visibility deltas:

- Indices are encoded densely to avoid a bitmap per component.
- Update entries remain strictly ordered.
- Limits are enforced during encode/decode.

See `WIRE_FORMAT.md` for the exact sparse layout.
