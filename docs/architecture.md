# Architecture

Aster is a native Rust desktop editor. Its desktop stack follows Neptune's
`eframe`/`egui` and `wgpu` approach, keeping the domain model independent of the
window system. Native platform acceptance and playback throughput require
the evidence in
[verification](verification.md).

## Ownership

| Component | Owns | Boundary |
| --- | --- | --- |
| `editor-core` | Project settings, assets, folders, tracks, clips, markers, effects, masks, animation tracks, time maps, transitions, trackers, title styles, text animations, word times, graphics, caption languages, SubRip/WebVTT text, edit validation, history, frame timing, the pure `resolve` to a render plan, and export data: settings, formats, presets, capability data, validation into an `ExportPlan`, output frame and sample spans, output geometry and render job reports | No windows, filesystem, font files, decoder handles, GPU or live workers |
| `editor-store` | Versioned documents, protected document sessions, atomic replacement, autosave and recovery bindings | Immutable project/history snapshots and explicit destination ownership in, load/save/conflict results out |
| `editor-media` | File/folder import, source probing, image/GIF/video decoding, cached frames, PCM audio mixing, `.cube` table reading, the analysis lane (region tracking, camera-motion measurement and its on-disk cache), the text renderer and title lane (fonts, shaping, title pictures), the speech lane (model downloads, offline 16 kHz mix, whisper.cpp), export encoding (encoder probe, FFmpeg arguments, the stdin-fed encoder, output check), the export source reader, the offline sound render for export, and the proxy lane and cache | Owned metadata, frame/audio results, title pictures, words, analysis results and proxy answers carrying project, request and source identity; codec, font and live worker details remain private. The export calls block and run on a thread the desktop owns |
| Desktop composition | Controller integration, worker scheduling, clipboard, file dialogs, input, playback clock, title textures, the preset file and subtitle file reads and writes, preview quality and which proxies the viewer reads | Converts UI actions and completions into targeted commands |
| Render queue (`src/export`) | `RenderQueue`, the `aster-render` thread, the job runner and `FrameRenderer` | Takes immutable project snapshots; publishes job reports and finished events; never edits a project |
| Compositor (`src/compose`) | Off-screen wgpu passes, WGSL pixel math, `ComposeBudget`, lookup-table textures, the layer assembly the viewer and export share, readback | Draws a render plan's non-plain runs into one canvas texture while the interface is drawn, never asking for a repaint and falling back to the mesh path; draws every layer of an export frame on the render thread at the output size, with no fallback |
| Native UI | Media library with text, captions, effects, transitions and looks tabs, preview, timeline with keyframe lane and graph editor, sectioned inspector, export dialog, render queue panel, dialogs and accessible controls | Draws prepared state and emits actions |
| `editor-agent` | Command/schema discovery, guarded local transport, native provider processes, captured planning context, edit proposals and the render job operations | Providers return data; reviewed application uses the same validated reducer as the UI |

The root desktop package is `aster-editor`, with `aster` and `aster-agent`
binaries. Consult the manifests and
[backend contract](../.scratch/core-editor/backend-contract.md) for current APIs.
The [user guide](usage.md) and [verification record](verification.md) distinguish
implemented features, runtime limits and accepted observations.

## Editing contract

Every durable mutation enters through a typed, serializable editing command.
Commands address stable project, asset, track and clip identities rather than
the current selection. Validate the entire command before committing it; a
failure leaves the project and history unchanged. A batch of agent edits must
either succeed as one reversible history entry or leave no partial changes.

The core validates track compatibility, asset references, nonnegative source
ranges, positive durations, supported canvas settings and finite transform
values. A locked track blocks modifications to its clips from every entry
point. Grouping and linkage are explicit relationships; changes to one linked
clip must not silently move unrelated clips.

History records successful commands and the state needed to reverse them.
Undo/redo is an editing transition. A new edit after undo abandons the old redo
branch. The recoverable document must preserve the history contract across
save/load; named checkpoints are part of that contract. Memory and storage
policy may compact history with explicit retained checkpoints, but must not
silently invalidate advertised recovery depth.

Selection, hovered controls, timeline zoom, playhead motion, decoder handles
and pending requests are ephemeral. They do not become edits or inflate project
history. Agent/provider credentials also stay outside project documents.

Each live `Editor` has a transient context identity renewed on construction or
recovery. Agent mutations require that context, the durable project identity
and expected revision. Reopening the same project at a reset revision therefore
cannot authorize an older editing plan. Recovery also rejects history roots
belonging to another project identity.

## Native AI boundary

