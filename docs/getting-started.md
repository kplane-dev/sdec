# Getting Started

This guide explains how to adopt sdec in two modes (global vs per-client) and
how the session protocol fits into production workflows.

## Concepts

### Global vs per-client replication

- **Global mode**: One stream of snapshots/deltas sent to all clients.
  - Use when every client sees the same world state (or you can tolerate
    over-sending).
- **Per-client mode**: Encode deltas per client based on their visibility.
  - Use when visibility is different per client or when bandwidth is tight.

### When to use session mode

Session mode is the preferred way to ship per-client deltas. It adds:

- **Session init** for schema/hash negotiation.
- **Compact headers** with tick/baseline deltas (lower overhead).
- **Strict validation** to catch schema/tick/baseline errors early.

Use session mode when:

- The server must handle reconnects or version mismatches safely.
- You want compact per-client deltas with predictable recovery behavior.

## Quick Start (per-client session mode)

1) **Build schema once** per server version.

2) **Send a session init packet** to each client:
   - Includes schema hash + header mode.
   - Client must ACK the session before accepting compact deltas.

3) **Send a full snapshot** for the first tick (baseline).

4) **Send compact delta snapshots** per client:
   - Use packed sparse indices for updates.
   - Encode against the last ACKed baseline for that client.

### Using `sdec-repgraph` for per-client deltas

The `sdec-repgraph` crate handles interest management and emits
`creates/destroys/updates` that feed directly into
`codec::encode_delta_from_changes`.

Typical flow per tick:

1) Update entity positions + dirty components in the `ReplicationGraph`.
2) For each client, call `build_client_delta(...)`.
3) Encode the delta with `encode_delta_from_changes`.
4) After all clients are processed, call `clear_dirty()` and `clear_removed()`.

5) **Track ACKs** per client:
   - ACKs advance the baseline tick.
   - Missing ACKs mean you must fall back to full snapshots or re-init.

## Baseline / ACK expectations

- The decoder expects baseline ticks to be present in its baseline ring.
- If the baseline is missing or mismatched, decoding fails with a structured
  error (no panic).
- Recovery is explicit: re-send session init or a full snapshot as needed.

## Capture inspection

The demo tooling writes capture artifacts you can inspect:

- `captures/schema.json`
- `captures/session_init.bin`
- `captures/delta_*.bin`
- `captures/summary.json`

Use `sdec-tools` to inspect or decode captures:

```
cargo run -p tools -- inspect captures/delta_000002_base_000001.bin
```

## Next steps

- See `docs/protocol.md` for the session init + compact header layout.
- See `docs/recovery.md` for baseline/session miss behavior and recovery.
