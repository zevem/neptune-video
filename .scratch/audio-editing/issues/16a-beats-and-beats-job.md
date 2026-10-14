# `beats()` and the `Beats` job

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 03

## Scope

Find the beats of a music file from its cached peaks. Pure and headless: no
desktop change.

- `crates/editor-media/src/sound/beats.rs`: `pub fn beats(peaks: &Peaks) ->
  Option<BeatGrid { bpm: f32, beats: Vec<u32> /* ms, container time */ }>`,
  pure.
  1. Onset envelope at 100 Hz: the half-wave rectified rise of `low` plus
     half that of `rms` (both already in dB), less a 1 s moving mean.
  2. Tempo: autocorrelation of the envelope over lags of 30 to 100 buckets
     (200 to 60 BPM), weighted by a log-normal prior centred on 120 BPM,
     peak refined by parabolic interpolation.
  3. Beats: dynamic programming over the envelope (Ellis 2007), each beat
     scored by its onset strength and by how close it is to one period
     after the previous one.
  Bounded to 16,384 beats; a file with no clear period returns `None`.
- `sound.rs`: job `Beats { asset }` that reads the peak file on the loader
  thread and runs `beats`, answering `SoundEvent::Beats { request, project,
  asset_id, source, result }`. Replacement target: (`Beats`, asset), so it
  never stops the same asset's `Peaks` job. A missing peak file is an error
  in words; the caller asks for peaks first.

Files: `crates/editor-media/src/sound/beats.rs`, `sound.rs`,
`sound/tests.rs`, `lib.rs`.

## Result

Implemented. Headless: nothing in the desktop asks for beats yet (ticket
16b), so there is no shortcut, menu, gesture, inspector control or agent
command to trace, and no project state, command, save or load is touched.

- `sound/beats.rs` (new): `pub fn beats(peaks: &Peaks) -> Option<BeatGrid>`,
  `BeatGrid { bpm: f32, beats: Vec<u32> }` and `MAX_BEATS` (16,384), all
  exported from `lib.rs`. Pure, the three steps as scoped:
  1. The onset envelope is the rise of `low` plus half the rise of `rms`
     (in dB, through `Peak::decibels`), none for a fall, less the mean over
     the 101 buckets around it (clipped at the file's ends). Before the
     file is silence, so the first bucket's rise is from the floor.
  2. The tempo is the lag from 30 to 100 buckets whose correlation, per
     bucket of overlap and over the envelope's own power, with half of each
     neighbour lag's added, is highest once weighted by a log-normal prior
     on 120 BPM of one octave. The period is clear when that lag's own
     correlation is at least 0.1 and at least four standard errors of noise
     of that length (`4 / sqrt(overlap)`); only lags that fit four times in
     the file are looked at. The parabola through the peak and its
     neighbours refines it, kept within 30 to 100 buckets (60 to 200 BPM).
  3. The beats are Ellis's dynamic programming over the envelope in units
     of its spread, smoothed by a bell a thirty-second of a period wide: a
     beat follows another from half a period to two periods on, paying
     `100 · ln(d / period)²`, and a path is followed only while it scores
     above nothing. The path ends at its best score in the last two
     periods; beats at either end under half the typical beat's score (the
     path running on through silence) are dropped. Fewer than two beats is
     `None`. Past 16,384 the later beats are not given.

  A beat is the middle of the bucket its onset is in, in milliseconds of
  container time, since bucket 0 is container time 0.
- `sound.rs`: `SoundService::beats(job: PeakJob)`. The job is a `PeakJob`
  (project, asset, cache), as `load_peaks` takes one: the beats are of the
  peaks that cache keeps. It runs on the thread that reads the cache
  (`aster-sound-load`), in the queue of kept-peak reads (`PEAK_LOADS`),
  which now holds a `Load::{Kept, Beats}`; the worker and its FFmpeg are
  not involved. The answer is `SoundEvent::Beats { request, project,
  asset_id, source, result: Result<Option<BeatGrid>, String> }`: `Ok(None)`
  is sound with no clear period, kept apart from an error.
