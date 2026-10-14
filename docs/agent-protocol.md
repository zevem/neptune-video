# Agent editing protocol

`editor-agent` lets external coding/AI agents inspect and edit Aster through the
same validated commands and undo history as the desktop. It supplies a local
transport and command/schema protocol. The crate also supplies
[native provider planning](native-ai.md); that reviewed proposal workflow uses
a narrower schema and the same reducer.

## Headless sessions

Build `aster-agent`, then run an in-memory session or a task-owned project:

```sh
cargo build --locked --bin aster-agent
target/debug/aster-agent
target/debug/aster-agent --project /absolute/project.aster --create
target/debug/aster-agent --project /absolute/project.aster --recover
target/debug/aster-agent --project /absolute/project.aster --recover --save-as /absolute/recovered.aster
```

On Windows the executable is `target/debug/aster-agent.exe`. With `--project`,
successful mutations save the current project and its recovery history.
`--create` immediately creates a new document and refuses every existing file.
`--recover` opens the sibling recovery archive under its recovery lease and
restores undo/redo. An ordinary project open starts a new undo/redo history.
`--create` and `--recover` are mutually exclusive; both require `--project`.

`--save-as NEWPATH` saves the loaded/recovered project and history to a new
destination without discarding undo/redo. It also requires `--project`, cannot
combine with `--create`, and refuses an existing destination. The source project
and recovery copy remain untouched. These argument and durability paths are
implemented in [aster-agent.rs](../src/bin/aster-agent.rs) and exercised through
the real Linux stdin/stdout CLI in
[eight acceptance checks](../artifacts/production-readiness/store/headless-bound-stdio.json)
alongside [12 binary tests](../artifacts/production-readiness/store/headless-bound-tests.log).

Each headless document owns an advancing protected `DocumentSession`. Its
canonical path and exact original byte/hash baseline prevent an independent
writer's intervening save from being silently adopted. Bound recovery restores
its archived baseline, even when today's valid ordinary project differs.
Legacy, copied/private or otherwise unusable recovery bindings restore dirty
unsaved work; they can be inspected, but durable mutation requires a new
`--save-as` destination. Neither inspection nor recovery opening authorizes
overwriting the current ordinary file.

| Storage error code | Meaning and next step |
| --- | --- |
| `storage_conflict` | Destination differs from the session's original baseline. Prior editing state/history/revision is restored; inspect its new context, then keep the work through Save As or reopen deliberately. |
| `storage_busy` | Another cooperating writer holds the ordinary write lock. Prior editing state is restored with a new context; inspect before retrying. |
| `storage_unsaved` | Restored work has no ordinary-file authority. Prior editing state is restored; restart using a create-only `--save-as` destination. |
| `storage` | Ordinary save failed. Editing state is restored with a new context; inspect and check disk before deciding what to save. |
| `recovery_busy` / `recovery_failed` | The ordinary project was saved, but its session history could not be written. The committed project stays adopted; these are not rolled-back edits. |