Native planning belongs to `editor-agent`, independent of the desktop drawing
loop. `AiContext::capture` retains an immutable project snapshot, the live
context identity, revision, selected clip IDs and playhead. `AiRuntime` owns one
planning worker and one active request, returning bounded typed events through
nonblocking polling. Claude print/structured-output and Codex app-server drivers
reuse the person's CLI sign-in and expose no credential fields in `AiConfig`.

```text
captured editing context + request → bounded metadata → owned provider process
    → closed typed proposal → one validated batch / PreparedEdit → review
    → context/project/revision/root recheck → exact reviewed state
    → changed: one undo entry; unchanged: preserve state/history
```

Planning sends project metadata, including source paths/names and text clips;
it sends no source media bytes and provides no media-analysis tools. Large
documents use a declared focused view that preserves complete selected
link/group relationships and source limits within 1 MiB. An incomplete view
must never be interpreted as an empty track or library. Provider tools, search,
MCP and native execution are disabled; unexpected execution or approval events
fail the request. Proposal validation rejects whole-project replacement and
new source-asset creation, although those remain available to trusted external
clients through the full command protocol.

Review does not mutate the live editor. A valid proposal contains full
before/after snapshots; applying it rechecks all captured identities. A changed
batch commits one reversible history entry; a no-op preserves revision, dirty
state, root and redo and reports "No changes". A stale, cancelled, failed or invalid request cannot
authorize an edit. See [native AI](native-ai.md) for exact APIs, budgets,
provider evidence and remaining native acceptance, and
[ADR 0004](adr/0004-native-ai-produces-reviewed-edit-proposals.md) for this
boundary's rationale.

## Timing and composition

Use integer microseconds and a rational project frame rate at domain boundaries.
Floating-point seconds are suitable for presentation, not persisted clip
endpoints. Frame stepping computes the frame boundary directly from its index
so rates such as 30000/1001 do not accumulate rounded-frame drift. Saturating
integer arithmetic and model range validation protect clip endpoints from
wrapping into unrelated timeline positions.

Treat intervals as half-open: a clip contributes from its start up to, but not
including, its end. Splitting at an interior boundary retains source continuity;
a split at an endpoint is invalid. Playback position and source position are
different concepts. A preview speed change affects the playback clock; source
retiming requires its own explicit editing command.

The composition path is:

```text
project + playhead + drafts → resolve → render plan (layers, bottom first)
       → media requests → owned decoded frames
       → plain layers: one quad each
         other runs:   off-screen compositor → one canvas texture
       → egui/wgpu → native window
```

`editor_core::resolve` is the one evaluator: it samples animation tracks,
applies follow, pairs transitions and drops disabled effects, and it reads no
clock. The viewer calls it once a painted frame and requests media from the
plan's layers. See [advanced editing](#advanced-editing).

The media runtime exposes an optional audio-device clock after initial buffering.
Desktop playback follows it when audio output is available, with a monotonic
fallback when silent. Actual device synchronization and display latency still
need native measurements; pitch-preserving speed changes and hardware decoding
remain future work.

## Advanced editing

[ADR 0006](adr/0006-advanced-editing-is-generic-data-resolved-once.md) records
the decision; [the design](../.scratch/advanced-editing/spec.md) holds the
binding rules and their dated changes. This section says where things live.

| Concern | Owner |
| --- | --- |
| `ParamKey`, `Ease`, `Keyframe`, `AnimTrack` | [animate.rs](../crates/editor-core/src/animate.rs) |
| `EffectKind`, `Effect`, the `spec` tables, `Mask`, `BlendMode`, `Transition`, `Tracker`, `Follow`, `LookPreset` and the look read back from a clip's effects | [look.rs](../crates/editor-core/src/look.rs) |
| `Retime`, `TimeKey` and the map's crop, rescale and `shows` | [retime.rs](../crates/editor-core/src/retime.rs) |
| `resolve`, `RenderPlan`, `Layer`, `ClipOverride`, `tracked_point` | [render.rs](../crates/editor-core/src/render.rs) |
| Bounds for all of it | `Project::validate` in [model.rs](../crates/editor-core/src/model.rs) |
| Split, trim, rate and retime rebasing of keys | The reducer in [editor.rs](../crates/editor-core/src/editor.rs) |
| Speed curve model over a time map; freeze and reverse commands | [src/speed.rs](../src/speed.rs) |
| Compositor: runs, ledger, budget; device passes; pass tables; lookup tables | [compose/mod.rs](../src/compose/mod.rs), [gpu.rs](../src/compose/gpu.rs), [pass.rs](../src/compose/pass.rs), [lut.rs](../src/compose/lut.rs) and `compose/shaders/*.wgsl` |
| Tracking and camera-motion analysis, the stabilization cache | [analysis.rs](../crates/editor-media/src/analysis.rs), [analysis/steady.rs](../crates/editor-media/src/analysis/steady.rs) |
| Stabilization paths the window holds and their staleness | [src/steady.rs](../src/steady.rs) |
| Inspector sections | [src/ui/inspector/](../src/ui/inspector) (`clip`, `look`, `masks`, `color`, `curve`, `speed`, `motion`, `transition`) |
| Keyframe lane and graph editor; transition markings; effects, transitions and looks tabs | [keyframes.rs](../src/ui/keyframes.rs), [transitions.rs](../src/ui/transitions.rs), [browser.rs](../src/ui/browser.rs) |
| Command-panel phrases | [src/agent.rs](../src/agent.rs) |

