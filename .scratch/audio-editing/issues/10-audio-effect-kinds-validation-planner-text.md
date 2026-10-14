# Audio effect kinds, validation and planner text

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 01, 05a, 05b, 05c

## Scope

Seven audio kinds in the effect table, legal on audio-track clips, named to
the agent. Nothing is heard yet: the mixer ignores effects until ticket 11.

Core (`crates/editor-core/src/`):

- `look.rs`: `EffectKind::{HighPass, EqBand, Compressor, Limiter, Denoise,
  Pitch, CenterCut}`, `ALL: [Self; 27]`, `is_audio()`, the seven spec rows
  from the spec (existing keys `Frequency`, `Gain`, `Width`, `Threshold`,
  `Amount` only; `Denoise` has `Amount` and `Threshold`), `needs_asset()`
  false, `is_curve()` false.
- `model.rs:647-661`: the audio-clip rule changes from "no effects" to "no
  effect that is not an audio kind". New rules: an audio kind on a clip that is
  not on an audio track is `Invalid("audio effect on a picture clip")`; a
  audio effect with `mix != 1.0` is invalid. The animation rule
  (`a.target.is_some() || a.key != ParamKey::Gain`) is unchanged, so a sound
  parameter cannot be keyed.

Seams that must follow:

- `crates/editor-agent/src/planner.rs`, three changes to `SYSTEM` (`:422`):
  1. a `"<kind> <keys…>"` line for each kind (the test at `:847-872` forces
     it): `high_pass frequency; eq_band frequency gain width; compressor
     threshold amount gain; limiter threshold; denoise amount threshold;
     pitch amount; center_cut amount`.
  2. One units sentence, since the text says effect "lengths are fractions
     of the layer box or canvas height, angles are degrees" and nothing in
     the schema carries a range: "Audio kinds: frequency is Hz, gain and
     threshold are dB, eq_band width is octaves, compressor amount is the
     ratio 1..20, pitch amount is semitones -12..12, other amounts are
     0..1."
  3. The sentence "Clips on audio tracks take only gain keyframes and
     retime." is replaced, not added to: "Clips on audio tracks take only
     audio kinds (mix 1, not keyed), gain keyframes and retime."
  Sizes re-measured against ticket 01's budget.
  `docs/agent-protocol.md:178` and `docs/native-ai.md`.
- `src/clipboard.rs`: the round-trip test gains a clip with an audio effect
  (the version is already 3 from ticket 05a).
- Every exhaustive `match` on `EffectKind` in `src/` (inspector labels in
  `src/ui/inspector/look.rs`, icons, `src/compose/`), with audio kinds
  drawing nothing. The Effects tab and the Look section's add menu filter
  `EffectKind::ALL` by `!is_audio()`, so no audio kind is offered to a
  picture clip.
- `crates/editor-store/tests/schema.rs`: a round trip with one of each kind
  on an audio clip.

## Result

Implemented. Seven audio kinds are in the effect table, legal on clips on
audio tracks and nowhere else, and named to the agent with their units.
Nothing is heard: the mixer does not read them until ticket 11.

- `look.rs`: the seven kinds after `Stabilize`, `ALL: [Self; 27]`,
  `is_audio()`, the seven rows as the spec has them. `Effect::validate`
  holds an audio kind to `mix == 1.0`; `points` and `asset_id` were already
  refused by the rules for kinds that are no curve and need no asset.
- `model.rs`: a clip on an audio track refuses any effect that is not an
  audio kind (`Invalid("audio clip look")`, as before), and a clip on any
  other track refuses an audio kind (`Invalid("audio effect on a picture
  clip")`). A title with one is refused as a title is, `"text clip look"`.
  A part mix, a value out of range and a key of another table are
  `Invalid("effect")`. The animation rule is unchanged, so a track that
  targets an audio effect is `"audio clip look"`.
- `planner.rs`: `SYSTEM` names the seven kinds with their keys, carries the
  units sentence word for word and has the audio-track sentence replaced.
  The seven lines are introduced by "and the audio kinds, for clips on audio
  tracks only:", which the ticket did not ask for (see Comments). The two
  caps are `MAX_SYSTEM_BYTES` (6 KiB) and `MAX_SCHEMA_BYTES` (23 KiB), one
  constant each: the two test asserts, a compile-time assert on `SYSTEM` and
  the runtime guard in `provider/mod.rs` (now `command_line_schema`) read
  them. This is owner decision 1's default, which the owner may reverse.