Read the error before assuming durability. Directory-sync failure can occur
after atomic replacement, so a generic storage error does not prove unchanged
disk bytes. Protected save arbitration covers cooperating writers; a program
ignoring the advisory `.write-lock` can race the final check and rename. See
[architecture](architecture.md#persistence-and-source-media) and
[recovery binding](adr/0005-bind-recovery-to-the-original-document.md).

## Live desktop sessions

The native app owns an `AgentHost` and publishes a session descriptor for its
local transport. The descriptor contains the loopback address, protocol
version, instance ID and session token. Connect to the descriptor path shown
by the desktop or supplied for an isolated verification run:

```sh
target/debug/aster-agent --connect /absolute/session.json
```

The descriptor is session-specific and contains a capability token; keep it
private. The host removes only descriptors belonging to its own instance.
Native permission behavior and descriptor discovery still require the platform
acceptance in [verification](verification.md).

## Requests and results

The CLI reads one JSON request per line and writes one JSON response per line.
Requests must end in a newline and fit within 1 MiB. Start with an inspection:

```json
{"request_id":"inspect-1","operation":"inspect"}
```

The response includes `revision` and a `result` tagged by `type`. Inspection
returns the current project, transient `context_id`, dirty flag and undo/redo
availability. Use the inspected revision, project ID and context ID for a
mutation (replace the example IDs with `result.project.id` and
`result.context_id` from inspection):

```json
{"request_id":"edit-1","operation":"apply","expected_revision":0,"expected_project_id":"00000000-0000-0000-0000-000000000000","expected_context_id":"00000000-0000-0000-0000-000000000000","command":{"type":"rename_project","name":"Film"}}
```

Mutations without `expected_revision` return `revision_required`; mismatches
return `stale_revision`. Missing project identity returns `project_required`;
an ID belonging to a project the desktop replaced returns `stale_project`.
Missing context identity returns `context_required`; a context from a previous
open/recovery returns `stale_context`, even when the same project has reopened
at the same revision. Invalid edits return `invalid_edit` and leave the document
unchanged. `undo` and `redo` also require all three guards.
`capabilities` returns the protocol version, every operation name in
`operations` and the supported command names. Fetch the exact serialized
command JSON schema in the same result's `command_schema`:

```json
{"request_id":"schema-1","operation":"capabilities"}
```

The current full schema is generated from `Command` by
[command_schema](../crates/editor-agent/src/planner.rs); use it or the crate's
serialized types for exact fields, optional values and nested batches. Full
protocol clients may submit `AddAssets` and `RestoreProject`; the native model's
`proposal_schema` excludes both. An external agent still needs valid asset
metadata and core preconditions; exposing a command does not bypass validation
or grant filesystem/provider tools.

### Look, motion and speed commands

These commands replace a whole value, so resend what should stay. Bounds are
enforced by `Project::validate`; a refused command leaves the document unchanged.

| Command | Fields | Rule |
| --- | --- | --- |
| `set_effects` | `clip_id`, `effects` | At most 16 effects, in render order. Keeping an effect's `id` keeps its keyframes; tracks of a removed effect are dropped. |
| `set_masks` | `clip_id`, `masks` | At most 8 masks; same ID rule. |
| `set_blend_mode` | `clip_id`, `mode` | |
| `set_keyframes` | `clip_id`, `target`, `key`, `keys` | Replaces one track; empty `keys` removes it. At most 256 keys per track and 64 tracks per clip. |
| `set_retime` | `clip_id`, `retime`, `duration` | 2–64 keys; follows links. A linked clip of another length is scaled in proportion and a linked text clip only changes length. `retime: null` restores constant speed. |
| `set_transition` | `clip_id`, `edge`, `transition` | `edge` is `in` or `out`; `null` removes. One canvas frame to `min(clip duration, 10 s)`. |
| `apply_preset` | `clip_ids`, `preset`, `strength` | Gives each clip the look as ordinary effects with new IDs, in place of the look it has; `strength` is 0–1. |
| `set_tracker` / `remove_tracker` | `tracker` / `tracker_id` | Upsert by `tracker.id`; at most 64 trackers of 1–2048 keys. Removal clears every `follow` naming it. |
| `set_follow` | `clip_id`, `follow` | The tracker must exist; `null` clears. |

- **Keyframe time** is `at`, integer microseconds from the clip's start, strictly
  increasing within `0..=duration`. Moving a clip leaves it unchanged.
- **Targets.** `target: null` addresses the clip: `position_x`, `position_y`,
  `scale_x`, `scale_y`, `rotation`, `opacity`, the four `crop_*` keys and `gain`.
  Otherwise `target` is the ID of an effect or mask on the same clip.
- **Ease** shapes the segment leaving its key: `hold`, `linear`, or `bezier`
  with `x1`, `x2` in 0–1 and `y1`, `y2` in -4–4 (0–1 inside a retime).
- **Effect parameters** are `{key, value}` pairs checked against the kind's
  table in [look.rs](../crates/editor-core/src/look.rs). A key foreign to the
  kind is invalid, a missing key takes its default and `mix` is 0–1. Lengths are
  fractions of the layer box or canvas height, angles are degrees. `points` is
  for `curve_*` kinds only (2–16 `[x, y]` pairs in 0–1, `x` increasing);
  `asset_id` is required for `lut` and `difference_matte` and null elsewhere.
- **Retime keys** map clip time `at` (first 0, last `duration`) to `source`,
  microseconds after `source_in`. `set_retime` multiplies each source by the
  clip's current `playback_rate` and sets the rate to 1.0, so on a clip already
  at rate 1.0 sources are plain source offsets. Rising sources play forward,
  falling sources reverse, equal sources freeze and an eased segment ramps.
  Each segment's mean speed is 0 or within 0.05–16.
- **A clip that has a retime** keeps `playback_rate` 1.0. `trim_clip` reads the
  change of `source_in` as timeline microseconds cut from the head, crops the
  map and cannot extend past it. `set_playback_rate` multiplies the map's
  current speed by `rate`, so sending the same rate twice applies it twice.
  Cutting inside an eased segment may add keys so the curve is kept; a cut that
  would leave a part outside the speed bounds is refused.
- **Keyframes follow a retime.** `set_retime` moves the clip's keyframes to
  where the new map shows the source moment each was on: a freeze shifts later
  keys by its length and a reverse mirrors them. A moment shown twice takes
  the showing nearest the key's old time, and one never shown the map key
  nearest in source. A linked text clip's keys scale with its length.
- **Looks.** A clip holds one look, read back from its effects: the first run
  of effects equal to a preset at a strength of 0.01 or more. `apply_preset`
  replaces that run where it stands and appends when there is none, so a look
  applied at strength 0 cannot be found again and the next one is appended.
- **Transitions** belong to one clip. `in` on the clip that starts at a cut
  pairs with the same-track clip ending there; `out` applies only when no clip
  abuts.
- **Trackers** bind a video or image asset. Each key has `source` (asset
  microseconds, strictly increasing), `center` (`[x, y]` fractions of the frame
  from its top left), `scale` (0.001–100) and `rotation` (degrees).
  `remove_assets` refuses an asset still used by an effect or tracker.
- **Text clips** accept transform and gain keyframes, transitions and follow;
  effects, masks, retime and a non-`normal` blend are refused.
- **Clips on audio tracks** accept audio effects, gain keyframes and retime
  only. A sound cannot be a `lut`, a `difference_matte` plate or a tracker's
  asset.
- **Transitions and the frame rate.** `set_settings` with a lower frame rate
  lengthens a transition shorter than the new frame to one frame.

- **Analysis is not a command.** Region tracking and stabilization analysis
  run in the desktop's analysis lane and have no protocol operation. A client
  can write a tracker's keys with `set_tracker` and can add a `stabilize`
  effect with `set_effects`, but cannot start either measurement; a
  `stabilize` effect draws uncorrected until the person analyses the video.
  Stabilization data is not part of the project a client inspects.

### Sound commands

| Command | Fields | Rule |
| --- | --- | --- |
| `set_fades` | `clip_id`, `fade_in`, `fade_out` | Replaces both lengths; 0 removes a fade. A clip on a locked track refuses it. Linked clips are not changed: each clip has its own fades. |
| `set_track_state` | `track_id` and any of `locked`, `hidden`, `muted`, `name`, `gain`, `solo` | An omitted or `null` field is left as it is. Works on a locked track. |

- **Fades** are timeline lengths in integer microseconds, `fade_in` from the
  clip's start and `fade_out` to its end, each 0 or more and together at most
  the clip's `duration`. They are as long as they are heard, whatever the
  clip's speed. Only a media clip whose asset has sound (`has_audio`, or kind
  `audio`) takes one; a title, a still or a silent video is refused. The
  curve is fixed.
