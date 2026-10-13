# ADR 0006, glossary, usage and verification docs

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 01, 02, 03, 04, 05, 06, 07, 08, 09, 10, 11, 12, 13, 14, 15, 16

## Scope

Document the epic as built, checked against the code.

## Result

Written or updated:
`docs/adr/0006-advanced-editing-is-generic-data-resolved-once.md`,
`GLOSSARY.md`, `docs/usage.md`, `docs/performance.md`, `docs/architecture.md`,
`docs/design.md`, `docs/agent-protocol.md`, `docs/native-ai.md`,
`docs/verification.md`, one issue file a ticket and the research answer for
ticket 16.

## Evidence

`python3 scripts/check-architecture.py` passes. No Rust source changed and no
build was run for this ticket; figures quoted are those the implementing
tickets measured.

The [verification matrix](../../../docs/verification.md#advanced-editing)
states the host, adapters and what is unverified on every platform.

## Known limits

- Test counts and pass results are quoted from the implementing tasks' runs,
  not re-run here.
- Wave 4 native captures live in the session's scratch directory and are not
  retained under `artifacts/`.
- Places where the spec's body and the code differ are listed in the spec's
  Comments entry for this ticket.

## Comments

2026-10-13: Written by ticket 17 from the code as it stands and the
implementing tasks' reports. Verified on one Linux host only; macOS, Windows,
a hardware GPU in a windowed session, HiDPI and playback rates are unverified.