- Docs: `agent-protocol.md` (the audio-track rule, and an "Audio effects"
  entry with each kind's keys, units, ranges and defaults under "Sound
  commands"), `native-ai.md` (the bounds row, and what the planner is told),
  `verification.md` (the one row that named 24 KiB).
- Desktop: `browser::offered` leaves audio kinds out, which is what the
  Effects tab and the `add effect`/`remove effect` phrases list. The Look
  section's menu adds a kind only of its four families, and the audio kinds
  are a family of their own (`Family::Audio`, "Audio"), so it offers none.
  They have names (`effect_name`), draw no swatch and count for no pass in
  `compose` (`most` is 0). Nothing else in `src/` matched exhaustively.
- Routes. An audio effect enters by `SetEffects` alone: as a serialized
  command from an external agent, in a native plan, or by paste. No
  shortcut, menu, gesture, inspector row or panel phrase adds, shows or
  removes one before ticket 13; the inspector of a clip on an audio track
  still shows the sound card and leaves the stack as it is.

## Evidence

On macOS 26.5.2 (arm64), Rust 1.97.1, 2026-10-13, `ci` profile. No provider
request was made, no application was launched and nothing was listened to.

- `cargo test --profile ci --locked -p editor-core -p editor-store`: all
  pass (core: 189 in eleven files, `sound.rs` 48; store: 88, `schema.rs`
  22). New in `crates/editor-core/tests/sound.rs`:
  `the_audio_kinds_are_the_last_seven_and_use_the_keys_there_are`,
  `each_audio_kind_is_taken_on_an_audio_clip_within_its_ranges_and_refused_
  outside` (defaults, both ends, the next `f32` beyond each end, NaN and the
  infinities, a foreign key), `an_audio_kind_is_refused_off_an_audio_track_
  and_a_picture_kind_on_one` (a picture with sound, a silent video, a still,
  a title; every picture kind on a song; a video's sound on an audio track
  takes them), `an_audio_effect_is_whole_takes_no_points_or_media_and_is_
  not_keyed` (`mix` 0.5 and four others, `points`, `asset_id`, a disabled
  effect, a `SetKeyframes` at each kind), `an_audio_clip_holds_sixteen_
  audio_effects_and_not_seventeen`, `a_refused_set_effects_leaves_the_
  stack_and_the_history_as_they_were` (alone and as the last of a batch,
  then undo and redo of the accepted one) and `audio_effects_are_written_
  by_name_and_read_back`. Each refusal is checked against the same snapshot
  pointer and revision.
- New in `crates/editor-store/tests/schema.rs`: one of each kind on an
  audio clip through a project file and a checkpoint (version 3, the seven
  names as written, a second save byte-equal), through recovery with the
  step undone and redone, and a file tampered four ways (on a picture clip,
  `mix` 0.5, 19 Hz, a `glow` on the sound) refused by load and save and
  left as it was; a project that breaks the rule is written nowhere.
- `cargo test --profile ci --locked -p editor-agent --lib -- --nocapture
  planner::tests::planner_text planner::tests::schema_is`: passed, printing
  `planner_instructions: 5308 bytes` (428 more than 4,880; 836 left under
  6 KiB) and `proposal_schema: 21859 bytes` (76 more than 21,783, as ticket
  01 estimated; 1,693 left under 23 KiB). New planner tests:
  `planner_text_gives_the_audio_kinds_their_units_and_their_place` (the
  units sentence and the new audio-track sentence present, the old one
  absent, the ranges the sentence states equal to the table's, exactly the
  audio kinds listed after the words that place them),
  `the_instruction_and_schema_caps_share_twenty_nine_kib`, and
  `audio_effects_are_planned_on_an_audio_clip_and_refused_on_a_picture`
  (each kind as provider JSON; a picture clip, `mix` 0.5, a value out of
  range, a picture kind and an unknown kind refused with the document
  untouched; one applied, not keyed, undone). New in `provider/mod.rs`:
  `a_schema_at_its_part_of_the_command_line_is_refused_before_launch`.
- `cargo test --profile ci --locked -p editor-agent --test protocol`: 16
  passed. New: `serialized_audio_effects_are_validated_inspected_and_
  undone` (six refusals as JSON text answer `invalid_edit` with the same
  snapshot and revision; one of each kind applied, read back by an
  inspection as sent, a keyframe on one refused, undone).
- `python3 scripts/test-workspace.py --profile ci --package editor-agent`:
  75 of 76 unit tests pass. The one failure is `provider::tests::an_
  unrepresentable_native_path_is_refused_before_creating_an_instruction_
  file`, which fails on this host's filesystem before the code under test
  runs, as tickets 01, 05a and 05c record. The script stops there, so the
  protocol tests were run by the command above.
- `cargo test --profile ci --locked -p aster-editor`: 692 and 12 passed.
  New: `clipboard::tests::a_copied_audio_effect_survives_the_system_
  clipboard` (text round trip, then pasted), `the_effects_tab_lists_the_
  picture_kinds_it_listed_before_the_audio_kinds` (the seventeen entries
  under their headings, in order, and none of the audio names drawn),
  `a_sound_with_audio_effects_keeps_them_through_the_sound_card`; the
  add-menu count (11) and the `add effect` phrase tests now also say that
  no audio kind is among them.
- `cargo clippy --profile ci --workspace --all-targets --locked -- -D
  warnings`, `cargo fmt --all -- --check` and
  `python3 scripts/check-architecture.py`: clean. The workspace was linted
  because `EffectKind` is read by every crate but `editor-media`, which
  was compiled with it.

Done when:

- Core tests: each kind validates on an audio-track clip at its defaults and
  at both ends of each range and is refused just outside; an audio kind on a
  video-track clip and a picture kind on an audio-track clip are both
  refused; `mix` 0.5 refused; an animation track targeting an audio effect
  refused; `points` and `asset_id` refused; sixteen audio effects accepted
  and seventeen refused; `SetEffects` failure leaves the project unchanged.
- `planner_text_names_every_parameter_table_and_stays_short` and the schema
  cap test pass, sizes printed. A planner test asserts that the units
  sentence is present and that the old audio-track sentence is not.
- The library's Effects tab lists the same picture kinds as before (a test
  over the list).