- Replacement target (`Beats`, asset), beside (`Kept`, asset) for kept-peak
  reads in the same queue. A newer request for the same file's beats gives
  up the one waiting and the one under way, neither of which answers; a
  kept-peak read, the file's `Peaks` job and anything of another file are
  left alone. A newer that would have to wait without room is `Busy` and
  gives up nothing.
- Refusals before the cache is read: "Media has no sound to find beats
  in", "Media has no length", "Media is too long to find beats in: the
  limit is 5.8 hours". Errors in words: "The media's sound has not been
  read: its peaks are needed to find its beats" (no peaks kept) and "The
  media changed since its sound was read: its peaks are needed again to
  find its beats" (stale peaks). Neither reads the sound.
- The desktop: `HeldPeaks::answered` (`src/peaks.rs`) passes over a `Beats`
  as it does a `Measured`, so the match stays exhaustive. No other desktop
  change.
- `docs/performance.md`: a Beats bound and a measurement row under the
  sound lane.

## Evidence

On macOS 26.5.2 (arm64, Apple M1), Rust 1.97.1, Homebrew FFmpeg and ffprobe
8.1.2 on `PATH` (`scripts/check-media-tools.py` passed), 2026-10-14, `dev`
profile unless named. No test was skipped: both fixtures ran against the
real tools. Nothing was listened to.

- `cargo test -p editor-media --locked sound::`: 40 passed, five runs in a
  row after one test race was fixed (see Comments); `cargo test -p
  editor-media --locked`: 287 passed, one run. `cargo test -p aster-editor
  --locked peaks`: 18 passed. `cargo clippy -p editor-media --all-targets
  --locked -- -D warnings`, `cargo clippy -p aster-editor --all-targets
  --locked -- -D warnings`, `cargo fmt` of both crates and
  `scripts/check-architecture.py` pass.
- Synthetic peaks, printed as `MEASURE:` under `--nocapture`: click tracks
  of three minutes over a quiet noise bed at 60, 90, 120, 150, 128 and
  170 BPM read 60.00, 89.78, 120.00, 150.00, 127.76 and 170.79 BPM, with a
  beat within 5.0 ms of every click, as many beats as clicks and none off a
  click; at 180, 190 and 200 BPM they read half (see Known limits), a beat
  on every other click. First clicks at 0, 0.37, 0.71 and 2.13 s are found
  in phase, the one at 0 s at 5 ms (after the review fixes, see Comments);
  silence before the first click
  and after the last has no beats. Silence, an empty file, a steady tone,
  five kinds of steady noise (three minutes, ten and five seconds) and a
  second of clicks give `None`. 5.8 hours of clicks give 16,384 beats, the
  first ones, on their clicks.
- Fixtures through the lane (the `Peaks` job, then `beats`): a 40 s WAV
  click track (a 55 Hz kick and a 1.5 kHz tick) at 128 BPM from 0.3 s read
  127.78 BPM, 85 beats, each within 5.0 ms of its click; ten seconds of
  100 BPM clicks as AAC in an MP4 with `-itsoffset 1` read 100.00 BPM, 17
  beats within 5.0 ms of the clicks at 1.3 s and on, in container time.
  Finding beats started no process.
- Lane, with stand-in runners at gates: beats asked while the same file's
  `Peaks` job is under way answer from the kept peaks and the reading goes
  on and answers; the file's sound and kept peaks asked for while its beats
  are found leave them answering. Beats replaced under way, replaced while
  waiting, given up by name waiting and under way, and given up by
  `cancel_all` never answer. Sixteen waiting reads of either kind make the
  seventeenth `Busy`; a newer for beats that wait takes their place without
  room. Missing and stale peaks are errors in words, kept silence is
  `Ok(None)`, and the refusals queue nothing.
- Measured, one run each: the beats of 2,097,152 buckets (the bound) in
  0.49 s on the `dev` test profile and 4.11 s on the `ci` profile.

Not verified: real music of any kind (no accuracy is claimed beyond the
synthetic tests), any other host or FFmpeg build, memory while it runs
(counted, not measured), and Windows or Linux.

The criteria this was held to:

