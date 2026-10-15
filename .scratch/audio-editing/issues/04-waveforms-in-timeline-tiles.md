# Waveforms in timeline tiles

Status: ready-for-human
Progress: resolved
Type: task
Blocked by: 03

## Scope

Draw the sound of a clip on an audio lane from the peak cache, asked for only
by what is in view.

- `src/peaks.rs`, modelled on `src/steady.rs` and the poster claim in
  `src/textures.rs:502-562`: `Peaks::want(asset)` per painted tile, claimed
  once, failures remembered, unclaimed when the lane is busy. A cache read is
  tried first; a miss or a stale file queues a `Peaks` job. Held peaks are a
  bounded map (32 MiB), least recently wanted evicted. An answer is taken
  only when its request is outstanding, `project == editor.instance_id()`
  and `SourceIdentity::of(asset) == source`. `forget_asset` on media
  replacement; `cancel_all` and a cleared map on project replace.
- `src/app.rs`: the service is started lazily, polled where analysis events
  are (`:2238`), cache directory `<user cache>/peaks` or beside `--settings`.
- `src/ui/timeline.rs`: in `paint_clip`, an audio-lane tile draws the
  waveform in place of the level line (`:2351-2357`); the poster-first branch
  (`:2334`) is reordered so an audio-track clip of a video asset shows waves,
  not a poster. One column per point of width across the visible part of the
  tile only; each column takes min and max over the source range
  `clip.source_time(left)..clip.source_time(right)`, so rate and time map
  draw correctly, a reversed stretch mirrored and a freeze flat. Drawn as one
  mesh in the audio tone. Until peaks are held the level line stays.

Files: `src/peaks.rs`, `src/app.rs`, `src/ui/timeline.rs`, `src/ui/mod.rs`
(what `paint_clip` is handed), `docs/usage.md` (waveforms leave the list of
non-features), `docs/design.md`.

## Result

Implemented as scoped. A waveform is derived data drawn from a cache: no
command, shortcut, menu item, inspector control or agent operation is added
or changed, nothing enters the project or its history, and save and load are
untouched. The one route is the painted tile.

- `src/peaks.rs`: `HeldPeaks`, the desktop's bounded map
  (`MAX_HELD_PEAK_BYTES`, 32 MiB), and `cache_dir()`. `want(asset)` is called
  for each sound tile drawn; `ask(sound, project, instance, cache)` puts what
  was wanted to the lane, the cache first (`Ask::Kept`) and the sound when the
  cache keeps none or stale peaks (`Ask::Sound`); `answered(instance,
  project, event)` takes a `SoundEvent` through `take`, which drops an answer
  for another request, another project or a changed source.
  `forget_asset` hands back the request still waited for; `clear` is for a
  replaced project; `room_is_due` asks for the one frame in which room that a
  scrolled-away tile held becomes free.
- `src/app.rs`: `sound: Option<SoundService>` started by the first sound tile
  drawn, `peak_cache` at `<user cache>/peaks` or `peaks` beside `--settings`,
  `ask_peaks` after the timeline is drawn, `take_sound` in `logic` beside
  `take_steadies`, `HeldPeaks::project_replaced` (`cancel_all` and `clear`)
  in `replace_editor`, `HeldPeaks::media_replaced` (`forget_asset` and
  `cancel`) in `media_changed`.
- `src/ui/timeline.rs`: `wave_assets` (the clips `clips_drawn` gives on
  audio rows) is the only caller of `want`. `wave_columns` gives a column for
  each point of the tile in view with the source times of its two sides
  through `Clip::source_time`; `paint_wave` draws them as one mesh in the
  audio tone under the name row. `paint_clip` is handed `Option<Wave>`;
  `show` and `draw_lanes` are handed `&mut HeldPeaks`. `src/ui/mod.rs` needed
  no change.
- `docs/usage.md`, `docs/design.md`, `docs/performance.md`.

Settled while building, beyond what the scope says:

- The held map is `HeldPeaks`, not `Peaks`: `editor_media::Peaks` is the
  data it holds.
- A sound lane draws no poster at all, before or after its peaks arrive, and
  asks for none: `poster_assets` leaves audio rows out. An audio-track clip
  of a video shows the level line and then waves, never the poster first.
- The level line moved three points down, to the middle of the waveform's
  area, so the line a tile carries before its peaks arrive is where the
  waveform draws silence.
- Peaks on screen are never evicted. Media whose peaks have no room beside
  those of the tiles on screen is not asked about until it has; evicting a
  tile in view would have it asked for again in the next frame, for ever.
  What a file's peaks take is reckoned from its length before they are read
  and corrected when they are.
