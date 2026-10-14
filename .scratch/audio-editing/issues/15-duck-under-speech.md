# Duck under speech

Status: ready-for-human
Progress: resolved
Type: task
Blocked by: 04, 05a, 08b, 13

## Scope

Lower selected clips wherever other clips have signal, as ordinary volume
keyframes the person can see and edit. No mixer change, no new type.

- `crates/editor-core/src/sound.rs`: `duck_keys(spans, duration, base,
  depth, attack, release) -> Vec<Keyframe>`. For each span `(from, to)`
  inside the clip: a key at `from - attack` at `base`, at `from` at
  `base · depth`, at `to` at `base · depth`, at `to + release` at `base`,
  all `Ease::Linear`, clamped to `0..=duration`, with keys that would
  collide merged. When the result would pass `MAX_KEYFRAMES` (256), the
  closest spans are merged until it fits. A key at 0 and at `duration`
  carry the level there.
- `src/duck.rs` (desktop, pure): the gate, built one voice asset at a
  time. The voices are the clips that are not selected and for which
  `editor_core::sounds` is true, which is the mixer's own admission: link
  ownership and assets without sound are honoured, and a clip at volume 0
  with volume keys counts. For one asset's peaks, walk each of its voice
  clips over the target's time in 10 ms steps; where
  `editor_core::audible_at` is true (so not inside a freeze, a reverse or
  a ramp, which the mixer keeps silent) and the `rms` read through
  `clip.source_time` is above -35 dBFS, the step is signal. The steps
  become raw spans in timeline time, at most 4,096 an asset with the
  closest merged beyond that, and the peaks are then let go. When every
  voice asset has been taken, the raw spans are united: spans shorter than
  200 ms are dropped, a 300 ms hangover is added, gaps under 1 s are
  merged.
- The gate never needs two assets' peaks at once, so it works with more
  voice assets than `src/peaks.rs` holds (a file at the bucket bound is
  8 MiB of the 32 MiB map). Each asset is wanted in turn; one whose peaks
  fail (no waveform, an over-long file) is left out and named in the
  notice.
- `Action::Duck { clips, depth }` with ids captured at the click. Defaults:
  depth 0.25 (-12 dB), attack 150 ms, release 400 ms. One
  `Command::Batch` "Duck under speech" of `SetKeyframes { target: None,
  key: Gain }`, one per target clip.
