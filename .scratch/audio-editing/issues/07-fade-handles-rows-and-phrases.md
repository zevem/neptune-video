# Fade handles, fade rows and phrases

Status: ready-for-human
Progress: resolved
Type: task
Blocked by: 05a, 06

## Scope

Three routes to a fade: the tile, the inspector and the command panel.

- `src/ui/timeline.rs`: `Drag::Fade { clip, edge, duration }`, a sibling of
  `Drag::Transition` (`:1638-1658`). On an audio-lane tile a handle sits at
  each top corner, placed with `marking`/`mark_handle` (`:1160-1180`) so it
  stays clear of the trim grips; `Hit::Clip` gains the fade edge. The drag is
  a preview that becomes one `SetFades` on release: snapped when snapping is
  on, quantized to frames, held to `0..=duration - other fade`. Escape gives
  up the drag. The fade is painted as its curve from `fade_at` over the
  waveform. Handles show on a selected or hovered tile wide enough for them.
- `src/ui/inspector/clip.rs`, `Pane::sound` (`:1037`) and the Sound rows of
  the Video section (`:986-989`): "Fade in" and "Fade out" fields in seconds,
  through `ClipDraft` (`src/ui/mod.rs` `:316`, `commands` at `:451`), one
  entry per committed change. The rows are offered by link ownership, not
  by the asset alone: a picture clip whose sound is owned by its linked
  audio-track clip (the pair Detach audio makes) shows one line, "Sound is
  on the linked audio clip", in place of its volume and fade rows, because
  edits there would enter history and never be heard. A clip that is only
  muted, soloed out or at volume 0 keeps its rows.
- `src/agent.rs`: `fade in 1s`, `fade out 2s`, `remove fades` on the selected
  clips; `EXAMPLES` rows. An owned picture clip is refused in the same
  words, as is a clip without sound.
- The clip context menu: "Fade in 0.5 s", "Fade out 0.5 s", "Remove fades",
  the keyboard-reachable route.

Files: `src/ui/timeline.rs`, `src/ui/inspector/clip.rs`, `src/ui/mod.rs`,
`src/app.rs`, `src/agent.rs`, `docs/usage.md`, `docs/design.md`.

## Result

Implemented as scoped, apart from the native captures that need a hand on the
pointer. A fade is reached four ways, and each ends in `SetFades` through
`Action::Edit`, so each is validated by the project, is one history entry and
undoes: the handle on the tile, the inspector's fields, the clip menu and the
command panel. The serialized `set_fades` is ticket 05c's and is untouched;
no shortcut is added; save and load are the fields of 05a and 05b.

- `src/ui/fades.rs` (new): what the four routes share, and nothing that
  draws. `FadeEdge`, `room` (the clip less its other fade), `refusal`,
  `set_command`, `command(project, clips, Asked)` for the menu and the
  phrases, `MENU`, `QUICK` (half a second) and `OWNED`, the one line.
- `src/ui/timeline.rs`: `Drag::Fade { clip, edge, duration }`;
  `fade_handles` places both handles with `mark_handle`; `fade_curve` and
  `paint_fades` draw each fade from `sound::fade_at`, a point every two of
  width over the part in view, with what the fade takes away darkened above
  the line; the three menu rows. `sound_area` is the part of a sound tile
  under its name, which the waveform, the level line, the curve and the
  handles share.
- `src/ui/inspector/clip.rs`: `sound_rows` (volume, then `fade_row` for each
  edge, or the one line) in `Pane::sound` and in the Sound card of the Video
  section. `src/ui/mod.rs`: `ClipDraft` holds `fade_in` and `fade_out` and
  asks for `SetFades` before a change of speed.
- `src/agent.rs`: `fade in 1s`, `fade out 2s`, `remove fades` and an
  `EXAMPLES` row (23 rows).
- `docs/usage.md`, `docs/design.md`. `src/app.rs` needed no change: an
  `Action::Edit` commits held drafts first, and its Escape already gives up
  whatever the timeline holds through `cancel_drag`.

Settled while building, beyond what the scope says:

- "Each top corner" is each top corner of the tile's sound, under the name
  row, not of the tile: a handle in the name row would cover the name and
  the chevron of a keyframe lane, and a transition's marking keeps under the
  name for the same reason. A handle is 10 by 12 points and stands where
  its fade ends.
- A tile shows handles from 40 points wide. Fades that meet keep a handle
  each, side by side about where they meet.
- The handles are named widgets ("Fade in of voice.wav"), as a transition's
  handle is, and claim their press before the clip does. `Hit::Clip`
  therefore did not gain a fade edge: nothing would read it.
