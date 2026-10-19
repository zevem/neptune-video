# `Loudness` job and the stderr reader

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 03, 11

## Scope

Measure the loudness of a span of a media file's sound, with a clip's audio
effects applied, on the sound lane. Headless: no desktop change.

- `crates/editor-media/src/sound.rs`: job `Loudness { asset, from, span,
  effects: Vec<Effect> }`. The desktop hands over the clip's effects; the
  filter string is built inside the crate by ticket 11's `chain` module (the
  effects part only: no speed stage, no `Pitch`, so no sample rate is
  needed), after the filter probe has answered.

  ```
  ffmpeg -hide_banner -nostdin -loglevel info -nostats -progress pipe:1
    -threads 1 -ss <from> -t <span> -i <path> -map 0:a:0 -vn -sn -dn
    -af "[<effects>,]ebur128=peak=true:framelog=verbose" -f null -
  ```

  The `-progress` lines on stdout feed the read deadline and the progress
  event (`out_time_us` over the span). `framelog=verbose` keeps the filter's
  ten lines a second out of stderr at this log level. The job reads the
  last `Summary:` block in the stderr tail and takes the number after `I:`
  (LUFS) and after `Peak:` (dBFS, or `-inf`). More lines follow the block
  (`[out#0/null …]` and a final `size=N/A time=…`), so nothing is assumed to
  be last.
- Replacement target: (`Loudness`, asset, from, span). Two spans of one
  asset both answer; a `Peaks` or `Beats` job for the same asset is left
  alone.
- `process.rs`: `Process::stderr_tail() -> String`. The reader thread
  (`:148-167`) is detached today, so a tail read just after the child exits
  can miss the last bytes, which is where the summary is. Keep its
  `JoinHandle` (or a done flag) on `Process` and have `stderr_tail` wait for
  the reader's end of file, bounded at 250 ms, once the child has exited.
  `failure` (`:239`) may use it too.
- Answer: `SoundEvent::Measured { request, project, asset_id, source,
  result: Result<Loudness, String> }`, where `Loudness` is one of
  `Measured { integrated, true_peak }`, `Silent` (`Peak` is `-inf`) and
  `TooShort` (`I` at or under -70 with a finite peak: the meter needs one
  0.4 s block above its gate, so a short or very quiet span reads as the
  floor).

Files: `crates/editor-media/src/sound.rs`, `process.rs`, `sound/tests.rs`,
`audio/chain.rs` (the effects-only entry), `docs/performance.md`.

## Result

Implemented. Nothing in the desktop asks for a measurement yet (ticket 14b),
so there is no shortcut, menu, gesture, inspector control or agent command
to trace, and no project state, command, save or load is touched.

- `sound/loudness.rs` (new, beside `peaks.rs`): `LoudnessJob { project,
  asset, from, span, effects }`, `Loudness::{Measured { integrated,
  true_peak }, Silent, TooShort}` (`f32`, LUFS and dBFS) and
  `LOUDNESS_FLOOR` (-70), all exported from `lib.rs`. The job is
  `LoudnessJob` and not `Loudness`, as the peaks job is `PeakJob`: the
  answer has the name 14b uses (`Loudness::Measured`).
- `SoundService::loudness(job)` queues it; the target is (`Loudness`,
  asset, from, span). The worker runs `chain::session()` before the first
  one and then the ticket's command, with the graph built by
  `loudness::graph`. Progress is `out_time_us` over the span, or what of it
  the file has; each read of the `-progress` lines is under the 10 s read
  deadline. The figures are read from `Process::stderr_tail()` after the
  exit status: the last `Summary:`, then the first `I:` and `Peak:` lines
  after it, each a number and its unit.
- The answer is `SoundEvent::Measured { request, project, asset_id, source,
  result }`. `Silent` when the peak is `-inf`; `TooShort` when `I` is at or
  under -70 with a finite peak.
- Refusals before any process: "Media has no sound to measure", "Media has
  no length", "The span to measure is empty" (a span of no length, or a
  start or span out of `Time`'s range), "The span to measure starts after
  the media ends", "A clip has at most 16 effects". A span that runs past
  the end measures what the file has.
- Errors in words: "ffmpeg: its report has no loudness summary", "… its
  loudness summary has no integrated loudness", "… has no true peak"; a run
  that fails is "ffmpeg: could not measure the sound: " and FFmpeg's tail,
  which at the `info` level this job needs carries the input's description
  before the error.
- `audio/chain.rs`: `stack_effects(stack, rate, channels, filters)` is the
  effects part of a stack without its clip; `effects(clip, …)` calls it.
  The measurement writes them for 48 kHz in two channels behind an
  `aresample=48000` (see Comments). The meter is always behind
  `aformat=channel_layouts=stereo`, so a measurement with no effect heard
  is `aformat=channel_layouts=stereo,ebur128=peak=true:framelog=verbose`.
