# Fades, track volume and solo: core types, `sound.rs`, commands, clipboard

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 01

## Scope

The durable fields of the most-used features in the pure core, the one
definition of what sounds, and the literal changes that keep the workspace
compiling. Nothing is heard or drawn yet. The store's version is ticket 05b
and the agent's surface is ticket 05c.

Core (`crates/editor-core/src/`):

- `model.rs`: `Track.gain` (default 1.0) and `Track.solo`; `Clip.fade_in` and
  `Clip.fade_out` (`Time`, default zero); all
  `#[serde(default, skip_serializing_if = …)]`. `Project::validate`: track
  gain finite and in `0.0..=8.0`; fades non-negative, summing to at most the
  clip's duration, and zero unless the clip is media whose asset has sound.
- `sound.rs` (new): `fade_at`, `level_at`, `level_varies` as in the spec,
  and:
  - `sounds(project, track, clip) -> bool`: the track is not muted, is not
    soloed out (another video or audio track is soloed and this one is not)
    and has `gain` above 0; the clip is media whose asset has sound
    (`has_audio` or kind `Audio`); no audio-track clip with the same asset
    and `linked_id` owns its sound when the clip is on a video track; and
    the clip's volume is not 0 without volume keys. Owners count whether or
    not their own track is muted or soloed out.
  - `owned(project, clip) -> bool`: the ownership test alone, for the
    inspector and the phrases, which hide controls from an owned picture
    clip and keep them for one that is merely muted.
  - `audible_at(project, track, clip, at) -> bool`: `sounds`, and `at` is
    inside the clip on a straight forward segment of its time map (a clip
    without a map is one such segment).
  The ownership set (`audio.rs:836-843`, `plan.rs:145-151`) and the
  straight-segment test (`plan.rs:17-35`) are written here once; ticket 06
  removes the copies in `editor-media`. A helper that builds the owners once
  for many clips is fine; the names are illustrative.
  `gain_at` stays for its other callers.
- `command.rs`, `editor.rs`: `SetFades { clip_id, fade_in, fade_out }`;
  `SetTrackState` gains `gain: Option<f32>` and `solo: Option<bool>`.
  `fit_fades` beside `fit_transitions` (`editor.rs:569`), called wherever a
  clip's duration changes: trim, split, rate, retime. Split leaves `fade_in`
  on the left half and `fade_out` on the right and zeroes the inner edges
  (`editor.rs:958-991`).

Compile-following changes in the same ticket:

- Every `SetTrackState { .. }` and `Track`/`Clip` literal in `src/`, the
  other crates and the tests, with the new fields at their defaults. The
  exhaustive destructures in `crates/editor-agent/src/context.rs` get the
  fields bound and ignored here; ticket 05c projects them.
- `src/clipboard.rs`: `VERSION` goes from 2 to 3; versions 1, 2 and 3 are
  read. The test `older_text_is_read_and_newer_text_is_left_alone`
  (`:198-210`) expects `"neptune_video_clips":3`, reads 1 and 2, and refuses 0 and
  4. The round-trip test gains a clip with fades. (An audio effect joins it
  in ticket 10.)

Files: `crates/editor-core/src/model.rs`, `sound.rs`, `command.rs`,
`editor.rs`, `lib.rs`, `crates/editor-core/tests/`, `src/clipboard.rs`, and
the literals wherever the compiler finds them.

## Result

Implemented. Nothing is heard or drawn: the mixer, the interface and the
agent's text do not read the new fields yet (tickets 06, 07, 08a, 05c).

- `model.rs`: `Track.gain` (1.0) and `Track.solo`, `Clip.fade_in` and
  `Clip.fade_out` (`Time::ZERO`), each skipped when at its default.
  `Project::validate` refuses with `Invalid("track volume")` and
  `Invalid("fades")`. Track volume is bound on every track, a text track's
  too.
