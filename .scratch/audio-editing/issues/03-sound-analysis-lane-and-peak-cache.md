# Sound analysis lane and peak cache

Status: ready-for-human
Progress: resolved
Type: task

## Scope

A bounded lane in `editor-media` that reads a media file's sound once and
keeps its peaks outside the project. No desktop change; the workspace gains a
service nothing calls yet.

- `crates/editor-media/src/sound.rs`: `SoundService::new(wake)` with one
  worker thread (`aster-sound`), one loader thread for cache reads, its own
  `Processes`, and a queue of 16 (`MediaError::Busy` beyond). A newer request
  with the same target replaces the waiting one and stops the running one;
  neither answers. The target is the job's own, as `Work::target()` is in
  the analysis lane (`analysis.rs:378-407`): (kind, asset) for `Peaks` and,
  later, `Beats`; (kind, asset, from, span) for `Loudness` and `Extract`.
  Requests with different targets never touch each other. Cache reads have
  their own queue, as `load_steady` does (`analysis.rs:362-376`).
  `cancel(request)` and `cancel_all()` are silent. One
  coalesced progress event per request, a wake per whole percent. An idle
  lane wakes nobody.
- Job `Peaks`: `ffmpeg -hide_banner -loglevel error -nostdin -threads 1 -i
  <path> -map 0:a:0 -vn -sn -dn -ac 1 -ar 8000 -af
  aresample=async=1:first_pts=0 -f s16le pipe:1`, read under the per-read
  deadline. The filter pads from pts 0, so bucket `i` is container time
  `i / 100` s even when the audio stream starts later; the mixer seeks in
  container time (`audio.rs:946-950`). Reduced 80 samples at a time to
  `[min: i8, max: i8, rms: u8, low: u8]`. `rms` and `low` are
  `round((dBFS + 96) · 255 / 96)` clamped to 0..=255; `low` is the level
  after a one-pole 150 Hz low-pass run in Rust.
- `crates/editor-media/src/sound/peaks.rs`: `Peaks { buckets: Vec<[u8; 4]> }`
  with `at(source: Time)` and a min/max over a source range, and `PeakCache`
  copied from `SteadyCache` (`analysis/steady.rs:890-1134`): file
  `fnv64(path).peaks`, magic `ASTRPEAK`, algorithm version 1, the same stamp
  (on-disk length and modification time, asset size and duration), trailing
  FNV checksum, temp-then-rename, least recently read evicted, stale temp
  files swept. Bounds: 64 MiB, 512 files, `MAX_PEAK_BUCKETS = 1 << 21`.
- Answers: `SoundEvent::Peaked { request, project, asset_id, source:
  SourceIdentity, result }`, `Loaded` (`Held`, `Stale`, `Missing`) for cache
  reads, and `Progress`.
- An asset without sound, or longer than the bucket bound, is refused before
  FFmpeg starts.

Files: `crates/editor-media/src/sound.rs`, `sound/peaks.rs`,
`sound/tests.rs`, `lib.rs` (exports), `docs/performance.md` (the new budgets
under the resource policy).

## Result

Implemented as scoped. Nothing in the desktop calls the lane yet (ticket 04),
so there is no shortcut, menu, gesture, inspector control or agent command to
trace, and no project state, command, save or load is touched.

- `crates/editor-media/src/sound.rs`: `SoundService::new(wake)`, `peaks(job)`,
  `load_peaks(job)`, `cancel(request)`, `cancel_all()`, `poll()` and `busy()`,
  with `PeakJob { project, asset, cache }`. Threads `aster-sound` and
  `aster-sound-load`, a `Processes` of its own, `SOUND_QUEUE` 16 and
  `PEAK_LOADS` 16. The replacement target is a private `Target` enum with one
  variant a kind, `Peaks(AssetId)` today, so tickets 14a, 16a, 20 and 21a add
  a variant each (with `from` and `span` where their target has them).
- `sound/peaks.rs`: `Peaks { buckets }` with `at(source) -> Option<Peak>`,
  `range(from, to) -> Option<(i8, i8)>` and `byte_len()`; `Peak { min, max,
  rms, low }` with `Peak::decibels`; `PeakCache` with `at`, `file`, `load` and
  `held`; `KeptPeaks::{Held, Stale, Missing}`; `PEAK_ALGORITHM`,
  `PEAKS_PER_SECOND`, `MAX_PEAK_BUCKETS`, `PEAK_CACHE_BYTES` and
  `PEAK_CACHE_ENTRIES`. All are exported from `lib.rs`.