- `beats` tests over synthetic peaks: a click track at 90, 120 and 150 BPM
  gives a tempo within 1 BPM and beats within 20 ms of the clicks over
  three minutes (no drift); a click track with an offset first beat is
  phase-aligned; steady noise and silence give `None`.
- A fixture test from an FFmpeg-generated click track through the `Peaks`
  job to the grid, and one whose audio stream starts late, whose beats are
  in container time.
- Lane tests: a `Beats` request does not stop a running `Peaks` job for the
  same asset; a stale or cancelled request never answers.

## Known limits

- An energy onset detector at 10 ms resolution: music without clear
  percussion, with a changing tempo or with swing will be wrong or refused.
  No accuracy is claimed beyond the synthetic tests.
- A detected tempo may be half or double the felt one. Above about 170 BPM
  a clean click track reads as half: 180, 190 and 200 BPM read 89.78,
  95.16 and 100.00 (the prior on 120 BPM prefers the longer period once a
  third of a bucket or more is off the grid). The tempo given is always
  within 60 to 200 BPM.
- No downbeats, bars or time signature.
- `rustfft` spectral flux (MIT OR Apache-2.0) is the upgrade if this is too
  weak, and is a dependency decision.

## Comments

2026-10-13: Written with the spec. Nothing implemented or measured.

2026-10-13 (review): Split from the former ticket 16; the desktop half is
ticket 16b.

2026-10-13 (implemented): Built as scoped, with these settled in building:

- *The job is a `PeakJob`.* The scope's `Beats { asset }` needs the cache
  it reads and the project it answers, which are a `PeakJob`'s three
  fields; no new job type.
- *`Ok(None)` is not an error.* The answer carries the `Option` that
  `beats` returns, so 16b can tell "no clear beat" from "no peaks kept".
- *Beats run on the loader thread, in its queue.* As the scope says; so a
  kept-peak read waits behind a beat finding, up to 0.49 s at the bucket
  bound (4.11 s unoptimized). A beat finding is not stopped part way: one
  replaced or given up runs to its end and says nothing.
- *Constants chosen, none tuned on music:* prior one octave wide, clear
  period at a correlation of 0.1 and four standard errors of noise, four
  periods in the file at least, tightness 100, a bell of a thirty-second of
  a period, the weak-end cut at half the typical beat.
- *A test race, fixed:* the lane test that lets a gated reading end while
  beats are found let the stand-in reading store other peaks first, which
  have no beats; it failed once in a full `sound::` run. The beats are now
  let go before the reading.

2026-10-14 (review fixes): Three findings of the adversarial review held
and are fixed, each with a test that failed before the fix:

- *Fast tempos read at a third.* A period off the bucket grid splits its
  correlation between two lags, so an integer multiple won: 180 BPM clicks
  read 60.00 BPM with a beat on every third click. The lag is now chosen
  on the correlation with half of each neighbour's added; the noise test
  and the parabola still use the lag's own. 180, 190 and 200 BPM now read
  as half (89.78, 95.16, 100.00), every beat on a click; 170 reads 170.79
  and 60 reads 60.00. Tests:
  `a_fast_click_track_reads_at_its_tempo_or_half_of_it`, and 60 BPM in the
  three-minute test. The parabola's shift could put the
  tempo at 59.7 to 203.4 BPM; the period is now kept within 30 to 100
  buckets.
- *A beat in the first bucket was lost.* The first bucket's onset was a
  fixed 0; it is now its rise from silence, so a file that starts on its
  beat has a beat at 5 ms. Test: a click at 0 s in
  `a_click_track_that_starts_late_is_found_in_its_phase`.
- *`sound.rs`'s module comment* is re-wrapped.

Rerun on the same Mac, `dev` profile: `cargo test -p editor-media --locked`
288 passed; `sound::` 41 passed; clippy (`-D warnings`) and `cargo fmt
--check` of `editor-media`, and `cargo check --locked` of the desktop, pass.
The bound's timing read 0.48 s, one run; the `ci` profile was not rerun.

2026-10-14 (ticket 22): "Nothing in the desktop asks for beats yet" is older
than the code: ticket 16b's Show beats asks for them.