- **Fades follow the clip's length.** A trim, a rate change or a retime that
  shortens a clip cuts its fades to fit, `fade_out` to what `fade_in` leaves.
  `split_clip` leaves `fade_in` on the left part and `fade_out` on the right.
- **Track volume** is `gain`, linear and finite within 0–8, where 1 leaves the
  track as it is. It multiplies the volume of every clip on the track.
- **Solo.** While any video or audio track has `solo`, only soloed tracks
  sound; `muted` wins over `solo`, and a text track's `solo` is ignored.
- **Defaults are not written.** An inspected project omits `gain` at 1, `solo`
  when false and a fade of 0.
- **Audio effects** are seven effect kinds carried by `set_effects`, legal
  only on a clip on an audio track; a clip on a video track refuses them, as
  a clip on an audio track refuses every other kind. Each has `mix` 1, no
  `points` and a null `asset_id`, and none of its parameters takes keyframes.
  A clip holds at most 16.

  | Kind | Parameters (range, default) |
  | --- | --- |
  | `high_pass` | `frequency` in Hz (20–2000, 80) |
  | `eq_band` | `frequency` in Hz (20–20000, 1000), `gain` in dB (-24–24, 0), `width` in octaves (0.1–4, 1) |
  | `compressor` | `threshold` in dB (-60–0, -18), `amount` as the ratio (1–20, 3), `gain` as make-up dB (0–24, 0) |
  | `limiter` | `threshold` as the ceiling in dB (-24–0, -1) |
  | `denoise` | `amount` (0–1, 0.4), `threshold` as the noise floor in dB (-80 to -20, -40) |
  | `pitch` | `amount` in semitones (-12–12, 0) |
  | `center_cut` | `amount` (0–1, 1) |

  The ranges and defaults are first guesses. Enabled audio effects are heard,
  in stack order, through the clip's own decoder chain; a kind the installed
  FFmpeg lacks is left out and that is said once.