- Answers: `SoundEvent::Progress { request, done }`, `Peaked { request,
  project, asset_id, source, result: Result<Arc<Peaks>, String>, unkept }`
  and `Loaded { request, project, asset_id, source, kept: KeptPeaks }`.
- Settled while building, beyond what the scope says:
  - `Peaked` carries `unkept: Option<String>`, as `SteadyEvent::Done` does:
    peaks that could not be written are still the answer, and the reason is
    not lost.
  - `min` is rounded down and `max` up to the next 128th of full scale, so no
    sample is outside its bucket's two.
  - What is left of a last bucket (under 80 samples) is a bucket.
  - FFmpeg's exit status is read after the stream ends: a process that fails
    part of the way answers with an error and keeps nothing.
  - A stream that turns out longer than the bucket bound, whatever its
    container said, is an error; nothing is cut short and kept as whole.
  - A cache file whose bucket has `min` over `max`, or which holds no bucket,
    reads as `Missing`.
  - The refusals in words: "Media has no sound to draw", "Media has no
    length", "Media is too long for a waveform: the limit is 5.8 hours".
    `load_peaks` refuses the same assets.
  - The test-only job of another kind (`Work::Other`, `#[cfg(test)]`) is run
    and answered as peaks are; it goes when ticket 14a or 16a adds a real one.

## Evidence

On macOS 26.5.2 (arm64, Apple M1), Rust 1.97.1, Homebrew FFmpeg and ffprobe
8.1.2 (`scripts/check-media-tools.py` passed), 2026-10-13. No test was
skipped: every fixture ran against the real tools.

- `cargo test -p editor-media --locked sound::`: 17 passed, seven runs in a
  row; also on `--profile ci`. `cargo test -p editor-media --locked`: 221
  passed. After the review fixes (see Comments): 18 passed, five runs in a
  row on the `dev` test profile, and 222 in the whole crate. `cargo clippy -p editor-media --all-targets --locked -- -D
  warnings` and `cargo fmt -p editor-media -- --check` are clean;
  `scripts/check-architecture.py` passes.
- Tone: a 1 kHz sine at half of full scale (WAV, 48 kHz) reads `rms` within
  one of 231 (-9.03 dBFS) and `min`/`max` within one of -64 and 64; the same
  on samples made in the test is exact. Silence (stereo, 44.1 kHz) is all
  zeros. 60 Hz reads `low` at most 6 steps under `rms`; 2 kHz reads it 54 to
  60 steps under (21.6 dB is 57).
- Count: each fixture's bucket count is its probed container length times
  100, within one.
- Late start: a 3 s MP4 whose AAC tone was given `-itsoffset 1` (ffprobe:
  stream start 0.978 s) gives 301 buckets, the first loud one at index 100,
  and the first 90 are zeros. With the filter taken out of the command the
  same test fails with 203 buckets.
- Targets: with `Peaks` under way, a job of another kind for the same asset
  leaves it answering and both answer; a second `Peaks` request for the asset
  replaces the first, waiting or under way.
- Cache: round trip; wrong magic, version, checksum (cut, one byte turned,
  grown, empty) and embedded path give `Missing`; a changed size, a touched
  file, a removed file and a changed asset size or duration give `Stale`;
  eviction keeps both bounds and the file just written; a temp file over an
  hour old is swept and a fresh one left; a store into a directory that
  cannot be made still answers with the peaks and the reason.
- Lane: a replaced request never answers, and a figure it had said and
  nobody read is dropped with it; a cancelled one never answers
  (waiting or under way), `cancel_all` leaves nothing, the 17th waiting
  request is `Busy`, cache reads answer while sound is still being read, one
  progress figure waits, and an idle lane's wake count does not move. On a
  two-hour silent FLAC the real FFmpeg child is given up at its first
  progress word, replaced, and given up by `cancel_all`: each time the lane
  goes idle with every process it started reaped (its watchdog retired),
  nothing unread and nothing kept.
- Refusals: a video without sound, a still, an asset one microsecond over
  the bound and one of no length are refused by `peaks` and `load_peaks` with
  no process started; an asset exactly at the bound is accepted.
- More sound than the length says: six hours of silent mono FLAC whose asset
  says one hour answers "Media has more sound than its length says: too much
  for a waveform" with nothing kept and its process reaped. This is the
  check made while reading; the same check after the stream ends is reached
  only when the last part-bucket is the one over the bound, and has no test.
