# Normalize loudness: gain rule and routes

Status: ready-for-human
Progress: resolved
Type: task
Blocked by: 05a, 13, 14a

## Scope

Set a clip's volume so the part of its source it plays reaches a target
loudness. The measurement is derived and kept nowhere; the result is a
`SetGain` or a `SetKeyframes`.

- `src/normalize.rs` (pure): from a `Loudness::Measured`, the target volume
  is `gain = 10^((-14 - integrated) / 20)`, lowered so
  `true_peak + 20·log10(gain) <= -1`, clamped to `0..=8`. The measurement is
  of the source before volume, so `gain` is an absolute volume, not a
  change.
  - A clip without volume keys: one `SetGain { gain }`.
  - A clip with volume keys: the stored `gain` is not what plays
    (`render.rs:225-230`), so the keys are scaled by
    `factor = gain / highest key value` in one `SetKeyframes`. The loudest
    stretch then sits at the target volume and every dip keeps its depth,
    which is what a clip ducked by ticket 15 needs. Refused in words if the
    highest key is 0 or any key would pass 8.
- `src/app.rs`: `Action::Normalize(Vec<ClipId>)` captures the clip ids.
  Each clip is checked with `editor_core::sounds` first: a clip that does
  not sound, an owned picture clip among them, is refused in words
  ("Sound is on the linked audio clip"). One `Loudness` request per clip;
  several clips are all queued (their targets differ, so none replaces
  another) and each is written as it answers, one history entry a clip. On
  an answer the clip must still be there with its asset, source span and
  effects unchanged; otherwise nothing is written and the notice says so.
- Notices: what was measured and what was set; when the clamp or the peak
  ceiling kept it short of the target; `Silent`: "This clip is silent";
  `TooShort`: "Too short or too quiet to measure (under 0.4 s)". Neither
  changes anything.
- Routes: clip context menu, a button in the Audio section and the Sound
  card, `normalize` in the command panel.

Files: `src/normalize.rs`, `src/app.rs`, `src/ui/mod.rs`,
`src/ui/timeline.rs`, `src/ui/inspector/clip.rs`, `src/agent.rs`,
`docs/usage.md`.

## Result

Implemented and tested headless, through a stand-in lane and once through
the real sound lane and FFmpeg; the native capture is not taken (see
Evidence). Nothing was listened to.

- `src/normalize.rs` (new). The rule: `volume(integrated, true_peak) ->
  Option<Volume { gain, held }>` is `10^((TARGET - I) / 20)`, lowered to
  `10^((CEILING - peak) / 20)` when that is less (`Held::Peak`), then held
  to `0..=MOST` (`Held::Bound`); `None` for figures that are not numbers.
  `TARGET` -14, `CEILING` -1, `MOST` 8. `command(clip, gain)` is one
  `SetGain`, or for a clip with volume keys one `SetKeyframes` scaled by
  `gain / highest key` (the highest set to `gain` exactly, every other
  held under it), each in a `Batch` labelled "Normalize loudness"; `None`
  when nothing would change. Refused in words when every key is at 0 or a
  key would pass 8 (see Comments: the rule cannot reach the second).
- Refusals before any request, `refusal(project, clip)`: the clip is gone;
  "That clip has no sound to normalize."; "Sound is on the linked audio
  clip." (an owned picture, by `sound::owned`); "That clip is not heard:
  its track is muted." / "another track is soloed." / "its track's volume
  is 0%." / "its volume is 0%." (by `sound::sounds`); "Unlock the clip's
  track to normalize its sound.". `takers(project, clips)` is the clips
  that pass, or the first reason, or "Select a clip first.".
