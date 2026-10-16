# Clip editing: report points 2 and 3

## Findings from the source

- Moving and trimming did not require delete/re-add. `MoveClips`, `TrimClip`
  and typed `clip_edit` already retain clip IDs. `TrimClip` already calls
  `crop_local`: animation curves are cropped with interpolated boundary keys,
  word times are held inside the new bounds, and text animations, transitions
  and fades are fitted. The report reveals a discovery problem here, rather
  than a missing trim implementation.
- An atomic in-place retarget/partial clip edit was missing. Replacing clips
  also exposes `DeleteClips` relationship expansion and new-object identity to
  callers that only meant to change one clip.
- Project validation named only error categories for animation keys, title
  animation, word times, effects and most clip data. This lost the owning
  clip, animation index and offending key/word.

## Implemented

`patch_clip` is a typed MCP tool backed by the new core `PatchClip` command.
It names exactly one existing clip, preserving identity and entering the same
validation, lock, overlap and undo boundary as other edits. Top-level fields
are `start`, `duration`, exact `source_in` and `asset_id` (including an
unambiguous asset prefix). `changes` accepts partial stored clip fields:
objects merge recursively, arrays replace and null removes optional fields.
Unknown fields, including nested misspellings and null removals, are refused.
The clip ID and link/group relationships are protected; dedicated commands
own relationship edits. Use the existing effect/keyframe tools for changes to
individual array entries.

`trim_animation: crop` (the default) crops existing timed data before explicit
changes, using the existing curve-preserving trim logic. Explicit replacement
keys and words must fit the new duration; they are never silently discarded.
`preserve` leaves existing timed data unchanged and validation rejects data
that no longer fits. Changing start/source-in assigns those fields directly;
only duration shortening invokes this tail-crop policy. Existing `clip_edit`
trimming remains the choice for source-aligned head trims and linked trims.

`offset_clips` accepts `clip_ids` and a signed `delta` TimeSpec. It builds the
existing `MoveClips` command, retaining its linked/grouped movement and lock
rules without serializing the clips. Microsecond/second deltas remain exact.

Clip-related Invalid errors now identify the clip. Animation failures name
the animation, target, parameter and offending key with time/value limits;
word failures name the index and bounds or the expected count. Text animation
failures identify the enter/exit field or overlapping spans. Invalid effects
and masks include the entry index and stable object ID. Missing-object and
locked-track enum errors retain their established protocol classification.

## Regression coverage and limits

`crates/editor-core/tests/patch.rs` covers exact identity, unspecified fields,
command serialization, undo/redo, curve samples and interpolated end keys,
strict preservation, explicit invalid replacement data, nested content/style,
array replacement, nested misspellings, protected fields, locks, source bounds,
atomic batch failure, existing trim behavior and contextual title errors.

`crates/editor-store/tests/patch.rs` covers saved project and recovery
round-trips, retained clip identity and recovered patch undo/redo. Typed tool
unit tests cover signed and exact subframe offsets. Existing validation tests
compare stable error categories separately from their new contextual detail.

The root agent is the sole compilation owner and records actual results.
No desktop UI was changed or native acceptance claimed by this subtask.
