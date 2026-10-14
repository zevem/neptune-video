# Voice filter chain, latency table and execution probe

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 06, 10

## Scope

Make stored audio effects heard, by extending the `-af` of the clip's own
decoder. `Pitch` is ticket 12.

- `crates/editor-media/src/audio/chain.rs` (new):
  `pub(crate) fn chain(clip: &Clip, clip_speed: f64, transport: f64, rate:
  u32, filters: &Filters) -> Result<String, MediaError>`, the only place
  filter syntax exists. It emits today's
  `aresample=R,asetrate=…,aresample=R` and then, in stack order, each
  enabled audio effect whose kind is available, per the spec's table. dB
  values become linear where the filter wants linear (`acompressor`
  `threshold` and `makeup`, `alimiter` `limit`). Numbers are formatted with
  a fixed precision and no locale. A second entry gives the effects part
  alone, without the speed stage and without `Pitch`, for ticket 14a.
- `Denoise` is `afftdn=nr=<6+30·Amount>:nf=<Threshold>` with no `tn`.
  `Amount` 0..1 maps to `nr` 6..36 dB and `Threshold` -80..-20 dB is the
  filter's own `nf` range.
- `Filters`, probed by running, not by listing. Once per session, in a
  `OnceLock`, with a `Processes` created for the probe alone:
  1. a control: `ffmpeg -hide_banner -nostdin -loglevel error -f lavfi -i
     anullsrc -t 0.05 -af anull -f null -`;
  2. for each of the seven kinds, the same command with that kind's own
     string at its defaults from `chain` (for `Pitch`, `atempo=1.5`).
  A kind whose run exits non-zero is unavailable. If the control fails
  (FFmpeg missing, no `lavfi`), nothing is known: every kind counts as
  available and that is said once. `ffmpeg -filters` is not parsed.
- Who runs it: the audio worker, first thing, before it looks for an output
  device (`audio.rs:553-555` returns early without one), and the sound lane
  before its first `Loudness` job. Both go through the `OnceLock` and may
  block on it for the eight short runs; the UI thread never does.
  `AudioService::sync` cannot cancel it: `sync` calls `kill_all` on the
  audio service's own `Processes` (`audio.rs:277-296`), which the probe does
  not use.
- `editor_media::missing_audio_filters() -> Option<Vec<EffectKind>>` reads
  the finished answer for the inspector, `None` until it exists. No
  `AudioService::unavailable()`: `AudioState::unavailable()` (`audio.rs:82`)
  already means that output stopped or failed.