- **Keep pitch.** A clip on an audio track with an enabled `pitch` effect,
  `amount` 0 included, keeps its pitch when its speed changes, and that
  `amount` is its only shift; without one, speed changes pitch. Only the
  first enabled `pitch` is read; later ones are legal and have no effect.
  Pitch is held while the clip's speed divided by `2^(amount/12)` is within
  0.5–2; past that the remainder is heard as a change of pitch. To keep
  pitch, write a `pitch` at 0 with `set_effects`; to let speed change it
  again, remove every enabled `pitch`.
- **Sound work is not a command.** Measuring loudness, ducking under other
  clips, finding beats, recording a voiceover, saving a sound to a file and
  separating stems run in the desktop on the person's request and have no
  protocol operation. Their results are ordinary commands a client can read,
  undo and write itself: a normalized volume is `set_gain` (or a scaled
  `set_keyframes`), a duck is `set_keyframes` on `gain`, beats a person marked
  are `add_marker`s labelled `Beat`, a take is a `batch` of `add_assets`,
  `add_track` when no audio track has room, and `add_clip`, and stems are a
  `batch` of `add_assets`, `add_track` and `add_clip` for each stem,
  `link_clips`, and `set_gain` 0 (with `set_keyframes` emptying its volume
  keys) on the original. Detach audio is a `batch` of `add_track` when needed,
  `add_clip` and `link_clips`; background music is `add_track` and `add_clip`s
  whose last carries its `fade_out`. Peaks and beat grids are not part of the
  project a client inspects.

### Title, caption and graphic commands

A title is a clip whose content is `{"type": "text", ...}`; a graphic's is
`{"type": "graphic", "size": [w, h], "elements": [...]}`. Add either with
`add_clip` on a text track. A text track with a `language` is a caption track,
and each title on it is one caption.

| Command | Fields | Rule |
| --- | --- | --- |
| `set_text` | `clip_id`, `text`, `font_size`, `color` | Keeps the style and animations. Word times are realigned to the new words: kept words keep their times and new ones share the time between their neighbours. |
| `set_text_style` | `clip_id`, `style` | Titles only. Replaces the whole style. |
| `set_text_animation` | `clip_id`, `edge`, `animation` | `edge` is `in` or `out`; `animation: null` removes it. |
| `set_words` | `clip_id`, `words` | One `{at, end}` per word of the text, or `[]`. At most 4096. |
| `set_graphic` | `clip_id`, `size`, `elements` | Graphics only. Replaces the design box and every element. |
| `set_track_language` | `track_id`, `language` | Text tracks only; `null` makes it a plain text track. |
| `copy_captions` | `from_track_id`, `track_id`, `name`, `language`, `texts` | Adds caption track `track_id` directly above `from_track_id` with one title per source title, in timeline order, fresh IDs and the source's times, and each text replaced by the one at its index; the source is hidden. `texts` has exactly one entry per clip of the source, which must hold only titles. Word times are spread over the new words by length. |