- `process.rs`: the stderr reader marks the tail ended and wakes a
  condition variable at its end of file. `stderr_tail()` waits for that,
  at most 250 ms, once the child has exited, and not while it runs.
  `failure` uses it, so every tool's error message waits for its last
  bytes too. `STDERR_TAIL` (8,192) is the kept length, now named.
- `sound.rs`: the lane runs `Runners { peaks, loudness }`; the test-only
  `Work::Other` is gone, as ticket 03 said it would be, and its test uses
  `Loudness`. The wait for a process's exit is `await_exit`, shared by both
  jobs.
- The desktop: `HeldPeaks::answered` (`src/peaks.rs`) passes over a
  `Measured` as it does a `Progress`, so the match stays exhaustive. No
  other desktop change.
- `docs/performance.md`: "Loudness measurement" under the audio-editing
  budgets.

## Evidence

On macOS 26.5.2 (arm64, Apple M1), Rust 1.97.1, Homebrew FFmpeg and ffprobe
8.1.2 on `PATH` (`scripts/check-media-tools.py` passed), 2026-10-13, `dev`
profile. No test was skipped: every fixture ran against the real tools.
Nothing was listened to.

- `cargo test -p editor-media --locked sound::`: 28 passed, eight runs in a
  row; `process::`: 6 passed, three runs; `audio::chain`: 26 passed.
  `cargo test -p editor-media --locked`: 275 passed in four of ten runs; in
  the others `decode::tests::a_paced_walk_backwards_at_preview_size_is_measured`
  failed ("the seek and the first run, and no more"). That test paces
  decoding by the wall clock, and it failed the same way in two of five
  runs with every test of this ticket skipped, with the host's load
  average between 8 and 18 from other processes. It is not this ticket's
  and is not fixed here, but this ticket's tests add FFmpeg work to the
  same run and may make it fail more often. `cargo clippy -p editor-media --all-targets
  --locked -- -D warnings`, `cargo clippy --locked --all-targets -- -D
  warnings` (the desktop), `cargo fmt -p editor-media -- --check`, `cargo
  check --locked`, `cargo test --locked -p neptune-video peaks` (15 passed)
  and `scripts/check-architecture.py` pass.
- Fixtures, printed as `MEASURE:` under `--nocapture`: a stereo 1 kHz tone
  at -20 dBFS reads -20.0 LUFS (BS.1770: -3.01 LUFS for full scale in one
  channel, so -20.0 in two at -20 dBFS) and a true peak of -20.0 dBFS; the
  same tone at -26 dBFS reads -26.0; through an `EqBand` of -12 dB at
  1 kHz, -32.0. A 1.5 s span from inside the 5 s tone reads as the whole.
  Two seconds of stereo silence answer `Silent`; 0.3 s of the tone, as a
  file and as a span of the long one, `TooShort`. A file of two seconds of
  silence and then two of the tone answers `Silent` for its first two
  seconds and -20 for its last two, and for a span from 2.5 s ten seconds
  long. Progress never goes back or past 1. A file that is no sound and a
  missing file answer in FFmpeg's words, with their processes reaped.
- Parser, over FFmpeg 8.1.2's own report of the tone (captured on this
  host): the summary followed by the `[out#0/null …]` and `size=…` lines;
  warnings before it, one of them holding `I:`; two summaries, of which the
  last is read; `-inf` and -70 told apart; the report behind 200 lines of
  warnings cut to its last 8,192 bytes, at a line's middle and at a
  character's; and no summary, no `I:` after the header, no `Peak:`, `nan`,
  another unit and `inf`, each an error in words.
- `stderr_tail`: a shell that writes a line and exits at once, leaving a
  child that writes another 50 ms later, returns both; with the line a
  second later, it returns the first after 250 to 450 ms; a tail whose pipe
  has ended returns at once. With the wait set to 0 this test fails. An
  FFmpeg that writes its loudness report and exits returns the whole of it,
  ending with its `size=` line, in each of 30 runs read the instant the
  exit is seen; that test also passed with the wait set to 0 on this host,
  so it does not show the race and the shell test is the one that does
  (Unix only). A tail of more than 8 KiB keeps 8,192 bytes ending with
  FFmpeg's last line; a running process's tail returns without waiting.
- Lane, with jobs that wait at a gate: a measurement and the same media's
  peaks both answer, whichever is under way; two spans of one media both
  answer, a longer span from the same start is another target, and the
  same span again replaces the one waiting or under way, which says
  nothing. With the real FFmpeg, a measurement of two hours of 8 kHz
  silence given up by name, replaced, and given up by `cancel_all`, each at
  its first progress word, leaves the lane idle with every process reaped
  and nothing said. Refusals start no process.
