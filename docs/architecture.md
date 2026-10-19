# Architecture

Neptune Video is a native Rust desktop editor. Its desktop stack follows Neptune Terminal's
`eframe`/`egui` and `wgpu` approach, keeping the domain model independent of the
window system. Native platform acceptance and playback throughput require
the evidence in
[verification](verification.md).

## Ownership

| Component | Owns | Boundary |
| --- | --- | --- |
| `editor-core` | Project settings, assets, folders, tracks, clips, markers, effects, masks, animation tracks, time maps, transitions, trackers, fades, track volume and solo, title styles, text animations, word times, graphics, caption languages, SubRip/WebVTT text, edit validation, history, frame timing, the pure `resolve` to a render plan, the pure `sound` rules (what sounds, its level, pitch keeping, ducking keys), and export data: settings, formats, presets, capability data, validation into an `ExportPlan`, output frame and sample spans, output geometry and render job reports | No windows, filesystem, font files, decoder handles, GPU or live workers |
| `editor-store` | Versioned documents, protected document sessions, atomic replacement, autosave, recovery bindings and the project's media folder for takes and stems | Immutable project/history snapshots and explicit destination ownership in, load/save/conflict results out |
| `editor-media` | File/folder import, source probing, image/GIF/video decoding, cached frames, PCM audio mixing with the voice filter chain and its probe, voiceover capture to WAV, `.cube` table reading, the analysis lane (region tracking, camera-motion measurement and its on-disk cache), the sound lane (peaks and their on-disk cache, loudness, beats, a sound written to a file, stem separation), the text renderer and title lane (fonts, shaping, title pictures), the speech lane (model downloads, offline 16 kHz mix, whisper.cpp), export encoding (encoder probe, FFmpeg arguments, the stdin-fed encoder, output check), the export source reader, the offline sound render for export, and the proxy lane and cache | Owned metadata, frame/audio results, title pictures, words, analysis results and proxy answers carrying project, request and source identity; codec, font and live worker details remain private. The export calls block and run on a thread the desktop owns |
| Desktop composition | Controller integration, worker scheduling, clipboard, file dialogs, input, playback clock, title textures, the preset file and subtitle file reads and writes, preview quality and which proxies the viewer reads | Converts UI actions and completions into targeted commands |
| Render queue (`src/export`) | `RenderQueue`, the `neptune-video-render` thread, the job runner and `FrameRenderer` | Takes immutable project snapshots; publishes job reports and finished events; never edits a project |
| Compositor (`src/compose`) | Off-screen wgpu passes, WGSL pixel math, `ComposeBudget`, lookup-table textures, the layer assembly the viewer and export share, readback | Draws a render plan's non-plain runs into one canvas texture while the interface is drawn, never asking for a repaint and falling back to the mesh path; draws every layer of an export frame on the render thread at the output size, with no fallback |
| Native UI | Media library with text, captions, effects, transitions and looks tabs, preview, timeline with keyframe lane and graph editor, sectioned inspector, export dialog, render queue panel, Agent panel, dialogs and accessible controls | Draws prepared state and emits actions |
| `editor-agent` | Command/schema discovery, guarded local transport, native provider processes, captured planning context, edit proposals, the Agent panel's `Conversation` and the render job operations | A planning provider returns data; a conversation's provider edits only through the MCP server it is handed, as guarded requests. Both reach the same validated reducer as the UI |

The root desktop package is `neptune-video`, with `neptune-video`, `neptune-video-agent` and
`neptune-video-mcp` binaries. Consult the manifests and
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

The Agent panel's Ask mode sends a request to the Claude Code or Codex CLI the
person has signed in to. Settings › Agent ("Edits") and the panel's menu
choose how it is answered. **Made as it works**, the default, is a
conversation in which the provider edits through Neptune Video's MCP server.
**Proposed for review** is the planner, which returns one proposal and changes
nothing until it is applied (`AiSettings.review`, false by default). Caption
translation always uses the planner; Command mode runs the local command
language. Both runtimes belong to `editor-agent`, independent of the desktop
drawing loop, and [src/ai.rs](../src/ai.rs) is the only place the application
touches either. They reuse the person's CLI sign-in and expose no credential
fields in `AiConfig`.
[ADR 0011](adr/0011-the-agent-tab-edits-through-neptune-video-mcp-started-in-its-window.md)
records why the panel's agent edits through `neptune-video-mcp`, and
[ADR 0004](adr/0004-native-ai-produces-reviewed-edit-proposals.md) the
planner.

