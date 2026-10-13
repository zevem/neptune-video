# Keyframe lane and graph editor

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 09

## Scope

Edit a clip's keyframes in the timeline.

## Result

`src/ui/keyframes.rs`: a lane of one row an animation track under the clip's
track (U, the tile's chevron or the tool strip), and a graph editor in the
tracks' place for one selected clip (G). Keys move on the frame grid, snap to
the playhead, stay inside the clip and replace a key they land on; values are
held to `look::param_spec`. Handles are the control points of `Ease::Bezier`.
Every gesture is one batch of `SetKeyframes`; a held one is drawn through
`ClipOverride.animation`. Arrows, Delete and the clipboard chords act on
selected keys.

## Evidence

`src/ui/keyframes_tests.rs`. Native at 1440x900 on lavapipe: move, delete,
value drag, easing change, handle drag (wave 3).

The [verification matrix](../../../docs/verification.md#advanced-editing)
states the host, adapters and what is unverified on every platform.

## Known limits

- The time map is not drawn as a curve; the Speed section edits it.
- Marquee, shift-add, cut, U, value nudges, handle arrow keys, the Shift axis
  lock and paste refusals are headless only.
- Accessible names were checked through accesskit in the bench, not with a
  screen reader.
- No measurement with many keys.

## Comments

2026-10-13: Written by ticket 17 from the code as it stands and the
implementing tasks' reports. Verified on one Linux host only; macOS, Windows,
a hardware GPU in a windowed session, HiDPI and playback rates are unverified.