- `Normalizing`, the desktop's waiting clips. `ask(project, instance,
  clips, lane)` captures each clip's media, `SourceIdentity`, source span
  (lowest to highest source shown) and effects, and hands a `LoudnessJob`
  to `lane` (the app passes `SoundService::loudness`; tests a stand-in).
  Clips of one span with the same effects share one request; a clip of a
  span already under way with other effects is held until that one
  answers, since the lane would stop it (see Comments); a full lane
  (`Busy`) holds it too, and `pump` asks again. At most `MOST_WAITING`
  (64) clips wait. A clip asked for again is waited for only as it is now.
  `answered(project, instance, event)` takes a `SoundEvent::Measured` and
  hands back, per clip, a `Written { name, command, said, problem }`: the
  step and its notice for a clip still as measured (same media and source,
  span and enabled audio effects) that `refusal` still lets through, and
  words otherwise. `clear` on
  project replace.
- Notices: "voice.wav measured -20.0 LUFS, peak -10.0 dBFS; volume set to
  200%.", "…; volume keys scaled so the loudest is N%.", "…; its volume is
  N% already.", plus " It stops short of -14 LUFS so that its peak stays
  at -1 dBTP." or " … at 800%, the most a clip's volume is."; "voice.wav:
  This clip is silent."; "voice.wav: Too short or too quiet to measure
  (under 0.4 s)."; "voice.wav was removed while it was measured; nothing
  was changed."; "voice.wav changed while it was measured, so its volume
  was not set. Normalize it again."; "voice.wav was not measured: " and
  FFmpeg's first line. On asking: "Measuring the loudness of voice.wav."
  or "… of 2 clips.", with " One was passed over: <reason>" when one was.
- `src/app.rs`: `normalizing: Normalizing`; `Action::Normalize(clips)`
  commits drafts, starts the sound lane, asks and says so. `take_sound`
  sends `Measured` answers to `Normalizing` (peaks keep the rest), takes
  each step through `edit` (a refused dispatch says "<name> was not
  normalized: <error>"), and pumps held clips. `replace_editor` clears it.
- `src/ui/mod.rs`: `Action::Normalize(Vec<ClipId>)`, the ids captured when
  the action is made.
- Routes. Clip menu (`timeline.rs`): "Normalize loudness" after "Key
  volume", for the selected clips, offered where the menu's clip has sound
  and disabled where `takers` refuses every selected clip (an owned
  picture among them). Inspector (`inspector/clip.rs`): a "Normalize
  loudness" button under the fade rows in `sound_rows`, so in the Audio
  section's Playback card and a video clip's Sound card; an owned picture
  shows the "Sound is on the linked audio clip" line in its place; a clip
  that is not heard has it disabled with the reason on hover. Command
  panel (`agent.rs`): `normalize`, `normalize loudness` (and `normalise`),
  through `takers`; `EXAMPLES` is 28 entries. No shortcut, as the ticket
  lists none.
- Serialized agent commands are unchanged: an agent cannot start a
  measurement and sets a volume with `SetGain` or `SetKeyframes`.
- Docs: `usage.md` ("Normalize loudness" under Playback), and one bound
  row in `performance.md`'s loudness table (not in the ticket's file list:
  the waiting clips are a new bound).

## Evidence

On macOS 26.5.2 (arm64, Apple M1), Rust 1.97.1, Homebrew FFmpeg and
ffprobe 8.1.2 on `PATH` (`scripts/check-media-tools.py` passed), 2026-10-13,
`dev` profile. Only the desktop crate changed, so no other crate was built
or tested. No native window was opened and nothing was listened to.

- `cargo test --locked -p aster-editor`: 740 and 12 passed;
  `cargo test --locked -p aster-editor --bin aster normalize`: 21 passed,
  three runs in a row. `cargo clippy --locked --all-targets -- -D
  warnings`, `cargo fmt -- --check`, `cargo check --locked` and
  `scripts/check-architecture.py` pass. No test was skipped.
- New tests in `src/normalize.rs`. The rule:
  `the_target_is_reached_when_nothing_holds_it` (also a loud source turned
  down, and `None` for NaN and infinity),
  `the_peak_ceiling_holds_the_volume_short_of_the_target` (-20 LUFS with a
  -3 dBFS peak ends with the peak at -1.0),
  `the_bound_holds_the_volume_at_eight`,
  `a_clip_without_keys_has_its_volume_set_in_one_step_that_undoes`,
  `a_keyed_clip_has_its_loudest_key_set_and_its_ratios_kept` (keys 1.0,
  0.25, 0.5 at 1.6 are 1.6, 0.4, 0.8; the stored volume left; undone),
  `a_ducked_clip_keeps_its_dips_at_the_same_depth_under_the_new_base` (keys
  written as ticket 15 specifies `duck_keys` for base 0.5 and depth 0.25,
  which is not built yet), `keys_that_cannot_be_scaled_are_refused_and_nothing_is_written`,
  `at_the_bound_no_key_is_scaled_past_it`. With a stand-in lane:
  `two_clips_of_one_media_are_both_measured_and_each_written_as_it_answers`
  (two jobs, spans 0–4 s and 5–9 s; answered in reverse order; one step
  each, each undone alone; an answer again is to nothing),
  `a_stale_answer_writes_nothing_and_says_so` (deleted, trimmed, effects
  changed: project unchanged),
  `a_picture_whose_linked_audio_clip_owns_its_sound_is_refused_before_any_request`
  (no job; with its audio clip, that one is asked and the picture said to
  be passed over), `a_clip_that_is_not_heard_or_has_no_sound_is_refused_in_words`,
  `silence_and_a_span_too_short_to_measure_each_say_so_and_change_nothing`
  (and an FFmpeg error), `one_span_is_measured_once_and_a_span_under_way_is_not_replaced`,
  `a_full_lane_holds_clips_until_it_has_room`,
  `an_answer_for_another_project_or_after_a_clear_writes_nothing`,
  `a_normalized_keyed_clip_is_saved_and_read_back_as_written` (through
  `editor_store::save_project` and `load_project` in a fresh temporary
  directory, removed after). Through the real lane:
  `a_tone_measured_by_the_lane_is_set_to_the_target`, a WAV the test writes
  (1 kHz at -20 dBFS, mono, 8 kHz, a 2 s clip from 0.5 s of it): "tone.wav
  measured -23.0 LUFS, peak -22.8 dBFS; volume set to 282%." in each of
  three runs, one "Normalize loudness" step of +9.0 dB. The mono tone reads
  -23 because the meter hears it summed to two channels, as ticket 14a
  built.
- Routes, through real egui passes: `src/ui/fades_tests.rs`
  `the_clip_menu_normalizes_the_selected_clips_that_are_heard` (two
  selected sounds asked by id, no step; no row on a title; disabled on an
  owned picture and choosing it asks nothing);
  `src/ui/inspector/tests.rs`
  `normalize_loudness_is_asked_beside_the_volume_of_a_clip_that_is_heard`
  (under Fade out in an audio clip's Playback card, in a picture's Sound
  card, absent on an owned picture, present on its audio clip, disabled on
  a muted track); `src/agent.rs`
  `normalize_asks_for_the_selected_clips_that_are_heard_and_refuses_the_rest_in_words`;
  `every_listed_example_is_understood` covers the new example.

Not verified: the native capture of the notice and the changed volume (it
needs a pointer or the keyboard in the running build; no capture was
taken), real speech or music, a compressed file, a clip through several
audio effects or with a time map measured for real, another host or FFmpeg
build, and how loud anything sounds.

The criteria this was held to:

- Pure tests of the rule: target reached; the peak ceiling limits it; the
  8× bound limits it; a keyed clip whose highest key is 1.0 and lowest 0.25
  comes back with its highest at `gain` and the ratio kept; a clip ducked by
  `duck_keys` (base 0.5, depth 0.25) keeps its dips at a quarter of the new
  base; a key that would pass 8 is refused and nothing is written.
- App tests with a fake lane: two selected clips of one asset are both
  written; a stale answer (clip deleted, trimmed, or effects changed since
  the request) writes nothing and says so; an owned picture clip is refused
  before any request; `Silent` and `TooShort` each give their notice.
- Native capture of the notice and the changed volume.

## Known limits

- Clip loudness, not programme loudness: the mix of several clips is not
  measured, and no compliance with any standard is claimed.
- A span under about 0.4 s cannot be normalized; no peak-only fallback.
- The measured span is the clip's source span; a time-mapped clip measures
  from the lowest to the highest source it shows.
- The target is one constant (owner decision 6).
- An agent cannot start a measurement; it can set a volume.

## Comments

2026-10-13 (review): Split from the former ticket 14. The factor for a
keyed clip is now defined (the old text said "the same factor" without one),
the routes ask `sounds`, and "one measurement at a time; a second replaces
the first" is gone: it contradicted measuring selected clips one after
another.

2026-10-13 (implemented): Built as scoped, with these settled in building:

- *Same span, one request.* "Their targets differ, so none replaces
  another" holds only for clips of different spans: two clips of one media
  over the same source span have one target, and the lane would stop the
  first job for the second. Clips of one span with the same effects share
  one request and are each written from its answer; one with other effects
  waits until that answer and is then sent. A full lane holds clips too.
- *A key cannot pass 8 by this rule.* `gain` is held to 8 and every key is
  at most the highest, so a key scaled by `gain / highest` is at most
  `gain`; the others are held under it so a rounding cannot say otherwise.
  The refusal is kept and tested by handing `command` a volume above 8.
- *Refused before measuring, and at the answer.* A locked track is refused
  in words before any request and again when the figures come; the reasons
  a clip is not heard are named (muted, soloed out, a track or clip at 0%).
- *History.* Each clip's step is a one-command batch labelled "Normalize
  loudness", so the history says what it was.
- *Bound.* At most 64 clips wait at once; the desktop's list is otherwise
  unbounded.
- *Ducked clips.* Ticket 15's `duck_keys` is not built; the test writes the
  keys it specifies.

2026-10-14 (review fixes): Three findings of the adversarial review held
and are fixed in `src/normalize.rs`, each with a test:

- *Refused again at the answer.* The answer checked only that the clip was
  as measured and its track unlocked. Detach audio leaves the picture's
  media, span and effects as they were, so a video clip detached while it
  was measured was written to, though its sound is now owned and never
  heard. The answer now asks `refusal` again: an owned picture, a clip no
  longer heard (muted, soloed out, at 0%) and a locked track each write
  nothing and say why ("shot.mp4: Sound is on the linked audio clip.").
  Tested in `a_stale_answer_writes_nothing_and_says_so`.
- *Only the effects it hears.* A clip is captured, compared and sent with
  its enabled audio effects alone, which is all the measurement runs. A
  picture effect, or a setting of a switched-off effect, changed while it
  is measured no longer throws the answer away, and clips that differ only
  so share one request. Tested in
  `only_the_audio_effects_it_hears_make_a_clip_changed`.
- *Every clip over the bound is counted.* Asking stopped at the first clip
  over `MOST_WAITING` and said one was passed over however many were. Each
  is now passed over and counted, and a clip already waiting takes its own
  place again rather than being dropped at the bound. Tested in
  `every_clip_over_the_bound_is_passed_over_and_counted` (66 clips: 64
  jobs, "2 were passed over").

`cargo test --locked -p aster-editor --bin aster normalize`: 23 passed
(macOS 26.5.2 arm64, Rust 1.97.1, FFmpeg 8.1.2, 2026-10-14).

2026-10-14 (native acceptance): Resolved. The captures this ticket waited for are taken and look right. Release build `target/release/aster` (sha256 fadbe5aa8244d800b2d9b16bf86bf121a6e9157af7e8067402761d40318e5763, `cargo build --release --locked`) on macOS 26.5.2 (25F84), Apple M1, Rust 1.97.1, Homebrew FFmpeg 8.1.2, built-in Retina display at scale 2. Fresh task-owned projects (made by `aster-agent` over ffmpeg-generated media: a 12 s testsrc2 interview with a speech-like tone, 30 s of 120 BPM clicks, a 0.8 s whoosh) and fresh `--settings` per run. Driven by real pointer and key events into the running window; the normal window (asked 1440x900) came up at 1280x709 points, the narrow one at 760x620 (and 760x560 for ticket 13). Captures are in `artifacts/audio-editing/captures/`. Every capture was looked at against `docs/design.md`. Dark theme only; Linux, Windows and the light theme were not run; nothing was listened to.

`normal-32`/`narrow-32` Normalize loudness chosen: "music.wav measured -22.8 LUFS, peak -5.5 dBFS; volume keys scaled so the loudest is 168%. It stops short of -14 LUFS so that its peak stays at -1 dBTP." with the Volume field reading 168 %; `narrow-32` also shows the "Measuring the loudness of music.wav." notice.
