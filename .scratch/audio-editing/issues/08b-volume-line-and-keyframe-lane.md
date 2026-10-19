# Volume line on the tile and the keyframe lane

Status: ready-for-human
Progress: resolved
Type: task
Blocked by: 04, 05a

## Scope

The clip's volume visible and editable where the clip is.

- `src/ui/timeline.rs`: on an audio-lane tile, a line at the clip's volume
  drawn from `clip.gain` and its `Gain` track (sampled like the graph,
  `SAMPLE` points apart) on a scale where 100 % sits at two thirds of the
  tile's height and the top is 200 %. Dragging the line where it has no keys
  is one `SetGain`. Alt-click on it adds a key through `keyframes::command`
  (`src/ui/keyframes.rs:388`); the lane then opens.
- `src/ui/keyframes.rs` (`:616-629`): `toggle_lane` opens for a clip that
  `sounds` and has no animation yet, showing an empty Volume row, and a
  "Key volume" item in the clip menu writes a key at the playhead.

Files: `src/ui/timeline.rs`, `src/ui/keyframes.rs`, `src/ui/mod.rs`,
`docs/usage.md`, `docs/design.md`.

## Result

Implemented as scoped, apart from the native captures that need a hand on the
pointer. A clip's volume is reached where the clip is three ways, and each
ends in a command through `Action::Edit`, so each is validated by the project,
is one history entry and undoes: a drag of the line (`SetGain`), Alt-click on
the line and "Key volume" in the clip menu (a `SetKeyframes` in a batch named
"Key volume", made by `keyframes::command`). The lane is opened by U, the tool
strip's toggle and "Keyframes" in the project menu, which all call
`toggle_lane`. The inspector's Volume field and diamond and the serialized
`set_gain` and `set_keyframes` are older and untouched; no shortcut and no
phrase is added, the ticket names none; save and load are fields that exist.

- `src/ui/keyframes.rs`: `takes_volume` (the clip's media has sound and no
  linked audio clip owns it); `lane_tracks`, the rows of a lane, which for a
  clip without animation that takes volume is one empty Volume row;
  `toggle_lane`, `can_toggle_lane`, `lane_rows`, `begin`, `show_lane` and
  `lane_heads` go by those rows. `key_volume(clip, local)` writes one key at
  the volume the clip has there; `volume_key(project, clip, playhead)` is the
  menu's, with its refusals in words. `SAMPLE` is shared with the timeline.
- `src/ui/timeline.rs`: `volume_level` and `volume_at_level` (the scale),
  `volume_line` (two points without keys; with them the track's evaluation a
  point every `SAMPLE` of width and one at each key, over the part in view),
  `paint_volume`, `volume_grip` (where a press is on the line),
  `Drag::Volume`, the Alt-click arm, the "Key volume" row, the cursor.
- `src/app.rs`: one sentence, what "Keyframes" says when there is no lane to
  open. Not in the ticket's file list. `src/ui/mod.rs` needed no change.
- `docs/usage.md`, `docs/design.md`.

Settled while building, beyond what the scope says:

- **The lane and "Key volume" go by the inspector's rule, not by `sounds`.**
  The ticket says the lane opens for a clip that `sounds`. `sounds` is false
  for a clip at volume 0 without keys and on a muted or soloed-out track,
  and keying the volume is how such a clip is brought back in. The row is
  offered as ticket 07 offers the Volume and fade rows: the media has sound
  and no linked audio clip owns it. A still, a title, a silent video and an
  owned picture have none.
- "The tile's height" is the height of the tile's sound, under the name row,
  where the waveform and the fades are drawn. 100% is at two thirds of it
  and 200% at its top; the scale is two straight pieces, 0 to 100% under and
  100 to 200% over.
- The line is taken hold of only on a selected tile, within 4 points of it,
  between the trim grips, on tiles from 40 points wide, on a track that is
  not locked and without Shift or Command. A press on an unselected clip is
  the clip's, as before. On a selected tile a strip 8 points high no longer
  moves the clip.
- The line follows the travel of the pointer from where it stood, after 3
  points of travel, and lands on a whole percent. A line above 200% is drawn
  at the top and comes down from there. A drag reaches 0 to 200%; the bound
  0..=8 is the core's and is applied too. A drag that ends where the volume
  was, and a press that does not travel, are no entry. A line that stands
  where the stored volume is drawn keeps that volume (changed after review):
  lifting a line above 200% does not turn it down to 200%, and a volume
  finer than a whole percent is not rounded by a drag that comes back.
- A drag is given up on release when the volume was keyed meanwhile; a track
  locked meanwhile has the release refused by the project.