- **Styles** name a font `family`, `weight` (100–900) and `italic`, never a
  file. `outline.width`, `shadow.offset`, `shadow.blur`, `background.padding`
  and `background.radius` are fractions of the font size. `wrap` is canvas
  pixels and 0 means no wrap. `highlight` (karaoke) has `mode` `word`, `box` or
  `fill`, a `color`, `scale` 1–2 and `upcoming` opacity 0–1.
- **Animations** are `{kind, unit, duration, delay}`. `kind` is `fade`, `pop`,
  `bounce`, `typewriter`, `slide_up`, `slide_down`, `slide_left`,
  `slide_right` or `wipe`; `unit` is `whole`, `line`, `word` or `letter`.
  `duration` is one canvas frame to `min(10 s, clip duration)`, `delay +
  duration` fits the clip, and an entrance and exit may not overlap.
- **Word times** are microseconds from the clip's start, with
  `0 <= at <= end <= duration` and `at` never decreasing. A word is a run
  between spaces, and each Han, Hiragana or Katakana character is a word.
- **Graphics** hold 1–32 elements with unique IDs. An element's `kind` is
  `text`, `rect`, `ellipse` or `line`, with a `frame` of `[left, top, width,
  height]` in design pixels from the box's top left, `rotation` in degrees,
  `opacity` and its own `enter` and `exit`. A line runs from the frame's top
  left to its bottom right, and either side may be negative.
- **Split, trim and rate.** `split_clip` on a title with words gives the words
  before the cut, and their text, to the left and the rest to the right; a
  title without words keeps its whole text on both sides. The entrance goes
  to the left and the exit to the right.
  `trim_clip` keeps every word and holds times and animations inside the clip.
  `set_playback_rate` scales word times and animations with the duration.
- **The native planner** offers a narrower set. Its `proposal_schema` leaves out
  `set_text_style`, `set_words` and `set_graphic`, and a proposed title has no
  style, animation or words, because styles and graphics are designed by hand,
  words come from speech and the schema shares the provider's command line
  with the planner text. Full protocol clients may send every command above.
- **Fonts** are assets of kind `font`, imported like media and never placed on
  a track. Captions from speech and subtitle files have no protocol operation:
  a client can write captions and their word times with the commands above.

Use a `batch` command to apply a multi-step edit as one all-or-nothing history
entry. Capture stable asset, track and clip IDs from inspection instead of
retargeting from whichever item the desktop currently selects.

## Render jobs

Export is not an edit. It has no command and no undo entry, and it never
changes the project. The desktop's render queue renders each job from a
snapshot of the project taken when the job was queued, so later edits do not
change a queued or running job. Only a live desktop session runs render
jobs: a headless `aster-agent` answers every operation below with
`unsupported`, so connect with `--connect`.

| Operation | Fields | Guards | Result |
| --- | --- | --- | --- |
| `export_capabilities` | | none | `export_capabilities` |
| `queue_export` | `settings`, `destination`, `overwrite` (default `false`) | revision, project and context | `render_job` |
| `render_jobs` | | none | `render_jobs` |
| `pause_render`, `resume_render`, `cancel_render`, `retry_render`, `remove_render` | `job` | none | `render_job` |

- **`export_capabilities`** returns `capabilities` (the FFmpeg version, the
  video and audio encoders that encoded a test picture on this host, the
  containers its muxers can write, and encoders that failed with the reason),
  `presets` (built-in and saved) and `settings_schema`, the exact JSON schema
  of `settings`. Encoder detection runs once in the background after the
  desktop starts; until it ends this and `queue_export` answer
  `capabilities_pending`, so ask again after a moment.
- **`queue_export`** carries the three identity guards of a mutation and
  answers with the same `revision_required`, `stale_revision`,
  `project_required`, `stale_project`, `context_required` and
  `stale_context` errors. The job renders the project at the inspected
  revision. `destination` is an absolute path to a file in an existing
  folder, with the container's extension, and not a source asset, the
  project file, or a file that another unfinished job writes. Names that
  differ only in case are the same file on every platform. An existing file
  is replaced only with `"overwrite": true`, and so is an existing caption
  file the settings would write beside it; the old files stay whole until
  the finished export is renamed over them. These checks compare names when
  the job is queued. A clash found only by resolving paths (a symbolic link,
  a hard link, `..` through a link), or a destination folder removed (or a
  folder made at the destination) after queueing, makes the queued job fail
  instead, with the same reason in its `message`.