## Known limits

- With this ticket alone an audio effect can be stored (by an agent or the
  panel's JSON route) and is not heard.
- Ratio is carried by `Amount`, make-up by `Gain` and the noise floor by
  `Threshold`: the names an agent sees are the keys, and the units sentence
  is all that tells it what they mean. No real provider request is made.
- Audio kinds on picture clips are refused (owner decision 2).
- The ranges and defaults are design guesses, to be set in ticket 11.
- A person cannot see, add or remove an audio effect in the desktop before
  ticket 13: `remove effect compressor` answers that no effect has that
  name. One written by an agent stays until `set_effects` replaces the
  stack or the step is undone.
- An audio-track clip with audio effects dragged or pasted onto a video
  track is refused whole by the project, as a picture with effects moved to
  an audio track already was; the words a person is shown for that were not
  looked at here.
- `look::animatable` still answers true for an audio kind's keys. The rule
  that they take no keys is the audio-track rule in `Project::validate`;
  ticket 13's rows must not offer a diamond.
- `SYSTEM` gives no range for `frequency`, `gain`, `threshold` or `width`,
  as it gives none for a picture kind; the protocol document does.
- ADR 0006 still says the provider schema has a 24 KiB cap; it is a record
  of that decision and was left for ticket 22, whose Scope now names the
  line and the caps the code has.
- Linux and Windows were not run. Windows command-line headroom for 29 KiB
  stays unmeasured.

## Comments

2026-10-13: Written with the spec. Nothing implemented or measured.

2026-10-13 (review): The planner seam said only to add a sentence. As
written the instructions would have told a provider both that audio-track
clips take only gain keyframes and retime and that audio kinds go on them,
with no unit for `gain` (linear 0..8 on a clip, dB here), `amount` (a ratio
on a compressor, semitones on `pitch`), `frequency` or `width`. `Denoise`
gained `Threshold` (ticket 11). The kinds are called audio effects, not
sound effects, which stays the name of an imported sound clip.

2026-10-13 (implemented): Built and tested on macOS as recorded under
Evidence. Read where the ticket left room, and one departure:

- *Budgets.* The owner has not answered decision 1, so ticket 01's
  recommended split is built as the default: instructions under 6 KiB,
  schema under 23 KiB. All seven kinds stay, `center_cut` among them
  (decision 8, also unanswered).
- *The guard is "under", as the tests are.* It refused a schema over
  24 KiB and now refuses one of 23 KiB or more, so a schema of exactly the
  cap is refused where the old guard would have passed its own.
- *The kinds are placed in the text* (a departure). The ticket's three
  changes never say which kinds are audio kinds or that a picture clip
  refuses them: `denoise` and `limiter` read as picture kinds too. The
  seven lines are therefore introduced by "and the audio kinds, for clips
  on audio tracks only:", 53 bytes; the units sentence and the replaced
  sentence are the ticket's, word for word.
- *Where the `mix` rule is.* In `Effect::validate`, with the other rules of
  one effect, so it answers `"effect"`; where the clip is, is the
  project's, and answers `"audio effect on a picture clip"`.
- *The desktop has a family for them,* "Audio", last in the library's
  order, and names ("High-pass", "EQ band", "Compressor", "Limiter",
  "Noise reduction", "Pitch", "Reduce centre"); nothing lists either yet.
  Ticket 13 may rename them.

2026-10-13 (review of the implementation): No rule changed. Two records
were loose and are tightened:

- Ticket 22 did not carry the ADR 0006 handoff named under Known limits;
  its Scope now does.
- The store test of tampered audio effects asked only that the refusal
  contain "effect" for the `mix` and 19 Hz cases, which "audio effect on a
  picture clip" and "effect or mask count" contain too. It now compares
  the reason whole, as the core tests do.

2026-10-14 (ticket 22): Four known limits above are older than the code. An
audio effect is heard (ticket 11, `Pitch` in 12); a person sees, adds and
removes one (13); ADR 0006 carries a dated note with the 23 KiB and 6 KiB
caps; owner decision 1 was taken on the owner's behalf as this ticket built
it. The instructions measured 5,377 bytes on 2026-10-14, after ticket 12's
pitch sentence; the schema is unchanged at 21,859.