The commands are whole-value setters (`SetEffects`, `SetMasks`, `SetBlendMode`,
`SetKeyframes`, `SetRetime`, `SetTransition`, `ApplyPreset`, `SetTracker`,
`RemoveTracker`, `SetFollow`); snapshot history reverses them without
inverses, and all reach clips through the path that refuses a locked track.
`SetRetime` also moves a clip's keyframes to where the new map shows the
source moment each was on. Existing commands gained rebasing: a split
partitions animation and time-map keys at the cut, a start trim rebases them,
and a rate change rescales them.

A draft never becomes project state. The inspector, the viewer's handles and
the keyframe editor hand `resolve` a `ClipOverride` (transform, effects,
masks, blend or animation tracks with the held keyframe written) and commit
one command, or one batch, when the gesture ends. A time-map draft is the
exception: `ClipOverride` carries no map, so a speed-curve drag is shown in
the graph and the duration only until release.

A title is not composited: it is drawn from its title-lane picture as one
quad and parts the plan into runs, each composited on a canvas of its own. A blended layer above a
title therefore blends only with the media between the two. The mesh path
blends in gamma space and the compositor in linear light, so a
half-transparent layer changes shade when its plan stops being plain. Both
are known limits, not accepted design.

Analysis answers enter like any other edit or not at all. A tracking result
becomes one `SetTracker` through `Action → App → Editor::apply` after its
request, editor instance and the media's path, size, duration and dimensions
have been checked; a stale answer is dropped and changed media is reported.
Nobody pressed for that edit, so a path that arrives over a redo stack or an
open plan waits in the Motion section until the person adds it or neither is
left. A followed point is placed by `render::picture_point` through the
tracked clip's own transform; how a stabilized layer reads its picture is not
project data, so the viewer hands it to `resolve` as `ClipOverride.reading`
and resolves a second time when a follower and a steadied layer share a frame.
A stabilization result is never an edit: the path is kept in the cache and in
the window's bounded table, and the project holds only the `Stabilize` effect.

## Titles, captions and speech

[ADR 0007](adr/0007-titles-and-captions-are-clips-drawn-off-the-frame.md)
records the decision; [the design](../.scratch/titles-and-captions/spec.md)
holds the binding rules. This section says where things live.

| Concern | Owner |
| --- | --- |
| `TextStyle`, `TextAnimation`, `WordTime`, `Element`, the word count, realignment and the spread of a progress over units | [title.rs](../crates/editor-core/src/title.rs) |
| SubRip and WebVTT parsing and formatting, cue grouping of spoken words | [captions.rs](../crates/editor-core/src/captions.rs) |
| `SetText`, `SetTextStyle`, `SetTextAnimation`, `SetWords`, `SetGraphic`, `SetTrackLanguage`, `CopyCaptions`; split, trim and rate rebasing of words and animations | The reducer in [editor.rs](../crates/editor-core/src/editor.rs) |
| Motions and karaoke word state in the plan (`LayerSource::Text`, `LayerSource::Graphic`) | [render.rs](../crates/editor-core/src/render.rs) |
| Font book, shaping, layout, the picture, graphic elements | [editor-media/src/text](../crates/editor-media/src/text) |
| The title lane (`aster-titles`) | [text/lane.rs](../crates/editor-media/src/text/lane.rs) |
| Model catalog, downloads through `curl`, the offline mix, whisper.cpp and the speech lane (`aster-speech`) | [editor-media/src/speech.rs](../crates/editor-media/src/speech.rs) |
| Title textures the window holds | [src/titles.rs](../src/titles.rs) |
| Speech choices, the one command captions become, and when they wait | [src/speech.rs](../src/speech.rs) |
| Kept looks and brand colours | [src/presets.rs](../src/presets.rs) |
| Language names and tags | [src/languages.rs](../src/languages.rs) |
| Inspector Text, Graphic and Animate sections | [src/ui/inspector/text.rs](../src/ui/inspector/text.rs) |
| Library Text tab and its built-in catalog | [src/ui/text_tab.rs](../src/ui/text_tab.rs), [text_tab/catalog.rs](../src/ui/text_tab/catalog.rs) |
| Library Captions tab, the speech panel and caption translation requests | [src/ui/captions_tab.rs](../src/ui/captions_tab.rs), [captions_tab/generate.rs](../src/ui/captions_tab/generate.rs) |

