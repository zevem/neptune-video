# Pitch and pitch-keeping speed

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 11

## Scope

A pitch shift in semitones, and a clip speed that leaves pitch alone when the
clip asks for it. No new field: the presence of an enabled `Pitch` effect is
the switch.

- `crates/editor-media/src/audio/chain.rs`: with the first enabled `Pitch`
  on the clip, `P = 2^(semitones / 12)`, `a = clamp(clip_speed / P, 0.5,
  2.0)` and `k = clip_speed / a`. The speed stage is
  `aresample=R,asetrate=round(R·transport·k),aresample=R` followed by one
  `atempo=a`, left out when `a` is within 0.001 of 1. The clip always plays
  at its speed. While `a` is not clamped, `k = P` and the pitch is the one
  asked for; beyond that the pitch moves by the remainder (speed 4 with
  `Pitch` 0 plays an octave up, not two). Never more than one stage: each
  one makes the sound arrive earlier. Without a `Pitch`, the stage is
  unchanged. The resampling bound (`audio.rs:932-936`) is checked on
  `R·transport·k`.
- `atempo`'s lead is not compensated and not in ticket 11's delay table,
  which drops frames and so can only correct a late signal.
- The transport's shuttle speed keeps shifting pitch in both cases.
- `src/ui/inspector/speed.rs`: a "Keep pitch" toggle in the constant-speed
  card of an audio-track clip, which adds a `Pitch` at 0 or removes a
  `Pitch` that is at 0 (a non-zero one is left and the toggle reads on).
  When `clip_speed / P` leaves 0.5 to 2 the card says "Pitch is kept from
  0.5× to 2×". On a picture clip the row reads "Detach audio to keep
  pitch". The warning
  at `:708` and `docs/usage.md:359` are reworded.
- `src/agent.rs`: `pitch up 2`, `pitch down 3`, `keep pitch`.

Files: `crates/editor-media/src/audio/chain.rs`, `audio.rs`,
`src/ui/inspector/speed.rs`, `src/agent.rs`, `docs/usage.md`,
`docs/architecture.md:337`.

## Result

Implemented. A clip on an audio track with an enabled `Pitch` keeps its
pitch at its speed and is shifted by the effect's semitones alone; without
one, speed changes pitch as before. Nothing was listened to.