- A cache read and a sound read the lane has no room for (`Busy`) stay
  wanted and are put again by the tile, in the frame an answer of the lane
  brings. Any other refusal (media too long) is remembered and said once:
  "<name> has no waveform: Media is too long for a waveform: the limit is
  5.8 hours".
- A sound that cannot be read is said once as a problem notice, with the
  first line of what FFmpeg said, and is not asked about again until the
  media changes or the project is replaced.
- Peaks that were read and could not be kept in the cache (`unkept`) are
  drawn and nothing is said.
- Silence is drawn as a line a point high; source the file has no sound for
  is left bare.
- A tile held by a trim or a move draws the sound it would have where it is
  drawn.

## Evidence

On macOS 26.5.2 (arm64, Apple M1), Rust 1.97.1, Homebrew FFmpeg and ffprobe
8.1.2 (`scripts/check-media-tools.py` passed), 2026-10-13. No test was
skipped.

- `cargo test --locked --bin aster`: 616 passed. `cargo test --locked --bin
  aster -- peaks:: timeline::tests`: 30 passed, five runs in a row.
  `cargo clippy --locked --all-targets -- -D warnings` and `cargo fmt --
  --check` on the desktop crate are clean; `scripts/check-architecture.py`
  passes. `editor-media` was not changed.
- `src/peaks.rs`, eight tests: a tile wanted every frame asks the cache once
  and the sound once; a question the lane had no room for is put again; a
  failed asset, and one the lane refused, is not asked about again until it
  is relinked or forgotten; an answer for another request, another project,
  the other kind of question under the same number, a relinked source or
  media that left the library is dropped; the bytes held never pass the
  bound and the least lately wanted make room; peaks with no room beside
  those on screen wait, ask for no frame, and are asked for in the second
  frame after a tile scrolls away. Two of them run the real lane: a WAV
  written by the test is asked of the cache, read by FFmpeg, held (a silent
  second reads 0 and a half-scale tone 63 to 65), asked of the cache alone
  after `clear`, and an answer taken after the project was replaced is held
  by nobody; a missing file is read, fails, and is said once in one line.
- `src/ui/timeline.rs`, four new tests and the poster test for lanes
  changed: only the audio rows in the lanes and the
  clips of them in view ask (the picture row's video with sound, the row
  below the lanes and a scrolled-away row ask for nothing), and three frames
  ask once; columns at rate 2 cover twice the source of rate 1 for each
  point, a trimmed clip starts at its `source_in` wherever it is scrolled, a
  tile with its head out of view starts at the source of its first point in
  view, and a moved tile keeps its own sound; a reversed stretch has each
  column's times the mirror of the forward one and a held second is one
  bucket; one real egui pass draws no mesh before the peaks are held and
  then one mesh a tile, a column for each point in view and none beyond the
  lanes, inside the tile under the name, silence a point high.
- Native captures of the release build (`target/release/aster`, sha256
  `eafe389e101c6753…`), each run with its own settings, session, cache and
  project under `artifacts/audio-editing/04-waveforms/<run>/`, on owned
  fixtures made by FFmpeg (a 12 s tone with a tremolo, a 6 s video with
  sound, five hours of tone as MP3). The window asked for at 1440x900 came
  up at 1280x678 points on this display; the narrow one is 760x620.
  - `waves-normal`, `waves-narrow`: an audio clip, the same media at 2x (the
    whole twelve seconds in half the width), and an audio-track clip of the
    video asset drawing its quiet tone, not its poster.
  - `before-normal`, `before-narrow`: taken 1.2 s after launch, while the
    five hours were still being read: the level line.
  - `minzoom-normal`, `minzoom-narrow`: the same project 25 s after launch,
    fitted to the minimum zoom (0.5 points a second), and a clip trimmed to
    start fifty minutes in.
  - `failed-normal`: a clip of a file that is not there keeps the level line
    under the notice "gone.wav has no waveform: ffmpeg: could not read the
    sound: … No such file or directory".
  Every process started exited by itself after its capture; none was left.
  `failed-normal`, `before-narrow`, `minzoom-narrow` and `waves-normal` were
  looked at from the final build; the other three were looked at from the
  build before the notice was cut to one line and retaken, not looked at
  again.

Not verified: the timeline at maximum zoom (the build takes no zoom at
launch and nothing was clicked or typed into it); a clip with a time map, a
held trim or a move in the running build (headless only); the light theme; a
project with more audio media in view than the lane's queue or the held
bound takes, in the running build; the paint cost of a frame; the time to a
first waveform; Linux and Windows. Nothing was listened to.

The acceptance list as written:

- `src/peaks.rs` tests: a tile wanted every frame asks once; a failed asset
  is not asked again; an answer for another project, another request or a
  changed source is dropped; the map keeps its byte bound.
