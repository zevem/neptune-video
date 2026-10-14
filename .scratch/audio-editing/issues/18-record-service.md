# `RecordService`

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 17

## Scope

Capture the default input device to a WAV file from a bounded worker, in
`editor-media`. No desktop change; nothing calls it yet.

Read the `cpal` 0.18.2 input documentation first, the version in
`Cargo.lock` (the manifest asks for 0.18.1): default input device and
config, building an input stream, the error callback. This design did
not.

- `crates/editor-media/src/record.rs`:
  `RecordService::start(path: PathBuf, wake) -> Result<Self, MediaError>`,
  `stop(&self)` (asks; does not wait), `poll() -> Option<RecordEvent>`
  (`Level(f32)`, coalesced; `Finished(Take { path, frames, rate, channels,
  bytes, dropped })`; `Failed(String)`), `frames() -> u64`, `rate() ->
  u32` and `elapsed() -> Time`.
- The service is its own clock. `frames()` is an atomic count of the
  frames the callback has put in the ring, which is their position in the
  file; `elapsed()` is that over the rate. Ticket 19 shows it as the take's
  time and pairs it with the playback position to place the take. Nothing
  about the timeline bounds a take.
- `Take` is all the desktop needs to build the asset without a probe:
  `path` is canonical (as `import.rs:116` makes paths, so the library's
  duplicate test agrees), `frames / rate` is the duration and `bytes` is
  the file's size, known to the writer.
- The CPAL input callback does one thing: copy samples into a bounded
  lock-free SPSC ring (the shape of `Ring`, `audio.rs:340-348`; two seconds
  at the device's rate and channel count, mono or stereo kept as delivered,
  more channels refused). No lock, allocation or file work. When the ring
  is full the frames are counted in an atomic and dropped.
- A writer thread (`aster-record`) drains the ring every 20 ms to a 32-bit
  float WAV with a hand-written 44-byte header, and rewrites the two size
  fields and syncs once a second, so a crash leaves a file that plays up to
  the last second. On `stop` the stream is dropped and the writer drains,
  writes the final sizes, syncs and reports `Finished` through `poll` with
  a wake; the UI thread never waits on the file. `Drop` joins.
- The file is created with `create_new`, never over an existing file. The
  service never deletes it, also not on failure.
- Bounds from ticket 17's answer: the longest take and the free-space
  check. A stream error, a lost device or a full disk stops the take,
  keeps what was written and reports through `poll`.
- No input device: `MediaError` with words for a quiet "No microphone"
  state.

Files: `crates/editor-media/src/record.rs`, `record_tests.rs`, `lib.rs`,
`docs/performance.md` (the ring and thread in the resource policy),
`docs/architecture.md`.

## Result

Implemented and tested headless in `editor-media` with a fake source that
calls the callback's own function; no real microphone was opened and
nothing was listened to. Nothing in the desktop calls it (ticket 19), so
there is no shortcut, menu, gesture, inspector control or agent command to
trace, and no project state, command, save or load is touched. The cpal
0.18.2 source in the registry was read for the input API: `HostTrait::
default_input_device`, `DeviceTrait::default_input_config`,
`build_input_stream(config, data, error, timeout)` (a built stream is still
until `play`), `ErrorKind`.

- `crates/editor-media/src/record.rs` (new): `RecordService::start(path,
  wake) -> Result<Self, MediaError>`, `stop(&self)`, `poll() ->
  Option<RecordEvent>`, `frames() -> u64`, `rate() -> u32`, `elapsed() ->
  Time` (exact microseconds of `frames / rate`); `RecordEvent::{Started {
  rate, channels }, Level(f32), Finished(Take), Failed(MediaError)}`;
  `Take { path, frames, rate, channels, bytes, dropped, ended }`;
  `MAX_TAKE_SECONDS` (7,200), `RECORD_RING_SECONDS` (2),
  `WAV_HEADER_BYTES` (44), all exported from `lib.rs`.