The core holds styles as data and never reads a font: a style names a family,
and the media crate looks it up in the project's font assets, the bundled Geist
faces and the system's faces, in that order. The system's faces are listed
once, on the lane's thread: the first time a title needs a family or a
character that the project and bundled faces lack, or a Text section asks for
the families.

A title reaches the screen in four steps. `resolve` gives its layer the text,
style, motions and word state at the frame-snapped time. The window asks the
lane for a picture when that layer, its quantized on-screen scale or the
project's fonts change. The lane draws the newest request per title and
answers with the clip, the exact layer and a serial. The window keeps the last
picture of each title on screen and draws it as one quad until a newer one
comes. An answer for a layer no longer asked for is dropped, and a replaced
project clears every texture.

Drafts work as they do for clips: the inspector hands `resolve` a
`ClipOverride::content`, the lane draws the draft's picture, and releasing the
pointer or leaving the field commits one command, or one batch when several
parts changed.

Speech enters like analysis. The window renders the chosen range of the
timeline mix offline at 16 kHz on the speech lane, which runs one job at a
time: a model download or a transcription. Words come back with the project
and request they were made for, and `speech::generated` turns them into one
command that adds a caption track. When the project changed meanwhile, or a
redo or an open plan would be lost, the captions wait in the panel until the
person adds or discards them. A translation is an ordinary
[native AI](native-ai.md) proposal whose one command is `CopyCaptions`.

Presets are desktop state, not project state. `presets.json` is read at start,
written atomically on a worker after a change and again on exit, and moved
aside rather than overwritten when this build cannot read it.

## Export and rendering

[ADR 0008](adr/0008-export-renders-originals-through-the-preview-compositor.md)
records the decision; [the design](../.scratch/export-and-rendering/spec.md)
holds the binding rules and their dated changes. This section says where
things live.

| Concern | Owner |
| --- | --- |
| `ExportSettings`, containers and codecs, `Capabilities` data, the `VIDEO_ENCODERS` table, `plan` into an `ExportPlan`, `FrameSpan`, `sample_count`, `Geometry`, `range_cues`, `render_project`, `builtin_presets`, `RenderJobInfo` | [export.rs](../crates/editor-core/src/export.rs) |
| `source_frame`, shared by the viewer and the export | [render.rs](../crates/editor-core/src/render.rs) |
| Encoder probe, FFmpeg arguments, the stdin-fed `Encoder`, the export sound render, `SourceReader`, `verify_output` | [editor-media/src/export](../crates/editor-media/src/export/mod.rs) (`caps`, `args`, `encoder`, `audio`, `reader`, `verify`) |
| `ProxyService` and the proxy cache (`aster-proxy`) | [proxy.rs](../crates/editor-media/src/proxy.rs) |
| Layer assembly the viewer and the export share; stacked drawing and non-blocking readback | [compose/assemble.rs](../src/compose/assemble.rs), [compose/gpu.rs](../src/compose/gpu.rs) |
| `FrameRenderer`, the headless device | [src/export/frame.rs](../src/export/frame.rs) |
| The export dialog's poster (`aster-poster`) | [src/export/poster.rs](../src/export/poster.rs) |
| Saved presets file; the window's export decisions and agent answers | [src/export/presets.rs](../src/export/presets.rs), [src/export/wiring.rs](../src/export/wiring.rs) |
| `RenderQueue` (`aster-render`), the job runner | [src/export/queue.rs](../src/export/queue.rs), [src/export/engine.rs](../src/export/engine.rs) |
| Export dialog; render queue panel | [src/ui/export_sheet.rs](../src/ui/export_sheet.rs), [src/ui/render_queue.rs](../src/ui/render_queue.rs) |
| Which proxies are asked for; which one a layer reads; preview quality | [src/proxies.rs](../src/proxies.rs), [src/textures.rs](../src/textures.rs), [src/settings.rs](../src/settings.rs) |
| Render operations on the wire | [editor-agent](../crates/editor-agent/src/lib.rs), [agent protocol](agent-protocol.md#render-jobs) |

The export path is:

```text
project, drafts committed → Arc<Project> snapshot + project id, context, revision
    → ExportSettings::plan(snapshot, capabilities) → ExportPlan
      (output frames, sample span, geometry, encoders, caption cues)
    → render_project (output frame rate; caption tracks hidden unless burned in)
sound:   OfflineMix over the sample span → raw f32le file in the job's
         working folder, .aster-render-<job> beside the destination
picture: each output frame n → resolve(render_project, frame_time(n))
         → SourceReader (originals, the preview's source frame)
         → FrameRenderer (one accumulator, linear light, runs on their own
           canvases, titles at the output scale) → readback → opaque RGBA
         → writer thread → FFmpeg stdin
FFmpeg (RGBA + f32le + captions) → .<name>.<job>.part beside the destination
    → exit 0 → ffprobe: streams and duration → sync → rename over destination
```

**Queueing.** The window commits drafts and hands `RenderQueue::enqueue` a
snapshot. `enqueue` reads no file: it plans the job with the probed
capabilities and compares names only, refusing a destination that is
relative or has an extension other than the container's, and a destination
or caption file that names a source of the snapshot, the project file or its
recovery, lock or checkpoint files, or a file another unfinished job writes
(`DestinationQueued`). Names are compared without regard to case on every
platform. Whether to replace an existing file, the export or its caption
file, is asked before queueing: the window asks the person, and an agent
passes `overwrite`. Later edits do not change a queued job.

When the job starts, the render thread asks the file system (`check_files`):
a destination that is a folder, a missing folder, or a destination or
caption file that resolves through links, `..` or case to a source or the
project's files, or is the same file by device and inode on Unix (a hard
link), fails the job with the same `EnqueueError` text. It then removes
what stopped exports left in the destination folder: part files and
working folders unchanged for an hour (`STALE_AFTER`) whose UUID names no
listed job, symbolic links skipped. When the queue starts it does the same
for `aster-render-<id>` folders in the system's temporary folder.

