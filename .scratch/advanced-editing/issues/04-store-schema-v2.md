# Store schema v2 with v1 read

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 01

## Scope

Persist the new data without stripping it in an older build.

## Result

`SCHEMA_VERSION` is 2 in `crates/editor-store/src/lib.rs`; version 1 is read
as a strict subset and never written. Project files, checkpoints and the
recovery archive round-trip every new field; the archive stores each tracker
once however many history states share it.

## Evidence

`crates/editor-store/tests/schema.rs` (v1 project, checkpoint and recovery
load and the next save writes 2; version 3 refused and protected; version 0
unsupported; full round trips).

The [verification matrix](../../../docs/verification.md#advanced-editing)
states the host, adapters and what is unverified on every platform.

## Known limits

- No build that knows only version 1 was run against a version 2 file. The
  refusal is shown for version 3 on this build through the same comparison.
- Stabilization data is not in the document or the recovery archive.

## Comments

2026-10-13: Written by ticket 17 from the code as it stands and the
implementing tasks' reports. Verified on one Linux host only; macOS, Windows,
a hardware GPU in a windowed session, HiDPI and playback rates are unverified.