- Measured, one run each: the two-hour silent FLAC (720,000 buckets) read in
  0.98 s on the `dev` test profile and 1.64 s on `ci`, FFmpeg included.

Not verified: any other host or FFmpeg build; real speech or music; a file
at the bucket bound; the `PEAK_LOADS` bound (the reader empties its queue too
fast to fill it in a test); `Busy` from `load_peaks`. Nothing was listened
to, and no waveform exists to look at until ticket 04.

## Known limits

- Mono downmix: a stereo file whose channels cancel shows a flat waveform.
- 10 ms buckets from 8 kHz audio; content above 4 kHz does not reach `rms`.
- The whole file is read, not a clip's span; time to first waveform on long
  media is unmeasured.
- `SourceIdentity` has no audio fields: a file remuxed to the same size,
  duration and modification time would keep a stale waveform.
- Only `0:a:0`.
- The peak file is container-aligned and the mixer is not quite: when a
  seek lands before a late-starting stream's first sample, the decoder
  gives that sample at once, so the mixer plays the stream early by the
  gap. That is today's behaviour and is not changed here; the waveform then
  shows the sound where the container says it is, later than it is heard
  from such a seek.

## Comments

2026-10-13: Written with the spec. Nothing implemented or measured. `Loudness`
and `Beats` jobs are added to this lane by tickets 14a and 16a.

2026-10-13 (review): Two changes. (1) `aresample=async=1:first_pts=0`: on
FFmpeg 8.1.2 a 3 s MP4 whose audio stream starts at 0.976 s gave 32,694
bytes (2.04 s) from the old command and 48,310 bytes (3.02 s) with the
filter. (2) Replacement by target in place of "per asset", which would have
let a loudness or beats request silently cancel the same asset's waveform.

2026-10-13 (implemented): Built and tested on macOS 26.5.2 (arm64, Rust
1.97.1, FFmpeg 8.1.2); see Result and Evidence. Ready for a person to read.

2026-10-13 (review fixes): Two findings, both held. (1) A request under way
that a newer one for the same target replaced left its unread `Progress`
figure in the queue, so a replaced request did say something; `queue` now
drops it as `cancel` does. The gated test now fails on any event that is not
one of the two expected answers, and failed with `Progress { request: 2,
done: 0.5 }` with the fix taken out. The analysis lane's `queue` has the same
gap and is not changed here. (2) The over-bound stream, stated in Result as
settled, had no test; one was added (see Evidence). No rule changed.

2026-10-14 (ticket 22): "Nothing in the desktop calls the lane yet" is
older than the code. Ticket 04 draws waveforms from it; 14a, 16a, 20 and
21a added the `Loudness`, beats, `Extract` and separation jobs (the
test-only job kind is gone with 14a); 15 and 16b read its peaks for work.

2026-10-14 (read against the code, decided on the owner's behalf):
Resolved. The owner delegated the reading this ticket waited for; it was
done by an agent, not a person. Result and Evidence were checked against
`crates/editor-media/src/sound.rs`, `sound/peaks.rs`, `sound/tests.rs` and
`lib.rs` as they stand, and hold: `SoundService::new(wake)` with the threads
`aster-sound` and `aster-sound-load`, `SOUND_QUEUE` and `PEAK_LOADS` both 16
and `Busy` beyond each; the private `Target` enum with `Peaks(AssetId)`,
since joined by the loudness, beats, extract and separation targets; the
FFmpeg command as scoped (`-threads 1`, `-map 0:a:0 -vn -sn -dn -ac 1 -ar
8000`, `-af aresample=async=1:first_pts=0`, `-f s16le pipe:1`);
`PEAK_ALGORITHM` 1, `PEAKS_PER_SECOND` 100, `MAX_PEAK_BUCKETS` `1 << 21`,
64 MiB and 512 files, magic `ASTRPEAK`; the refusals in the words Result
gives; `Peaked`, `Loaded` and `Progress` as listed; everything exported from
`lib.rs`. One contract has grown since, and does not change what this
ticket answers: `cancel(request)` now returns a `Cancel` (ticket 20), so a
written file's answer is not lost; a cancelled peaks request is still
`GivenUp` and answers nothing. The ticket 22 comment above already records
that the desktop now calls the lane. Run on macOS 26.5.2 (Apple M1, Rust
1.97.1, FFmpeg 8.1.2): `cargo test -p editor-media --locked sound::`, 64
passed (this ticket's tests and those of 14a, 16a, 20 and 21a), none
skipped. "Not verified" and Known limits above stand unchanged.