**Running.** One `aster-render` thread runs the first runnable job in list
order. The queue's lock is taken briefly and never held across job work; the
job reports through a handle. A job plans again with the capabilities of the
moment. It then builds its `FrameRenderer`, on the window's device or one it
opens itself, renders the sound and writes any caption stream into its
working folder (`.aster-render-<id>` beside the destination, or the system's
temporary folder when that cannot be made), starts the encoder and feeds
frames. Frames go to an `aster-render-writer` thread
through two recycled buffers, so drawing the next frame overlaps encoding the
last and at most three output frames exist. Every allocation and submit on
the render thread runs inside `Gpu::checked`. Readback maps a staging buffer
and polls the device without blocking, checking for a stop and a deadline.
Each job runs inside `catch_unwind`; a panic fails the job and its files are
removed.

The `FrameRenderer` owns its compositor state and sizes it from the job, so
the viewer's `ComposeBudget` steps and canvas count do not apply. A frame
needing more than `MAX_PASSES` passes fails the job with its timecode. Each
clip is decoded at the most it needs over its keyframed scale and crop. A
stabilized clip reads the stabilization cache as the viewer does; a clip
never measured, or measured on another file, is drawn unsteadied. An
unreadable lookup table is left out. Both add a warning to the job instead of
failing it. Unlike the viewer, the export draws titles through the
compositor, laid over with Normal blending.

**Finishing.** `Encoder::finish` runs on a scoped `aster-render-finish`
thread watched by `FinishWatch`: FFmpeg is given at least `FINISH_DEADLINE`
plus the part file's size at `FINISH_PACE`, then as long as the part file's
length or modified time keeps changing, and is killed after `FINISH_STALL`
without a change. A stall fails the job without a fallback.

**Pause, cancel and fallback.** Pause stops between frames: FFmpeg waits on
its pipe, a paused running job holds the queue, and paused time is not
counted. A job paused before it started is held: it is passed over while the
jobs after it render, and `resume_all` starts every held job. Cancel sets
the job's stop flag and calls the cancellers of its sound render, reader and
encoder, killing their processes. Under `EncoderChoice::Auto`, a hardware
encoder that fails to start, write, finish or pass the output check makes the
job start again once from frame 0 with the software encoder, reusing the
rendered sound; the job's message says so, and when the software encoder
fails too it gives both reasons. A missing folder, an FFmpeg reason that
names the disk or the file (`file_problem`: no space, quota, read-only file
system, name too long, I/O error, file too large; permission denied or no
such file only on a line about the part file or the output) and a stalled
finish fail at once instead. Retry puts a failed or cancelled job back in the
queue with the same snapshot, unless another unfinished job now writes its
file.

