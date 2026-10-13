# Enforce the audible decoder budget independently of clip order

Status: ready-for-agent
Progress: resolved
Type: task

## Problem

The mixer checks its 16-voice budget only while opening a decoder, using the
number of clips visited so far. A new earlier-track clip can therefore open
decoder 17 before 16 retained later-track voices are visited. Conversely,
sequential clips within one chunk can trigger the limit despite not overlapping.

## Acceptance

- Reproduce both cases through the actual private Mixer with owned PCM files;
  preserve the failing log and prove a budget error occurs before source I/O.
- Admit clips by half-open output-sample intervals, preserving trim/rate/gain,
  zero-gain and mute rules, hidden-track sound and explicit linked ownership.
- Preflight peak audibility before decoder I/O with bounded reusable metadata.
  Retire and reap ended decoder children before spending replacement slots;
  enforce the actual live decoder budget independently of planning order.
- Verify sequential clips, a simultaneous 16-voice cut and retained-decoder
  continuity. Do not silently omit audible clips or increase concurrency.

The [read-only plan](../../../artifacts/production-readiness/audio-device-probe/voice-budget-plan.md)
records sample mapping, scratch bounds, test fixtures and Neptune's reservation
ownership reference. This fixes the existing preview policy; unlimited model
tracks do not imply an unlimited simultaneous decoding budget.

## Comments

2026-10-12: Scheduled PCM/generation corrections are a separate preceding
change. Source ownership transfers only after its owning media tests finish.
Root owns Cargo and actual FFmpeg validation; backend agents own source-only
changes. No frontend, native output or platform runtime proof is implied.

## Answer

Admission now counts half-open sample intervals before touching decoders. Its
reused bounded scratch rejects a seventeenth simultaneous voice before source
I/O. Ended decoder children are dropped and reaped before replacement; an
independent live-reservation guard still enforces 16. Continuing decoders retain
their ownership across a cut.

The original Mixer reproduced both failures with actual owned PCM files in
[the failing run](../../../artifacts/production-readiness/audio-device-probe/voice-budget-before-fix.log).
Five pure planner tests compare an independent per-frame oracle, track order,
boundaries, resource bounds and reuse. Four actual PCM regressions verify
seventeen sequential clips, retained-voice admission, a simultaneous 16-to-16
cut with owned child-handle auditing, and continuing decoder IDs/PCM. Their
[passing run](../../../artifacts/production-readiness/audio-device-probe/voice-budget-after-fix-audio-tests.log)
is included in the [333-test final checkpoint](../../../artifacts/production-readiness/final/round9-build-record.json).
All 34 media tests passed with FFmpeg 8.0.1; Linux and three cross-target
all-target Clippy checks passed with warnings denied. Cross checks establish
source validity, not native target output or linking. The 16-voice preview
policy remains explicit; this resolution makes no acoustic or A/V claim.
