# Let every playing layer finish an admitted preview

Status: ready-for-agent
Progress: resolved
Type: task

## Problem

Continuously advancing five video owners repeatedly superseded cold decoder
work. In the optimized R15 native window, two tiles stayed at frame zero for
all forty playing screen reads and caught up only after pause. The four-pipe
bank's overflow churn was confirmed by a matched private capacity experiment.

## Answer

Add explicit `request_playback_preview` admission: one unfinished pending or
in-flight request per owner, with `Busy` returned before admitted identity or
queue order changes. Preserve immediate `request_preview` for seeks and
source/bounds replacement. The viewer distinguishes clock advancement from
explicit jumps, edits, cuts and grant changes. Resource limits remain unchanged.

The [updated build](../../../artifacts/production-readiness/playback-admission-round1/build-record.json)
passes 538 workspace tests, including nine new backend controls and eight new
viewer controls. A real-video public comparison delivers advancing frames to
all five owners while requests continue, with exact pixels and final fences.
The [native rerun](../../../artifacts/native/playing-multilayer-round2/root-attempt-01/observation.json)
shows 21/21/21/17/17 distinct frame codes at forty reads, all five at frame 119
after pause, and unchanged project state. Thirteen existing native editing
checks also pass on the same Linux build.

This resolves the measured Linux liveness failure. It does not resolve the
[decoder throughput issue](15-decoder-retention-churn.md), establish 30 fps,
physical A/V or GPU acceptance, or establish macOS/Windows native behavior.

## Comments

Root preserved the behavioral admission RED, original native failure and
matched experiment. The viewer integration was implemented and the updated
native feedback harness was prepared.
