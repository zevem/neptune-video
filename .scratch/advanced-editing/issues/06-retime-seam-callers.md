# Linear-time callers move to the retime seam

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 02

## Scope

Every `source_in + Δ·rate` consumer reads `Clip::source_time`, `source_span`
and `speed_at`.

## Result

Rules, viewer, timeline and audio use the seam. The viewer snaps a clip's
source time to the asset's frame grid with 18 µs of slack and, in a backwards
stretch, shows the source reached by the frame's end. Preview audio follows
gain keyframes and splits a clip's chunk at the map's segment boundaries: it
plays straight forward stretches and is silent on a freeze, a reverse and a
ramp. `rules::trim_delta` stops a trim at the furthest frame whose cropped map
validates.

## Evidence

Headless tests in `src/rules.rs`, the viewer test files and
`crates/editor-media/src/audio/`. Forward, freeze and reverse source frames
were read off a counter fixture on three adapters in wave 2.

The [verification matrix](../../../docs/verification.md#advanced-editing)
states the host, adapters and what is unverified on every platform.

## Known limits

- No reverse or ramped sound, and no pitch preservation.
- A linked partner with a different map can still make the core refuse a trim
  the gesture offered.
- GIF assets on retimed clips have no test.
- Many mapped clips against the 16-voice mixer limit were not checked.

## Comments

2026-10-13: Written by ticket 17 from the code as it stands and the
implementing tasks' reports. Verified on one Linux host only; macOS, Windows,
a hardware GPU in a windowed session, HiDPI and playback rates are unverified.