- `sound.rs` (`editor_core::sound`, not re-exported at the root): `fade_at`,
  `level_at`, `level_varies`, `sounds`, `owned`, `audible_at`, and four names
  the ticket does not list: `volume_keyed` (the mixer's `gain_animated`),
  `steady_speed` and `audible_run` (the straight-segment test and the runs,
  as `plan.rs` has them, for ticket 06 to call), and `Sounding`, which is
  built once from a project and answers `sounds`, `owned` and `audible_at`
  for many clips without reading the links again. The free functions
  allocate nothing; one private `admitted` holds the rule for both.
- `command.rs`, `editor.rs`: `SetFades` ("Change fades") through `clip_mut`;
  `SetTrackState` with `gain` and `solo`; `fit_fades` after `TrimClip`,
  `SplitClip` (both halves, inner edges zeroed first), `SetPlaybackRate` and
  `SetRetime`, on every linked clip those commands touch.
- Routes. The serialized command is the only route that exists: `set_fades`
  and the two `set_track_state` fields are read from JSON by the same
  `Command` an agent sends, and a `set_track_state` written before the
  fields existed still reads. No shortcut, menu, gesture or inspector row
  sets a fade, a track volume or solo yet (tickets 07 and 08a); the track
  head's lock, hide and mute cells and the `mute track` phrases send the new
  fields as `None`.
- Compile-following: 21 `SetTrackState` literals in `src/`, `editor-agent`
  and the core tests; the two destructures in
  `crates/editor-agent/src/context.rs` bind and ignore the four fields. No
  `Track` or `Clip` struct literal needed a field: `rules::detach_audio`
  builds its clip with `..picture.clone()`, so a detached audio clip carries
  the picture's fades (the spec's ticket 02 comment).
- `src/clipboard.rs`: `VERSION` 3, reading 1, 2 and 3.
- `editor-media` is untouched: its `gain_animated`, `straight`,
  `steady_speed`, `audible_run` and `audio_owners` are still the mixer's
  until ticket 06 removes them.

## Evidence

On macOS 26.5.2 (arm64), Rust 1.97.1, 2026-10-13, `dev` profile unless said:

- `cargo test -p editor-core --locked`: 180 passed in 11 files, 40 of them
  in the new `crates/editor-core/tests/sound.rs`, which holds every case
  listed below and, since the review, a split, a rate change and a retime
  of a linked pair with fades. Fades are integer microseconds, so NaN and infinity are
  tried on track volume only; fades are tried negative, past the clip and at
  `i64::MAX`.
- `cargo test --locked -p neptune-video`: 623 and 12 passed; the six
  `clipboard::tests` among them, with the new
  `copied_fades_survive_the_system_clipboard`.
- `cargo test -p editor-store --locked` (76 passed) and
  `cargo test --locked -p editor-media` (222 passed, FFmpeg 8.1.2 on `PATH`).
- `cargo test --profile ci --locked -p editor-agent --all-targets
  --no-fail-fast` with the fixture `scripts/test-workspace.py` builds: two
  failures as first built, one after the review.
  - `capabilities_name_every_command_in_the_schema` failed: the schema had
    `set_fades` and the capability list did not. The review added the name
    to the list in `crates/editor-agent/src/lib.rs`;
    `cargo test -p editor-agent --locked --test protocol` now passes, 14 of
    14 (`dev` profile). The whole `editor-agent` suite was not run again.
  - `provider::tests::an_unrepresentable_native_path_is_refused_before_creating_an_instruction_file`:
    `create_dir` of a name that is not UTF-8 fails on this host's filesystem
    ("Illegal byte sequence", os error 92) before the code under test runs.
    Not caused by this ticket.
  - Printed: `planner_instructions: 4778 bytes`, `proposal_schema: 21783
    bytes`. The schema grew by 1,008 bytes from ticket 01's 20,775, not the
    644 estimated, before any audio kind; 2,793 are left under 24 KiB.