- **Job operations** name a `job` from `render_job` or `render_jobs` and need
  no guards. Pausing a running job stops feeding frames, and resume
  continues the same encode; until then it holds the queue. Pausing a queued
  job holds it: it reads `paused` with `frames_done` 0 and is passed over
  while later jobs render, and resume lets it take its turn. Cancel stops the
  job and removes its partial file. Retry re-runs a failed or cancelled job
  from its snapshot; it is refused while another unfinished job writes the
  same file. Remove drops a job that is not running and answers with it as
  it was. Finished jobs stay listed until removed. Reordering waiting jobs
  and clearing finished ones are in the window only; agents have no
  operation for them.
- **Progress is polled.** Every operation answers at once; nothing streams.
  Poll `render_jobs` about once a second and read each job's `state`
  (`queued`, `preparing`, `rendering`, `paused`, `finalizing`, `done`,
  `failed`, `cancelled`), `progress` (0–1), `frames_done`, `frames_total`,
  `speed_fps`, `eta_seconds`, `encoder` and `message`, which gives a failure's
  reason or a fallback from a hardware to the software encoder.

```json
{"request_id":"caps-1","operation":"export_capabilities"}
{"request_id":"export-1","operation":"queue_export","expected_revision":12,"expected_project_id":"00000000-0000-0000-0000-000000000000","expected_context_id":"00000000-0000-0000-0000-000000000000","settings":{"container":"mp4","video":{"codec":"h264","resolution":{"kind":"short","pixels":1080}},"audio":{"codec":"aac","kbps":256,"sample_rate":48000,"channels":2}},"destination":"/absolute/Film.mp4"}
{"request_id":"poll-1","operation":"render_jobs"}
{"request_id":"pause-1","operation":"pause_render","job":"00000000-0000-0000-0000-000000000000"}
```

| Render error code | Meaning and next step |
| --- | --- |
| `capabilities_pending` | Encoder detection has not finished; ask again shortly. |
| `invalid_export` | The settings contradict themselves or the limits, or the destination is relative, in a missing folder, a folder, a source or the project, has the wrong extension, or is written by another unfinished job ("Another export in the queue already writes <name>; choose another name"). The message names what to change. |
| `unsupported_export` | This host's FFmpeg cannot make what was asked; choose from `export_capabilities`. `export_capabilities` and `queue_export` also answer it when FFmpeg is missing or its encoder check failed, or the queue has stopped; the message says why, and nothing can be exported from this session until FFmpeg is fixed and checked again from the window's export dialog. |
| `empty_export` | The timeline or range holds nothing to export, or the caption track has no captions in it. |
| `destination_exists` | A file is at `destination`, or at the caption file the settings would write beside it; the message names it ("<path> exists; send overwrite: true to replace it"). Resend with `"overwrite": true` to replace them. |
| `queue_full` | The queue already lists 64 jobs; remove finished ones. |
| `unknown_job` | No job has that ID, or its state does not allow the operation (pausing a finished job, or retrying one whose file another unfinished job now writes). |
| `unsupported` | This session cannot run render jobs (headless). |

The desktop answers render operations through the same queued call as
edits. A `timeout` before acceptance means no job was queued or changed, and
it cannot start later; an `indeterminate` `queue_export` may have queued a
job, so list `render_jobs` before sending it again.

## Transport states

The [host](../crates/editor-agent/src/host.rs) distinguishes queued, executing,
cancelled and completed calls. A `timeout` before acceptance atomically cancels
the queued call; it cannot later begin execution. An `indeterminate` response
means acceptance occurred but its acknowledgement missed the deadline: inspect
before retrying, since the edit may have committed. A disconnected client also
needs inspection before repeating a mutation. These transport states are
separate from cancellation of a native provider planning request, which cannot
apply edits by itself.
