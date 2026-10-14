---
status: accepted
---

# Export renders originals through the preview compositor

Export turns the timeline, or a range of it, into a file whose picture is the
preview's composition drawn from the original media at the chosen size, with
the timeline's sound. Jobs wait in a render queue and render one at a time in
the background, where they can be paused, cancelled and retried. The preview
gains a quality setting and proxies for large media. The existing contracts
stay as they were: every edit is a validated reversible command, the core
knows no GUI, codec or filesystem, and workers are bounded and stale-safe.

**Export is not an edit.** A job holds an immutable `Arc<Project>` snapshot
taken when it is queued, after drafts are committed, with the project
identity, the editing context (`Editor::instance_id`) and the revision. Later
edits never change a queued job, and a retry renders the same snapshot again.
There is no `Command` for it: agents get non-mutating `AgentOperation`s that
answer at once with a job, never waiting for the render. Export as a command
was set aside: it changes no project state, so there would be nothing to undo,
and a history entry that names a file outside the project would make undo
mean something it cannot do.

**One picture path: the compositor, from the plan.** Every output frame is
`editor_core::resolve(render_project, frame_time(n), overrides)` drawn by
[`src/compose`](../../src/compose/mod.rs) at the output size. A
`FrameRenderer` owns its own compositor device state, sized from the job and
never stepped down by the viewer's budget. It draws every layer, plain or
not, onto one accumulator in linear light, starting from the canvas colour.
Titles and graphics are drawn by the same `editor-media::text` renderer at the
export's scale and laid over with Normal blending. A composite run that
starts above a title is drawn on a transparent canvas of its own and laid
over, so it blends only with the media between it and the title, as in the
viewer. FFmpeg receives opaque RGBA on its standard input and only converts
it to Y'CbCr (BT.709, limited range) and encodes it. Three alternatives were
set aside:

- composing in an FFmpeg filter graph, for the reasons
  [ADR 0006](0006-advanced-editing-is-generic-data-resolved-once.md) gives:
  a second implementation of every formula, tied to one codec tool;
- a CPU renderer, a third implementation of the same pixel math;
- reproducing the viewer's mesh path for plain layers. The mesh path blends in
  gamma space and exists for windows without a graphics device. Matching it
  would have meant a second blend for the file, so the export blends plain
  layers in linear light and the difference stays a recorded limit.

**Originals only.** The export reads each asset's own path from the snapshot.
The export modules have no access to the proxy index, and a proxy is never
written into a `Project`. Proxies in export were set aside: an export would
then be the approximation the preview showed. A missing or damaged proxy
must not change a file either.

**The preview's source frames.** `editor_core::render::source_frame`, moved
out of the viewer, picks the source frame for both. The export's
`SourceReader` decodes with the preview's seek and frame-grid arguments and
a better scaler. It decodes at the size a clip needs at its largest on the
output, never above the source and never above 8192 pixels a side.

**Exact timing.** Output frame `n` is `rate.frame_time(first + n)`, and a
range is snapped outwards to whole output frames. The sound covers exactly the
same span, `sample_count(start, end, rate)` sample frames. It is rendered by
the playback mixer run offline (`OfflineMix`), so cuts, gain, mutes and
rates sound as they do in playback. It goes to a raw `f32le` file owned by
the job, then into the file beside the picture, both starting at zero.

**The person's FFmpeg, probed per host.** Encoding is the external FFmpeg the
preview already uses. At start, every encoder Aster knows how to drive that
FFmpeg lists is asked to encode a test picture or a moment of silence through
the same argument builder an export uses. Only encoders that succeeded, and
containers whose muxer exists, are offered. The others are shown with the
reason they failed. `EncoderChoice::Auto` takes the best working hardware
encoder that accepts the output size, otherwise software. A hardware encode
that fails under `Auto` runs once more from the first frame with the software
encoder, and the job says so; an explicitly chosen hardware encoder fails
with the reason. Bundling an encoder library was set aside with codec
packaging in general (see
[the codec and distribution ticket](../../.scratch/core-editor/issues/03-codec-and-distribution.md)).

**A file is never half-written.** FFmpeg writes a hidden part file,
`.<name>.<job>.part`, in the destination's folder. Only after FFmpeg exited
successfully and `ffprobe` found the expected streams and duration is the
part file synced and renamed over the destination. A caption file asked for
beside it is written the same way, and replacing an existing one is asked
about like the file itself. Cancel, failure or a panic removes the part files
and the job's working folder, which is hidden beside the destination rather
than in a temporary folder that may be held in memory.

**One render at a time.** The render queue runs jobs in list order on one
`aster-render` thread. Pause stops feeding frames, so FFmpeg waits on its
pipe and no signal is sent; resume continues the same encode. Cancel kills
the job's own processes. Running jobs side by side was set aside: they would
share one GPU and disk, FFmpeg already uses several threads for one encode,
and each job would multiply the decode pipes and frames in flight that the
bounds hold.

**Settings live outside the project.** Built-in presets are code
(`editor_core::export::builtin_presets`). The person's saved presets, the
last settings and folder used, and the preview settings live beside the
editor's settings, read leniently so that a value this build does not know
falls back alone. Export settings stored in the project were set aside:
encoder choices depend on the computer, the same project is exported several
ways, and changing an export setting would otherwise become an edit with
history.

**Proxies and preview quality belong to the preview.** A proxy is derived
data, like a stabilization path. It is a short-GOP H.264 copy (MJPEG when
libx264 is missing) without sound, at most 1280 pixels on the long side, with
the original's aspect, rate and frames. Proxies are kept in a bounded cache
keyed by the source's identity and the proxy recipe. Which proxies exist is
window and worker state. The viewer reads a proxy only for pictures, only
when it matches the file as it is now and has at least the pixels the layer
is drawn at. Preview quality (Auto, Full, Half, Quarter) divides the
viewer's decode boxes and compositor canvas. Auto steps to half only after
frames were measured late during playback, and stays there until playback
stops.

**Consequences.**

- An export needs a graphics device: the window's, or one it opens itself,
  which may be a software adapter. Without one the job fails.
- A half-transparent plain layer is a slightly different shade in an export
  (linear light) than in a viewer that draws it with the mesh path (gamma
  space). Composited runs match within ±2 levels in the parity test.
- Titles are drawn at most 4096 pixels a side, so a very large title on an
  8K output is softer than the output.
- Export sound has the playback mixer's limits: 16 simultaneous audible
  clips, pitch that changes with speed, silence on frozen, reversed and ramped
  stretches, and sound from hidden video tracks.
- Which codecs and hardware encoders a person gets depends on their FFmpeg
  build and drivers. H.264 and H.265 are patent-encumbered formats; Aster
  bundles no encoder.
- The headless `aster-agent` binary answers render operations `unsupported`.
  Render jobs run in the desktop window.
- This supersedes the statements in
  [ADR 0006](0006-advanced-editing-is-generic-data-resolved-once.md) that no
  export path exists and that the plan is the seam an export renderer "would"
  consume, and the consequence in
  [ADR 0007](0007-titles-and-captions-are-clips-drawn-off-the-frame.md) that
  captions cannot be burned into a file. Those records are left as written.

The binding rules and their dated changes are in
[the export and rendering design](../../.scratch/export-and-rendering/spec.md).
What was measured, and on which host, is in
[performance](../performance.md#export-and-proxy-budgets); native
verification of export on Linux is in
[verification](../verification.md#export-and-rendering) (ticket 12 of
[the export and rendering tracker](../../.scratch/export-and-rendering/issues/12-native-verification.md)).
