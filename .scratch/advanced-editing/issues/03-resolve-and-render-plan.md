# `resolve` and `RenderPlan`

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 02

## Scope

One pure evaluator from project, time and drafts to the layers of the canvas.

## Result

`render::resolve(project, at, overrides) -> RenderPlan` in
`crates/editor-core/src/render.rs`. A `Layer` carries clip, track,
`LayerSource`, resolved `Transform`, blend, `ResolvedEffect`s with every
parameter present, masks, `ResolvedTransition`, `seed` and `time`.
`ClipOverride` carries transform, effects, masks, blend and animation drafts.
`Layer::is_plain` selects the mesh path and `RenderPlan: PartialEq` is the
compositor's dirty check. `tracked_point` and `tracked_clip` are the one
reading of a tracker path.

## Evidence

`crates/editor-core/tests/render.rs` (order, keyframe values, transition
pairing, follow, seed and time determinism, edges of time).

The [verification matrix](../../../docs/verification.md#advanced-editing)
states the host, adapters and what is unverified on every platform.

## Known limits

- `Layer.time` is not in the spec's struct listing; it was added for shake and
  glitch and is recorded in the ticket 08a comment.
- `ClipOverride` carries no time map and no transition, so a speed-curve drag
  and a transition handle drag are not previewed on the canvas until release.
- The plan's `source_time` is unsnapped; the viewer snaps to the asset's frame
  grid.
- The viewer pays a scan of each track's clips every painted frame; not
  measured on a large project.

## Comments

2026-10-13: Written by ticket 17 from the code as it stands and the
implementing tasks' reports. Verified on one Linux host only; macOS, Windows,
a hardware GPU in a windowed session, HiDPI and playback rates are unverified.
