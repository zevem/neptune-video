# Analysis lane and motion tracking

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 03

## Scope

A bounded cancellable analysis worker and single-region tracking that produces
a tracker.

## Result

`crates/editor-media/src/analysis.rs`: one thread and one FFmpeg process, four
jobs waiting, newest-per-target replacement, coalesced progress, cancel.
Tracking is template matching of one luma patch (384 pixels a side at most) by
zero-mean normalized cross-correlation with sub-pixel refinement; the template
is re-taken under 0.8 and the path ends as lost under 0.5. A result is thinned
to 2048 keys and enters as one `SetTracker` after request, editor instance and
`SourceIdentity` are checked (`app::tracker_command`). The Motion section has
Track a region, Follow with offsets, Centre on point, Stop following, rename
and remove; the viewer draws the box and the paths.

## Evidence

`crates/editor-media/src/analysis/tests.rs`,
`src/ui/inspector/motion_tests.rs`, core follow tests. Native on lavapipe at
both sizes, 44 captures (session scratch, not under `artifacts/`). Measured:
125 keys within 1.8 source pixels on a generated 1280x720 fixture; about 220
to 240 % of one core while tracking.

The [verification matrix](../../../docs/verification.md#advanced-editing)
states the host, adapters and what is unverified on every platform.

## Known limits

- Translation only: `scale` is 1 and `rotation` 0 in every key. No detection,
  no occlusion recovery, forwards only, video only.
- The app tracks one region at a time although the lane queues four.
- Follow is position only in the interface. The tracked point is put where
  the tracked clip draws that place of its picture (`render::picture_point`),
  through a stabilized picture's reading too; over a stabilized clip the path
  overlay draws every key through the current frame's reading.
- Track is disabled where the clip plays backwards. A map that turns back
  below the playhead's source later in the clip is tracked only from the
  playhead's source upwards.
- A loss is said once in a notice; the tracker does not record it.
- A path that arrives while there is something to redo or a plan under review
  waits for Add tracker (or for the redo or plan to go) instead of editing.
- A held offset is not live when the follower's position is keyframed.
- Dropping a stale result is tested at `tracker_command` and the lane, not
  through a running app that replaces its project mid-job.
- Not tried on real footage or an optimized build. The final binary was not
  re-captured after formatting and a two-line lint fix.
- The spec says results carry "generation, request, asset and source
  fingerprint"; the code carries request, project, target, asset ID and
  `SourceIdentity`, and the app checks the editor instance.

## Comments

2026-10-13: Written by ticket 17 from the code as it stands and the
implementing tasks' reports. Verified on one Linux host only; macOS, Windows,
a hardware GPU in a windowed session, HiDPI and playback rates are unverified.