- A fade follows the travel of its handle: its end moves as far as the
  pointer has since the press, and lands on a frame or a snap. A handle
  stands clear of the trim grips, so the handle of no fade or of a short one
  is not where the fade ends, and a fade that went to the pointer jumped by
  that distance when taken hold of. A handle brought back to where it was
  taken hold of leaves the fade it has, on a frame or not. Taking hold of
  one selects its clip. The held length is written beside the handle.
- A drag that ends where the fade already was is no history entry.
- The menu rows and the phrases act on the selected clips, each held to its
  own room, as one step. Clips that take no fade are passed over; when none
  is changed the reason is said: "That clip has no sound to fade.", "Sound
  is on the linked audio clip.", "Unlock the clip's track to fade its
  sound.", "The clip's other fade leaves no room for that one.", "That fade
  is already there.", "There are no fades to remove.". The menu shows the
  rows for a clip whose media has sound and disables a row that would
  change nothing.
- `fade in` and `fade out` without a length are half a second, `for` may
  precede the length, and a length of 0 removes that fade.
- The inspector's fields are not quantized to frames: what is typed is what
  is stored, held to the room. A field reads hundredths of a second and a
  fade may be held finer, as one a handle left on a frame is: a field
  focused and left, or typed to what it already reads, changes nothing and
  is no history entry.
- The rows are not added to the Clip part of the project menu, where Detach
  audio also is: the ticket names the clip menu only.

## Evidence

On macOS 26.5.2 (arm64), Rust 1.97.1, Homebrew FFmpeg and ffprobe 8.1.2
(`scripts/check-media-tools.py` passed), 2026-10-13. No test was skipped.

- `cargo test --locked --bin aster`: 642 passed. `cargo test --locked --bin
  aster -- fading:: fades:: inspector::tests agent::tests`: 142 passed, three
  runs in a row. `cargo clippy --locked --all-targets -- -D warnings` and
  `cargo fmt -- --check` on the desktop crate are clean;
  `scripts/check-architecture.py` passes. No other crate was changed.
- `src/ui/fades_tests.rs`, ten tests through one real egui pass of the
  timeline a frame, the handles and menu rows found by their accessible
  names: a handle drag draws the handle and the curve where it is held,
  edits nothing until release and is then one "Change fades" entry that
  undoes; Escape mid-drag leaves the project equal to what it was and keeps
  the clip selected; a drag stops at the clip, at nothing and at the other
  fade, and one that changes nothing is no entry; a fade lands on a frame
  and, with snapping on, on a playhead that is on no frame; a locked track
  shows no handle and a drag from where it stood moves nothing, and a track
  locked while a fade is held has the release refused by the project;
  handles are on a selected sound tile and on the one under the pointer
  (and stay while the pointer is on a handle), not on a picture tile and
  not under 40 points; over tile widths from 2 to 640 points in tenths and
  81 pairs of fades the handles are absent under 40 points and otherwise
  never in a trim grip (checked against `hit`), never over one another and
  inside the sound area; the curve's points equal `fade_at`, the root of a
  half midway, and only the part in view is worked out; the menu rows fade
  one and several clips in one step, are disabled where nothing would
  change and on an owned picture, and are absent on a title.
- `src/ui/fades.rs`, five tests: the room, the refusals in words, several
  clips as one batch, an owned picture refused until unlinked though the
  project itself takes the command, a locked track refused by both.
- `src/ui/inspector/tests.rs`, three tests: a typed 99 is held to the two
  seconds of room and Return is one entry, an Escape gives up what was
  typed; a field drag is one entry on a sound and a typed value one on a
  picture with sound, and a still and a silent video have no rows; after
  Detach audio the picture shows the one line and no Volume, Animate volume,
  Fade in or Fade out while the audio clip shows them, after unlinking the
  picture shows them again, and at volume 0 on a muted track with another
  track soloed (`sounds` false) it keeps them.
- `src/agent.rs`: `the_sound_of_the_selected_clips_fades_by_name` (the three
  phrases, held to the room, refused in words on a title and on an owned
  picture, taken by the owning audio clip and by the picture once
  unlinked); `every_listed_example_is_understood` covers the new row.
- Native captures of the release build (`target/release/aster`, sha256
  `c2dab54de37e4a9b…`, built before `rustfmt` rewrapped five test asserts),
  each run with its own settings, session, cache and project under
  `artifacts/audio-editing/07-fades/<run>/`, on ticket 04's owned fixtures
  with fades written into the project file: `rest-normal` (asked for at
  1440x900) and `rest-narrow` (760x620). Both were looked at: the curves
  rise and fall over the level line of the video's sound and over the
  waveforms, a fade in beside a fade out, a fade in alone on the 2x clip.
  Both processes exited by themselves; none was left.