**Reports.** `jobs()` is a cheap copy of every `RenderJobInfo`; the window
reads it each frame the panel is open and is woken at most every 100 ms for
progress, and at once when a job starts, changes stage or ends. Done and
Failed jobs also send one finished event, for a notice, which the window
leaves out while the render queue panel is open. The desktop answers
the agent's render operations from the queue without blocking; the headless
`handle` answers them `unsupported`.

**Poster.** The export dialog's picture is drawn by `PosterService` on an
`aster-poster` thread that owns a `FrameRenderer` and a `SourceReader` of its
own, from the originals, at the output's shape scaled to at most
`POSTER_SIDE` (640) on the long side. A request is keyed by context,
revision, geometry, rate, burn-in and the output frame's time (the playhead,
clamped into the range); it is sent once it has stayed the same for `SETTLE`
(150 ms), only the latest waits, and a picture answering an older request is
dropped. The thread stops when the dialog closes.

### Proxies and preview quality

The proxy lane is one `aster-proxy` thread with its own FFmpeg process,
separate from preview, import, analysis and export. It writes one proxy at a
time from a bounded queue. Each finished proxy gets a manifest beside it, and
the lane keeps the cache directory within its budget, least recently used
first. Manifests are read on the lane when it starts; after that `lookup`
reads only memory. A source that changed on disk gets a new key, so its old
proxy is not used. The proxy's `fps` grid starts at the source file's start
(`start_time=0`, `PROXY_PROFILE` 2), so a time seeks to the same frame in
proxy and original even when the picture starts after the sound. A source
with an alpha pixel format gets no proxy: the job fails with "it has
transparency, which a proxy cannot keep, so the original is read". A proxy
removed for the budget is reported in a `ProxyEvent::Removed`; the window
sets that asset's status from the lane, and the mode does not remake it
until the library, project or mode changes, so two proxies do not push each
other out. `ProxyService::shutdown`, called from `App::on_exit`, waits at
most 750 ms for the proxy being written to stop.

The window decides what is asked for. [src/proxies.rs](../src/proxies.rs)
requests proxies by the person's mode (Off, Auto for videos above 1920 pixels
a side or 50 Mbit/s, Always for every video), or one asset at a time from the
library. It also tells the lane which proxy files the viewer has open, so
eviction and Clear skip them. [src/textures.rs](../src/textures.rs) decides
for each layer: it reads a proxy when one is ready for the file as it is now
and is at least as large as the box the layer is drawn at, and otherwise the
original. That box is the one the layer is granted: after the viewer budget
holds it, and while playing no larger than the box already granted. A proxy
file that fails to decode is not read again until it is removed, made again
or replaced; the original is read meanwhile. Between the viewer opening a
proxy and the lane hearing it is in use there is one frame in which eviction
can still remove it; the `Removed` event and the retry above recover from
that.
Proxies are put into picture requests only; sound, analysis, speech, titles
and export receive the project, which names only originals.

Preview quality divides the long side of each decode box and of the
compositor canvas by 1, 2 or 4. The preview settings are read leniently: an
unknown `quality` or `proxies` value, or a `proxy_budget_gib` that is not a
number, reads as the default and is written back unchanged until the person
chooses that setting; a `preview` that is not an object resets to the
defaults. During playback, each time a video layer
moves on to a new frame, Auto notes whether the frame it asked for before had
come. When 4 of the last 16 such advances, across layers, were late, it asks
at half until playback stops, so starting playback does not restart every
decoder pipe.

## Asynchronous work

Directory scans, probing, decoding, serialization and filesystem sync run on
workers. Interactive frames use nonblocking result
drainage and prepared render state. Queues and cache budgets have explicit
owners; submitting work at capacity produces a controlled retry or visible
status, not unbounded allocation.

Worker requests capture project identity, document revision, asset identity and
seek/request generation as applicable. Closing a project or superseding a seek
invalidates its results. Cancellation must release decoder/subprocess resources
even when the UI has already stopped displaying the request. Repaint requests
coalesce; worker completion does not cause one repaint per byte or sample.

The timeline has no arbitrary product limit on track count. Virtualize visible
tracks and clip intervals; offscreen objects remain editable without producing
widgets or GPU uploads every frame. This is separate from bounded runtime memory
and worker concurrency.

The desktop retains a pure `ProjectQuery` for its current immutable project
root. Prefix maximum clip ends let active-layer queries skip expired prefixes
while preserving nested overlaps and track/clip order. Root replacement reuses
bounds only for identical `Arc<Track>` roots; changed tracks rebuild their
bounds. The derived index retains one `Time` per clip, is absent from serialized
projects/history, and allocates nothing during warm queries. Long nested clips
can still require scanning a start-qualified suffix. The convenience model
queries remain available without an index. UUID validation uses a pre-sized
membership set while document traversal determines the first error.