- `cargo check --workspace --all-targets --locked`,
  `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  `cargo fmt --all -- --check` and `python3 scripts/check-architecture.py`:
  clean.

Not done: no native build was run and nothing was listened to; there is
nothing to see or hear yet. Linux and Windows were not run. The cost of
`sounds` or `Sounding::of` over a large project is not measured.

Done when:

- Core tests (`crates/editor-core/tests/`): bounds, NaN and infinity for
  each field; `SetFades` on a title, a still and a silent video is refused
  and leaves the project unchanged; undo restores; a start trim, an end trim
  and a rate change clamp fades; a split's four edges; a locked track refuses
  `SetFades` and accepts `SetTrackState`; `fade_at` is 0 at the start, 1 past
  `fade_in`, `sin(π/4)` half way; `level_at` multiplies track, clip (keyed
  and not) and fade; v1 and v2 JSON decode and a default clip and track emit
  no new key.
- `sounds` tests, one case each: a muted track; a track soloed out; a soloed
  and muted track; track volume 0; a still and a silent video; a picture
  clip whose linked audio-track clip of the same asset exists, and the same
  with that owner's track muted (still owned); a linked pair of different
  assets (not owned); volume 0 without keys (false) and with keys (true).
  `audible_at` tests: outside the clip; inside a freeze, a reverse and a
  ramp (false); on a straight segment between them (true).
- Clipboard tests above.
- `python3 scripts/check-architecture.py` passes (core stays pure).

## Known limits

- The store still writes version 2 until ticket 05b, so no build between
  the two tickets is released; nothing new is in a saved file's version.
- Fades do not follow links: a detached pair fades on the audio-track clip,
  and the picture clip of the pair shows no fade controls (ticket 07).
- Validation does not refuse fades on an owned picture clip; they are heard
  if the pair is unlinked.
- A fade is a timeline length; it is not rescaled with the clip's speed.
- `sounds` rebuilds or is handed the owners; its cost over a large project
  is unmeasured.
- A clip without volume keys whose volume is 0 does not sound, and a clip
  whose keys are all 0 does: it is admitted and silent.
- The agent's context drops the four fields (ticket 05c), most visibly on a
  clip summarized for a long animation track.

## Comments

2026-10-13: Written with the spec. Nothing implemented or measured.

2026-10-13 (review): Split from the former ticket 05, which also held the
store version and the agent surface; the advanced-editing epic kept those
apart (its tickets 01, 02, 04, 05). `sounds` and `audible_at` were added
after the review found the definition only inside `editor-media` and
re-derived differently by three tickets. The clipboard version is now
stated: `src/clipboard.rs:11-13` is at 2 and refuses other versions.

2026-10-13 (implemented): Built and tested on macOS as recorded under
Evidence. Read where the ticket left room:

- *An edge without a fade* is level 1 on both sides of it, so `fade_at` of a
  clip without fades is 1 at any time and the mixer's product is unchanged
  for it; a faded edge is 0 beyond the clip.
- *A soloed and muted track* does not sound and still solos the rest out:
  "another video or audio track is soloed" has no exception for mute.
- *`owned`* takes no track, so it finds whether the clip is itself on an
  audio track while it looks for an owner; two audio-track clips of one
  media in one link both sound, as in the mixer today.
- *Owner decision 4* (solo is durable) has no answer yet; the spec's choice
  is built: `solo` is a saved field and a history entry.
- `Progress: resolved` with the capability test red, because the ticket
  split gives that line to 05c.

2026-10-13 (review fixes): No rule changed.

- `set_fades` is in the capability list now, taken from ticket 05c's scope
  so that the `editor-agent` suite is not left red between tickets. 05c
  keeps the context projection, the `SYSTEM` sentence and the docs.
- `detached_audio_is_one_linked_clip_of_the_same_span_and_one_step`
  (`src/rules.rs`) gives the picture fades and checks the detached clip has
  them, which was true by construction only.
- Three tests in `crates/editor-core/tests/sound.rs` split, change the rate
  of and retime a linked pair with fades, naming the picture, and check both
  clips. The pair is of one length; a linked clip of another length is not
  tried.

2026-10-14 (ticket 22): The "not yet" statements above are older than the
code. The store writes version 3 (05b); the mixer hears fades, track volume
and solo (06) and its own copies of the straight-segment test and ownership
are gone; the agent's context sends the four fields (05c); handles, rows,
the track head and phrases set them (07, 08a).
