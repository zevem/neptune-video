# Detach audio

Status: ready-for-human
Progress: resolved
Type: task

## Scope

Put a picture clip's sound on an audio track as a clip of its own. No schema
change, no new command, no file.

"Detach audio" on a selected media clip on a video track whose asset
`has_audio` builds one `Command::Batch` labelled "Detach audio":

1. `AddTrack` of an audio track named "Audio N", only when no unlocked audio
   track is free over the clip's span (`rules::free_start` leaves it where it
   is).
2. `AddClip` on that track: a clone of the clip with a fresh id, default
   transform, no effects, masks, blend, transitions or follow, and only its
   clip-level `Gain` animation track. Same asset, `start`, `source_in`,
   `duration`, `playback_rate`, `retime` and `gain`.
3. `LinkClips { clip_ids: [picture clip, new clip] }`.

The clip and track ids are captured when the action is made, not when it runs.
The audio-track clip then owns the sound (`audio.rs:836-843`,
`plan.rs:145-150`) and the pair trims, splits and moves together. Refused in
words when the clip is already linked to an audio-track clip of its asset,
has no sound, or sits on a locked track.

Files: `src/ui/mod.rs` (`Action::DetachAudio(ClipId)`), `src/app.rs`
(`perform`, the batch builder beside `insert_asset`), `src/ui/timeline.rs`
(clip context menu near `:2118`), `src/ui/toolbar.rs` (Clip group),
`src/agent.rs` (phrase `detach audio`, `EXAMPLES`), `docs/usage.md`.

## Result

Implemented; one piece of evidence is missing, the native capture of the
menu item, which needs a pointer and is a person's to take.

- `rules::detach_audio(project, clip) -> Result<Command, String>` builds the
  batch and `rules::detach_refusal` says why it would not; both are pure and
  in `src/rules.rs`, which `src/app.rs` (`App::detach_audio`, beside
  `insert_asset`), the timeline menu and the command line all ask.
- Routes: the clip's context menu (shown for a picture clip with sound,
  disabled when refused), "Detach audio" under Clip in the project menu (the
  one selected clip), the phrase `detach audio`, and the batch as serialized
  commands. No shortcut and no inspector control: the ticket lists neither.
- The picture clip stays selected. The new clip keeps the picture's name,
  group and any fields this ticket does not name; its link is the picture's,
  so clips the picture was linked to before are in the same link after.