## Persistence and source media

The initial `.aster` project file stores the current project, not the undo/redo
session. A sibling `.aster.recovery` file stores the current state and complete
undo/redo chains with interned shared document roots. Recovery through that
sidecar retains session history; opening only the ordinary project starts a new
history. Named checkpoints are project snapshots in a sibling
`.aster.checkpoints` directory. Keep these distinctions visible in recovery
guidance and completion claims.

Checkpoint restoration uses `RestoreProject` through the reducer as one history
entry. It preserves the current project identity, destination and live context,
so it can be undone. A recovery lease uses an adjacent `.aster.lock` OS lock;
active owners suppress recovery offers in another editor, and lease lifetime
extends through queued recovery writes. Lease-file existence or a PID is not
ownership evidence. This lease protects recovery; ordinary writes use an
independent protected document session.

The [protected document API](../crates/editor-store/src/document.rs)
`load_project_with_session` reads the project and its byte-count/SHA-256 baseline
from the same open file. Each independently opened editing context owns its own
`DocumentSession`; queued saves within that context share its advancing baseline.
`save_project_with_session` serializes and syncs a temporary file before taking
the adjacent `.aster.write-lock`, rechecking the original destination and
atomically replacing it. A successful durable save advances that baseline;
an observed change, deletion or independently created destination returns a
conflict. The lock makes the check/replacement indivisible for cooperating Aster
writers. An external writer that ignores it can still race the final check and
rename; this is not a global filesystem compare-and-swap.

`DocumentSession::prepare` records a deliberately chosen valid Save As
destination; it must not refresh an open session after a conflict or rebase
restored recovery. `create` refuses an existing file and is the policy for new
documents, duplication and acquiring authority for unbound recovery. Both CLI
and desktop implement create-only recovery Save As. The desktop carries the
restored origin until a fresh destination is successfully adopted; preparing
or refusing a destination does not turn recovery into ordinary work. Deliberate
ordinary Save As may replace a confirmed valid destination. The protected
desktop flow passed native GTK input on the db30 checkpoint recorded in
[verification](verification.md), following
[ADR 0005](adr/0005-bind-recovery-to-the-original-document.md).

Named recovery stores an optional versioned `document_binding` captured together
with the immutable history at submission. `load_recovery_with_document` restores
the archived destination/baseline instead of adopting today's bytes. If a valid
ordinary document changed, history restores dirty and the original baseline
still blocks overwriting it. Legacy, private, copied, inaccessible or unusable
bindings restore dirty unsaved work with no ordinary save authority; Save As
must deliberately acquire a new destination. A recovery captured during this
editor's own in-flight manual save may conservatively retain the earlier
baseline and require Save As after restart. See
[ADR 0005](adr/0005-bind-recovery-to-the-original-document.md) and the
[recovery integration contract](../.scratch/native-ai/recovery-contract.md).

Interactive storage callers use `SaveWorker::request_document` for protected
ordinary saves and `request_recovery` with the exact session/lease for bound
recovery. Their submissions capture state in memory; startup/open/Save As
preparation and fingerprint checks belong on storage workers. Compatibility
`save_project`/`SaveWorker::request` ordinary writes remain one-shot APIs and
must not replace a document session in an editing workflow. History-only
recovery APIs return no authority to overwrite an ordinary document.

The [66-test storage record](../artifacts/production-readiness/store/bound-recovery-tests.log)
covers document conflicts, advancing baselines, recovery ownership and bound
history restoration. Its [recovery-binding regressions](../crates/editor-store/tests/recovery_binding.rs)
include an actual independent writer after the crash snapshot, preserved exact
undo/redo, unchanged/current baselines and safe unsaved restoration. Headless
integration evidence is linked in the [agent protocol](agent-protocol.md).

Project documents are versioned independently of live runtime handles. The
writer emits schema version 2; version 1 is read as a strict subset through
serde defaults and is not written, so a build that knows only version 1
refuses a version 2 file through its future-schema protection. The clipboard
text follows the same rule. Derived stabilization data is not in the document.
Loading passes through core validation. A future schema, damaged or unreadable document
is protected from replacement; restore available recovery as unsaved work and
save to a new destination. Missing source media remains an offline asset with
its timeline references preserved.

