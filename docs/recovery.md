# Recovery and Failure Modes

This document describes how to recover when session or baseline expectations
are violated. The goal is explicit recovery signals and bounded errors.

## Session missing or mismatched

Symptoms:

- Decoder reports a session state error.
- Schema hash or header mode does not match.

Recovery:

1) Discard compact deltas.
2) Re-send a session init packet.
3) Send a full snapshot to re-establish baseline state.

## Baseline missing or mismatched

Symptoms:

- Decoder reports baseline tick missing.
- Delta applies fail for the requested baseline tick.

Recovery:

1) Request or send a full snapshot for the current tick.
2) Reset baseline ring if ticks diverged.

## Reordering and loss

Compact headers assume strictly increasing ticks. If packets are dropped or
reordered:

- Late packets should be ignored.
- Missing baselines should trigger a full snapshot + session recovery.

## Safety guarantees

- No panics on invalid input.
- Errors are structured and indicate recovery actions.