- A fourth refusal the scope does not list: a picture linked to another
  video-track clip of the same media ("Unlink the clip from the other clips
  of its media to detach its audio."). See Known limits.
- `plan::audio_owners` is the owner set `Mixer::new` built inline, moved
  unchanged so the plan test asks the mixer's own rule.

## Evidence

On macOS 26.5.2 (arm64), Rust 1.97.1, Homebrew FFmpeg 8.1.2, 2026-10-13:

- `cargo test --locked -p aster-editor --bin aster -- rules:: agent:: toolbar`:
  38 passed. The five `rules::tests::detached_*`, `audio_that_cannot_*` and
  `a_detach_that_fails_*` tests cover the batch, the span and rate, the shared
  link, one history entry that undo removes whole, a time map and volume
  keyframes carried, the lane chosen or made, each refusal (with two linked
  cuts of one shot), a batch refused whole after its lane or its picture's
  track was locked, the batch read back from JSON, and a store save and load.
  `a_clips_sound_is_detached_by_name_once` and
  `every_listed_example_is_understood` cover the phrase.
  `ui::toolbar::tests::the_menu_detaches_the_audio_of_the_one_selected_clip`
  covers what the project menu's row asks for with no, one and two clips
  selected, and nothing else of the interface.
- Not tested: the row drawn in the project menu, the context menu's shown and
  disabled states in `src/ui/timeline.rs`, and the `Action::DetachAudio` arm
  of `App::perform` with its draft commit and toast. `App` is not built in a
  unit test; these wait on the captures below.
- `cargo test --locked -p editor-media --lib audio::plan`: 9 passed, with
  `a_detached_pair_is_planned_as_its_audio_clip_alone`.
- `cargo clippy --locked -p aster-editor -p editor-media --all-targets -- -D warnings`
  and `cargo fmt --all`: clean.
- `target/debug/aster-agent --project … --create` took the batch as an agent
  writes it over a generated six second fixture: applied, undone to the
  picture alone and unlinked, redone to the pair
  (`artifacts/audio-editing/02-detach-audio/headless-agent.jsonl`).
- The release build opened that project and was captured at 1440x900 and
  760x620: both tiles carry the link mark
  (`artifacts/audio-editing/02-detach-audio/pair-normal.png`,
  `pair-narrow.png`). The pair in them was made by the headless batch, not by
  the menu. The audio tile in them shows the video's poster frame, not a
  waveform: see Known limits.

Not done: a capture of the menu item, normal and narrow. Nothing was clicked
in the running build and nothing was listened to; the menu rows, the disabled
state and the toast of a refusal are unseen. Linux and Windows were not run.

For a person, before this is resolved: at 1440x900 and 760x620, capture the
context menu of a video clip with sound (row enabled), the same menu on a clip
already detached (row disabled), and the row under Clip in the project menu;
choose the row and confirm the pair it makes. And confirm or change the
reading of "ids are captured when the action is made" in the comment of
2026-10-13 (implemented): the action carries the clip id only.

Done when:

- An app or rules test builds the batch for a video clip with sound and finds
  one new audio-track clip with the same span and rate, both clips sharing a
  `linked_id`, and one history entry that undo removes whole.
- A test over a retimed clip and one with volume keyframes shows both carried.
- `Plan::prepare` over the result admits the audio-track clip and not the
  picture clip (an existing-style plan test in `crates/editor-media`).
- The refusals above each have a test; `every_listed_example_is_understood`
  covers the phrase.
- A native capture shows the menu item and the linked pair, normal and narrow.

## Known limits

- Unlinking the pair gives the picture clip its sound back, so both sound. A
  silent picture needs its volume set to 0.
- Deleting either clip of the pair deletes both, as for any linked clips.
- Only the first audio stream (`0:a:0`).
- Inserting a video still makes no linked audio clip on its own.
- The pair spends one voice, as before.
- Sound is owned by link and media, not by span (`plan::audio_owners`): an
  audio-track clip silences every video-track clip of its media in its link.
  Detach is therefore refused for a picture linked to another video-track
  clip of the same media, such as two cuts of one shot linked to move
  together; they are unlinked and detached each for itself. Linking such a
  clip to a detached pair by hand afterwards still silences it; nothing
  refuses that link. Ownership by span belongs with ticket 06, which moves
  ownership into core.
- The audio clip's tile shows the video's poster frame under its name, as
  any video asset on an audio track does (`paint_clip` in
  `src/ui/timeline.rs`; `a_lane_too_low_for_a_poster_asks_for_none` holds it
  so). It is told from the picture by its lane and tone. Ticket 04 reorders
  that branch so an audio-track clip of a video shows waves, not a poster.

## Comments

2026-10-13: Written with the spec. Nothing implemented or measured.

2026-10-13 (implemented): Built and tested on macOS as recorded under
Evidence. The ticket's "clip and track ids are captured when the action is
made" is read as: `Action::DetachAudio` carries the picture clip's id, and
the lane is chosen from the project when the action is carried out, in the
same frame, so a lane locked or filled since cannot be written on. Left
`claimed` for the menu capture.

2026-10-13 (after review): One rule changed. `LinkClips` joins the picture's
whole link, and the mixer skips every video-track clip whose link and media
have an audio-track owner, so detaching one of two linked cuts of a shot left
the other silent with no audio clip over it, and then refused it too. Detach
is now refused for such a clip, with a rules test of it; nothing was listened
to. A test of the project menu's row was added, and the Evidence no longer
reads as covering the menus or `perform`. The poster on the audio tile is
recorded as a limit rather than changed here: ticket 04 owns that branch of
`paint_clip`. Still `ready-for-human` for the captures and for the owner's
word on where the lane is chosen.

2026-10-14 (ticket 22): Two statements above are older than the code. The
audio clip's tile no longer shows the video's poster frame: since ticket 04
a sound lane draws the level line and then the waveform, of a video asset
too, and asks for no poster. Ownership by span was not added by ticket 06:
ownership stays by link and media, the core's `sound::owned`. The menu
capture is still a person's to take.

2026-10-14 (native acceptance): Resolved. The captures this ticket waited for are taken and look right. Release build `target/release/aster` (sha256 fadbe5aa8244d800b2d9b16bf86bf121a6e9157af7e8067402761d40318e5763, `cargo build --release --locked`) on macOS 26.5.2 (25F84), Apple M1, Rust 1.97.1, Homebrew FFmpeg 8.1.2, built-in Retina display at scale 2. Fresh task-owned projects (made by `aster-agent` over ffmpeg-generated media: a 12 s testsrc2 interview with a speech-like tone, 30 s of 120 BPM clicks, a 0.8 s whoosh) and fresh `--settings` per run. Driven by real pointer and key events into the running window; the normal window (asked 1440x900) came up at 1280x709 points, the narrow one at 760x620 (and 760x560 for ticket 13). Captures are in `artifacts/audio-editing/captures/`. Every capture was looked at against `docs/design.md`. Dark theme only; Linux, Windows and the light theme were not run; nothing was listened to.

`normal-19`/`narrow-19` video clip menu with Detach audio enabled; `normal-20`/`narrow-20` the pair it made (Audio 3, link marks on both tiles) and the picture's inspector line "Sound is on the linked audio clip"; `normal-21`/`narrow-21` the same menu after detaching, the row disabled and Unlink offered; `normal-22`/`narrow-22` the Clip group of the project menu with Detach audio; `normal-23`/`narrow-23` its refusal toast on the detached picture. The owner has not answered on where the lane is chosen; the spec's reading stands (the action carries the clip id; the lane is chosen when it is carried out), as ticket 09 also reads it.