- `audio.rs:962-966` calls `chain`. A kind left out is reported once per
  session through the event channel ("Audio: noise reduction is not
  available in this FFmpeg").
- Latency, constant-delay filters only. A table in `chain.rs` in
  microseconds per kind, first values from the design host: `Denoise`
  25,000, `Limiter` 5,000, the rest 0. When a voice starts, the sum for its
  chain is turned into frames with the mixer's rate and that many leading
  frames of its first reads are dropped. `alimiter`'s `latency` option is
  not passed. `atempo` is not in this table (ticket 12).
- Set the constants by measurement and, on a host with sound, by ear:
  compressor attack and release, the `nr` range, the default noise floor.
  Record what was chosen and how.

Files: `crates/editor-media/src/audio/chain.rs`, `audio.rs`,
`audio/chain_tests.rs`, `lib.rs`, `docs/architecture.md`,
`docs/performance.md`.

## Result

Implemented. A stored audio effect is heard through its clip's decoder,
except `Pitch` (ticket 12). Nothing was listened to.

- `audio/chain.rs`: `chain(clip, clip_speed, transport, rate, channels,
  filters)` is the only place filter syntax is written and holds the range
  check on the changed rate that `audio.rs` had. `effects(clip, rate,
  channels, filters)` is the effects part alone, empty when there is none,
  for ticket 14a. `latency_frames(clip, rate, channels, filters)` is the
  table in frames, and `left_out(clip, channels, filters)` what a chain
  went without. Formats: Hz, dB,
  octaves and ratio to three decimals, linear values to six, the centre
  amount to four.
- A frequency is written as at most 0.45 of the output rate (see Comments).
- `CenterCut` is left out of a chain whose output has one channel, and that
  is said once a session: "Audio: centre reduction needs two output
  channels and is not heard on this device" (see Comments).
- `Filters` is the probe's answer; `probe(runs)` takes the runner, `runs`
  is the real one and `session()` the `OnceLock` with a `Processes` of its
  own. `editor_media::missing_audio_filters()` reads it.
- `AudioService::new` is `start(wake, chain::session, open_output)`: the
  worker probes, says what the probe could not find out, and only then
  opens the device, which moved unchanged into `open_output`. Tests stand
  in for either.
- `Mixer` holds the `Filters` (every kind available until a probe says
  otherwise, so the mixer's own tests run no probe) and the bits of what its
  chains went without; `Voice` holds the frames still to drop, read and
  discarded before its first samples under the same read deadline.
- Notices go through the audio service's event channel, once a session
  each, and one that finds the channel full is said the next time: "Audio:
  noise reduction is not available in this FFmpeg" when a voice first goes
  without a kind, and "Audio: FFmpeg's filters could not be checked; audio
  effects are taken as available" after a failed control. The desktop shows
  them with its "Sound: " in front, as it does every message of the service.
- Constants: all are the spec's first values. Attack 10 ms and release
  150 ms, `nr` 6 to 36 dB and the default floor of -40 dB meet the readback
  tests below; none was set by ear, which this ticket asked for on a host
  with sound.
- Not done here: the sound lane does not call the probe, because it has no
  `Loudness` job until ticket 14a. `audio::chain` is `pub(crate)` for it.
- Docs: `architecture.md` (codec boundary) and `performance.md` (audio
  editing budgets, "Voice chain and filter probe").

## Evidence

On macOS 26.5.2 (arm64), Rust 1.97.1, Homebrew FFmpeg 8.1.2 on `PATH`
(`scripts/check-media-tools.py` passed), 2026-10-13, `dev` profile. No
output device was opened by a test and nothing was listened to.

`cargo test -p editor-media --locked`: 255 passed, none ignored, none
skipped, 19 of them new, in `crates/editor-media/src/audio/chain_tests.rs`.
`cargo clippy -p editor-media --all-targets --locked -- -D warnings`,
`cargo fmt -p editor-media -- --check`, `cargo check --locked` (the desktop
too) and `scripts/check-architecture.py` pass. The figures below are printed
by the tests as `MEASURE:` under `--nocapture`, one run.

- Strings: a clip without audio effects gets exactly
  `aresample=R,asetrate=…,aresample=R` at three speeds, and the old refusal
  outside the resampling range; nineteen strings of the six filter kinds at
  defaults and extremes; disabled effects absent, stack order kept, `Pitch`
  adding nothing, kinds the probe refused absent with their delay.
- Equalizer, -24 dB at 1 kHz one octave wide: -24.00 dB on a 1 kHz tone,
  -0.34 dB at 100 Hz. High-pass at 200 Hz: -24.10 dB at 50 Hz, -0.01 dB at
  1 kHz. Limiter at -12 dB over a full-scale tone: peak 0.2512, the
  ceiling. Compressor at its defaults: -7.52 dB on a tone at -6 dBFS peak,
  0.00 dB on one at -40. Centre reduction at 1: a dual-mono file is exact
  silence; sound on the left alone leaves on the left bit for bit, and on
  the right turned over.
- `Denoise`, on the fixture this ticket describes: the noise between bursts
  falls 16.3 dB at the defaults (which are the Voice preset's), 11.5 dB at
  `Amount` 0.2 and 20.8 dB at 1; the bursts change by 0.00 dB; with the
  floor at -80 dB, under the noise, nothing changes. The mapping stands.
- Delay of a 5 ms burst through `Mixer::chunk`, against the same clip
  without effects: `afftdn` 24,979 us at 48 kHz and 24,965 at 44.1 kHz,
  `alimiter` 4,979 and 4,965, the others 0 (centre reduction at 0.5 reads
  one frame late because it halves the burst). With the frames dropped a
  voice with `Denoise` and `Limiter` lands 41 us early at 48 kHz and 68 us
  early at 44.1 kHz; each kind alone within 45 us. The test allows 1 ms.
- Probe, with a fake runner: the control and then seven strings in order,
  `atempo=1.5` for `Pitch`; each kind refused in turn is the only one
  missing; a failing control runs nothing more, leaves every kind
  available and is said once. Through `AudioService::start` with an `open`
  that finds no device: the probe ran and its notice arrives before "No
  native audio output device is available". With a child of the probe held
  open: two `sync`s and a `stop` leave it alive, and a probe run after them
  succeeds. Against the host's FFmpeg: `alimiter=limit=1:nosuchoption=1`
  and an unknown filter fail, the control succeeds, and the probe with that
  option added to the limiter's string marks the limiter and nothing else.
  Its eight runs took 297 ms.

The criteria this was held to:

- String tests: no audio effects gives exactly today's string; each kind's
  string at defaults and extremes; disabled effects absent; stack order
  kept; an unavailable kind absent.
- Fixture readback through `Mixer::chunk`: `EqBand` -24 dB at 1 kHz lowers a
  1 kHz tone by 20 to 24 dB and a 100 Hz tone by under 1 dB; `HighPass` at
  200 Hz lowers a 50 Hz tone; `Limiter` at -12 dB holds a full-scale tone at
  or under it; `Compressor` lowers a tone above threshold and not one below;
  `CenterCut` at 1 silences a dual-mono file and leaves a hard-panned one.
- `Denoise`, numerically. Fixture: 300 Hz plus 1 kHz in bursts of 0.5 s on
  and 0.5 s off, over white noise at -45 dBFS RMS. At the defaults the
  noise in the gaps drops by at least 10 dB; it drops by more at `Amount` 1
  than at 0.2; the level of the bursts changes by under 1 dB. If the
  mapping cannot meet this, change the mapping and record it; do not weaken
  the test. The Voice preset's `Denoise` values are rechecked against it.
- Delay: a 5 ms tone burst through each kind's chain, read at 44.1 and
  48 kHz, with the measured delay printed in microseconds; a voice with
  `Denoise` and `Limiter` lands the burst within 1 ms of its place at both
  rates once the frames are dropped.
- Probe: with a fake runner, a kind whose run fails is the only one
  missing; a failing control leaves every kind available and says so once;
  a `sync` during the probe does not cancel it; the probe runs with no
  output device. Against the host's FFmpeg, a string with an unknown option
  is reported unavailable (it exits 8 and writes nothing).

## Known limits

- A value is heard after the edit commits and the decoders restart, not
  while a slider moves.
- Stateful filters start cold at every seek and every edit: a compressor's
  envelope and `afftdn`'s noise estimate take a moment to settle. No
  pre-roll.
- Up to sixteen voices, each with a single-threaded chain: CPU cost and
  underruns are unmeasured.
- The probe proves each kind at its defaults. A build that accepts the
  defaults and refuses some other value makes that decoder exit, and a
  failed decoder stops all preview sound, not only that clip
  (`audio.rs:1007-1017`, `:690-699`), with FFmpeg's message. No restart
  without effects is attempted.
- `Denoise` has no automatic noise floor. With the floor under the real
  noise it does little: white noise at -35 dBFS fell 5 dB at
  `nr=18:nf=-40` and 24 dB at `nr=24:nf=-30`.
- The probe adds eight short FFmpeg runs to the first start of sound in a
  session; their time is unmeasured.
- `CenterCut` forces stereo and silences a mono source at full amount.
- On an output device of one channel `CenterCut` is not heard at all; the
  clip sounds as it would without it. Ticket 14a's export at two channels
  will hear it, so a mono preview and the export differ there.
- Spectral subtraction leaves audible artefacts at high amounts; nothing
  here was listened to.
- The last 25 ms of a source file are not heard through `Denoise`, and the
  last 5 ms through `Limiter`: FFmpeg ends the filtered stream as long as it
  came in, and the delay is taken off its start.
- The measured delays are one frame under the table at both rates (1,199
  and 1,101 frames for `afftdn`), so a compensated voice is 20 to 70 us
  early.
- Centre reduction puts what is on one channel alone on the other too,
  turned over; that is the spec's `pan` expression.
- A device under 44.1 kHz moves a band above 0.45 of its rate down to
  there; what that sounds like is not measured.
- No route of the desktop writes an audio effect yet (ticket 13); today
  one arrives by an agent's `set_effects`, a paste or a loaded file. Every
  applied command goes through `document_changed` (`src/app.rs`), which
  syncs the audio service while playing, so the chain is rebuilt after an
  edit and after its undo. That was read in the code, not run in the app.

## Comments

2026-10-13: Written with the spec. Option names were read from
`ffmpeg -h filter=…` on FFmpeg 8.1.2.

2026-10-13 (review): Rewritten after filters were run on the design host
(Homebrew FFmpeg 8.1.2, synthetic signals, nothing listened to).

- The probe by name is gone. It parsed "the third column" of
  `ffmpeg -filters`, which is the I/O signature (the name is the second
  token); it ran through the audio service's `Processes`, which every
  `sync` kills; it never ran on a host without an output device; and a
  listed filter says nothing of its options: `alimiter=…:nosuchoption=1`
  prints "Error applying option", writes 0 bytes and exits 8.
- `afftdn` with `tn=1`: steady white noise at -35.3 dBFS came out at -35.6
  with `nr` 6 and 36 alike. Without `tn`: -40.7 at `nr=18:nf=-40`, -49.6 at
  `nr=20:nf=-35`, -59.1 at `nr=24:nf=-30`. Hence two values and no
  tracking.
- Delay of a 5 ms burst at 48 kHz: `afftdn` +25.0 ms, `alimiter` +5.0 ms,
  `alimiter` with `latency=1` 0. The reviewer's impulse runs gave 1102,
  1200 and 2400 frames for `afftdn` at 44.1, 48 and 96 kHz, so the table is
  in time, not frames.

2026-10-13 (implemented): Built and tested in `editor-media` on macOS
26.5.2 (arm64, Rust 1.97.1, Homebrew FFmpeg 8.1.2). The spec's constants
and mapping are unchanged. Two things no ticket worded: a filter's
frequency is held under 0.45 of the output rate, since a headset at 16 kHz
would otherwise be handed a band at 20 kHz, past half its rate; and the
sound lane's call of the probe waits for ticket 14a, which adds the job it
goes before.

2026-10-13 (review fix): On an output of one channel, centre reduction at
its default silenced every source, not only a centred one. The voice asks
FFmpeg for `-ac 1` after the chain, and the sum of `c0-a*c1` and `c1-a*c0`
is `1 - a` times the plain sum. Run by hand on the design host (FFmpeg
8.1.2): a stereo file with 997 Hz on the left and 440 Hz on the right came
out at RMS 0.0884 plain and 0.0 with the filter at `-ac 1`, and 0.1250 with
it at `-ac 2`. The chain now takes the output's channel count; with one
channel `CenterCut` is left out, its bit is `ONE_CHANNEL`, and the notice
"Audio: centre reduction needs two output channels and is not heard on
this device" is said once a session. Two tests pin it:
`centre_reduction_is_left_out_of_a_chain_for_one_channel_and_said_once`
(strings, bits, notice) and
`one_output_channel_hears_a_clip_with_centre_reduction_as_it_is_without`
(a mixer at one channel reads the clip back bit for bit as the clip without
the effect). The second failed with the old rule. Channels of 2 and more
keep the filter; at 6 FFmpeg puts its two channels on the front pair.

2026-10-14 (ticket 22): "No route of the desktop writes an audio effect yet"
is older than the code: ticket 13's Audio section, presets, library tiles
and phrases write them, and the sound lane runs the probe before its first
loudness measurement (14a). "Ticket 14a's export" was a loudness
measurement, not an export; there is still no export.