### Made as it works

```text
Agent panel → App::agent_converse (drafts committed, window note prefixed)
    → ai::Agent → editor_agent::Conversation (one worker thread, one provider process)
    → provider CLI: claude (stream-json) or codex app-server, its only tools Neptune Video's
    → neptune-video-mcp --window <session file> --tag neptune-video-tab-<instance>
      --roots-file <session>.roots
      (the provider's child)
    → the window's AgentHost session → editor_agent::handle on the window's thread
      (or its own local authority for a project no window shows)
    → one validated command, one history entry; the panel lists the step
```

`Conversation` owns one worker thread per window, and the worker owns the
provider process; `ask`, `stop`, `end` and `poll` do no provider or filesystem
I/O. One request runs at a time, and another is refused as busy. A request is
1 to 65,536 bytes without NUL. The window prefixes it with the playhead and the
selected clips, which the agent cannot see, and the provider receives it on
stdin, never in its arguments. `poll` returns `TurnEvent`s in order: `Started`
first, then `Said`, `Called` and `Returned`, and exactly one of `Finished`,
`Failed` or `Cancelled` last. At most 4,096 unread events are kept; the oldest
steps go first, never a request's end. A provider process lives at most
12 hours, and a request that hears nothing from the provider for 600 s fails.
The conversation continues while the provider, its program, model, effort and
the tool server's command and arguments stay the same. The folders are not
among them, so a new folder does not end it. Otherwise the next request starts
a new one, and the panel says the agent remembers nothing of the last. The
window also ends the conversation when the person opens or creates another
project in it.

`editor-agent` is handed the server as a `ToolServer`, a program and its
arguments as text, and passes it to the provider. It links nothing of the root
package, `editor-store` or `editor-media`, and
`scripts/check-architecture.py` holds that. The window builds the server in
`App::tool_server`. The program is `NEPTUNE_VIDEO_MCP`, else `neptune-video-mcp` beside the
running `neptune-video`; without either the request fails with a message.

The window passes no `--root`. It names a roots file, `--roots-file`: the
agent session file with the extension `.roots`, a JSON object
`{"roots": [DIR, …]}` readable by its owner only on Unix and written whole (a
part file renamed over it). A thread of its own, `neptune-video-agent-roots`, writes
it, and the file is removed when the window closes. The wanted folders are the
project file's folder, the folder of every asset, the folders the person added
in Settings › Agent › Folders (`AiSettings.folders`) and the person's Videos
(Movies), Desktop, Documents, Downloads, Music and Pictures folders, looked up
once. While the agent talks the window checks each frame whether the
revision, the project path or the person's folders changed, and only then
recomputes the list and sends it to the thread. The window sends the folders
as they are; the server reads the file again whenever its modified time,
length or (on Unix) inode changes, at most 256 KiB, and `usable_roots` keeps
them in that order as canonical directories, without one that is gone, too
wide (a home, drive or system folder) or inside another. A missing, too large
or unreadable file names no folder; with a roots file and no `--root` the
working directory is not a root. Media outside them is refused
`outside_roots` until the person imports it or adds its folder, and the same
conversation can use it at the next call. `project_status` lists the current
roots.