- `start` returns at once. The thread `aster-record` finds the device and
  its default config, refuses more than two channels ("The microphone
  delivers N channels; only one or two can be recorded") or a rate outside
  1..=384,000 Hz, builds the stream and plays it, and only then makes the
  file with `create_new` and writes its header (the ring holds what comes
  meanwhile), so no microphone, a device that will not open or play and a
  file already there all leave nothing behind and answer `Failed`, never
  after a `Started`. A header that cannot be written into the file just
  made is `Failed` too, and that file, holding no sound, stays. Then it
  publishes the feed and says `Started`: `frames()` and `rate()` are 0
  until then, and `frames()` counts from when the device began to play, so
  it may be past 0 when `Started` is read. `start` itself refuses only a
  path without a file name and a thread that will not start.
- No input device is `Failed(MediaError::NoMicrophone)`, a new variant
  whose words are "No microphone"; `DeviceNotAvailable` and
  `HostUnavailable` at opening say the same. `PermissionDenied` says
  "Aster may not use the microphone. Allow it in the system's privacy
  settings"; anything else "Cannot open the microphone: …".
- The callback (`Feed::capture`, generic over cpal's ten sample formats,
  converted to `f32`) copies whole frames into the ring with atomics and
  publishes them with one release store of the write cursor, which is
  `frames()`. A full ring drops what it has no room for and counts it;
  frames past the take's limit are not taken and not counted as dropped;
  after `stop` nothing is taken. The error callback keeps the first fatal
  error (all but `DeviceChanged`, `RealtimeDenied` and `Xrun`, as the
  output does).
- The writer drains every 20 ms (`park_timeout`, unparked by `stop`) in
  pieces of 4,096 frames, keeps the loudest sample for `Level`, and rewrites
  the RIFF and data sizes and `sync_data`s after each second of sound
  written and after a second of the clock with sound unsynced. On stop, a
  device error, a failed write or the limit it closes the source first
  (no callback after), writes what the ring still holds (not after a
  failed write), rewrites the sizes, cuts off part of a piece a failed
  write left past the counted samples (only ever shortening), `sync_all`s
  and says `Finished`.
  `Take::path` is `fs::canonicalize`d as an import makes it; `bytes` is
  44 plus the data written; `ended` says why the take ended by itself, or
  is `None` when it was asked to stop; a later reason (a failed write or
  close) is added after an earlier one, not put in its place.
- The limit: 2 hours, or data bytes ≤ `u32::MAX − 36` rounded down to
  whole frames, whichever comes first ("The take ended at its limit of 2
  hours", "The take ended at the most a WAV file can hold").
- `Drop` stops and joins. `Started`, `Finished` and `Failed` wake once
  each; `Level` never wakes.
- Docs: `docs/performance.md` (Voiceover recording: bounds and the
  fixtures' figures), `docs/architecture.md` (the paragraph after the
  filter probe, and the ownership row).

## Evidence

On macOS 26.5.2 (arm64, Apple M1), Rust 1.97.1, Homebrew FFmpeg and ffprobe
8.1.2 on `PATH` (`scripts/check-media-tools.py` passed), 2026-10-14, `dev`
test profile. No test was skipped: ffprobe ran in both fixtures that use it.

- `cargo test -p editor-media --locked --lib record::`: 12 passed in three
  runs of the code before review and eight before its last changes of wording,
  visibility and printing. `cargo test -p editor-media --locked`: 309
  passed, two runs, the second on the final code.
  `cargo clippy -p editor-media --all-targets --locked -- -D warnings`,
  `cargo fmt -p editor-media -- --check`, `cargo check -p aster-editor
  --locked --all-targets` and `scripts/check-architecture.py` pass.
- Printed as `MEASURE:` under `--nocapture`, three runs: the longest of 300
  callbacks of 80 mono frames into a stalled writer took 0.3 to 12.2 us (the
  test allows 5 ms); `stop` with the writer stalled 1.3 to 3.5 us (allows
  20 ms); a writer ended as a crash would, after 20,000 frames at 8 kHz were
  written, left a header counting 19,096 and ffprobe read 2.387 s of 2.5 s;
  a closed 96,000-frame stereo 48 kHz take read as 2 s.

The tests, in `record/tests.rs` (the precedent of `sound/tests.rs`):

- A stereo 48 kHz ramp of distinct floats comes back bit for bit, under a
  header whose every field is checked; `frames()` equals what was fed after
  each call, the file never holds more, and it catches up to `frames()`;
  `Take` equals the expected value, `bytes` the file's length, and
  ffprobe's length is `frames / rate` within one frame.
- With the writer held, three seconds into a two-second ring: `frames()` is
  the ring's 16,000, 8,000 are counted dropped, every call is bounded, and
  the file is the first two seconds as they came.
- `stop` with the writer held returns at once, takes nothing after it, and
  `Finished` comes only once the writer is let go.
- A writer ended without closing (a test hook, as a crash would): the header
  is consistent, at most a second short, and ffprobe reads 1.5 to 2.5 s.
- A file already there: `Failed` with `AlreadyExists`, the file unchanged,
  the source played (before the file) and dropped, `frames` and `rate` 0.
- A device whose `play` fails: `Failed` with its words, no `Started`, no
  file, the source dropped, `frames` and `rate` 0.
- 800 frames handed over as the device plays, before the file is made:
  `frames()` is 800 at `Started`, and the file holds them at their place.
- A full disk (a test hook writes half a piece plus two bytes, then fails
  with `StorageFull`): `Finished` with the 4,000 frames written before it,
  `ended` "Cannot write the take: No space left on device", `bytes` the
  file's length (the part piece cut off) and a consistent header. With a
  device error already seen, `ended` keeps both, the device's first.
  Checked against the code without the cut: both tests fail.
- No microphone: "No microphone", no file, `frames`, `rate` and `elapsed`
  0. Six channels: refused before the source is built, no file.
- A reported stream error: `Finished` with the 4,000 frames and the error's
  words in `ended`; the level said is the samples' 0.25.
- The limit: a 0.25 s limit ends a take fed 0.5 s at 2,000 frames, none
  counted dropped, and finishes by itself. The bound: `limit` for 192 kHz
  stereo is 536,870,907 frames (4,294,967,256 bytes); a `Wav` given that
  length, not those bytes, writes a RIFF size of 4,294,967,292 and a data
  size of 4,294,967,256; past it the header holds the bound.
- The source is dropped and the thread ends on `stop`, and on `Drop`
  without a stop, which closes the file whole.
- A path with no file name is refused before any thread.

Done when, with the stream replaced by a fake source that calls
the same callback function:

- A known ramp fed through comes back sample for sample from the file, with
  a header whose sizes match.
- A writer stalled past the ring's length counts the dropped frames and
  the callback never blocks (bounded time per call asserted).
- Killing the writer mid-take (the test drops it without `stop`) leaves a
  file that ffprobe reads, at most one second short.
- An existing path is refused and untouched.
- A reported stream error stops the take and `Finished` still carries the
  file.
- `frames()` equals the frames in the file at every point of a take
  without overruns, and counts only frames that reached the ring when
  there are overruns; `Take::bytes` equals the file's length; ffprobe's
  duration for the file is `frames / rate` within one frame.
- `stop` returns at once with the writer stalled (bounded time asserted).
- The thread exits and the stream is dropped on `stop` and on `Drop`.

## Known limits

- The default device only; no device or channel choice, no monitoring.
- A real microphone, permission prompts and device loss are not covered by
  these tests. macOS needs a microphone usage string in a packaged build,
  and no packaging exists.
- Float WAV at the device's rate: 1.38 GB an hour for stereo 48 kHz, 0.69
  GB for mono (the 690 MB first written here was mono's figure).
- The 44-byte header has no `fact` chunk, which a float WAV is meant to
  carry; FFmpeg 8.1.2 reads it. Other readers were not tried.
- No free-space check before a take (ticket 17's answer). A full disk is
  covered only by a test hook that fails a write, not by a real disk.
- A header that cannot be written into the file just made leaves that
  file (no sound in it) and answers `Failed`; the service deletes nothing.
  No test reaches this path.
- The classic WAV size limit bounds a take: with the 44-byte header, data
  bytes ≤ `u32::MAX − 36` (4,294,967,259), rounded down to whole frames
  (4,294,967,256 for mono or stereo float), so neither size field wraps.
- Capture latency is not measured or compensated.

## Comments

2026-10-13: Written with the spec. Nothing implemented or measured.

2026-10-13 (review): `stop` returned the take and joined the writer on the
caller, which is the UI thread; it now answers through `poll`. `frames()`
and `Take::bytes` were added so that ticket 19 can anchor a take to the
playback clock and build its asset without `MediaService::import`. `cpal`
is locked at 0.18.2 (`Cargo.lock:738-739`).

2026-10-14: Ticket 17 is resolved. The path handed to `start` comes from
`editor_store::new_take`, which has made the media folder and returns a
name that did not exist; `create_new` still guards the race. Bounds: a take
ends itself at 2 hours or at the WAV size limit, whichever comes first, and
finishes as if stopped. No free-space check before starting (no query in
the standard library, no crate added); a failed write, a full disk
included, stops the take and keeps what was written.

2026-10-14 (review): The size limit is not 4 GiB of data: the RIFF size is
36 + data and both are `u32`, so the bound is data bytes ≤ `u32::MAX − 36`
rounded down to whole frames (4,294,967,256 for mono or stereo float).
Add a test: a writer stopped at that bound, given a fake length rather
than 4 GiB of samples, writes a header whose RIFF and data sizes are
36 + data and data, not wrapped. `start` is handed a path the desktop
resolved off the frame loop (ticket 19).

2026-10-14 (implemented): Built as above, with these rules settled. (1)
`start` does no device or file work on the caller: the desktop presses
Record on the UI thread, and opening a device was not to wait there. So
"No input device: `MediaError`" comes as `Failed(MediaError::NoMicrophone)`
through `poll`, not as `start`'s `Err`, and `RecordEvent::Started { rate,
channels }` is added: `frames()` and `rate()` are 0 until it. Ticket 19
starts playback on `Started`, and its anchor needs `Started` as well as a
device position. (2) `Take::ended` is added, so a take that ended by itself
says why. (3) No file is made before the device has opened, so a refusal
leaves nothing in the media folder. (4) The sizes are rewritten after a
second of sound written as well as of the clock, so a drained backlog is
not left uncounted. (5) Tests are in `record/tests.rs`, after
`sound/tests.rs`, not `record_tests.rs`.

2026-10-14 (review fixes): (1) The file was made before `play`, so a
device that built but would not play left an empty 44-byte WAV and a
`Finished` take with no `Started`, against `Failed`'s contract. The stream
is now played before the file is made; a refused `play`, `create_new` or
header write answers `Failed` before any `Started` (the header case leaves
the file just made, with no sound). (2) `frames()` and `rate()` were said
to be 0 until `Started`; they are 0 until the file is made, and `frames()`
counts from the device's start, so it may be past 0 when `Started` is read.
Ticket 19's anchor pairs `frames()` with a position and never takes
`Started` as frame 0; the spec comment and ticket 19's note say so. (3) A
failed write could leave part of a piece past the counted samples, so the
file was longer than `Take::bytes`; closing now cuts it off (only ever
shortening). A failed write after a device error keeps both reasons. Four
tests added (16 in `record::`).