- Alt-click works on an unselected clip too and selects it. The key is at
  the frame nearest the pointer, held to the clip, at the volume the clip
  has there (the stored one, or the track's value). The first key of a clip
  is linear. One among other keys is put on the curve by the core's
  `AnimTrack::insert` (changed after review): the stretch it lands in is
  divided as a split divides it, so a held stretch stays held and an eased
  one keeps its shape, with further keys where the subdivision needs them.
  Where a key already is, nothing is written; the lane opens all the same.
- "Key volume" acts on the clip the menu was opened on, a picture clip with
  sound too. It is shown and disabled where it would write nothing: playhead
  off the clip, a key already there, a locked track, an owned picture.
- The tile carries the lane's chevron while its empty row is open, to close
  it; a selected clip without keyframes has no chevron, as before.
- A clip whose last key is deleted keeps an open lane with the empty row if
  it takes volume. The empty row takes a paste of copied keyframes.
- A volume line follows keys held in the lane or the graph while they are
  dragged. Volume keys are a 2-point dot each on the line.
- The graph editor is unchanged: it says a clip without keyframes has none.

## Evidence

On macOS 26.5.2 (arm64), Rust 1.97.1, Homebrew FFmpeg and ffprobe 8.1.2
(`scripts/check-media-tools.py` passed), 2026-10-13. No test was skipped.

- `cargo test --locked --bin neptune-video`: 679 passed. `cargo test --locked --bin
  neptune-video -- volume:: keyframes::`: 53 passed. `cargo test -p editor-core
  --locked` passed, with `cargo clippy -p editor-core --all-targets`.
  `cargo clippy --locked --all-targets -- -D warnings` and `cargo fmt --all
  -- --check` are clean; `cargo check --locked --workspace` and
  `scripts/check-architecture.py` pass. After review `editor-core` gained
  `AnimTrack::insert`; no other crate was changed.
- `src/ui/volume_tests.rs`, thirteen tests, eleven through one real egui pass of
  the timeline a frame: the scale and its inverse, and a dragged volume
  inside 0..=8 for any travel; the line's points without keys, pinned above
  200%, with keys equal to `gain_at` with a corner at each key and a held
  step, and only the part in view; a drag edits nothing while held and is
  one "Change clip volume" that undoes; it stops at 0 and at 200% and a
  line at 800% comes down from the top; a press that does not travel is no
  entry; Escape leaves the project equal to what it was and keeps the clip
  selected; a keyed clip's line is not dragged (the press holds the clip);
  no line is held on an unselected tile, in a trim grip, on a picture tile,
  with Shift, or on a locked track, where Alt-click writes nothing either; a
  track locked while the line is held has the release refused; a line keyed
  while held writes no volume; Alt-click on an unselected clip is one "Key
  volume" at the nearest frame with the clip's volume, opens the lane, and
  on a bent line takes the value there; the menu row keys at the playhead
  and opens the lane, for a picture with sound too, is disabled off the
  clip, where keyed and on an owned picture, and is absent on a title.
- `src/ui/keyframes_tests.rs`, four tests: an audio clip with no keys opens
  one empty Volume row by U and by the tool strip with no edit made, carries
  the chevron while open, keeps the row when its last key is deleted and
  takes a paste; a picture with sound opens it and keeps it muted at volume
  0; a still, a silent video and an owned picture do not, the owning audio
  clip does, and the picture does again once unlinked; `volume_key`'s value
  and its five refusals in words, and the project's own refusal on a locked
  track.
- Native captures of the release build (`target/release/neptune-video`, sha256
  `1fbc4e6ca91456c1…`, built from the final source), each run with its own
  settings, session, cache and project under
  `artifacts/audio-editing/08b-volume-line/<run>/`, on ticket 04's owned
  fixtures with volumes and volume keys written into the project file:
  `rest-normal` (asked for at 1440x900) and `rest-narrow` (760x620). Both
  were looked at. `rest-normal` shows a level line at 100% over a silent
  sound, a line at 40% under two fade curves, and a keyed line with its
  five dots, a held stretch and a step. `rest-narrow` shows the first and
  the top of the other two; its third audio track is below the window in
  both. Both processes exited by themselves; none was left.

Not done, and why this is not resolved: nothing was clicked, dragged or typed
in the running build, so **no capture shows the empty Volume row**, a line
held mid-drag with its percent, a selected tile with its fade handles beside
the line, or the "Key volume" row. Unseen with them: whether the line is told
apart from a fade's curve, which is in the same ink and half a point wider;
the line over a loud waveform; the light theme; Linux and Windows. Nothing
was listened to: that a volume or a volume key is heard is the mixer's
readback of earlier tickets.

For a person, at 1440x900 and 760x620: select an audio clip without keys and
press U, and capture the empty Volume row; hold its line mid-drag and capture
it; Alt-click the line and capture the lane with its key.

Done when:

- Line tests: a drag is one `SetGain` held to 0..=8; Alt-click writes one key
  at the frame under the pointer; a keyed clip's line is not draggable as a
  whole; Escape gives up a drag.
- Lane test: an audio clip with no keys opens an empty Volume row; a still
  and an owned picture clip do not.
- Native captures, normal and narrow: the line with and without keys, the
  empty Volume row.

## Known limits

- A volume key written in an eased stretch replaces the stretch's easing
  with those of its two parts, and a stretch whose easing leaves 0..=1 can
  take more than one key; samples agree to 2e-4. A value the curve carries
  outside 0..=8 is clamped at the key, as at a split.

- The line shows up to 200 %; a higher volume pins to the top and is read
  from the inspector.
- Percent, not dB.
- The line is on audio-lane tiles only.
- A drag of the line sets 0 to 200% in whole percent, about six percent to a
  point on a 44-point lane; finer and louder is the inspector's field.
- The line is dragged on a selected tile only, and has no keyboard route of
  its own: the inspector's Volume field and "Key volume" are that route.
- "Key volume" has no shortcut and no phrase.
- The sound follows a drag when it is let go, not while it is held.

## Comments

2026-10-13 (review): Split from the former ticket 08. It no longer waits on
the mixer (ticket 06): it writes volume, which is heard today.

2026-10-13 (implemented): Built and tested headless on macOS 26.5.2 (arm64,
Rust 1.97.1); two native captures show the line at rest with and without
keys, and the captures that need a selection, a held drag or an open lane
are still to be taken. Nothing was listened to. The lane and "Key volume"
are offered by sound and ownership, not by `sounds`: details are under
Result and Evidence.

2026-10-13 (after review): Two findings held and are fixed. A volume key
was always linear, so one written in a held or eased stretch changed what
was heard after it (a step became a ramp); it is now put on the curve by a
new `AnimTrack::insert` in `editor-core`, which reuses the subdivision of
split and trim. A line above 200%, or at a volume finer than a whole
percent, was rewritten by a drag that lifted it or came back to where it
started (800% became 200%); the line now keeps the stored volume wherever
it stands where that volume is drawn. Each fix has tests that fail on the
earlier code: `a_key_inserted_on_the_curve_keeps_every_sampled_value`
(`crates/editor-core/tests/advanced_commands.rs`), the held and eased cases
of the Alt-click and `volume_key` tests, and
`a_line_left_where_it_was_taken_keeps_a_volume_it_cannot_show`. "Key volume"
on a stretch that cannot be carried by keys is refused with "The volume's
curve cannot take a key there."; no test reaches that refusal. On the same
host: `cargo test --locked --bin neptune-video` 679 passed, `-- volume::
keyframes::` 53 passed, `cargo test -p editor-core --locked` passed.
Nothing native was captured or listened to for these fixes.

2026-10-14 (native acceptance): Resolved. The captures this ticket waited for are taken and look right. Release build `target/release/neptune-video` (sha256 fadbe5aa8244d800b2d9b16bf86bf121a6e9157af7e8067402761d40318e5763, `cargo build --release --locked`) on macOS 26.5.2 (25F84), Apple M1, Rust 1.97.1, Homebrew FFmpeg 8.1.2, built-in Retina display at scale 2. Fresh task-owned projects (made by `neptune-video-agent` over ffmpeg-generated media: a 12 s testsrc2 interview with a speech-like tone, 30 s of 120 BPM clicks, a 0.8 s whoosh) and fresh `--settings` per run. Driven by real pointer and key events into the running window; the normal window (asked 1440x900) came up at 1280x709 points, the narrow one at 760x620 (and 760x560 for ticket 13). Captures are in `artifacts/audio-editing/captures/`. Every capture was looked at against `docs/design.md`. Dark theme only; Linux, Windows and the light theme were not run; nothing was listened to.

`normal-02`/`narrow-02` the line without keys on a selected tile beside its fade handles; `normal-05`/`narrow-05` the line held mid-drag with its percent; `normal-06`/`narrow-06` U on a clip with no keys, the empty Volume row; `normal-07`/`narrow-07` Alt-click on the line, a key dot on it and a diamond in the lane; `normal-08`/`narrow-08` the Key volume row. Seen and left: the held line's percent is drawn at the tile's left start and overlaps a fade-in curve there (`paint_volume`, `first.x + 6`); it stays legible.

2026-10-14 (fix): The held percent's overlap with the fade in is fixed: it
is said beside the point taken hold of and clear of both fades' curves and
handles, or in the name's row where fades leave no room. See the spec's
Comments of the same date; captures `fix-normal-02`, `fix-normal-03`,
`fix-narrow-02`, `fix-narrow-03` of the rebuilt release.