`provider::converse` launches the provider on the conversation's worker with
Neptune Video's server as its only tools: Claude Code as `claude --print` with
stream-json input and output, built-in tools off and one strict MCP server,
`neptune-video`, in a private config file; Codex as `codex app-server` with the shell
tool off and the person's own MCP servers disabled for the thread beside one
server, `neptune_video_window`. Claude runs without the planner's `--safe-mode`, which
in 2.1.300 drops every MCP server, and Codex keeps `code_mode_host`, through
which 0.162.1 calls MCP tools. [Launching the agent](native-ai.md#launching-the-agent)
gives the exact arguments.

The server is started in its window (`--window`), its home window: it
attaches to the window whose session file it names, at once or at the first
call if the window is not answering yet, and follows it. It is not locked
there. Like any MCP server it may create, open, save and close projects, edit
one no window shows through its own local authority (saved after every edit),
show one in a new window with `desktop` `open` and attach to another window by
`instance_id`. Doing so by choice stops it following its window; `desktop`
`attach` naming no window brings it back and it follows again. While it
follows a window that is gone, and for `attach` naming no window, it answers
`desktop_closed`. The agent's undo takes back only its own entries since it
last attached, as for any MCP client, so after it leaves and comes back
earlier entries need `allow_person` or the person's own undo. Its request
ids are `neptune-video-tab-<instance>-N` (`--tag`), `<instance>` being the window's
agent session instance id, and a window claims only requests with its own
tag. By that tag the window counts the request's changes for the panel
and leaves out the "Agent: …" rows it shows for outside MCP clients; tool
results that report a change and are not this window's are counted as changes
in other projects. `person_editing` and `desktop_busy` refusals apply as they
do to any agent. The window answers one request at a time, so the panel's
agent and an outside client can edit the same window, each typed edit that
passes `expected_revision` guarded by it. Tests in `tests/mcp.rs` cover
leaving the window, editing a project of its own and coming back, and a roots
file read again as it changes; [verification](verification.md#in-app-agent)
records which native runs exist.

Unlike the planner, this agent sees pictures: rendered frames, contact sheets,
transcripts and the other results of its tools reach the provider under the
person's own sign-in ([what reaches the provider](native-ai.md#what-reaches-the-provider)).
It cannot read outside the roots, run shell commands, browse the web or use
other MCP servers.

Stop kills the provider at once and ends the conversation; there is no
graceful interrupt. Changes already made stay, and an export the agent queued
belongs to the window's render queue and keeps running there.
[Stopping and ending](native-ai.md#stopping-and-ending) lists the other ways
a conversation ends and [limitations](native-ai.md#limitations) the rest;
[verification](verification.md#in-app-agent) records the macOS debug runs and
what is unverified on Linux, Windows and release builds.

### Proposed for review

Native planning belongs to `editor-agent`. `AiContext::capture` retains an
immutable project snapshot, the live context identity, revision, selected clip
IDs and playhead. `AiRuntime` owns one planning worker and one active request,
returning bounded typed events through nonblocking polling. Claude
print/structured-output and Codex app-server drivers run the planner.

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
provider evidence and remaining native acceptance of both modes.

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

`editor_core::resolve` and `ProjectQuery::resolve` share one layer evaluator:
animation, follow, transition pairing and effect rules remain identical, and
neither reads a clock. The viewer uses the query for its exact immutable
project root, with direct evaluation as a fallback, and requests media from
the plan's layers. Stabilized followers can require a second evaluation.
Interval bounds skip expired clip prefixes during initial layer enumeration;
follower target lookup and timeline painting keep their separate searches.
See [advanced editing](#advanced-editing).

The media runtime exposes an optional audio-device clock after initial buffering.
Desktop playback follows it when audio output is available, with a monotonic
fallback when silent. Actual device synchronization and display latency still
need native measurements; hardware decoding remains future work. Speed keeps
pitch only on a clip with a `Pitch` audio effect (see the codec boundary).

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

## Audio editing

[ADR 0008](adr/0008-sound-is-levels-in-the-mixer-and-filters-in-the-decoder.md)
records the decision; [the design](../.scratch/audio-editing/spec.md) holds the
binding rules, the owner decisions and their dated changes. This section says
where things live; how the mixer, the chain and the recorder work is under
[the codec boundary](#codec-boundary).

| Concern | Owner |
| --- | --- |
| `Track.gain`, `Track.solo`, `Clip.fade_in`, `Clip.fade_out` and their bounds | [model.rs](../crates/editor-core/src/model.rs) |
| The seven audio `EffectKind`s and their `spec` rows; `EffectKind::is_audio` | [look.rs](../crates/editor-core/src/look.rs) |
| `SetFades`, the two `SetTrackState` fields, `fit_fades` after trim, split, rate and retime | [command.rs](../crates/editor-core/src/command.rs), [editor.rs](../crates/editor-core/src/editor.rs) |
| What sounds and when (`sounds`, `owned`, `soloed_out`, `audible_at`, `Sounding`), the level (`fade_at`, `level_at`), pitch keeping (`pitch`, `pitch_kept`, `PITCH_KEPT`) and ducking keys (`duck`, `duck_keys`) | [sound.rs](../crates/editor-core/src/sound.rs) |
| Mixer admission, the level stage and the master limiter | [audio.rs](../crates/editor-media/src/audio.rs), [audio/plan.rs](../crates/editor-media/src/audio/plan.rs) |
| Filter strings, the delay table and the filter probe | [audio/chain.rs](../crates/editor-media/src/audio/chain.rs) |
| The sound lane: peaks and their cache, loudness, beats, Save audio as, stem separation and the separator probe | [sound.rs](../crates/editor-media/src/sound.rs) and `sound/{peaks,loudness,beats,extract,separate}.rs` |
| Voiceover capture | [record.rs](../crates/editor-media/src/record.rs) |
| The media folder and a take's path | [media.rs](../crates/editor-store/src/media.rs) |
| Peaks and beat grids the window holds | [src/peaks.rs](../src/peaks.rs) (`HeldPeaks`, `HeldBeats`) |
| Detach audio and background music batches | [src/rules.rs](../src/rules.rs) |
| Audio presets, the kinds a person adds and their refusals | [src/audio.rs](../src/audio.rs) |
| Keep pitch | [src/speed.rs](../src/speed.rs) |
| Normalize, Duck under speech, Save audio as, Separate stems, the take | [src/normalize.rs](../src/normalize.rs), [src/duck.rs](../src/duck.rs), [src/extract.rs](../src/extract.rs), [src/stems.rs](../src/stems.rs), [src/record.rs](../src/record.rs) |
| Waveforms, fade handles, the volume line, beat ticks and snapping, the track head's solo and volume | [timeline.rs](../src/ui/timeline.rs), with the shared rules in [fades.rs](../src/ui/fades.rs) and [tracks.rs](../src/ui/tracks.rs) |
| The Audio section, fade rows, Normalize and Ducking cards | [inspector/audio.rs](../src/ui/inspector/audio.rs), [inspector/clip.rs](../src/ui/inspector/clip.rs) |
| The record control; cards of a sound being saved and a separation | [viewer.rs](../src/ui/viewer.rs), [sheets.rs](../src/ui/sheets.rs) |
| Command-panel phrases | [src/agent.rs](../src/agent.rs) |

Every sound edit is an ordinary command through `Action → App →
Editor::apply`: `SetFades`, `SetTrackState`, `SetGain`, `SetKeyframes`,
`SetEffects`, `AddMarker`, or a `Batch` of `AddAssets`, `AddTrack`, `AddClip`
and `LinkClips`, one history entry each. The sound lane is a worker of its own
beside the analysis lane, so a long tracking run holds up no waveform: one
thread with one FFmpeg or separator process at a time, a thread that only
reads the peak cache and finds beats, sixteen jobs waiting on each, and
replacement by target ((kind, media) or (kind, media, span)), so a loudness
measurement does not stop the same media's waveform. Its answers carry the
request, project, asset and source identity, and the desktop drops any it no
longer waits for. A measurement, a duck or placed stems are written only after
the clip, its media and the project are checked again; a stale answer writes
nothing and says so. `cancel_all` on a project replace spares a file being
written and a separation, which are the person's: their files are named, and
nothing is offered to the project that came after.

A person starts every one of these jobs and the recording; none is a command,
so an agent and the headless CLI can write their results and cannot start
them. Bounds are in [performance](performance.md#audio-editing-budgets).

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
| The title lane (`neptune-video-titles`) | [text/lane.rs](../crates/editor-media/src/text/lane.rs) |
| Model catalog, downloads through `curl`, the offline mix, whisper.cpp and the speech lane (`neptune-video-speech`) | [editor-media/src/speech.rs](../crates/editor-media/src/speech.rs) |
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

[ADR 0009](adr/0009-export-renders-originals-through-the-preview-compositor.md)
records the decision; [the design](../.scratch/export-and-rendering/spec.md)
holds the binding rules and their dated changes. This section says where
things live.

| Concern | Owner |
| --- | --- |
| `ExportSettings`, containers and codecs, `Capabilities` data, the `VIDEO_ENCODERS` table, `plan` into an `ExportPlan`, `FrameSpan`, `sample_count`, `Geometry`, `range_cues`, `render_project`, `builtin_presets`, `RenderJobInfo` | [export.rs](../crates/editor-core/src/export.rs) |
| `source_frame`, shared by the viewer and the export | [render.rs](../crates/editor-core/src/render.rs) |
| Encoder probe, FFmpeg arguments, the stdin-fed `Encoder`, the export sound render, `SourceReader`, `verify_output` | [editor-media/src/export](../crates/editor-media/src/export/mod.rs) (`caps`, `args`, `encoder`, `audio`, `reader`, `verify`) |
| `ProxyService` and the proxy cache (`neptune-video-proxy`) | [proxy.rs](../crates/editor-media/src/proxy.rs) |
| Layer assembly the viewer and the export share; stacked drawing and non-blocking readback | [compose/assemble.rs](../src/compose/assemble.rs), [compose/gpu.rs](../src/compose/gpu.rs) |
| `FrameRenderer`, the headless device | [src/export/frame.rs](../src/export/frame.rs) |
| The export dialog's poster (`neptune-video-poster`) | [src/export/poster.rs](../src/export/poster.rs) |
| Saved presets file; the window's export decisions and agent answers | [src/export/presets.rs](../src/export/presets.rs), [src/export/wiring.rs](../src/export/wiring.rs) |
| `RenderQueue` (`neptune-video-render`), the job runner | [src/export/queue.rs](../src/export/queue.rs), [src/export/engine.rs](../src/export/engine.rs) |
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
         working folder, .neptune-video-render-<job> beside the destination
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
for `neptune-video-render-<id>` folders in the system's temporary folder.

**Running.** One `neptune-video-render` thread runs the first runnable job in list
order. The queue's lock is taken briefly and never held across job work; the
job reports through a handle. A job plans again with the capabilities of the
moment. It then builds its `FrameRenderer`, on the window's device or one it
opens itself, renders the sound and writes any caption stream into its
working folder (`.neptune-video-render-<id>` beside the destination, or the system's
temporary folder when that cannot be made), starts the encoder and feeds
frames. Frames go to an `neptune-video-render-writer` thread
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

**Finishing.** `Encoder::finish` runs on a scoped `neptune-video-render-finish`
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
`neptune-video-poster` thread that owns a `FrameRenderer` and a `SourceReader` of its
own, from the originals, at the output's shape scaled to at most
`POSTER_SIDE` (640) on the long side. A request is keyed by context,
revision, geometry, rate, burn-in and the output frame's time (the playhead,
clamped into the range); it is sent once it has stayed the same for `SETTLE`
(150 ms), only the latest waits, and a picture answering an older request is
dropped. The thread stops when the dialog closes.

### Proxies and preview quality

The proxy lane is one `neptune-video-proxy` thread with its own FFmpeg process,
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

## MCP server

`neptune-video-mcp` ([guide](mcp.md), [ADR 0010](adr/0010-mcp-reaches-the-editor-through-the-agent-protocol.md),
[ADR 0011](adr/0011-the-agent-tab-edits-through-neptune-video-mcp-started-in-its-window.md))
lives in `src/mcp/` of the root package's library and depends on the same
crates as the desktop; the core gains nothing for it. Outside MCP clients start
it, and so does the provider of the window's Agent panel, as its child (see
[made as it works](#made-as-it-works)).

```text
MCP client or the Agent panel's provider
    ── stdio or loopback HTTP (rmcp) ──▶ NeptuneVideoServer (tools, resources)
    │ typed tools build Commands with the window's rules (rules.rs, titles,
    │ captions, fades, effects) from a fresh snapshot
    ▼
authority thread (bounded queue) ── one guarded AgentRequest at a time ──▶
    local:   Editor + DocumentSession, saved after every edit (headless.rs)
    desktop: the window's AgentHost session → editor_agent::handle on its thread
frames thread: headless FrameRenderer + SourceReader (frame_render, contact_sheet)
jobs: transcription, model installs, scenes, loudness (≤ 4 running, 64 kept);
      exports in the local RenderQueue or the window's render queue
```

- **Workspace** (`workspace.rs`) owns the authority, the binding
  `(project_id, context_id, path)`, the agent's own undo and redo pairs, the
  windows it found (looked for at most once a second), its home window and
  whether it still follows it, its request tag and the local render queue.
  Only the authority thread touches it.
- **Home window** (`--window FILE`) starts the server in the window whose
  session file is `FILE`. `Workspace::pin` attaches to it, or at the next
  call while the server still follows it and has no project, and
  `following` is true. Creating, opening or closing a project or launching a
  window by choice sets it false; `desktop {op: attach}` with no
  `instance_id`, or naming the home window, attaches there and sets it true
  again. While it follows a window that is gone, or is asked to attach to
  no window, it answers `desktop_closed`. `--tag TAG` (1 to 64 ASCII
  letters, digits and dashes) makes request ids `TAG-N` instead of `mcp-N`.
- **Roots** (`paths.rs`) are the `--root` folders, first, and those named
  in `--roots-file FILE`, read again whenever the file changes. A `--root`
  folder that is missing or too wide stops the server at start; the file's
  folders pass `usable_roots`, which leaves such a folder out.
- **Tools** (`tools/*.rs`) parse typed parameters (`TimeSpec` for
  frame-accurate times), check paths against the roots, build one `Command`
  and hand it to `Workspace::apply`. `apply` validates it as a dry run on the
  snapshot, refuses new overlaps unless asked, and sends it guarded by the
  snapshot's revision.
- **Jobs** (`jobs.rs`) run on their own threads with progress and a cancel
  flag. They read snapshots and never edit.
- **Server** (`server.rs`) routes tools, serves the `neptune-video://` resources and
  polls them twice a second while a client is subscribed, to notify changes.
- **HTTP** (`http.rs`) wraps rmcp's `StreamableHttpService` in axum with
  bearer authentication, `Host` and `Origin` checks and a body limit.

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

The initial `.nvp` project file stores the current project, not the undo/redo
session. A sibling `.nvp.recovery` file stores the current state and complete
undo/redo chains with interned shared document roots. Recovery through that
sidecar retains session history; opening only the ordinary project starts a new
history. Named checkpoints are project snapshots in a sibling
`.nvp.checkpoints` directory. Keep these distinctions visible in recovery
guidance and completion claims.

Checkpoint restoration uses `RestoreProject` through the reducer as one history
entry. It preserves the current project identity, destination and live context,
so it can be undone. A recovery lease uses an adjacent `.nvp.lock` OS lock;
active owners suppress recovery offers in another editor, and lease lifetime
extends through queued recovery writes. Lease-file existence or a PID is not
ownership evidence. This lease protects recovery; ordinary writes use an
independent protected document session.

The [protected document API](../crates/editor-store/src/document.rs)
`load_project_with_session` reads the project and its byte-count/SHA-256 baseline
from the same open file. Each independently opened editing context owns its own
`DocumentSession`; queued saves within that context share its advancing baseline.
`save_project_with_session` serializes and syncs a temporary file before taking
the adjacent `.nvp.write-lock`, rechecking the original destination and
atomically replacing it. A successful durable save advances that baseline;
an observed change, deletion or independently created destination returns a
conflict. The lock makes the check/replacement indivisible for cooperating Neptune Video
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
writer emits schema version 4; versions 1 to 3 are read as strict subsets
through serde defaults and are not written, so a build that knows only an
older version refuses a version 4 file through its future-schema protection.
Version 3 added titles, graphics and captions; version 4 added fades, track
volume and solo, and audio effects.
The clipboard text follows the same rule. Derived stabilization data is not in the document.
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
it into managed storage. Edits do not alter original source files. The editor
creates media in one place only: voiceover takes and separated stems are
written to the `<document stem> media/` folder beside the saved document
(`editor_store::media_folder`, `new_take`), made on first use, never emptied
or deleted by the editor and refused for a project without a file. They are
stored by absolute path, as imports are, so they do not move with a moved
project; relative paths are a follow-up that changes no file format today. Project
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
queue mutex. Each FFmpeg pipe uses four input decode threads, one raw-video
output thread and a one-packet output queue. Requested pictures outrank
speculative reads; frames above 1 MiB stay demand-driven. The four-millisecond
speculative deadline and decoded cache/result budgets remain independent of
external codec working memory.

Continuous playback can reuse a compatible forward pipe across the larger of
twelve source frames or two source seconds, capped at 240 frames. This lets a
slow cold start catch up without repeatedly paying the same seek. Explicit
seeks and posters keep the twelve-frame reuse window. Preparation carries each
request's own purpose, including other admitted videos prepared in its cohort.
Skipped pictures are read into the same scratch buffer with cancellation
checked between reads; only the requested picture is published. This bounds
source distance, not elapsed decoding time.

GIF playback retains sequential decoder state and reuses pixels throughout a
native display interval rather than reopening and decoding from frame zero for
every request. Its separate working-state budget is 64 MiB and four retained
states. Uniform validated delays expose a rational source frame rate; variable
delays retain exact display intervals without inventing a fixed-rate grid.

Native audio uses CPAL with background FFmpeg decoding/mixing and a bounded
lock-free callback ring. The initial mixer accepts up to 16 simultaneous audible
clips, with explicit failure beyond that budget that names how many sound and
when. What is in the mix is decided in one place, `editor_core::sound`: a clip
sounds when its track is not muted, not soloed out and not at volume 0, its
asset has sound, its own volume is not 0 without volume keyframes, and no
Audio-track clip of the same asset in its link owns that sound. Hidden video
still sounds. The explicit Audio-track clip remains authoritative when its
track is muted or soloed out, so neither reveals embedded sound. Linked
different assets still mix independently. A clip's level is track volume times
clip volume (stored or keyed) times its edge fades, read once for a clip where
it does not change and at every output frame where it does; source trim, clip
rate and preview speed participate as before. The sum ends in a peak limiter
with a ceiling of 0 dBFS in place of a hard clamp: a frame past the ceiling is
turned down to it at once and the gain rises back with a 200 ms time constant.
A frame within the ceiling is not altered while the gain is 1; after a peak the
frames that follow are turned down as well until the gain is 1 again, 1.4 s
after a peak of twice the ceiling at 48 kHz. It has no lookahead and limits
sample peaks, not true peaks. All of this runs on the audio worker; the device
callback only copies. A clip's speed changes its pitch unless the clip has an
enabled `Pitch` effect, and the preview speed always does.

A clip on an audio track may carry audio effects, and they are heard through
the `-af` of that clip's own decoder: `audio/chain.rs` is the one place filter
syntax is written. After the speed stage it writes each enabled audio effect
in stack order (`highpass`, `equalizer`, `acompressor`, `alimiter`, `afftdn`,
and `aformat` with `pan` for centre reduction), with decibels made linear
where the filter wants linear and a frequency held under 0.45 of the output
rate; a clip without audio effects gets the string it always had. Centre
reduction is left out for an output of one channel, where the sum of its two
channels would be silence, and that is said once. `Pitch` is no filter after
the speed stage but the speed stage itself: without one, `asetrate` takes the
rate times preview speed times clip speed, as it always did; with the first
enabled `Pitch`, its ratio `P` is taken out of the clip speed, one `atempo`
stage takes what is left held to 0.5..2, and `asetrate` takes the rest, so the
clip keeps its speed and its pitch is `P` while the hold does not bite.
Which `Pitch` is read and the 0.5..2 hold are `editor_core::sound`'s, which
the inspector reads too. `atempo` is never chained: each stage makes the
sound arrive early by an amount that moves from one event to the next (8 to
59 ms measured), which no frame drop can correct, so that lead is not
compensated. FFmpeg reads a chain once, when the decoder starts, so a value
is heard after the edit commits and the decoders restart, and a
compressor's envelope or the noise estimate starts cold at every seek. The
constant delay of a chain (25 ms for noise reduction, 5 ms for the limiter)
is turned into frames at the device's rate and that many leading frames of the
voice are read and dropped.

Which audio kinds the host's FFmpeg runs is found by running them, once a
session: a control run of `anull` over 50 ms of silence and then each kind's
own string at its defaults, on the audio worker before it looks for an output
device, with processes of the probe's own that a seek does not stop. A kind
whose run fails is left out of every chain and said once, when a clip first
goes without it; if the control fails nothing is known, every kind counts as
available and that is said once. `editor_media::missing_audio_filters()` hands
the finished answer to the interface and never waits. A chain that passed the
probe at its defaults and is refused at another value ends that decoder, which
stops the preview's sound with FFmpeg's message.

A voiceover take is recorded by `RecordService` in
[record.rs](../crates/editor-media/src/record.rs): CPAL's default input
device, at its own rate, one or two channels as delivered (more are refused),
written to a 32-bit float WAV at a path the desktop chose (ticket 17's
`editor_store::new_take`). The input callback only copies into a bounded
lock-free ring of two seconds, or counts the frames that found it full; one
thread, `neptune-video-record`, opens the device and the file, drains the ring every
20 ms and rewrites the header's two sizes and syncs once a second of sound, so
a crash leaves a file that plays to within a second of its end. `start`,
`stop` and `poll` return at once: opening, writing and closing are the
thread's, and `Started`, `Level`, `Finished(Take)` or `Failed` come through
`poll`. A file at the path is refused and left as it was (`create_new`), and
no file is made before the device plays, so a device that will not open or
play answers `Failed` with nothing in the media folder; the service never
deletes one, and a device error, a failed write or the take's limit (2 hours,
or the most a classic WAV header can count) ends the take with what was
written, cut to the samples counted. The service is its own clock: `frames()`
counts the frames that reached the ring since the device began to play, which
is their place in the file and may be past 0 when `Started` is read, and
nothing about the timeline bounds a take.

The desktop's [record.rs](../src/record.rs) owns the take as session state
beside playback, never in the project. Record (the transport's control or R)
is refused in words for a project without a file or while a take is under
way; otherwise `new_take` runs on a short worker whose answer carries the
editor instance, and only the take still waiting in that instance starts the
service. `Started` starts the timeline from its playhead at 1×. The take is
anchored to the playback clock: the first frame after `Started` that reads an
`AudioService::position` pairs it with `frames()`, and without one within a
second, or with nothing playing, the transport's position is used. The
project's end stops the transport only. When `Finished` arrives the media is
built from the `Take` itself, without an import or a probe, and enters with
`AddTrack` when no unlocked audio track has room and `AddClip` as one `Batch`,
"Record voiceover", through the ordinary command path; `MediaService::import`
is not used, so a user import answered meanwhile keeps its own step. A
replaced project stops the take and is given nothing; the file is named in a
notice. An agent cannot start or stop a take.

The analysis lane is separate from preview decoding: its own thread, one
FFmpeg process at a time reading luma no larger than 384 pixels a side, four
jobs waiting at most, and a second thread that only reads the stabilization
cache. Its bounds are in [performance](performance.md#analysis-lane-and-stabilization-cache).
Sound for a clip with a time map plays on straight forward stretches only.

Export opens its own FFmpeg processes, apart from the preview's: an
`neptune-video-render-probe` thread that test-encodes each known encoder at start (at
most four at once), one decode pipe per clip, picture and size in the
`SourceReader`, the mixer's decoders for the sound render, and the encoder.
They belong to the job and are killed when it is cancelled. The dialog's
poster has its own `SourceReader` on the `neptune-video-poster` thread while the
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

## Neptune Terminal reference

The read-only reference is
`/home/biggabo/Documents/Projects/rust-terminal/rust-terminal`, inspected on
2026-10-12. Its `AGENTS.md`, `docs/architecture.md`, `docs/development.md`,
`docs/performance.md`, `docs/verification.md` and scoped source guides establish
the patterns adapted here: pure model/controller ownership, stable targets,
generation checks, worker-owned storage, bounded resources, quiet idle and
honest native acceptance evidence. Terminal/PTY semantics, browser helpers and
Neptune Terminal's release history do not transfer to the editor.

Neptune Terminal's `src/runtime/project_lead.rs` and Claude/Codex drivers also provide the
reference for worker-owned provider processes and pure protocol state machines.
Neptune Video adapts that ownership to tool-disabled edit proposals and to the Agent
panel's conversation, whose only tools are Neptune Video's own MCP server, with
authenticated loopback TCP for external editing clients; Neptune Terminal's Unix shell/MCP adapters
are not evidence of Windows integration. The
[read-only source audit](../.scratch/production-readiness/audit.md#how-neptune-integrates-ai)
records the specific source seams and installed protocol schemas.