Not done, and why this is not resolved: nothing was clicked, dragged or typed
in the running build, so no capture shows a handle (a tile has one only when
selected or under the pointer), a drag in progress with its curve and length,
or the inspector's rows on an audio clip and on a video clip. The dot's size
and contrast, the readout beside a held handle, the row alignment in the card
and the one line's wrapping at 240 points are unseen. The light theme, Linux
and Windows were not run. Nothing was listened to.

For a person, at 1440x900 and 760x620: select an audio clip and capture its
handles at rest; hold a handle mid-drag and capture the curve and the
readout; capture the inspector with an audio clip selected and with a video
clip with sound selected, and with a detached picture selected.

Done when:

- Timeline tests: a handle drag is one history entry; Escape leaves the
  project unchanged; the drag cannot pass the other fade or the clip; a
  locked track takes no drag; the handle rect never overlaps a trim grip at
  any tile width that shows it.
- Inspector test: typing a fade longer than the room is held to the room;
  Return is one entry.
- `src/agent.rs` tests for the three phrases, including on a clip without
  sound and on an owned picture clip (both refused in words).
- Inspector test: after Detach audio the picture clip shows the one line
  and no fade or volume row, and the audio-track clip shows them; after
  unlinking, the picture clip shows them again.
- Native captures of the final build, normal and narrow: handles at rest,
  mid-drag with the curve, the inspector rows on an audio clip and on a video
  clip.

## Known limits

- Handles are on audio-lane tiles only; a picture clip's sound fades from
  the inspector, the menu or the panel.
- One curve; no shape choice, no crossfade object. Two clips cross-fade by
  overlapping on two tracks.
- The viewer and the sound follow a handle on release, not during the drag.

## Comments

2026-10-13: Written with the spec. Nothing implemented or measured.

2026-10-13 (review): The rows were offered "for every clip whose asset has
sound", which includes the silent half of a detached pair. Ownership now
comes from `editor_core::sound` (ticket 05a).

2026-10-13 (implemented): Built and tested headless on macOS 26.5.2 (arm64,
Rust 1.97.1); two native captures show stored fades drawn, and the captures
that need a selection or a held drag are still to be taken. Nothing was
listened to. Details are under Result and Evidence.

2026-10-13 (after review): Two findings held and are fixed, each with a test
that failed before. A fade field that was only focused and left rewrote a
frame-exact fade to the two decimals it reads and added a history entry; the
field now takes a change only when what it reads changes. A handle of no
fade or of a short one stands 12 points inside the tile, and the fade went
to the pointer, so it jumped when taken hold of; the fade now follows the
travel. `cargo test --locked --bin aster`: 644 passed on macOS 26.5.2
(arm64, Rust 1.97.1). The native captures are still to be taken, and
neither fix was tried in the running build.

2026-10-14 (native acceptance): Resolved. The captures this ticket waited for are taken and look right. Release build `target/release/aster` (sha256 fadbe5aa8244d800b2d9b16bf86bf121a6e9157af7e8067402761d40318e5763, `cargo build --release --locked`) on macOS 26.5.2 (25F84), Apple M1, Rust 1.97.1, Homebrew FFmpeg 8.1.2, built-in Retina display at scale 2. Fresh task-owned projects (made by `aster-agent` over ffmpeg-generated media: a 12 s testsrc2 interview with a speech-like tone, 30 s of 120 BPM clicks, a 0.8 s whoosh) and fresh `--settings` per run. Driven by real pointer and key events into the running window; the normal window (asked 1440x900) came up at 1280x709 points, the narrow one at 760x620 (and 760x560 for ticket 13). Captures are in `artifacts/audio-editing/captures/`. Every capture was looked at against `docs/design.md`. Dark theme only; Linux, Windows and the light theme were not run; nothing was listened to.

`normal-02`/`narrow-02` selected audio clip, handles at rest, Playback card with Volume, Fade in, Fade out; `normal-03`/`narrow-03` a fade-in handle held mid-drag with its curve and readout; `normal-04`/`narrow-04` the fade set; `normal-08`/`narrow-08` the clip menu rows Fade in/out 0.5 s and Remove fades; `normal-18`/`narrow-18` a video clip with sound, its Sound rows; `normal-20`/`narrow-20` the detached picture with the one line and no fade rows.

One defect was found in the running build and fixed: while a tile was carried or trimmed its fade handles stayed drawn at the tile's old place. The faders loop in `src/ui/timeline.rs` now skips a tile being moved or trimmed; test `a_tile_carried_or_trimmed_shows_its_handles_only_once_let_go` (`src/ui/fades_tests.rs`). `normal-12` and `narrow-12` show the move with no stale handles.