- A target that already has volume keys is asked about first ("Replace
  this clip's volume keyframes?"). Peaks that are not held yet are waited
  for with progress, asset by asset; the batch is written when the last
  has been taken, and the action is given up on Escape or a project
  change. A selected clip that does not sound (an owned picture clip among
  them) is refused in words before anything is asked for.
- Routes: clip context menu on audio-lane clips, a button in the Audio
  section with the depth field, `duck under speech` in the command panel.

Files: `crates/editor-core/src/sound.rs`, `src/duck.rs`, `src/app.rs`,
`src/ui/mod.rs`, `src/ui/timeline.rs`, `src/ui/inspector/clip.rs`,
`src/ui/sheets.rs` (the question), `src/agent.rs`, `docs/usage.md`.

## Result

Implemented and tested headless, through a stand-in lane and once through
the real sound lane and FFmpeg; the native captures are not taken (see
Evidence). Nothing was listened to.

- `crates/editor-core/src/sound.rs`: `duck(spans, duration, base, depth,
  attack, release) -> Duck { keys, dips, merged }` and `duck_keys(...)`,
  its keys. Spans that do not reach the clip are dropped; spans whose
  ramps would cross (`next.from - attack < prev.to + release`) are one
  dip; each dip is four `Ease::Linear` keys, held to `0..=duration` (a
  key cut off at an edge carries the level there, keys of equal time
  merged); past 256 keys the dips with the shortest gap are merged first
  and `merged` counts them. `merge_closest(spans, most)` does the same
  for raw spans.
- `src/duck.rs` (new). Constants `DEPTH` 0.25, `ATTACK` 150 ms, `RELEASE`
  400 ms, `STEP` 10 ms, `SIGNAL_DB` -35, `SHORTEST` 200 ms, `HANGOVER`
  300 ms, `BRIDGED` 1 s, `MOST_SPANS` 4,096. The voices are the clips not
  selected (a selected clip passed over is no voice either) for which
  `sound::sounds` is true and which overlap the ducked clips' time; for one file's peaks, `asset_spans` walks each of its
  voice clips in 10 ms steps where `steady_speed` holds (not in a freeze,
  reverse or ramp) and the RMS at `clip.source_time` is over -35 dBFS;
  `unite` drops spans under 200 ms, adds 300 ms and bridges gaps under
  1 s. `command` writes one `Batch` "Duck under speech" of
  `SetKeyframes { target: None, key: Gain }` a clip, under the clip's
  volume, or its loudest volume key when it has keys.
  `Ducking::start/step/give_up/progress` is the run: one voice file at a
  time, wanted through `HeldPeaks::want_work`, taken and let go; a file
  that fails is left out and named; any change of the project's instance
  or revision gives it up with nothing written. Refusals in words, before
  anything is asked: gone, "That clip has no sound to duck.", "Sound is on
  the linked audio clip.", "That clip is not heard: …", "Unlock the
  clip's track to duck its sound.", "Every volume key of that clip is at
  0%: there is nothing to duck."; with no voice, "No other clip is heard
  under <name>: there is nothing to duck it under.".
- `src/peaks.rs`: the work route (`want_work`, `failed`, `let_go`, a
  bound per map for tests). Work is asked for whatever the room, is never
  given up as unwanted or evicted, and may evict on-screen peaks, least
  lately wanted first; those tiles wait and ask again once it lets go. A
  file that failed stays failed when work lets it go, so neither a tile
  nor a later duck reads it or reports it again.
- `src/app.rs`: `Action::Duck { clips, depth }` commits drafts, refuses
  or asks "Replace this clip's volume keyframes?" (`sheets::replace_keys`,
  `Overlay::Duck`), then starts the run; `step_ducking` runs each frame in
  `logic`, asks the lane for the file it waits for there (so it reads on
  while the window is hidden), writes the batch through `edit` and says what was ducked;
  Escape, when nothing else is held, sends `Action::CancelDuck`; project
  replace clears it.
- Routes. Clip menu (`timeline.rs`): "Duck under speech" after "Normalize
  loudness" on a clip of an audio track, for the selected clips at the
  default depth, disabled with the reason when none would duck. Inspector
  (`inspector/audio.rs`, where the Audio section now lives): a Ducking
  card under Playback with a Depth field (dB, -1 to -60, -12 by default)
  and the button; while it reads, a bar "Reading <file>, n of N" and Stop
  ducking. Command panel (`agent.rs`): `duck`, `duck under speech`, with
  an optional depth (`duck 18 db`); `EXAMPLES` is 29 entries.
- Serialized agent commands are unchanged: an agent writes the keys with
  `SetKeyframes` and cannot run the gate.
- `normalize.rs`: `unheard` is public for the refusals; its ducked-clip
  test now takes the keys from `duck_keys` itself.
- Docs: `usage.md` ("Duck under speech" after Normalize loudness);
  `performance.md` (the held-peaks exception for work, and a duck bound
  row).

## Evidence

On macOS 26.5.2 (arm64, Apple M1), Rust 1.97.1, Homebrew FFmpeg and
ffprobe 8.1.2 on `PATH` (`scripts/check-media-tools.py` passed),
2026-10-14, `dev` profile. No native window was opened and nothing was
listened to.

- `cargo test --locked -p editor-core`: every suite passed, `--test
  sound` 55. `cargo test --locked -p aster-editor`: 761 and 12 passed;
  `cargo test --locked -p aster-editor --bin aster duck`: 19 passed, three
  runs in a row. After the review fixes: 763 and 12, and `duck` 20. `cargo clippy --locked -p aster-editor --all-targets -- -D
  warnings`, the same for `editor-core`, `cargo fmt --all -- --check`,
  `cargo check --locked` and `scripts/check-architecture.py` pass.
- Core (`crates/editor-core/tests/sound.rs`):
  `one_span_dips_the_clip_with_four_keys` (and no spans, or none in the
  clip, give no keys), `a_span_at_the_clip_edges_is_held_to_them_and_the_keys_still_rise`,
  `spans_whose_ramps_cross_are_one_dip`,
  `two_hundred_spans_fit_the_key_bound_by_merging_the_closest` (64 dips,
  136 merged, `AnimTrack::validate` passes),
  `the_closest_spans_are_merged_first`,
  `ducked_keys_are_written_as_one_step_and_undone`.
- Gate, over synthetic peaks (`src/duck.rs`):
  `a_short_burst_is_ignored_and_bursts_half_a_second_apart_are_one_span`
  (and -36 dBFS opens nothing), `a_muted_voice_track_is_no_voice`,
  `a_voice_at_twice_its_speed_is_read_at_its_source_time`,
  `the_picture_of_a_detached_pair_is_no_voice_and_its_audio_clip_is`,
  `a_voice_at_volume_zero_counts_only_with_volume_keys`,
  `loud_source_under_a_freeze_opens_nothing`.
- `six_voices_are_taken_one_at_a_time_through_a_map_that_holds_two`: six
  files of 8,000 bytes through `HeldPeaks::bounded(20_000)` and a stand-in
  lane, three kept and three read; each asked in track order, the map
  never over its bound, empty at the end, and the step and notice equal to
  a run with every file to hand.
- `a_voice_whose_peaks_fail_is_left_out_by_name_and_the_rest_still_duck`
  (run again, the failed file is not asked for again; with none read,
  "Nothing was ducked." naming both);
  `a_selected_clip_that_is_passed_over_is_no_voice` (a clip on a locked
  track selected beside the music is passed over, named, not read, and
  does not duck it).
- `the_duck_is_one_step_that_undoes_to_the_envelope_before` (a keyed clip,
  the question's words, one history entry labelled "Duck under speech",
  keys at 2.85, 3.0, 4.3, 4.7 s, undo restores the keys before);
  `a_ducked_clip_is_saved_and_read_back_as_written` (through
  `editor_store::save_project` and `load_project` in a fresh temporary
  directory, removed after); `the_depth_is_given_in_db_within_its_bounds`;
  `a_clip_that_does_not_sound_is_refused_in_words_before_anything_is_asked`;
  `a_run_is_given_up_when_the_project_changes_and_on_escape`.
- Through the real lane: `a_tone_read_by_the_lane_ducks_the_clip_under_it`,
  a WAV the test writes (1 kHz at -20 dBFS from 1 s to 2 s of 3 s, mono,
  8 kHz) as a voice from 2 s: keys at 2.85, 3.0, 4.3 and 4.7 s at 1.0,
  0.25, 0.25, 1.0, in each of three runs.
- `src/peaks.rs`:
  `work_is_read_unseen_and_takes_its_room_from_tiles_until_it_lets_go`,
  `sound_work_could_not_read_stays_failed_once_it_is_let_go`.
- Routes, through real egui passes: `src/ui/fades_tests.rs`
  `the_menu_of_an_audio_lane_clip_ducks_the_selected_clips` (asked by id
  at 0.25, no step; no row on a video-lane clip; disabled on a locked
  track; beside a ducked clip the locked one is still asked for);
  `src/ui/inspector/audio_tests.rs`
  `ducking_is_asked_from_the_audio_section_at_the_depth_set_there` (under
  Fade out, above the audio effects; -12 and -18 dB; progress and Stop;
  disabled on a muted track; none on a picture); `src/ui/sheets.rs`
  `the_duck_asks_before_volume_keys_are_replaced` (Replace, Cancel,
  Escape); `src/agent.rs`
  `duck_asks_for_the_selected_clips_that_are_heard_at_the_depth_given`;
  `every_listed_example_is_understood` covers the new example.

Not verified: the native captures (the menu item, the question, a ducked
clip's volume line) need a pointer in the running build and none was
taken; the app's own frame loop (question, Escape, step, notice) is
tested only in its parts; real speech or music; how the duck sounds; the
gate's cost on the UI thread with long or many voice files; another host
or FFmpeg build.

The criteria this was held to:

- Core tests of `duck_keys`: one span gives four keys at the stated times
  and values; a span at the clip's start or end is clamped and still
  strictly increasing; overlapping ramps merge; 200 spans come back as at
  most 256 keys that pass `AnimTrack::validate`; no spans gives no keys.
- Gate tests over synthetic peaks: a burst shorter than 200 ms is ignored;
  two bursts 0.5 s apart are one span; a muted voice track is ignored; a
  voice clip at rate 2 is read at the right source time; the picture clip
  of a detached pair is not a voice and its audio-track clip is; a voice at
  volume 0 with keys counts; a loud stretch of source under a freeze
  segment makes no span.
- A test with six voice assets and a held map that takes two: every asset
  is taken in turn, the spans equal those of an unbounded run, and the map
  never passes its bound.
- A voice asset whose peaks fail is left out, named, and the rest still
  duck.
- The batch is one history entry and undo restores the previous envelope.
- Native captures: the menu item, the question, a ducked clip's volume line.

## Known limits

- An energy gate, not speech detection: music, effects or noise on the
  other clips trigger it, and quiet speech under -35 dBFS does not.
- The duck is written once. It does not follow speech that is moved or
  trimmed later; run it again.
- At most 256 keys a clip, about 64 separate dips; beyond that neighbouring
  dips are merged and the notice says how many.
- The gate reads the source's level, not the clip's volume, fades or
  effects: a voice whose keys hold it at 0 for a stretch still triggers a
  dip there.
- A late-starting audio stream is read where the container puts it; the
  mixer can play it early after a seek (ticket 03).
- An agent can write the keyframes itself; it cannot run the gate.

## Comments

2026-10-13: Written with the spec. Nothing implemented or measured. A live
sidechain was set aside: the mixer has one process a clip and no bus, and a
duck without lookahead arrives late.

2026-10-13 (review): Two changes. The gate listed its own idea of an
audible clip ("track not muted or soloed out, volume above 0"), which
missed link ownership and silent assets, refused a keyed clip at volume 0
that the mixer plays, and read through segments the mixer keeps silent; it
now asks the core. And it wanted every voice's peaks held at once, which
the 32 MiB map cannot give for more than four long assets: the least
recently wanted would be evicted while the next loaded, for ever.

2026-10-14 (implemented): Built as the Result says. Owner decision 7 is
taken by default: the keys are replaced after asking. Two things the
ticket did not settle: the level a dip comes back to is the clip's volume
or its loudest key, and the held peaks gained a work route that may take
the room of tiles on screen, so a duck always finishes. The Audio section
is in `src/ui/inspector/audio.rs`, so the card is there rather than in
`clip.rs`. The menu ducks at the default depth.

2026-10-14 (review): Three findings held and were fixed. A file whose sound
a duck could not read was forgotten when the duck let it go, so a tile read
it again and said "has no waveform" a second time; `let_go` now keeps a
failure. The voices were the clips not ducked, so a selected clip that was
passed over (on a locked track, say) was ducked under; the routes now hand
the whole selection over and no selected clip is a voice, as the scope and
`usage.md` say. And the lane was asked for a duck's file only by a drawn
frame, so a duck stalled while the window was hidden; `step_ducking` now
asks from `logic`. That last path is not tested: there is no headless
harness for the application's frame loop, and no hidden-window run was
made.

2026-10-14 (native acceptance): Resolved. The captures this ticket waited for are taken and look right. Release build `target/release/aster` (sha256 fadbe5aa8244d800b2d9b16bf86bf121a6e9157af7e8067402761d40318e5763, `cargo build --release --locked`) on macOS 26.5.2 (25F84), Apple M1, Rust 1.97.1, Homebrew FFmpeg 8.1.2, built-in Retina display at scale 2. Fresh task-owned projects (made by `aster-agent` over ffmpeg-generated media: a 12 s testsrc2 interview with a speech-like tone, 30 s of 120 BPM clicks, a 0.8 s whoosh) and fresh `--settings` per run. Driven by real pointer and key events into the running window; the normal window (asked 1440x900) came up at 1280x709 points, the narrow one at 760x620 (and 760x560 for ticket 13). Captures are in `.scratch/audio-editing/captures/`. Every capture was looked at against `docs/design.md`. Dark theme only; Linux, Windows and the light theme were not run; nothing was listened to.

`normal-08`/`narrow-08` the clip menu with Duck under speech; `normal-24`/`narrow-24` the Ducking card; `normal-28`/`narrow-28` the question "Replace this clip's volume keyframes?" with Cancel and Replace; `normal-29`/`narrow-29` the notices just after Replace; `normal-30`, `normal-33`, `narrow-30`, `narrow-31` the ducked volume line.
