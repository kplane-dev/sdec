# sdec-bevy

Bevy ECS integration for the SDEC snapshot + delta codec.

## What it provides

- Build a `schema::Schema` from Bevy components via `BevySchemaBuilder`.
- Extract per-tick creates/destroys/updates from a `World`.
- Apply decoded SDEC updates back into a Bevy `World`.
- Map Bevy `Entity` IDs to stable SDEC `EntityId` values.

## Typical flow

1. Define replicated components with `ReplicatedComponent`/`ReplicatedField`.
2. Build a schema with `BevySchemaBuilder`.
3. Call `extract_changes` (or `extract_changes_with_scratch`) each tick.
4. Encode deltas with `sdec-codec` and send over your transport.
5. Decode and apply on the client with `apply_changes`.

## Notes

This crate focuses on Bevy ECS integration only. Transport, packetization,
relevancy, and chunking are handled outside `sdec-bevy`.

For a working example, see `sdec-bevy-demo`.
