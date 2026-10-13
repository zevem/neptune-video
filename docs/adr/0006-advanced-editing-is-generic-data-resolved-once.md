---
status: accepted
---

# Advanced editing is generic durable data resolved once

Keyframes, effects, looks, masks, blend modes, transitions, speed changes,
tracking, stabilization and background removal are added to the clip as a few
generic, strictly validated values, read by one pure evaluator and drawn by one
compositor. The existing contracts stay as they were: every edit is a
validated reversible command, the core knows no GUI, codec or filesystem, and
workers are bounded and stale-safe.

**Generic tables.** An effect is `{kind, params}` with one flat `EffectKind`
and one flat `ParamKey`; [`look::spec`](../../crates/editor-core/src/look.rs)
is the single table that validates a parameter, fills its default, builds the
inspector row and documents the agent surface. A look preset expands into
ordinary effects and a clip's look is read back from them, so a look is not a
stored type. A struct per effect was set aside: twenty kinds would have meant
twenty schemas, twenty inspector panels and twenty keyframe mechanisms, and
the provider schema has a 24 KiB cap.

**Clip-relative animation.** A keyframe's `at` is integer microseconds from
its clip's start, and a tracker's keys are in source time. Moving, rippling or
pasting a clip touches neither; split, a start trim and a rate change rebase
keys in the reducer. Timeline-absolute keyframes were set aside because every
move would have had to rewrite them, and clip-to-clip references (a transition
object naming two clips, a follow naming a clip) because delete, split and
paste would each need referential cleanup. A transition belongs to the clip it
brings in or takes out and finds its partner when evaluated.

**Time maps.** Variable speed is an optional `Retime`: 2 to 64 keys mapping
clip time to source offset, where equal sources hold a frame, falling sources
reverse and an eased segment ramps. A clip without one behaves as before
through `playback_rate`. Every consumer reads `Clip::source_time`,
`source_span` and `speed_at`. Separate freeze, reverse and ramp fields were
set aside: they compose badly and each consumer would have had to combine
them again.

**One pure resolve.** `editor_core::resolve(project, time, overrides)` returns
a `RenderPlan` of layers, bottom first. It reads no clock: grain is seeded
from the clip and canvas frame, and shake and glitch move by the layer's
frame-snapped time, so a paused viewer has nothing to repaint and equal plans
mean nothing to draw. The plan is the headless test oracle and the seam an
export renderer would consume. Evaluating in the viewer was set aside: it
would have put domain rules in the desktop crate and left them untestable
without a window.

**GPU compositor, mesh fallback.** A plan whose layers are all plain is drawn
as before, one quad a layer. Any other run of layers is composited off screen
by [`src/compose`](../../src/compose/mod.rs) in linear light and published as
one canvas texture. Pixel math exists once, in WGSL, and is held by device
readback tests that print `SKIP:` when no adapter exists. Without a device, or
when the device refuses, the mesh path draws transform and opacity and the
viewer says "Effects not shown". A CPU reference renderer was set aside as a
second implementation of every formula; FFmpeg filter graphs because they
cannot follow a drag at interactive latency and would tie the pixels to one
codec tool; egui paint callbacks into the window's own pass because blend
modes and multi-pass effects need intermediate targets.

**Derived analysis data stays outside the project.** A stabilization path is
regenerable from the media, so it lives in a bounded cache under the user's
cache directory, stamped with the file's size, modification time and probe
facts and with the algorithm version; the project holds only the `Stabilize`
effect and its two parameters. A tracker's path is different: it is small
(at most 2048 keys), the person can rename, remove and follow it, and it must
survive without the media, so it is project data entering through
`SetTracker`. Storing stabilization paths in the project was set aside: four
numbers a frame for every analysed video would grow every save, every history
entry and every agent context with data that is not an editing decision.

**Strict validation.** `Project::validate` holds every bound and rejects what
the renderer cannot draw: effects, masks and blend on text, looks on sound,
a parameter foreign to its kind, a ramp outside 0.05 to 16 times. Loosening a
rule later invalidates no saved file; tightening one would. The file is schema
version 2; version 1 is read through serde defaults and never written, so an
older build refuses the file through its future-schema protection instead of
saving it back without the new data.

The binding rules, and the dated changes made to them during implementation,
are in [the advanced-editing design](../../.scratch/advanced-editing/spec.md).
Everything here is preview at preview resolution: no export path exists, and
[verification](../verification.md#advanced-editing) lists what was measured
and on which host.
