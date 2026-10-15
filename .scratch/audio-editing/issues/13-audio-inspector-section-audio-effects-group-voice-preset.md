# Audio inspector section, Audio effects group, Voice preset

Status: ready-for-human
Progress: resolved
Type: task
Blocked by: 11

## Scope

The human route to audio effects: equalizer, compressor, limiter, noise
reduction and centre reduction on an audio-track clip.

- `src/ui/inspector.rs`: `Section::Audio`. `sections()`
  (`inspector/clip.rs:109`) returns `&[Section::Audio]` for a clip on an
  audio track; the Playback card (speed, volume, fades) stays at the top of
  it. Below it, one card per audio effect in stack order with an enable
  toggle, rows from `param_row` (`inspector/look.rs:257`) with no keyframe
  diamond, reset, remove and reorder; an add menu listing the audio kinds
  less `Pitch` (its home is the Speed card) with "3-band EQ" and "Voice" at
  the top.
- `inspector/look.rs`, `presented` (`:191`): Hz with a logarithmic drag for
  `Frequency` on audio kinds, dB for `Gain` and `Threshold`, "Ratio" n:1 for
  a compressor's `Amount`, "Make-up" for its `Gain`, "Ceiling" for a
  limiter's `Threshold`, "Noise floor" in dB for a denoise's `Threshold`,
  octaves for `Width`, "Reduce centre" for `CenterCut`.
- A kind in `editor_media::missing_audio_filters()` shows its card with
  "Not available in this FFmpeg build" and disabled rows. While that answer
  is `None` (the probe has not finished) every kind shows as available.
- The section and the group are named "Audio effects" and "Audio". "Sound
  effect" is not used for them anywhere in the interface: it means an
  imported sound clip.
- "Voice": one `SetEffects` that appends `HighPass` 80 Hz, `Denoise` at
  its defaults (0.4, floor -40 dB, as rechecked in ticket 11),
  `EqBand` +3 dB at 3.5 kHz one octave wide and `Compressor` at defaults,
  refused in words when the stack would pass sixteen. "3-band EQ": three
  `EqBand`s at 120 Hz, 1 kHz and 8 kHz.
- `src/ui/library.rs` and `src/ui/browser.rs` (`takes_effects`, `:98-103`):
  an Audio group in the Effects tab whose tiles go to media clips on audio
  tracks only; a picture clip is refused in words ("Detach audio first").
- `src/agent.rs`: `reduce noise 60%`, `enhance voice`, `add eq`,
  `add compressor`, `add limiter`.
- `docs/design.md:49` and `docs/usage.md:103` (a sound clip now has a
  section).

Files: `src/ui/inspector.rs`, `inspector/clip.rs`, `inspector/look.rs`,
`src/ui/library.rs`, `src/ui/browser.rs`, `src/ui/mod.rs` (`ClipDraft`
already carries `effects`), `src/agent.rs`, the two docs.

## Result

Implemented and tested headless; the native captures are not taken (see
Evidence). A person can see, add, switch, set, reorder, reset and remove an
audio effect on a clip on an audio track, from the inspector, the library
and the command panel. Nothing was listened to.