Write a new file beside its destination, sync it, then atomically replace the
destination using platform-correct replacement behavior. Autosave and manual
save completion identify the exact context, project, destination and revision;
an old acknowledgement cannot clear newer dirty state. Intentional shutdown
flushes accepted work with a bounded wait and reports failure. Filesystem
atomicity and durability need native failure tests on each supported OS.

Imported media remains a source reference unless the person explicitly copies
it into managed storage. Edits do not alter original source files. Project
duplication gives the document a new identity and independent save destination
while retaining references to the same source media.

## Codec boundary

The media adapter decodes PNG/JPEG/WebP/BMP/GIF through Rust's `image` crate,
probes video/audio with `ffprobe`, and retains FFmpeg software-decoding processes
for sequential preview. Its frame cache has a 64 MiB/256-frame budget and up to
five retained video decoders; imports cap a batch at 10,000 entries. Thumbnail
and preview requests occupy independent lanes with generation/latest-request
checks. These are runtime bounds, not maximum editable track counts.

The preview worker may prepare the current admitted video and at most four
admitted pending videos before reading requested pixels. Preparation retains
request identity, does not publish or evict another lane, and runs outside the
queue mutex. Each FFmpeg pipe uses two input decode threads, one raw-video
output thread and a one-packet output queue. Requested pictures outrank
speculative reads; frames above 1 MiB stay demand-driven. The four-millisecond
speculative deadline and decoded cache/result budgets remain independent of
external codec working memory.

GIF playback retains sequential decoder state and reuses pixels throughout a
native display interval rather than reopening and decoding from frame zero for
every request. Its separate working-state budget is 64 MiB and four retained
states. Uniform validated delays expose a rational source frame rate; variable
delays retain exact display intervals without inventing a fixed-rate grid.

Native audio uses CPAL with background FFmpeg decoding/mixing and a bounded
lock-free callback ring. The initial mixer accepts up to 16 simultaneous audible
clips, with explicit failure beyond that budget; gain, source trim, clip rate,
preview speed and mute participate in mixing. Hidden video still contributes
unmuted sound unless an explicit Audio-track clip owns the same linked asset's
sound; the explicit clip remains authoritative when muted. Linked different
assets still mix independently. Rate changes currently change pitch.

The analysis lane is separate from preview decoding: its own thread, one
FFmpeg process at a time reading luma no larger than 384 pixels a side, four
jobs waiting at most, and a second thread that only reads the stabilization
cache. Its bounds are in [performance](performance.md#analysis-lane-and-stabilization-cache).
Sound for a clip with a time map plays on straight forward stretches only.

Export opens its own FFmpeg processes, apart from the preview's: an
`aster-render-probe` thread that test-encodes each known encoder at start (at
most four at once), one decode pipe per clip, picture and size in the
`SourceReader`, the mixer's decoders for the sound render, and the encoder.
They belong to the job and are killed when it is cancelled. The dialog's
poster has its own `SourceReader` on the `aster-poster` thread while the
dialog is open. Proxies are
written by their own lane, described under
[proxies and preview quality](#proxies-and-preview-quality). The bounds are in
[performance](performance.md#export-and-proxy-budgets).

Codec executables remain explicit runtime dependencies until packaged.
Decoding, for preview and export alike, is software RGBA, not
hardware/zero-copy decoding, HDR processing or exact variable-frame-rate PTS
handling. Export encodes with the hardware encoders FFmpeg offers on the host,
but writes 8-bit 4:2:0 (4:2:2 or 4:4:4 10-bit for ProRes) BT.709 pictures
without alpha, at a constant frame rate. Packaging, format coverage, native
audio/device-clock integration and licensing require the evidence in
[verification](verification.md).

## Neptune reference

The read-only reference is
`/home/biggabo/Documents/Projects/rust-terminal/rust-terminal`, inspected on
2026-10-12. Its `AGENTS.md`, `docs/architecture.md`, `docs/development.md`,
`docs/performance.md`, `docs/verification.md` and scoped source guides establish
the patterns adapted here: pure model/controller ownership, stable targets,
generation checks, worker-owned storage, bounded resources, quiet idle and
honest native acceptance evidence. Terminal/PTY semantics, browser helpers and
Neptune's release history do not transfer to the editor.

Neptune's `src/runtime/project_lead.rs` and Claude/Codex drivers also provide the
reference for worker-owned provider processes and pure protocol state machines.
Aster adapts that ownership to tool-disabled edit proposals, with authenticated
loopback TCP for external editing clients; Neptune's Unix shell/MCP adapters
are not evidence of Windows integration. The
[read-only source audit](../.scratch/production-readiness/audit.md#how-neptune-integrates-ai)
records the specific source seams and installed protocol schemas.