- Measured, one run each: twenty minutes of silent mono 8 kHz FLAC took
  2.54 s through the lane (an earlier form of the cancellation test); the
  same through `ffmpeg` with the job's options, 2.50 s, and 0.19 s without
  `peak=true`. Ten minutes of stereo 48 kHz white noise in FLAC took 2.77 s
  through `ffmpeg`.

Not verified: any other host or FFmpeg build (the summary's layout is
relied on), real speech or music, a compressed file, a span through more
than one effect, a stream that starts after container time 0, Windows (the
shell test is Unix only), and how loud anything sounds.

The criteria this was held to, in `crates/editor-media`:

- Fixture tests: a stereo 1 kHz tone at -20 dBFS measures within 0.5 LU of
  its expected loudness; the same tone 6 dB quieter measures 6 LU lower;
  silence answers `Silent`; a 0.3 s tone answers `TooShort`; a tone through
  an `EqBand` of -12 dB at its frequency measures about 12 LU lower.
- Parser tests over captured stderr: the summary followed by the two
  trailing lines; warnings before it; a tail cut at 8,192 bytes; no summary
  at all is an error in words.
- `stderr_tail` test: a process that writes its report and exits at once
  returns the whole report every time over many runs.
- Lane tests: two spans of one asset both answer; a `Loudness` request
  leaves a running `Peaks` job for the same asset answering; cancellation
  reaps the process and says nothing.

## Known limits

- Relies on the layout of the `ebur128` filter's summary. If that proves
  fragile across FFmpeg builds, the fallback is the `ebur128` crate (MIT)
  over the PCM pipe, which is a dependency decision.
- A long measurement holds the sound lane; waveforms wait behind it. Ten
  minutes of stereo 48 kHz took 1.75 s on the design host (FFmpeg 8.1.2,
  `-threads 1`, lavfi source); `loudnorm` took 10.68 s for the same input.
  Not measured on real media or another host.
- The measured span is in source time, at clip speed 1, without `Pitch`.
- It is measured as a two-channel output plays it: the source is summed to
  stereo before the meter, as a voice is by `-ac 2`. On a device of one
  channel or more than two, what is heard differs from the figures.
- Nothing is rewritten to disk and nothing is kept.

## Comments

2026-10-13: Written with the spec. Nothing implemented.

2026-10-13 (review): Split from the former ticket 14. `loudnorm` was
replaced by the `ebur128` filter after measurement (figures above; the same
20 s tone read -41.1 LUFS, and its true peak -38.1 dBFS). The report layout
was captured this time: the old ticket's "the JSON block is the last thing
on stderr" was false, `-inf` was reported for a 0.3 s tone and not only for
silence, and the detached stderr reader could lose the tail. The job now
carries the clip's effects, because `chain` is `pub(crate)` and needs the
probe's answer, neither of which the desktop has.

2026-10-13 (implemented): Built as scoped, with these settled in building:

- *The effects run at 48 kHz.* The scope said no sample rate is needed,
  but ticket 11 holds a frequency under 0.45 of the rate a chain runs at,
  and a source of 8 kHz would put a band above half its rate. A voice runs
  its effects behind `aresample` at the device's rate; the measurement runs
  them behind `aresample=48000`, in two channels (so centre reduction is
  heard as on a stereo output). The meter is alone when nothing is heard
  (behind the stereo sum, see the review fix below).
- *Names.* `LoudnessJob` for the job and `Loudness` for the answer, as
  `PeakJob` and `Peaks`. `LOUDNESS_FLOOR` is exported for ticket 14b's
  notice.
- *Refusals.* Five, listed under Result. A span that starts inside the
  media and runs past its end is measured as far as the media goes.
- *`failure` waits too,* for every tool, at most 250 ms and only once the
  child has exited.
- *Cost.* The true peak is most of it, and a low rate is not cheaper for
  it: twenty minutes of mono 8 kHz took 2.5 s, against 0.19 s without
  `peak=true`, and ten minutes of stereo 48 kHz 2.77 s.

2026-10-13 (review fix): The meter heard the source's own layout, but a
voice is summed to the output's channels after its chain (`-ac`), so a 5.1
or mono source's figures were not those a stereo output plays. Measured on
the design host with FFmpeg 8.1.2: a 5.1 tone at -6 dBFS in left, right and
centre read -4.2 LUFS and a true peak of -6.0 dBFS, where a voice's own
output read -1.3 and -1.4 (14b's gain to -1 dBTP would have played above
full scale); a mono tone at -20 dBFS read a peak of -20.0 against -23.0.
The graph now ends `aformat=channel_layouts=stereo,ebur128=…`, which read
the same as the voice for both. A test with a 5.1 and a mono fixture holds
it, and fails with the sum taken out.

2026-10-14 (ticket 22): "Nothing in the desktop asks for a measurement yet"
is older than the code: ticket 14b's Normalize loudness asks for it.