- `src/audio.rs` (new; not in the ticket's file list, see Comments): what
  the three routes share, as `src/speed.rs` is for pitch. `Preset::{Voice,
  ThreeBandEq}` with `effects()` (fresh ids each call), `added()` (the
  audio kinds less `Pitch`), `unavailable(kind, missing)`, `target`,
  `with_given`, `preset_command`, `give_command` and `noise_command`, and
  the words `DETACH_FIRST` and `UNAVAILABLE`. Voice is `HighPass` 80 Hz,
  `Denoise` at its defaults (no stored values: 0.4, floor -40 dB),
  `EqBand` 3.5 kHz +3 dB one octave and `Compressor` at its defaults;
  3-band EQ is three flat `EqBand`s at 120 Hz, 1 kHz and 8 kHz. A preset is
  one `SetEffects` in a batch labelled "Add Voice" or "Add 3-band EQ".
  Refusals in words: a picture with sound ("Detach audio first: audio
  effects go on clips on audio tracks."), a picture whose sound is already
  detached (`ON_LINKED`: "Sound is on the linked audio clip: give it audio
  effects there."), a clip without sound or off an
  audio track, a locked track, a full stack, and a preset past sixteen
  ("Voice adds 4 effects and that clip has room for 3; a clip holds 16 at
  most.").
- Inspector. `Section::Audio` ("Audio"); `sections()` gives `&[Section::Audio]`
  for a clip on an audio track, so no section control is drawn.
  `src/ui/inspector/audio.rs` (new) draws the Playback card (speed, Keep
  pitch, volume, fades, as before), then "Audio effects": a card per effect
  in stack order, Pitch included, with the Look section's heading (now the
  shared `Pane::effect_head`) whose switch is a speaker named "Hear
  <name>", rows from `look::spec` by the new `Pane::plain_param_row` (field
  and reset, no diamond, no mix), and "Add audio effect" with Voice and
  3-band EQ above a hairline and then High-pass, EQ band, Compressor,
  Limiter, Noise reduction, Reduce centre. A kind added from the menu goes
  through the draft ("Change effects"); a preset is an action of its own,
  made of the stack as shown, and opens its first effect. The stack edit
  shared with the Look section is `Pane::restack`. Timing stays under the
  section.
- `inspector/look.rs`. `presented`: Hz with `Field::log` (a drag moves the
  value by a part of itself, the whole range in 300 points) for an audio
  kind's `Frequency`; dB for its `Gain` and `Threshold`; ":1" for a
  compressor's `Amount`; " oct" for `Width`; " st" for `Pitch`. `label`:
  Frequency, Gain, Ratio, Make-up, Ceiling, Noise floor, Shift; centre
  reduction's row is "Amount" (see Comments).
- Unavailable kinds. `InspectorInput::missing` is
  `editor_media::missing_audio_filters()` read each frame by `App`, never
  waited for. While it is `None`, and when it is empty, nothing is
  unavailable. A missing kind's card says "Not available in this FFmpeg
  build" whether open or not and its rows are disabled; its switch, moves
  and removal still work. It is never refused: the menu row, Voice, a tile
  and a phrase all add it (see Comments, review).
- Library. `browser::offered` now lists every kind but `Pitch`, tables,
  plates and Stabilize, so the Effects tab ends with the heading "Audio"
  and six tiles drawn as small response plots. `browser::apply` gives an
  audio kind through `audio::give_command`: to the selected clips on audio
  tracks (others passed over), or to the clip it is dropped on; a picture
  is told why in words and nothing changes.
- Panel. `enhance voice`, `add eq` (also `add 3-band eq`: the preset, not
  one band), `add compressor`, `add limiter`, `reduce noise`, `reduce noise
  60%`, `reduce noise by 60%`, for the selected clips. `add effect` and
  `remove effect` now name the audio kinds too (`add effect high-pass`,
  `remove effect compressor`), which lifts ticket 10's limit. `EXAMPLES` is
  27 entries.
- Docs: `design.md` (the sections, the Audio rows, unavailable kinds, the
  library heading, "audio effect" against "sound effect") and `usage.md`
  (the sections paragraph, the phrase list and an "Audio effects" section).
- Serialized agent commands are unchanged: an agent's `set_effects` stack
  is what the section shows.

## Evidence

On macOS 26.5.2 (arm64, Apple M1), Rust 1.97.1, 2026-10-13, `dev` profile.
Only the desktop crate changed, so no other crate was built or tested. No
native window was opened and nothing was listened to.

- `cargo test --locked -p aster-editor`: 717 and 12 passed, three runs;
  719 and 12 after the review fixes (one run).
  New tests:
  - `src/audio.rs`: `the_presets_write_the_effects_the_spec_names_with_fresh_ids`,
    `a_preset_is_one_step_undone_whole_and_saved_as_ordinary_effects`
    (one history entry, JSON round trip with no trace of the preset, twice
    is twice, undone to the project as it was),
    `a_clip_that_takes_no_audio_effect_is_refused_in_words` (picture,
    title, nothing selected, a mixed selection, locked track, 13 + Voice
    refused and 13 + 3-band EQ taken, 16 + one refused, and a 17-effect
    `SetEffects` refused by the project with the snapshot and history
    unchanged), `noise_reduction_is_set_where_it_is_and_added_where_it_is_not`,
    `a_kind_is_unavailable_only_once_the_probe_says_so`.
  - `src/ui/inspector/audio_tests.rs`, through real egui passes:
    `an_audio_clip_has_the_audio_section_under_its_playback_and_a_video_clip_has_none`
    (also a locked track's add button disabled),
    `each_row_reads_its_unit_and_range_from_the_table_and_has_no_diamond`
    (every row of the seven kinds reads its default with its unit, e.g.
    "80 Hz", "3.0:1", "-40.0 dB", "1.00 oct", "0.0 st"; min and max are the
    table's; only Frequency is logarithmic; no "Animate", keyframe step or
    mix control), `a_committed_drag_is_one_entry_and_a_frequency_moves_by_octaves`
    (held: no step and the project untouched; released: one "Change
    effects"; the same 30-point drag multiplies 100 Hz and 1 kHz by the
    same ratio within 5%), `an_audio_effect_is_added_switched_reordered_reset_and_removed`
    (menu order, no Pitch or picture kind), `voice_and_three_band_eq_write_fresh_effects_in_one_entry_each`,
    `voice_past_sixteen_effects_is_refused_in_words_and_changes_nothing`,
    `an_unfinished_probe_shows_every_kind_and_a_finished_one_disables_the_missing`
    (`None` and an empty answer disable nothing; `[Denoise, CenterCut]`
    gives exactly two lines, disables those rows and no other, and the
    card can still be removed),
    `a_missing_kind_is_added_as_every_route_adds_it_and_its_card_says_so`
    (review: with `Denoise` missing every menu row and Voice are enabled,
    Voice and the lone row both add it, and each card says so),
    `a_picture_clip_with_sound_is_told_to_detach_it_first`,
    `the_audio_section_fits_a_narrow_panel` (300 and 240 points, Voice
    applied, a band open, a kind missing), `the_audio_section_asks_for_no_frame_while_nothing_is_done`.
  - `src/ui/transitions_tests.rs`: `an_audio_tile_goes_to_a_sound_and_a_picture_is_told_why_not`
    (a tile dropped on a video with sound and on a silent one changes
    nothing and says why; given to a selection of three it goes to the
    sound alone, one step, undone) and
    `the_effects_tab_lists_the_picture_kinds_it_listed_before_then_the_audio_kinds`
    (replaces the ticket 10 test of the same list).
  - `src/agent.rs`: `a_sound_on_an_audio_track_is_given_audio_effects_by_phrase`
    (each phrase on a picture refused with `DETACH_FIRST`, then, after
    `detach audio` with the picture still selected, with `ON_LINKED`; then
    applied as one step on its detached sound; noise set, added and refused
    when unchanged or out of range).
  - Review: `src/audio.rs`
    `a_picture_whose_sound_is_detached_is_pointed_to_the_linked_clip`
    (a kind, both presets and noise on the detached picture say
    `ON_LINKED`; with its sound also selected, the sound takes it).
  - Changed: `a_sound_shows_the_audio_effects_an_agent_wrote_and_the_card_leaves_them`
    (was ticket 10's "keeps them through the sound card"), the sections
    assert of `a_title_and_a_sound_are_offered_only_what_the_project_accepts`,
    and `effects_are_added_and_removed_by_name`.
- `cargo clippy --locked --bin aster --bin aster-agent --all-targets -- -D
  warnings`, `cargo fmt --all -- --check` and
  `python3 scripts/check-architecture.py`: clean.
- One test was flaky while written: the centre-reduction row and its card
  had one name, and the accessibility tree's order chose between them.
  The row is "Amount" now; three later runs of the package passed.

Done when:

- Inspector tests: an audio clip offers the Audio section and a video clip
  does not; each row's unit and range come from `look::spec`; a committed
  drag is one entry; no diamond is drawn; Voice and 3-band EQ write the
  effects named above with fresh ids and are one entry each.
- Browser test: an Audio tile dropped on a video clip changes nothing and
  says why.
- Agent phrase tests.
- Native captures of the final build, normal and narrow: the Audio section
  empty, with a Voice preset, with an unavailable kind, and the Audio
  group.
- A test that an unfinished probe shows no kind as unavailable and a
  finished one disables exactly the missing kinds.

All but the captures are met by the tests above.

Not done, and why this is not resolved: **no native capture was taken.**
None of the four states can be reached at launch: a clip is selected and a
library tab chosen only by pointer or keyboard, and neither is kept in a
project or the settings; this host offers no private display to drive.
For a person, at 1440x900 and 760x560 with a release build, a sound on an
audio track and fresh `--settings`: select the sound (the empty Audio
section), Add audio effect → Voice (four cards, High-pass open), the same
with `afftdn` unavailable (an FFmpeg without it, or `ASTER_FFMPEG` pointing
at one; the Noise reduction card says "Not available in this FFmpeg
build"), and the library's Effects tab scrolled to Audio.

## Known limits

- No EQ curve display; bands are rows of numbers.
- No attack, release or knee controls; no gain-reduction meter.
- A change is heard on release, when the decoders restart.
- Nothing remembers that a preset made its effects; applying Voice twice
  adds them twice.
- A picture clip's sound has no section until it is detached.
- The probe's answer is read when the inspector is drawn; a probe that
  finishes while nothing is drawn shows on the next frame, not at once.
- A preset is not a library tile; Voice and 3-band EQ are in the add menu
  and the panel.
- A tile held over a picture clip is still drawn as accepted; the refusal
  is said when it is let go.
- The ranges, defaults, the presets' values and the units are as designed;
  none was set by ear.
- Linux and Windows were not run.

## Comments

2026-10-13: Written with the spec. Nothing implemented or measured.

2026-10-13 (review): Renamed from "Sound effects group": the owner's
feature 4 uses "sound effects" for whooshes and hits, and a group of that
name would read as a library of sounds, which the epic does not ship.
Unavailable kinds come from the execution probe, not from
`AudioService::unavailable`. `Denoise` shows two rows.

2026-10-13 (implemented): Built and tested headless on macOS 26.5.2 (arm64,
Rust 1.97.1) as recorded under Evidence; the native captures are a
person's to take, and nothing was listened to. Read where the ticket left
room, and two departures:

- *A module the ticket did not list.* The presets, the refusals and the
  noise rule are in `src/audio.rs`, so that the inspector, the library and
  the phrases cannot disagree, as `src/speed.rs` is for pitch.
- *Centre reduction's row is "Amount"* (a departure). The ticket asks for
  "Reduce centre" in `presented`; the card already bears that name (the
  spec's, kept from ticket 10), and a row of the same name made two
  controls one name for a screen reader and the tests.
- *The library lists kinds, not presets* (a reading). The Audio heading has
  the six kinds the add menu lists; Pitch is left out of both, since its
  home is the Playback card. Voice and 3-band EQ are in the menu and the
  panel only.
- *Cards open one at a time,* as the Look section's do, which keeps a
  stack of Voice and 3-band EQ short and each row's name unique. The
  switch is a speaker, "Hear <name>".
- *Missing kinds are never refused* (changed in review, below): their
  cards say so and disable their rows, and keep the switch, the moves and
  the removal.
- *Several clips.* The library and the phrases give to every selected clip
  on an audio track and pass the rest over; when none takes it, the first
  reason is said (ticket 07's rule for fades).
- *Phrases.* `add eq` is the 3-band EQ preset; one band is `add effect eq
  band`. `reduce noise 60%` sets the first enabled noise reduction or adds
  one with the default floor; without a number it is the default 40%.
  `add effect` and `remove effect` take the audio kinds.
- *A picture is told in three ways:* one with sound to detach it first,
  one whose sound is already detached that it is on the linked audio clip
  (added in review), and one without that only a sound on an audio track
  takes an audio effect.

2026-10-13 (review fixes): Two findings of the adversarial review held and
are fixed, each with a test that fails on the code before it:

- *A detached picture was told to detach again.* `audio::target` answered
  "Detach audio first" to a picture whose sound a linked audio-track clip
  already owns, and `detach audio` then refused it: a loop, reached at once
  by `detach audio` then `enhance voice`, since the picture stays
  selected. It now checks `sound::owned` first and says `ON_LINKED`
  ("Sound is on the linked audio clip: give it audio effects there.", the
  words of `fades::OWNED`). Tested in `src/audio.rs` and in the agent
  phrase test.
- *The routes disagreed on unavailable kinds.* Only the inspector's
  single-kind menu rows were disabled; Voice, the library tiles and the
  phrases added the same kind silently, and `editor-agent`, which depends
  on `editor-core` alone, cannot see the probe at all. The rule is now one
  for every route: availability never refuses an edit. A missing kind is
  added like any other; its card says "Not available in this FFmpeg build"
  and disables its rows, and the mixer leaves it out of the sound. This
  keeps the project independent of one machine's FFmpeg. The menu-row
  disabling is removed, which returns to what the ticket asked for.

2026-10-14 (native acceptance): Resolved. The captures this ticket waited for are taken and look right. Release build `target/release/aster` (sha256 fadbe5aa8244d800b2d9b16bf86bf121a6e9157af7e8067402761d40318e5763, `cargo build --release --locked`) on macOS 26.5.2 (25F84), Apple M1, Rust 1.97.1, Homebrew FFmpeg 8.1.2, built-in Retina display at scale 2. Fresh task-owned projects (made by `aster-agent` over ffmpeg-generated media: a 12 s testsrc2 interview with a speech-like tone, 30 s of 120 BPM clicks, a 0.8 s whoosh) and fresh `--settings` per run. Driven by real pointer and key events into the running window; the normal window (asked 1440x900) came up at 1280x709 points, the narrow one at 760x620 (and 760x560 for ticket 13). Captures are in `artifacts/audio-editing/captures/`. Every capture was looked at against `docs/design.md`. Dark theme only; Linux, Windows and the light theme were not run; nothing was listened to.

At 1280x709 and 760x560: `normal-24`/`narrow560-01` the empty Audio section; `normal-25`/`narrow560-02` the Add audio effect menu (Voice, 3-band EQ, the kinds); `normal-26`/`narrow560-03` Voice with High-pass open at 80 Hz; `normal-27`, `narrow-27` the four cards; `normal-41`, `normal-42`, `narrow560-04`, `narrow560-05` Noise reduction saying "Not available in this FFmpeg build" with its rows disabled; `normal-38`/`narrow560-06` the Effects tab's Audio group. The unavailable kind was made with `ASTER_FFMPEG` pointing at a task-owned wrapper that runs the host FFmpeg but fails any command naming `afftdn`, not with a real FFmpeg build lacking it.