- `editor_core::sound` (not in the ticket's file list; see Comments):
  `pitch(clip)` is the first enabled `Pitch`'s semitones, `pitch_ratio`,
  `PITCH_KEPT` (0.5..=2) and `pitch_kept(speed, semitones)`. The chain and
  the inspector read the same rule.
- `audio/chain.rs`: `speed_stage(clip_speed, semitones) -> (k, Option<a>)`
  as the ticket words it; `chain` writes `atempo=<a>` (six decimals) after
  the second `aresample` and checks the resampling bound on
  `R·transport·k`. A `Pitch` the probe found missing is not read: the string
  is today's and "Audio: pitch is not available in this FFmpeg" is said once.
  Within 0.001 of 1 the stage is left out and `k` is the clip speed, so the
  length stays right and the pitch is off by under 2 cents. `atempo` is not
  in the delay table and its lead is not compensated.
- `src/speed.rs`: `keep_pitch_command`, `pitch_command`, `pitch_range`,
  `pitch_kept_speeds`, `DETACH_TO_KEEP_PITCH` and `Curve::heard_speeds`,
  shared by the inspector and the panel. Each is one `SetEffects` in a
  labelled one-command batch ("Keep pitch", "Let speed change pitch",
  "Change pitch"). Refusals in words: no sound, a picture clip ("Detach
  audio to keep pitch." / "Detach audio to change its pitch."), a locked
  track, a full stack of 16, a shift outside -12..12, and nothing to change.
- Inspector: an audio-track clip has no Speed section, so its "constant-speed
  card" is the Playback card; the "Keep pitch" switch is there under the
  speed row (`Pane::pitch_row` in `src/ui/inspector/speed.rs`, called from
  `clip.rs`). On adds a `Pitch` at 0 at the end of the stack; off removes
  every enabled `Pitch` when all are at 0, and refuses in words when a later
  enabled one shifts, since it would then be read. A shifted `Pitch` holds
  the switch on and disabled, and its hover says the shift. When a speed the clip is heard
  at (the typed or dragged one included, and each straight stretch of a map)
  is past the hold, the card says "Pitch is kept from 0.5× to 2×." with the
  range for the shift (1× to 4× an octave up). A picture clip with its own
  sound has the line "Detach audio to keep pitch" in the Speed card; one
  whose sound its linked audio clip owns has none. The silent-stretch note
  now reads "Sound is silent where the clip is held, reversed or ramped;
  elsewhere speed changes its pitch."
- Panel: `pitch up 2`, `pitch down 3` (also `… semitones`, `…st`) and `keep
  pitch` on the one selected clip. The shift is set, not added to; `pitch
  down 0` is keep pitch. Listed in `EXAMPLES` (now 26) and `docs/usage.md`.
- Docs: `usage.md` (speed paragraph, panel phrases, the mixer paragraph and
  a "Keep pitch and pitch shift" section) and `architecture.md` (the two
  "pitch" sentences and the codec boundary paragraph).

## Evidence

On macOS 26.5.2 (arm64, Apple M1), Rust 1.97.1, Homebrew FFmpeg 8.1.2 on
`PATH` (`scripts/check-media-tools.py` passed), 2026-10-13, `dev` profile.
No output device was opened by a test, no native capture was taken and
nothing was listened to.

- `cargo test -p editor-media --locked`: 262 passed, none ignored; in
  `audio/chain_tests.rs` five are new and one changed (a `Pitch` in the
  stack-order test is now the speed stage).
- `cargo test -p editor-core --locked`: all pass; one new test in
  `tests/sound.rs`.
- `cargo test --locked --bin aster`: 699 passed; new tests in
  `src/speed.rs` (3), `src/agent.rs` (1) and
  `src/ui/inspector/speed_tests.rs` (3).
- `cargo clippy -p editor-core -p editor-media -p aster-editor --all-targets
  --locked -- -D warnings`, `cargo fmt --all -- --check`, `cargo check
  --locked --workspace --all-targets` and `scripts/check-architecture.py`
  pass.

Figures, printed as `MEASURE:` under `--nocapture`, the same on three runs:

- Strings: at 0.05, 0.5, 1, 2, 3 and 16× with `Pitch` 0, +12 and -12 there
  is at most one `atempo`, within 0.5..2, `k·a` is the speed to 1e-6 and
  `k = P` wherever speed over `P` is in range; without a `Pitch` the string
  is today's at every speed. 4× keeping pitch is
  `asetrate=96000,…,atempo=2.000000` (an octave up). 16× at a 4× transport
  is refused without a `Pitch` and is 1,536,000 Hz, at the bound, with one.
- Readback through `Mixer::chunk` of a 0.8 s 440 Hz tone followed by
  silence: speed 2 with `Pitch` 0 reads 440.0 Hz and ends at 399.9 ms
  (400 wanted); `Pitch` +12 at speed 1 reads 880.0 Hz and ends at 790.8 ms
  (800 wanted, 9.2 ms early, the lead); speed 2 without a `Pitch` reads
  877.5 Hz (one zero crossing short over 0.2 s) and ends at 400.0 ms; speed
  1 plain reads 439.2 Hz and ends at 799.9 ms.
- Onsets of 5 ms 1 kHz bursts at 1, 10 and 30 s of source, early by:
  `atempo` 0.5: 40.0, 58.5, 58.5 ms; 0.84: 27.5, 28.2, 28.4 ms; 1.5: 8.0,
  16.0, 16.0 ms; 2: 12.0, 13.3, 18.5 ms. None late. Bounds held: 70, 35, 25
  and 25 ms early, 2 ms late.
- Inspector: the switch is one `SetEffects` entry each way, undone as one;
  a shifted `Pitch` holds it on and a click changes nothing.

The criteria this was held to:

- String tests: speeds 0.05, 0.5, 1, 2, 3 and 16 with pitch 0, +12 and -12
  give at most one `atempo` within `0.5..=2.0` and `k · a = speed` to 1e-6;
  `k = P` whenever `speed / P` is in range; no `Pitch` gives today's string
  at every speed.
- Fixture readback: a 440 Hz tone at clip speed 2 with `Pitch` 0 reads
  440 Hz (zero-crossing count) and lasts half as long; with `Pitch` +12 at
  speed 1 it reads 880 Hz and lasts as long; without `Pitch` at speed 2 it
  reads 880 Hz as today.
- Onset test, not an impulse (`atempo` does not preserve one: a single
  sample came out at 0.30 of its height): 5 ms tone bursts at 1, 10 and
  30 s of source through the chain at `a` of 0.5, 0.84, 1.5 and 2. Each
  onset is early by no more than 70 ms at 0.5, 35 ms at 0.84 and 25 ms at
  1.5 and 2, and never late by more than 2 ms. The measured figures are
  printed and written under Result.
- Inspector test: the toggle is one `SetEffects` entry each way.

## Known limits

- With keep-pitch or a pitch shift the sound leads the picture: by about
  13 ms at `a` = 2, 16 ms at 1.5, 27 ms at 0.84 and 38 to 59 ms at 0.5 on
  the design host, moving a few milliseconds from one event to the next
  (measured here: 12.0 to 18.5 ms at 2, 8.0 to 16.0 at 1.5, 27.5 to 28.4 at
  0.84, 40.0 to 58.5 at 0.5).
  It is not compensated. Two stages of 0.5 measured 151 ms, which is why
  only one is ever used.
- Pitch is held only while clip speed over the pitch ratio is within 0.5 to
  2; outside it the remainder is heard as a pitch change.
- `atempo` is adequate for speech and audibly rough on music, more so far
  from 1. No formant preservation. FFmpeg's `rubberband` filter is not in
  ordinary builds and is not used; preferring it when the probe finds it is
  a possible follow-up.
- Keeping pitch needs an audio-track clip (owner decisions 2 and 3).
- Freeze, reverse and ramp segments stay silent.
- A retimed clip's straight segments each restart `atempo` cold.
- Nothing here was listened to at design time or since.
- No native capture shows the switch, the note or the picture clip's line.

## Comments

2026-10-13: Written with the spec. Nothing implemented or measured.

2026-10-13 (review): The ticket said the shift of an `atempo` chain would
be "measured and compensated as in ticket 11". It cannot be: on FFmpeg
8.1.2 at 48 kHz a 5 ms burst at 10 s came out 13.3 ms early through
`atempo=2.0`, 58.6 ms early through `atempo=0.5` and 151.5 ms early through
two stages of 0.5, while `afftdn` was 25.0 ms late. The reviewer's runs
gave -8.0 to -16.0 ms at 1.5 and -26.4 to -27.6 ms (once -8.9) at 0.84.
Hence one clamped stage, an onset test with a tolerance, and the lead in
Known limits.

2026-10-13 (implemented): Built and tested in the core, `editor-media` and
the desktop on macOS 26.5.2 (arm64, Rust 1.97.1, Homebrew FFmpeg 8.1.2).
The rule went into `editor_core::sound` beside `steady_speed`, so that the
inspector's note and the chain cannot disagree on which `Pitch` is read or
where the hold bites. The switch is in the Playback card, the only card an
audio-track clip has. Its note names the range for the shift ("1× to 4×" an
octave up), which reads as the ticket's words at 0. The phrases set the
shift and do not add to it. The onset at 2 measured 18.5 ms early at 30 s,
past the "about 13 ms" written above and inside the 25 ms bound.

2026-10-13 (review fixes): Three findings, all upheld. (1) Off removed only
the first enabled `Pitch`, so on a stack of [`Pitch` 0, `Pitch` +5] it
promoted the +5 to the one read and shifted the clip. Off now removes every
enabled `Pitch` when all are at 0 and refuses "Another pitch effect on that
clip would then shift it +5 semitones; remove it first." otherwise;
disabled ones stay. `src/speed.rs`'s test covers both stacks. (2) The
planner text now says "An enabled pitch, 0 included, makes the clip's speed
keep its pitch." (asserted in `planner.rs`'s audio-kinds test, within the
6 KiB budget); `docs/agent-protocol.md` no longer says audio effects are not
heard and gains a "Keep pitch" rule; `docs/native-ai.md` matches; ticket 22
now checks those docs against 11 and 12 as well. (3) A new inspector test
types 3 into the Playback card's speed field and sees the note before any
step, gone again after Escape, and checks a map's straight stretches (2× and
1× quiet, 2.5× said).