- A column test: columns for a clip at rate 2 cover twice the source per
  point of a clip at rate 1; a trimmed clip starts at its `source_in`.
- A scrolled-away tile asks for nothing (the virtualized `clips_drawn` is the
  only caller).
- Native captures of the final build at normal and narrow widths: an audio
  clip, an audio-track clip of a video asset, a tile before its peaks arrive,
  and the timeline at minimum and maximum zoom.

## Known limits

- No waveform on video-lane tiles or in library cells.
- At maximum zoom (640 points a second) a bucket is 6.4 points wide: the
  waveform is blocky, not sample-accurate.
- The waveform shows the source, not the clip's volume, fades or effects,
  and is drawn whether or not the clip `sounds` (a muted track still shows
  its waves).
- Paint cost per frame with many long tiles in view is unmeasured; so is the
  time to first waveform when a large project opens.

- The lane wakes the window once for each whole percent of a read. A
  waveform shows no progress, so those frames draw nothing new.
- A file at the bucket bound takes 8 MiB of the 32 MiB held: four such tiles
  in view fill it and a fifth keeps its level line while they are on screen.
  Shorter sound that fits beside them is still asked for and drawn.
- A sound read is given up when its tile has not been drawn for two frames
  and started again, from the start of the file, when the tile is drawn
  again: scrolling a long file out of view and back loses what was read of
  it. How long that costs is unmeasured. A question to the cache is not
  given up.
- No test builds the application (`App::new` needs a window's
  `eframe::CreationContext`), so that `replace_editor` and `media_changed`
  call `project_replaced` and `media_replaced` is established by reading;
  what those two do to the map and the lane is tested with the real lane.

## Comments

2026-10-13: Written with the spec. Nothing implemented or measured.

2026-10-13 (implemented): Built and tested on macOS 26.5.2 (arm64, Rust
1.97.1, FFmpeg 8.1.2); see Result and Evidence. Ready for a person to read
and to look at the captures. Maximum zoom is not captured.

2026-10-13 (after review): Five findings, all held. Rules changed:

- One refusal for want of room no longer refuses everything wanted after it
  in the frame, only peaks no smaller than the least refused; a short sound
  drawn after a long one that had no room is asked for.
- A sound read (`Ask::Sound`) whose tile has not been drawn for two frames is
  given up on the lane (`take_unwanted`, `give_up_unwanted`, called by `ask`
  and by `ask_peaks` when nothing is wanted) and asked again when the tile is
  drawn again.
- Held peaks are told from a relinked file's by `is_of`, which compares the
  source in place; a `SourceIdentity` is built only when an entry is made.
- The lane calls of project replace and media replacement moved into
  `HeldPeaks::project_replaced` and `media_replaced`, tested with the real
  lane. An `App` cannot be built in a test, so the two call sites in
  `src/app.rs` remain untested.
- Maximum zoom has headless cover now: a column is a 640th of a second, six
  or seven columns start in each bucket, and a real egui pass at 640 points a
  second draws a column a point for the lanes' width only. **The native
  capture at maximum zoom is still not taken**: the build takes no zoom at
  launch, and no input was injected into the desktop this ran on. That
  acceptance item is unmet until a person takes the capture or the owner
  waives it.

Tests after review, same host: `cargo test --locked --bin aster -- peaks::
timeline::tests`, 36 passed, none skipped.

2026-10-14 (native acceptance): Resolved. The captures this ticket waited for are taken and look right. Release build `target/release/aster` (sha256 fadbe5aa8244d800b2d9b16bf86bf121a6e9157af7e8067402761d40318e5763, `cargo build --release --locked`) on macOS 26.5.2 (25F84), Apple M1, Rust 1.97.1, Homebrew FFmpeg 8.1.2, built-in Retina display at scale 2. Fresh task-owned projects (made by `aster-agent` over ffmpeg-generated media: a 12 s testsrc2 interview with a speech-like tone, 30 s of 120 BPM clicks, a 0.8 s whoosh) and fresh `--settings` per run. Driven by real pointer and key events into the running window; the normal window (asked 1440x900) came up at 1280x709 points, the narrow one at 760x620 (and 760x560 for ticket 13). Captures are in `artifacts/audio-editing/captures/`. Every capture was looked at against `docs/design.md`. Dark theme only; Linux, Windows and the light theme were not run; nothing was listened to.

`normal-01`/`narrow-01` waveforms in every audio tile at rest; `normal-11`/`narrow-11` zoomed in; `normal-45`/`narrow-42` the timeline zoomed in to the slider's end (frame ticks in the ruler), the clicks drawn as waveforms with no gaps or seams; minimum zoom was captured earlier under `artifacts/audio-editing/`. The zoom was reached by 25 clicks of the zoom-in button with the slider at its end; the zoom value itself was not read out.
