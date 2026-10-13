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
`capabilities` returns protocol version and supported command names. Fetch the
exact serialized command JSON schema in the same result's `command_schema`:

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
- **Clips on audio tracks** accept gain keyframes and retime only. A sound
  cannot be a `lut`, a `difference_matte` plate or a tracker's asset.
- **Transitions and the frame rate.** `set_settings` with a lower frame rate
  lengthens a transition shorter than the new frame to one frame.

- **Analysis is not a command.** Region tracking and stabilization analysis
  run in the desktop's analysis lane and have no protocol operation. A client
  can write a tracker's keys with `set_tracker` and can add a `stabilize`
  effect with `set_effects`, but cannot start either measurement; a
  `stabilize` effect draws uncorrected until the person analyses the video.
  Stabilization data is not part of the project a client inspects.

Use a `batch` command to apply a multi-step edit as one all-or-nothing history
entry. Capture stable asset, track and clip IDs from inspection instead of
retargeting from whichever item the desktop currently selects.

The [host](../crates/editor-agent/src/host.rs) distinguishes queued, executing,
cancelled and completed calls. A `timeout` before acceptance atomically cancels
the queued call; it cannot later begin execution. An `indeterminate` response
means acceptance occurred but its acknowledgement missed the deadline: inspect
before retrying, since the edit may have committed. A disconnected client also
needs inspection before repeating a mutation. These transport states are
separate from cancellation of a native provider planning request, which cannot
apply edits by itself.
