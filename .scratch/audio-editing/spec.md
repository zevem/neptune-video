# Audio editing

Status: ready-for-human
Progress: claimed
Type: task

Built, on one macOS host (macOS 26.5.2, Apple M1, Rust 1.97.1, Homebrew
FFmpeg 8.1.2): tickets 01 to 21a, and their documents (ticket 22,
[ADR 0008](../../docs/adr/0008-sound-is-levels-in-the-mixer-and-filters-in-the-decoder.md)).
Every owner decision below was delegated and was taken on the owner's behalf
on 2026-10-14; each is marked, the build follows it, and the owner may
reverse any of them.

Resolved: tickets 01 to 18 and 22. Native acceptance on 2026-10-14 (release
build, 94 captures of the running window at normal and narrow widths,
dark theme) resolved 02, 04, 07, 08a, 08b, 09, 13, 14b, 15 and 16b; the two
defects it left (the held volume percent drawn over a fade in, and no menu
from a right-click on a library tile's play button) are fixed, with
captures of the rebuilt release; see Comments. Ticket 03 was read against
the code on the owner's behalf and resolved.

Still `ready-for-human`, each with what is left of it captured or tested
headless: **19**, because recording needs a person with a microphone: Record
was not pressed, since it opens the host's microphone and may raise the
system permission prompt on that person's desktop; **20**, because the save
dialog and its progress card would show the person's real folders in the
system save panel, and this host has no private display to drive it on;
**21a**, because no stem separator is installed on the host, so no
separation, progress card or placed stems were seen. Tickets **21b** and
**21c** are research left open by decision (owner decision 11): neither is
built and the shipped behaviour stays.

Nothing was listened to, no test opened an output device or a microphone,
only the dark theme was captured, and nothing was run on Linux or Windows.
See [verification](../../docs/verification.md#audio-editing).

Waveforms, fades, track volume and solo, detached audio, background music,
voiceover recording, noise reduction, equalizer, compressor, limiter, pitch,
pitch-keeping speed, ducking, beat snapping and loudness normalization, added
to the core editor without weakening its contracts: validated reversible
commands, pure core, bounded workers, a callback that only copies, quiet idle.
Vocal and instrument separation is done by an external separator tool the
person installs (ticket 21a); the editor ships no model and no runtime.

This document is the binding design. Where a ticket and this file disagree, this
file wins; change it deliberately and note the change under Comments.

## Decisions

1. **Sound is shaped in two places.** Whatever is a level (clip volume, volume
   keyframes, fades, track volume, the master limiter) is arithmetic in the
   mixer's sum loop on the `neptune-video-audio` worker. Whatever is spectral or
   dynamic (equalizer, compressor, limiter, noise reduction, pitch, centre
   reduction) is an FFmpeg filter in the `-af` chain of the clip's own decoder
   process, which already exists. No DSP crate is added, nothing runs in the
   CPAL callback, and no second process is started for a clip.
2. **An audio effect is an `Effect`.** Seven kinds join the flat `EffectKind`
   table with rows in `look::spec`, built from parameter keys that already
   exist. No new `ParamKey`, no parallel `audio_effects` field, no struct per
   effect. `SetEffects` carries them. "Audio effect" is the one name for these
   kinds; "sound effect" stays the name of an imported sound clip (a whoosh,
   a hit), which is what feature 4 asks for.
3. **Strict now, relax later.** Audio kinds are legal only on clips on audio
   tracks, with `mix` 1 and no keyframes on their parameters: FFmpeg reads a
   filter's values once, when the decoder starts. A picture clip is one
   "Detach audio" away. Loosening either rule later invalidates no file.
4. **Fades are clip fields, not keyframes.** `fade_in` and `fade_out` are
   lengths held to the clip's edges. A keyframed fade is cropped away from the
   edge by a split or a trim; a field stays on it. One fixed equal-power curve,
   no choice of shape.
5. **Track volume and solo are track fields** beside `muted`, set by
   `SetTrackState`. Solo is durable so that what is heard is what the project
   says; see owner decision 4.
6. **One new command.** `SetFades`. Everything else is a `Batch` of commands
   that exist: detach audio, a music bed, a voiceover take, a normalized
   volume, a ducked envelope and beat markers are data any agent could write.
7. **Derived sound data is not project data** (advanced-editing decision 7).
   One peak file per media file, in a bounded cache outside the project, feeds
   the waveform, the ducking gate and the beat grid. Loudness is measured when
   asked and kept nowhere. The project holds only what the person decided: a
   volume, keyframes, markers they asked for.
8. **Analysis and recording are not commands.** A person starts them; their
   results enter through commands. An agent can write the result and cannot
   start the work, as with tracking and stabilization.
9. **A recording is the person's work.** It is written to a place that is never
   evicted and never deleted by the editor, including on undo: the
   `<document stem> media/` folder beside the saved document, made on the
   first take. Recording in an untitled project is refused until it is
   saved (ticket 17).
10. **Schema version 3, once.** A new `EffectKind` reads as a damaged file in a
    build that does not know it, so the version goes to 3 with the first
    durable field (ticket 05b) and the kinds arrive under it (ticket 10). Read
    accepts 1, 2 and 3 through serde defaults; write is always 3. Version 3 is
    not complete until both tickets are resolved.
11. **No neural runtime, model or download in the editor.** Noise reduction is
    spectral (`afftdn`), the ducking gate is signal energy, beats are an
    energy onset detector. Stem separation runs an external separator
    executable found the way FFmpeg is found (ticket 21a). Neural denoise and
    speech detection are research tickets 21b and 21c, with the same open
    decisions as advanced-editing ticket 16.
12. **FFmpeg filters are probed by running them, never assumed.** FFmpeg stays
    an external host tool. Once per session each audio kind's own filter
    string, at its defaults, is run over 50 ms of silence; a kind whose run
    fails (a missing filter or a refused option alike) is left out of every
    chain, said once, and shown as unavailable. That covers what the build
    does with the defaults. A chain that still fails when a voice starts
    stops the preview's sound with FFmpeg's message, as any decoder failure
    does today.

## Domain model (`crates/editor-core/src/`)

Every new field is `#[serde(default, skip_serializing_if = …)]`, so a version
1 or 2 file decodes and a default clip or track emits no new key. Helper names
below are illustrative.

```rust
// model.rs
pub struct Track { /* existing fields */
    pub gain: f32,            // linear, default 1.0, finite, 0.0..=8.0
    pub solo: bool,           // default false
}
pub struct Clip { /* existing fields */
    pub fade_in: Time,        // default ZERO; from the clip's start
    pub fade_out: Time,       // default ZERO; to the clip's end
}

// look.rs: appended to EffectKind; ALL becomes [Self; 27]
HighPass, EqBand, Compressor, Limiter, Denoise, Pitch, CenterCut,

impl EffectKind { pub fn is_audio(self) -> bool }

const HIGH_PASS: &[ParamSpec] = &[p(ParamKey::Frequency, 20.0, 2_000.0, 80.0)];        // Hz
const EQ_BAND: &[ParamSpec] = &[p(ParamKey::Frequency, 20.0, 20_000.0, 1_000.0),       // Hz
    p(ParamKey::Gain, -24.0, 24.0, 0.0), p(ParamKey::Width, 0.1, 4.0, 1.0)];           // dB, octaves
const COMPRESSOR: &[ParamSpec] = &[p(ParamKey::Threshold, -60.0, 0.0, -18.0),          // dB
    p(ParamKey::Amount, 1.0, 20.0, 3.0), p(ParamKey::Gain, 0.0, 24.0, 0.0)];           // ratio, make-up dB
const LIMITER: &[ParamSpec] = &[p(ParamKey::Threshold, -24.0, 0.0, -1.0)];             // ceiling dB
const DENOISE: &[ParamSpec] = &[p(ParamKey::Amount, 0.0, 1.0, 0.4),
    p(ParamKey::Threshold, -80.0, -20.0, -40.0)];                                      // noise floor dB
const PITCH: &[ParamSpec] = &[p(ParamKey::Amount, -12.0, 12.0, 0.0)];                  // semitones
const CENTER_CUT: &[ParamSpec] = &[p(ParamKey::Amount, 0.0, 1.0, 1.0)];

// sound.rs (new, pure)
/// Whether this clip is admitted to the mix at all: its track is not muted,
/// not soloed out and not at volume 0; its asset has sound; no linked
/// audio-track clip of the same asset owns that sound; and its volume is not
/// 0 without volume keys. The one definition; the mixer and the UI share it.
pub fn sounds(project: &Project, track: &Track, clip: &Clip) -> bool;
/// The ownership part alone: a picture clip silenced by its linked audio clip.
pub fn owned(project: &Project, clip: &Clip) -> bool;
/// `sounds`, and `at` is on a straight forward stretch of the clip's time map.
pub fn audible_at(project: &Project, track: &Track, clip: &Clip, at: Time) -> bool;
/// Track volume x clip volume (stored or keyed) x the edge fades.
pub fn level_at(track: &Track, clip: &Clip, playhead: Time) -> f32;
/// Whether `level_at` changes inside the clip: volume keys or a fade.
pub fn level_varies(clip: &Clip) -> bool;
/// 0..=1: sin(pi/2 * t) up over fade_in, the same down over fade_out.
pub fn fade_at(clip: &Clip, local: Time) -> f32;
/// Volume keyframes that dip a clip under `spans` (clip-relative, sorted).
pub fn duck_keys(spans: &[(Time, Time)], duration: Time, base: f32, depth: f32,
    attack: Time, release: Time) -> Vec<Keyframe>;   // <= MAX_KEYFRAMES
```

### Rules

- **Track.** `gain` is finite and within `0.0..=8.0`, the clip volume's own
  bound. `solo` is meaningful on video and audio tracks; a text track's is
  ignored. When any track is soloed, only soloed tracks sound. Mute wins over
  solo.
- **Fades.** `fade_in >= 0`, `fade_out >= 0`, `fade_in + fade_out <= duration`.
  Both are zero unless the clip is media whose asset has sound. They are
  timeline lengths: a fade is as long as it is heard, whatever the clip's
  speed. The curve is equal-power, so two clips overlapped on two tracks with
  matching fades cross at constant power.
- **What sounds.** `sound::sounds` is the only definition of "this clip is
  in the mix", and `audible_at` of "it is heard at this time". Link ownership
  (today private to `editor-media`, `audio.rs:836-843` and `plan.rs:145-151`)
  and the straight-segment test of a time map (`plan.rs:17-35`) move into
  core with them. `Plan::prepare` admits by `sounds`; the Sound card, fade
  rows, Normalize, the ducking gate and beat ticks ask the same function.
  A picture clip whose sound is owned by its linked audio-track clip shows
  "Sound is on the linked audio clip" in place of volume, fade and Normalize
  controls, and `fade`, `normalize` and `duck` phrases refuse it in words.
  Validation is not tightened: fades on an owned picture clip stay legal in
  a file, because unlinking the pair makes them heard.
- **Audio effects.** A audio kind is legal only on a clip on an audio track
  and a picture kind only elsewhere, as today. A audio effect has `mix == 1.0`,
  no `points` and no `asset_id`, and its parameters take no animation track:
  the existing rule that an audio-track clip animates only its own `Gain`
  stays as it is. `MAX_EFFECTS` (16) is unchanged. A parametric equalizer is
  several `EqBand` effects in the stack. At most one `Pitch` is read (the
  first enabled); more are legal and inert. `Denoise` has two values:
  `Amount` is how much is taken away and `Threshold` is the noise floor, the
  level under which sound counts as noise. It works only when the floor is
  set at or above the recording's real noise.
- **Presets are not a type.** "Voice" is a desktop action that writes ordinary
  effects (`HighPass` 80 Hz, `Denoise`, `EqBand` +3 dB at 3.5 kHz one
  octave wide, `Compressor` at its defaults) through `SetEffects`. "3-band EQ"
  writes three `EqBand`s at 120 Hz, 1 kHz and 8 kHz. Nothing remembers that a
  preset made them. `Denoise` in "Voice" is at its defaults (0.4, floor
  -40 dB).
- **Speed and pitch.** A clip's speed changes its pitch, as today, unless the
  clip has an enabled `Pitch` effect; then speed leaves pitch alone and the
  effect's semitones are the only shift. "Keep pitch" is a `Pitch` at 0. One
  `atempo` stage does the work, so pitch is held while clip speed divided by
  the pitch ratio is within 0.5 to 2; beyond that the clip still plays at
  its speed and the pitch moves by the remainder. The transport's shuttle
  speed always shifts pitch. Freeze, reverse and ramp segments stay silent.
- **Detached audio.** No type. A video asset with sound is already legal on an
  audio track, and the audio-track clip of a linked same-asset pair already
  owns the sound.
- **Ducking, beats, loudness.** No type. Ducking writes `Gain` keyframes;
  beats are a derived grid and, on request, ordinary markers; normalization
  writes a volume.

## Commands

All bounds live in `Project::validate`. A failed command leaves the project
unchanged.

| Command | Rule |
| --- | --- |
| `SetFades { clip_id, fade_in, fade_out }` (new) | Whole-value setter through `clip_mut`, so a locked track refuses it. Does not follow links: each clip has its own fades |
| `SetTrackState { …, gain: Option<f32>, solo: Option<bool> }` (extended) | Same pattern as `muted`; works on a locked track as the other fields do |
| `SetEffects` | Unchanged; now accepts audio kinds on audio-track clips |
| `SetGain`, `SetKeyframes`, `AddClip`, `LinkClips`, `AddTrack`, `AddAssets`, `AddMarker`, `Batch` | Unchanged; carry detach, music bed, voiceover, normalize, duck and beat markers |

Existing arms change. `TrimClip`, `SetRetime` and anything else that shortens a
clip clamp the fades with a `fit_fades` beside `fit_transitions`: each is cut
to the clip, then `fade_out` to what `fade_in` leaves. `SplitClip` leaves
`fade_in` on the left half and `fade_out` on the right and zeroes the two inner
edges, as it clears the inner transitions. `SetPlaybackRate` keeps the fades'
lengths and clamps them. Paste keeps them. The desktop's clipboard text
(`src/clipboard.rs`) goes from `VERSION` 2 to 3 in ticket 05a and reads 1, 2
and 3: it was raised to 2 for the same reason, so that a build which would
strip fields does not take the text for clips, and it refuses any other
version. The core `Clipboard` has no version.

## Evaluation (the audio plan)

There is no `AudioPlan` type. The mixer already holds an immutable project
snapshot and a `Plan` of sample intervals; two things are added to it.

**Level, in core.** `level_at` replaces `gain_at` in the sum loop
(`audio.rs`, the per-frame branch) and `level_varies` replaces
`plan::gain_animated` as the test for evaluating per frame. Admission
(`Plan::prepare`) asks `sound::sounds` in place of its own tests, which adds
two skips to today's: a track that is not soloed while another is, and a
track whose `gain` is 0. A clip with volume 0 and volume keys is still
admitted. Owners are counted whether or not their track is muted or soloed
out, so neither reveals embedded sound. Its runs of straight segments come
from the same core test `audible_at` uses.

**Chain, in media.** `editor_media::audio::chain(clip, speed, transport, rate,
filters) -> String` is the only place filter syntax exists. The voice's `-af`
becomes:

```
aresample=R,asetrate=round(R·transport·k),aresample=R[,atempo=…]…[,<effect>]…
```

- Without an enabled `Pitch`: `k = clip_speed`, no `atempo`. This is today's
  string, so a project without audio effects decodes exactly as before.
- With one: `P = 2^(semitones/12)`, `a = clamp(clip_speed / P, 0.5, 2.0)`,
  `k = clip_speed / a`, then one `atempo=a` (none when `a` is within 0.001
  of 1). The length is always right; the pitch is `P` while `a` is not
  clamped. One stage, never a chain of them: every stage makes the sound
  arrive earlier (see latency below).
- Effects follow the speed stage in stack order, so a frequency is the one
  heard. Disabled effects and kinds the probe found unavailable are left
  out.

| Kind | Filter |
| --- | --- |
| `HighPass` | `highpass=f=<Hz>` |
| `EqBand` | `equalizer=f=<Hz>:t=o:w=<octaves>:g=<dB>` |
| `Compressor` | `acompressor=threshold=<linear>:ratio=<r>:attack=10:release=150:makeup=<linear>` |
| `Limiter` | `alimiter=limit=<linear>:level=false` |
| `Denoise` | `afftdn=nr=<6+30·a>:nf=<floor dB>` |
| `Pitch` | the speed stage above |
| `CenterCut` | `aformat=channel_layouts=stereo,pan=stereo\|c0=c0-<a>*c1\|c1=c1-<a>*c0` |

Option names were read from `ffmpeg -h filter=…` on FFmpeg 8.1.2; the mapped
constants (attack, release, the `nr` range) are first guesses to be set in
ticket 11. `afftdn` runs without noise tracking: with `tn=1` the measured
reduction of white noise was under 1 dB whatever `nr` was, and without it
`nr` scales the reduction once `nf` is at or above the noise (Comments,
review entry).

A chain is fixed for the life of a `Mixer`: every `sync` kills all decoders and
builds a new mixer over a new snapshot, so `Voice` needs no chain fingerprint.
`-ss` stays on the input side, so the chain does not move the seek point. The
clock counts frames in the ring and is unaffected. Stateful filters start cold
at every seek and every edit.

**Latency.** Two cases, measured on FFmpeg 8.1.2 with a 5 ms tone burst.

- *Constant delay.* `afftdn` delays by 25.0 ms, the same at 44.1, 48 and
  96 kHz, and `alimiter` by 5.0 ms at 48 kHz; `highpass`, `equalizer` and
  `acompressor` by nothing. `chain.rs` holds these as microseconds per kind. When a
  voice starts, the sum for its chain is turned into frames at the mixer's
  own rate (the device's, `audio.rs:561-568`) and that many leading frames
  of the voice are dropped. `alimiter`'s `latency` option is not used: it
  removes the delay on this build, but it is one more option an older build
  may refuse, and the table already covers it.
- *`atempo`.* Its output arrives early, not late, by an amount that depends
  on the factor and moves a few milliseconds from one event to the next:
  about 13 ms early at 2, 16 ms at 1.5, 38 to 59 ms at 0.5. It is not
  compensated. One stage bounds the lead at about 60 ms; ticket 12 tests it
  with a tone-burst onset and a stated tolerance, not with an impulse, which
  `atempo` does not preserve.

**Master.** The hard clamp at the end of `Mixer::chunk` becomes a peak limiter
of a few dozen lines: per frame, the gain that brings the loudest channel to
the ceiling is taken at once and let go over about 200 ms; its one value of
state lives on `Mixer` and is reset in `clear()`. The ceiling is 1.0
(0 dBFS), the level the clamp cut at, so the limiter acts only where the old
code clipped: a single clip at unity, and any sum within ±1.0, leaves the
mixer bit for bit as it does today, in the timeline and in the source
monitor, which share the `Mixer`. It is always on and has no setting. A final
clamp stays behind it. A lower ceiling is owner decision 12.

`Mixer::chunk` already returns its samples and the crate's tests already drive
it without a device, so fades, track volume, solo, the limiter and filter
latency are proved by readback there. That is the headless oracle; there is
still no export, and a future one would call `level_at` and `chain` too.

## Workers, caches and UI

- **`crates/editor-media/src/audio/chain.rs`**: the chain, the latency table
  and the filter probe. The probe runs once per session, held in a
  `OnceLock`, with a `Processes` of its own so that `AudioService::sync`,
  which kills every child of the audio service, cannot cancel it. For each
  audio kind it runs `ffmpeg -f lavfi -i anullsrc -t 0.05 -af <the kind's
  string at its defaults> -f null -` and reads the exit status, after one
  control run with `anull`; if the control fails nothing is known, every
  kind counts as available and that is said once. The audio worker calls it
  before it opens the output device, so a host without one is still probed,
  and the sound lane calls it before a `Loudness` job; both may block on it.
  The UI thread only reads the finished answer
  (`editor_media::missing_audio_filters() -> Option<Vec<EffectKind>>`,
  `None` until the probe is done).
- **`crates/editor-media/src/sound.rs`**: `SoundService`, a lane of its own
  beside the video analysis lane, whose four-deep queue and long stabilization
  runs would starve automatic waveform requests. One worker thread, one FFmpeg
  at a time, its own `Processes`, a loader thread for cache reads, a queue of
  16, silent cancellation, one coalesced progress event. A newer request
  replaces an older one with the same target, as `Work::target()` does in the
  analysis lane (`analysis.rs:378-407`): the target is (job kind, asset) for
  `Peaks` and `Beats` and (job kind, asset, from, span) for `Loudness` and
  `Extract`. Requests with different targets never cancel each other, so a
  loudness measurement does not stop the same asset's waveform. Answers carry
  `request, project, asset_id, SourceIdentity, Result<_, String>` and the
  desktop drops any that match nothing it waits for. `cancel_all` on project
  replace. Jobs: `Peaks` (automatic, for clips in view), `Loudness` and
  `Beats` (on request).
- **Peak file.** `ffmpeg … -map 0:a:0 -vn -ac 1 -ar 8000 -af
  aresample=async=1:first_pts=0 -f s16le pipe:1`, so that bucket 0 is
  container time 0 even when the audio stream starts later, which is the
  time the mixer seeks in. Without the filter a stream that starts at 0.98 s
  decoded 0.98 s short and every waveform column, duck span and beat was
  early by that much. Reduced in Rust to 100 buckets a second of four bytes:
  `[min: i8, max: i8, rms: u8, low: u8]`, where `rms` and `low` are levels
  from -96 to 0 dBFS and `low` is the level under a one-pole 150 Hz low-pass.
  About 1.4 MB an hour. `PeakCache` is `SteadyCache`'s shape with its own
  magic (`ASTRPEAK`) and algorithm version: one file per media path, stamped,
  checksummed, temp-then-rename, least recently read evicted. Proposed
  bounds: 64 MiB, 512 files, `1 << 21` buckets (5.8 hours) per file; a longer
  file gets no waveform and says so. Directory `<user cache>/peaks`, beside
  the settings when `--settings` is given.
- **`src/peaks.rs`**: the desktop's held peaks, a bounded map like `Steadies`
  (proposed 32 MiB), asked for only by clips in view, claimed once, failures
  remembered, as `Textures::want_poster` does. One file at the bucket bound
  is 8 MiB, so the map holds as few as four assets: nothing may need every
  asset's peaks at once. The ducking gate takes one asset at a time, reduces
  it to spans and lets it go.
- **`crates/editor-media/src/record.rs`**: `RecordService`. A CPAL input
  stream on the default device whose callback only writes into a bounded
  lock-free ring; a writer thread drains it to a WAV whose header is rewritten
  every second, so a crash leaves a playable file. Overruns are counted and
  reported, never waited on. It keeps its own clock: an atomic count of the
  frames it has taken, which is the take's elapsed time and the anchor for
  placing it. Stopping is asked for and answered through `poll`, so the UI
  thread never waits for the file to be closed.
- **Timeline.** A waveform replaces the level line in audio-lane tiles,
  including audio-track clips of video assets. Fade handles at a tile's top
  corners (`Drag::Fade`, the geometry of `marking`/`mark_handle`). A volume
  line on the tile. A solo cell in the track head and a volume field in its
  menu. Beat ticks on a tile that shows them, and beats as snap candidates
  added in the UI's `snapped()` the way the playhead is.
- **Inspector.** `Section::Audio` for clips on audio tracks, rows generated
  from `look::spec` by `param_row`; `presented` gains dB, Hz, semitone and
  ratio cases. Fade rows in the Sound card for every clip that `sounds`.
- **Library.** A kind filter on the Media tab; an Audio group in the Effects
  tab that goes to audio-track clips.
- **Viewer transport.** A record button and an elapsed time.

## Feature by feature

| # | Feature | Chosen approach | Not done |
| --- | --- | --- | --- |
| 1 | Multiple audio tracks, waveforms | Tracks already have no fixed count. Add `Track.gain` and `Track.solo` (one multiply in the sum loop, one skip in admission) and a waveform drawn from the cached peak file, sampled through `Clip::source_time` per pixel column. | Pan, track effects, buses, track automation, meters, waveforms on video-lane tiles, sample-accurate waveforms (buckets are 10 ms), more than 16 voices, a stream other than `0:a:0` |
| 2 | Trim, split, fade, volume, audio keyframes | Trim, split, volume and volume keyframes exist. Add `fade_in`/`fade_out` with `SetFades`, folded into `level_at`; handles on the tile; a volume line; the keyframe lane opens on a clip with no keys yet. A picture clip whose sound its linked audio clip owns offers none of these and says where the sound is. | Fade curve shapes, a crossfade object on one track (overlap two tracks), keyframed effect parameters, dB display of volume |
| 3 | Audio extraction from video | "Detach audio": one `Batch` of `AddTrack` (if no lane is free), `AddClip` of the same asset and span on an audio track, and `LinkClips`. The audio clip then owns the sound; no file, instant, undoable. A separate ticket (20, needs triage) writes a WAV to a path the person picks. | Automatic linked audio on insert, stream copy, other streams, an app-owned media folder for extracted files |
| 4 | Sound effects and background music | Imported sound already drops on audio lanes and auditions in the source monitor. Add a kind filter in the library and "Add as background music": one `Batch` that repeats the asset on a "Music" track to the project's end at half volume, the last clip trimmed and faded out over 2 s. | A bundled or online sound library, generated sound, loop-point detection, track roles |
| 5 | Voiceover recording | `RecordService` (CPAL input, bounded ring, crash-tolerant hand-written WAV) records on its own clock while the timeline plays. A take is not bounded by the timeline: it runs on in an empty project and past the last clip, with the playhead held at the project's end. It is anchored to the playback clock, not to the button: the first time the sound device reports a position, that position and the frames captured so far are kept, and the clip is placed there with the pre-roll trimmed by `source_in`. On stop the asset is built from what the writer reports (no import, no ffprobe) and one `Batch` of `AddAssets`, `AddTrack` if needed and `AddClip` is the only history entry. The take is written to `<document stem> media/` beside the saved document (`editor_store::new_take`, called off the frame loop); recording in an untitled project is refused with "Save the project to record" (ticket 17). | Device choice, monitoring, punch-in, takes, compensation of device round-trip latency, echo cancellation, agent-started recording, the macOS microphone usage string of a packaged build |
| 6 | Noise reduction, voice enhancement | `Denoise` is `afftdn` in the voice chain with two values, Amount (`nr`) and Noise floor (`nf`), and no noise tracking, which measured as nearly inert. "Voice" is a preset of ordinary effects (high-pass, denoise, presence band, compressor). | Neural or learned-profile denoise, an automatic noise floor, de-reverb, de-essing, rendered files. `arnndn` needs a model file FFmpeg does not ship. Neural denoise is research ticket 21b |
| 7 | Equalizer, compression, pitch, speed | `EqBand` (`equalizer`), `Compressor` (`acompressor`), `Pitch` (`asetrate` then one `atempo`), all in the voice chain. Speed stays `playback_rate` and `Retime`; a `Pitch` effect on the clip makes speed keep pitch while speed over pitch ratio is within 0.5 to 2. | An EQ curve display, attack/release controls, multiband or sidechain, formant preservation, `rubberband` (not in ordinary builds), pitch held beyond one `atempo` stage, compensation of `atempo`'s lead (sound up to about 60 ms early at 0.5), sound on freeze/reverse/ramp, keeping pitch on a picture clip without detaching |
| 8 | Automatic ducking | "Duck under speech" on selected clips: an energy gate over the cached `rms` of every other clip, wherever `audible_at` says it is heard, taken one asset at a time, gives spans, `duck_keys` turns them into `Gain` keyframes, and one `Batch` of `SetKeyframes` writes them. Visible, editable, undoable. | A live sidechain, re-ducking when speech moves (run it again), speech detection (any loud sound triggers it) |
| 9 | Beat detection and synchronization | "Show beats" on a music clip: an onset envelope from the cached `low` and `rms`, tempo by autocorrelation over 60 to 200 BPM, beats placed by dynamic programming. Ticks on the tile; edges, trims and the playhead snap to them; "Markers from beats" writes at most 256 markers. The grid is derived and kept in memory only. | Cutting picture to the beat, downbeats and bars, any accuracy claim, beats visible to a headless agent |
| 10 | Vocal/instrument separation | "Separate stems" (ticket 21a) runs an external separator executable, found by `NEPTUNE_VIDEO_SEPARATOR` or on `PATH` and probed as FFmpeg is, as a file-writing job on the sound lane. Its stems are probed in the job and placed in one `Batch` as linked clips on new audio tracks, with the original clip's volume set to 0. The item is disabled, with words, when no tool is found. No crate, no bundled model, no network request by the editor. FFmpeg has no stem filter (none in `ffmpeg -filters` on 8.1.2). The ticket's first step is to verify the tool's command line and the licence of its weights; it also waits on ticket 17. `CenterCut` ("Reduce centre") is a separate, optional audio effect and is not separation: it lowers whatever is panned to the centre of a stereo file. | A bundled or downloaded model, an in-process runtime, separation without the tool installed, any quality or run-time claim, live separation |
| 11 | Normalization and limiting | "Normalize loudness": one FFmpeg run over the clip's span with its effects and `ebur128=peak=true` (the same figures as `loudnorm`, measured about six times faster), then `SetGain` so the span reaches -14 LUFS with true peak at most -1 dBTP and volume within 0..=8; a keyed clip has its keys scaled so its loudest key reaches that volume. A clip that does not sound is refused in words. A `Limiter` effect (`alimiter`) per clip. The master limiter replaces the hard clamp at the same level, 0 dBFS. | A span under 0.4 s (too short to measure; refused in words), `loudnorm` as a live filter, two-pass file normalization, programme loudness, a loudness meter, true-peak safety at the master |

## Dependencies added

None. No crate is added to any `Cargo.toml` and `Cargo.lock` does not change.

| Used | Version | Licence | Note |
| --- | --- | --- | --- |
| `cpal` | required 0.18.1, locked 0.18.2 | Apache-2.0 | Gains an input stream. Its input API was not read for this design; ticket 18 reads the 0.18.2 documentation first |
| FFmpeg, ffprobe | host tool, not bundled | the host build's own | Filters `highpass`, `equalizer`, `acompressor`, `alimiter`, `afftdn`, `atempo`, `pan`, `aformat`, `aresample`, `ebur128`, and the `lavfi` source `anullsrc` for the probe. All present in Homebrew FFmpeg 8.1.2 on the design host; no minimum version is established, which is why each kind is probed by running it |
| Demucs (`demucs`), the stem separator | host tool, not bundled, optional; 4.1.0 run | MIT code; weights unsettled | Only for ticket 21a. Its model is fetched by the tool, not the editor; 4.1.0 needs NumPy installed beside it |

Considered and left out: the `ebur128` crate (MIT) for loudness, kept as the
fallback if parsing the `ebur128` filter's summary proves fragile; the
`loudnorm` filter as the meter, which gave the same figures at about six
times the cost; `rustfft` (MIT OR Apache-2.0) for spectral-flux onsets, the
upgrade if the energy detector is too weak; `hound` (Apache-2.0), since a WAV
header is 44 bytes; `nnnoiseless` (BSD-3-Clause, RNNoise weights embedded),
`ort` and any model run in-process, in research tickets 21b and 21c; FFmpeg's
`rubberband` filter, which is not in the default Homebrew build (a GPL build
that still lacks it) and needs librubberband, so it is not relied on; with
the execution probe it could later be preferred when present, with
`asetrate` and `atempo` as the fallback, which is a follow-up and not scope;
Rust bindings that link `rubberband` or `aubio` (GPL), excluded for a
proprietary application. Versions and licences of the crates left out are as
reported by the proposals and were not re-checked.

## Acceptance

| Area | Proof |
| --- | --- |
| Model and commands | Core tests: fade and track bounds, NaN, reversal, atomic failure; trim clamps, split parts and rate keeps fades; audio kinds refused off audio tracks and with `mix` under 1 or keyed parameters; `sounds` and `audible_at` over mute, solo, track volume 0, a silent asset, an owned picture clip, volume 0 with and without keys, and freeze, reverse and ramp segments; v1 and v2 JSON decode |
| Store | v1 and v2 fixtures load; writer emits 3; version 4 is refused and protected everywhere; round trips of file, checkpoint and recovery with the new fields |
| Agent | Provider schema under 23 KiB and instructions under 6 KiB (owner decision 1's default, see the ticket 10 comment), both printed; capability list names `set_fades`; context projects the new fields; the instructions carry the units sentence for audio kinds; panel phrases parse |
| Clipboard | Text is written as version 3; 1, 2 and 3 are read; 0 and 4 are refused; a round trip keeps fades and an audio effect |
| Mixer | `Mixer::chunk` readback with FFmpeg fixtures: fade curve values at known frames, track volume, solo and mute admission, admission equal to `sounds` over a mixed project, limiter never above 1.0, a full-scale single tone bit for bit as before, a project without audio effects producing today's `-af` string |
| Chain | String tests per kind and for pitch at several speeds; the probe marks a kind whose string FFmpeg refuses and no other; fixture readback: a tone attenuated by a band, gap noise under a tone burst lowered by at least 10 dB by `Denoise` at its defaults, the constant delay of `afftdn` and `alimiter` removed at two sample rates, pitch ratio of a tone, duration preserved under `atempo`, a tone burst's onset under `atempo` within the stated tolerance |
| Analysis | Peak buckets of a known tone and of silence; a file whose audio starts late keeps bucket index equal to container time; cache stamp, checksum, eviction and stale tests; stale-answer and cancellation tests; jobs with different targets on one asset both answer; a synthetic click track's tempo and beat times; loudness of a tone of known level, of silence and of a span too short to measure |
| Ducking | `duck_keys` in core: spans to keys, merge under the 256-key cap, clip edges, validation of the result; a gate over more voice assets than the held map takes |
| Recording | Ring overrun counted, header valid after a simulated crash, with a fake input source; a take in an empty project and one that outlasts the last clip; placement from an anchor that arrives 300 ms after the service starts; one history entry with a user import in flight; a real device needs native acceptance |
| UI | One history entry per gesture; Escape cancels a fade drag; an owned picture clip offers no volume, fade or Normalize; captures of the running final build at normal and narrow widths for waveform, fade handles, track head, Audio section and record state |

## Not claimed

Export or any rendered audio file other than ticket 20's plain copy and
ticket 21a's stems (preview is the only consumer of the mix); audible quality
of any filter, of `atempo` or of the master limiter, since no listening
evidence exists and the measured audio clock rates were taken on silence;
real-time playback under sixteen filtered voices, CPU cost and underrun-free
output; A/V sync or sample-accurate alignment once filters are in the chain;
sound in time with the picture under `atempo` (it leads by about 13 ms at a
factor of 2 and by up to about 60 ms at 0.5, uncompensated); pitch held when
clip speed over pitch ratio leaves 0.5 to 2; that a chain which passed the
probe at its defaults is accepted at every value by every FFmpeg build; a
stream that starts after container time 0 played in time when the seek lands
before its start (the mixer plays it early by the gap, as it does today; the
peak file is container-aligned and the mixer is not); live parameter
audition (a value is heard after release, when the decoders restart);
keyframed effect parameters, pan, buses, track automation, metering; audio
effects on picture clips; speech detection (ducking is an energy gate); beat
accuracy, tempo changes, downbeats; neural denoise or enhancement; stem
separation without an external tool the person installs, and any quality,
licence or run-time statement about such a tool before ticket 21a verifies
one; loudness compliance with any standard; true-peak safety at the master;
microphone permission in a packaged build, input device choice, compensation
of device round-trip latency in a take; sound on freeze, reverse or ramp
segments; more than 16 simultaneous audible clips; analysis or recording
started by an agent or the headless CLI; assets added by native AI plans;
that the schema and instructions fit their caps (unmeasured until ticket 01);
a minimum FFmpeg version; macOS, Windows or Linux acceptance.

## Owner decisions

1. **Agent budgets** (ticket 01). The planner instructions have 124 bytes left
   of 5 KiB by a static count. The epic needs about 400: seven forced
   `"<kind> <keys…>"` lines (about 160), one units sentence for the audio
   kinds, a replacement for the sentence on audio-track clips, and one
   sentence on fades and track volume. Trim existing prose, drop kinds, or
   raise the cap deliberately. Schema headroom under 24 KiB is unknown until
   measured. Decided on the owner's behalf (2026-10-14): instructions under
   6 KiB and schema under 23 KiB (`MAX_SYSTEM_BYTES`, `MAX_SCHEMA_BYTES`),
   29 KiB between them as before, as ticket 10 built; no kind dropped.
2. **Audio effects on audio-track clips only.** A talking-head clip needs
   Detach audio before it can be denoised or keep its pitch. Allowing audio
   kinds on picture clips costs a skip in `resolve` and a shared 16-effect
   stack. Decided on the owner's behalf (2026-10-14): audio-track clips
   only, as built; loosening it later invalidates no file.
3. **Speed and pitch.** Chosen: speed changes pitch unless the clip has a
   `Pitch` effect, so existing projects sound the same. The alternative is
   that clip speed always keeps pitch, which changes documented behaviour.
   Decided on the owner's behalf (2026-10-14): the chosen rule, as built.
4. **Solo is durable** project state like `muted`, and enters undo history.
   AGENTS.md's separation of playback state from the project argues for
   session state. Decided on the owner's behalf (2026-10-14): durable and in
   the history, as built, because what is heard is then what the saved
   project says.
5. **Where recordings live** (ticket 17). Decided on the owner's behalf
   (2026-10-14): a `<document stem> media` folder beside the saved document,
   made on the first take and never deleted by the editor; recording in an
   untitled project is refused with "Save the project to record", the
   proposed default. Paths stay absolute (no store format change), so a take
   does not move with the project. The remaining alternative, an untitled
   project saved and then recording by itself, was also decided on the
   owner's behalf (2026-10-14): not done. The microphone starts only on a
   fresh press, and the person saves (Command/Ctrl+S) and presses Record
   again; the optional Save button beside the words was not built
   (ticket 19).
6. **Normalization target.** Proposed: one constant, -14 LUFS with a -1 dBTP
   ceiling. Decided on the owner's behalf (2026-10-14): that constant
   (`normalize::TARGET`, `CEILING`), as ticket 14b built.
7. **Ducking replaces a clip's existing volume keyframes**, after asking.
   Decided on the owner's behalf (2026-10-14): as built in ticket 15
   ("Replace this clip's volume keyframes?"), and Undo brings them back.
8. **`CenterCut`** is an optional audio effect beside the real separation
   route, not a substitute for it. Drop it if an effect that is not
   separation should not sit near that feature. Decided on the owner's behalf
   (2026-10-14): kept, as "Reduce centre", and documented as not
   separation.
9. **"Save audio as…"** (ticket 20): wanted as a feature, or is Detach enough?
   Decided on the owner's behalf (2026-10-14): wanted; built in ticket 20,
   with Detach audio unchanged.
10. **The stem separator** (ticket 21a). Decided on the owner's behalf
    (2026-10-14): the Demucs command line (`demucs`, 4.1.0, MIT code),
    found on `PATH` or by `NEPTUNE_VIDEO_SEPARATOR`, installed by the person. The
    licence of its pretrained weights is unsettled (the author: "provided
    for research purpose", 2023); the documentation says so and recommends
    the tool, not the weights for any use. The editor bundles and downloads
    nothing.
11. **Neural denoise and speech detection** (tickets 21b, 21c): licence
    policy, native library size, first network request, training-data
    provenance. Decided on the owner's behalf (2026-10-14): neither is built.
    The shipped `afftdn` noise reduction and the energy gate stay, and both
    tickets stay research with their questions unanswered.
12. **Master ceiling.** Chosen: 0 dBFS, so nothing that plays cleanly today
    changes. A ceiling of -1 dBFS leaves room for inter-sample peaks and
    turns down every mastered music file on each peak; it alters existing
    playback and the source monitor, so it is the owner's to ask for.
    Decided on the owner's behalf (2026-10-14): 0 dBFS, as ticket 06 built.
13. **A stock sound or music library**: content licensing and the editor's
    first network dependency. Not designed here. Decided on the owner's
    behalf (2026-10-14): none; the editor ships, fetches and generates no
    sound or music.

## Tickets

Sizes: S is under half a session, M about one, L a full session with native
captures or fixtures. A ticket split by the review keeps its number with a
letter, so earlier references stay readable.

| Id | Title | Depends | Size |
| --- | --- | --- | --- |
| 01 | Measure agent budgets and record FFmpeg filters | – | S |
| 02 | Detach audio | – | M |
| 03 | Sound analysis lane and peak cache | – | L |
| 04 | Waveforms in timeline tiles | 03 | M |
| 05a | Fades, track volume and solo: core types, `sound.rs`, commands, clipboard | 01 | L |
| 05b | Store schema v3 | 05a | S |
| 05c | Agent surface for fades, track volume and solo | 01, 05a | M |
| 06 | Mixer: `level_at`, admission by `sounds`, master limiter | 05a | M |
| 07 | Fade handles, fade rows and phrases | 05a, 06 | M |
| 08a | Track head solo and volume | 05a, 06 | M |
| 08b | Volume line on the tile and the keyframe lane | 04, 05a | M |
| 09 | Library kind filter and background music | 05a | M |
| 10 | Audio effect kinds, validation and planner text | 01, 05a, 05b, 05c | M |
| 11 | Voice filter chain, latency table and execution probe | 06, 10 | L |
| 12 | Pitch and pitch-keeping speed | 11 | M |
| 13 | Audio inspector section, Audio effects group, Voice preset | 11 | L |
| 14a | `Loudness` job and the stderr reader | 03, 11 | M |
| 14b | Normalize loudness: gain rule and routes | 05a, 13, 14a | M |
| 15 | Duck under speech | 04, 05a, 08b, 13 | M |
| 16a | `beats()` and the `Beats` job | 03 | M |
| 16b | Beat ticks, snapping, markers and phrases | 04, 16a | M |
| 17 | Where recordings live (grilling) | – | S |
| 18 | `RecordService` | 17 | M |
| 19 | Voiceover: record button, take clock and placement | 18 | L |
| 20 | Save audio as a file | 03 | M |
| 21a | Separate stems with an external tool | 03, 17 | L |
| 21b | Neural denoise (research) | – | S |
| 21c | Speech detection for ducking (research) | – | S |
| 22 | ADR 0008, glossary, usage, design, performance and verification docs | all | M |

Tickets 02, 03 and 17 can start at once. 05b is the only ticket that changes
the store's version; 10 is the only other one that changes what a file may
hold.

## Comments

2026-10-13: Spec synthesized from three independent architecture proposals
(FFmpeg-filter, in-process DSP, product-risk) over five subsystem maps.
Nothing is implemented, built or measured. The in-process DSP graph was set
aside: it adds five crates and a per-track bus to gain live parameter changes
and keyframed effect parameters, neither of which is asked for. Settled by
reading the code: `Mixer::chunk` returns its samples and is already driven by
tests, so no offline mix harness is a ticket; a mixer never outlives its
snapshot, so a voice needs no chain fingerprint; no bare `R` shortcut exists
in `src/`; `Process` keeps its stderr tail private, so ticket 14a adds a
reader. Filter option names were read from FFmpeg 8.1.2 on macOS; no filter
was run.

2026-10-13 (adversarial review): Twenty-six findings were checked against
the code and, where they rested on FFmpeg's behaviour, by running Homebrew
FFmpeg 8.1.2 on the design host with synthetic signals. None was rejected.
One, the column to read in `ffmpeg -filters`, was true and is moot: that
listing is no longer parsed. Nothing is implemented or built, and nothing
was listened to. What changed:

- *Voiceover* (feature 5, tickets 18, 19). The transport and the audio
  worker both stop at `project.duration()` (`playback.rs:47-86`,
  `audio.rs:681-689`), which is 0 in an empty project, so a take now runs on
  `RecordService`'s own clock and is not ended by the project's end. The
  take is anchored to the first position the sound device reports, with the
  pre-roll trimmed by `source_in`, because the worker waits for the device
  and its decoders before any sample plays (`audio.rs:621-647`). The asset
  is built from the writer's own figures: `MediaService::import` returns the
  shared generation, not a request (`service.rs:304-323`), and the desktop
  commits every import answer as its own "Import media" entry
  (`app.rs:2354-2389`). Stop is answered through `poll`.
- *Peak file* (ticket 03). `-af aresample=async=1:first_pts=0` added. A 3 s
  MP4 whose audio starts at 0.976 s decoded to 32,694 bytes (2.04 s) without
  it and 48,310 bytes (3.02 s) with it.
- *Master limiter* (ticket 06). Ceiling 1.0 in place of -1 dBFS; the clamp
  it replaces is an identity within ±1.0 (`audio.rs:1035-1037`). The lower
  ceiling became owner decision 12.
- *What sounds* (tickets 05a, 06, 07, 14b, 15). `sound::sounds` and
  `audible_at` are the one definition; ownership and the straight-segment
  test move from `editor-media` into core. An owned picture clip no longer
  offers controls that are never heard.
- *Sound lane* (tickets 03, 14a, 16a, 20). Replacement is by target, (kind,
  asset) or (kind, asset, from, span), not by asset.
- *Probe* (decision 12, ticket 11). By execution, in a `OnceLock` with its
  own `Processes`, before the device is opened. `ffmpeg -filters` is no
  longer parsed (its name column is the second token, not the third). A
  refused option exits 8 with no output, which the old probe could not see.
  `AudioService::unavailable` is not added; `AudioState::unavailable`
  already means something else.
- *Denoise* (tickets 10, 11, 13). `tn=1` dropped and a noise-floor value
  added from the existing `Threshold` key. Steady white noise at -35.3 dBFS
  read -35.6 with `tn=1` at `nr` 6 and 36 alike; without it, -40.7 at
  `nr=18:nf=-40`, -49.6 at `nr=20:nf=-35` and -59.1 at `nr=24:nf=-30`.
- *Latency* (tickets 11, 12). Table in microseconds, turned into frames at
  the device's rate. Measured: `afftdn` +25.0 ms, `alimiter` +5.0 ms (0 with
  `latency=1`, not used), `atempo` -13.3 ms at 2, -58.6 ms at 0.5 and
  -151.5 ms for two stages of 0.5. Pitch is therefore one `atempo` stage,
  clamped, with the lead stated and not compensated.
- *Loudness* (tickets 14a, 14b). The `ebur128` filter in place of
  `loudnorm`: ten minutes of stereo noise took 1.75 s against 10.68 s
  (0.26 s for the source alone). Its summary reports `I: -70.0 LUFS` both
  for silence (`Peak: -inf`) and for a 0.3 s tone (`Peak: -38.1`), so the
  two are told apart by the peak. `framelog=verbose` keeps its per-frame
  lines out of stderr (922 bytes against 32,322 for 20 s). The reader waits
  for the stderr thread, whose handle `Process` does not keep
  (`process.rs:148-167`). A keyed clip is scaled by target ÷ its highest
  key.
- *Clipboard* (ticket 05a). `VERSION` 3, stated, not left to the ticket.
- *Agent text* (tickets 01, 05c, 10). A units sentence, the sentence on
  audio-track clips replaced, not added to, and the estimate raised to
  about 400 bytes.
- *Ducking* (ticket 15). The gate takes one asset's peaks at a time.
- *Separation* (feature 10). A chosen route, ticket 21a, in place of "not
  delivered"; neural denoise and speech detection split off as 21b and 21c.
- *Names.* "Audio effect" for the filter kinds, "sound effect" for an
  imported sound clip.
- *Facts.* `cpal` is locked at 0.18.2. `rubberband` is absent from this GPL
  build; availability, not licence, is the reason it is not relied on.
- *Tickets.* 05, 08, 14, 16 and 21 were split along the advanced-editing
  precedent; ticket 20 gained its missing dependency on 03.

2026-10-13 (ticket 01, measured): On macOS 26.5.2 (Apple M1, Rust 1.97.1,
Homebrew FFmpeg 8.1.2). No provider request was made. Details are in the
ticket.

- *Schema fits.* 20,775 bytes measured; the epic adds an estimated 644
  (`set_fades` 284, `set_track_state` 100, `Track` 82, `Clip` 102, seven
  kind names 76), leaving about 3,156 under 24 KiB.
- *Instructions do not fit, and owner decision 1 is still open.* The epic
  needs 477 bytes with the sentences as tickets 05c and 10 word them, not
  about 400: forced lines 163, units sentence 180, the replaced sentence 32,
  fades and track volume 102. `SYSTEM` was reworded from 4,995 to 4,778
  bytes with every rule kept, which leaves 341. Dropping `center_cut` and
  `high_pass` brings the need to 437, so dropping kinds alone does not
  settle it; the rest is deleting existing rules or moving the cap.
  Recommended: instructions under 6 KiB and schema under 23 KiB, the same
  29 KiB in all. That is the two test asserts in `planner.rs`, the runtime
  guard at `crates/editor-agent/src/provider/mod.rs:33` (which must move to
  23 KiB with them, or the shipped bound becomes 30 KiB) and
  `docs/native-ai.md:155`. Nothing was raised here. Until it is decided, ticket 05c's
  sentence fits and ticket 10's text does not.
- *Filters.* The control, the seven kinds' strings at their defaults,
  `ebur128=peak=true` and `aresample=async=1:first_pts=0` each exit 0 over
  50 ms of `anullsrc`; a refused option exits 8 and an unknown filter 234.
  This says nothing of older builds.
- *Sizes are printed* by the two planner tests under `-- --nocapture`;
  `scripts/test-workspace.py` does not show them.

2026-10-13 (ticket 02, implemented): Detach audio is built and tested on
macOS 26.5.2 (arm64, Rust 1.97.1); the native capture of its menu item is
still to be taken, and nothing was listened to. Details are in the ticket.
Settled by building it:

- *Where the lane is chosen.* The action carries the picture clip's id; the
  audio track is picked, or a new one named, from the project when the
  action is carried out. A batch made earlier for a lane or a picture track
  that was locked since is refused whole by the project.
- *What the new clip keeps.* Besides what the ticket lists it keeps the
  picture's name and group and every clip field the ticket does not name,
  so fields later tickets add (the fades of 05a) are carried unless that
  ticket says otherwise. Its link is the picture's: clips the picture was
  already linked to end in the same link.
- *Refusals.* A clip on an audio track, a clip without sound (a title, a
  still, a silent video), a locked track, and a picture whose link already
  holds an audio-track clip of its asset. A link to an audio clip of other
  media does not refuse it. Added after review: a picture linked to another
  video-track clip of the same media is refused too, because ownership is by
  link and media and the new audio clip would silence that clip with nothing
  over its span. Linking such a clip to a detached pair by hand is not
  refused; ownership by span is ticket 06's to decide.
- *The audio tile.* It shows the video's poster frame until ticket 04 draws
  waves there.
- *Owner set.* `plan::audio_owners` in `editor-media` is the set
  `Mixer::new` built inline, moved unchanged; ticket 06 still moves
  ownership into core.

2026-10-13 (ticket 03, implemented): The sound lane and the peak cache are
built and tested in `editor-media` on macOS 26.5.2 (arm64, Rust 1.97.1,
Homebrew FFmpeg 8.1.2); nothing in the desktop calls them yet and nothing
was listened to. Details are in the ticket. Settled by building it:

- *Names.* `SoundService::peaks` and `load_peaks` take a `PeakJob { project,
  asset, cache }`. The cache answers `KeptPeaks::{Held, Stale, Missing}`, in
  `SoundEvent::Loaded { …, kept }`. `Peaks::at` hands back a `Peak { min,
  max, rms, low }`. The queues are `SOUND_QUEUE` and `PEAK_LOADS`, 16 each.
- *An answer that could not be kept.* `SoundEvent::Peaked` has one field
  more than listed, `unkept: Option<String>`, as `SteadyEvent::Done` has:
  the peaks are the answer and the reason the cache refused them goes with
  it.
- *Bucket edges.* `min` is rounded down and `max` up to a 128th of full
  scale. The samples left after the last whole bucket are a bucket, so a
  file's count is its length times 100 rounded up, give or take what the
  decoder adds at the end.
- *A read that fails.* FFmpeg's exit status is read after the stream ends;
  a failure, or more sound than the bucket bound whatever the container
  said, is an error and nothing is kept.
- *The other kind.* Until ticket 14a or 16a adds a real second job, the
  target rule is proved with a `#[cfg(test)]` job kind, which that ticket
  removes.
- *Measured.* Two hours of silent mono FLAC (720,000 buckets) read in
  0.98 s on the `dev` test profile and 1.64 s unoptimized, one run each.
  Silence is the cheapest sound to decode; no real recording was timed.

2026-10-13 (ticket 04, implemented): Waveforms are drawn in audio-lane
tiles on macOS 26.5.2 (arm64, Rust 1.97.1, Homebrew FFmpeg 8.1.2), with
headless tests and native captures at two widths; maximum zoom is not
captured and nothing was listened to. Details are in the ticket. Settled by
building it:

- *Names.* The desktop's held map is `HeldPeaks` in `src/peaks.rs`; `Peaks`
  stays the data in `editor-media`. Its seam is `want`, `ask`, `answered`,
  `forget_asset` and `clear`.
- *A sound lane draws no poster.* An audio-lane tile shows the level line
  until its peaks are held and waves after, of a video asset too, and asks
  for no poster. This replaces ticket 02's "shows the video's poster frame".
  The level line sits where the waveform draws silence.
- *Peaks on screen are not evicted.* The 32 MiB bound is kept by not asking
  for media that has no room beside the tiles on screen, not by evicting one
  of them, which would be asked for again in the next frame without end.
  Tickets 15 and 16b, which take peaks for work and not for a tile, need a
  route that does not count as a tile on screen.
- *Failures.* Media the lane refuses or cannot read keeps the level line,
  is said once in a notice and is not asked about again until the media
  changes or the project is replaced. A full lane is not a failure: the tile
  asks again.

2026-10-13 (ticket 04, after review): A sound read whose tile has not been
drawn for two frames is given up and asked again when the tile is drawn
again, so the lane reads for what is in view; tickets 15 and 16b, which take
peaks for work and not for a tile, need a route this does not give up. A
refusal for want of room refuses only peaks no smaller than the least
refused in that frame. The seam gained `project_replaced`, `media_replaced`
and `give_up_unwanted`. The native capture at maximum zoom is still not
taken; that acceptance item waits for a person or the owner's waiver.

2026-10-13 (ticket 05a, implemented): The fields, `SetFades`, the two
`SetTrackState` fields, `sound.rs` and clipboard version 3 are built and
tested in the core and the desktop on macOS 26.5.2 (arm64, Rust 1.97.1).
Nothing reads them for sound or drawing yet and nothing was listened to.
Details are in the ticket. Settled by building it:

- *Names.* The module is `editor_core::sound`. Beside the functions listed
  under Domain model it has `volume_keyed`, `steady_speed` and
  `audible_run` (the mixer's `gain_animated` and its straight-segment test
  and runs, for ticket 06 to call in place of its own) and `Sounding`,
  built once from a project, which answers `sounds`, `owned` and
  `audible_at` for many clips. `Plan::prepare` should hold one `Sounding`.
- *Refusals.* `Invalid("track volume")` and `Invalid("fades")`. Track
  volume is bound on a text track too.
- *An edge without a fade is 1 on both sides.* `fade_at` of a clip without
  fades is 1 at any time, inside the clip or not; a faded edge is 0 beyond
  the clip.
- *Solo and mute.* A track that is soloed and muted does not sound and
  still solos the others out.
- *Schema.* 21,783 bytes measured with these fields and `set_fades`: 1,008
  more than ticket 01's 20,775, where 644 were estimated. 2,793 bytes are
  left under 24 KiB before ticket 10's seven kind names; under the 23 KiB
  ticket 01 recommends, 1,769.
- *Between 05a and 05c* `capabilities_name_every_command_in_the_schema` is
  red: the schema has `set_fades` and the capability list does not.

2026-10-13 (ticket 05c, implemented): The agent's context, planner text and
protocol documents carry fades, track volume and solo, tested in
`editor-agent` on macOS 26.5.2 (arm64, Rust 1.97.1). No provider request was
made. Details are in the ticket. Settled by building it:

- *Defaults are skipped in every view.* The focused view omits `gain`,
  `solo` and the fades at their defaults as the complete one does.
- *Instructions.* 4,880 bytes measured with the fades and track-volume
  sentence, 239 left under 5 KiB. Ticket 10's text needs 375, so owner
  decision 1 is still open and ticket 10 waits on it. The schema is
  unchanged at 21,783 bytes.
- *Solo is not in the instructions;* the schema names it.

2026-10-13 (ticket 06, implemented): The mixer reads `level_at`, admits by
the core's `sounds` and ends in the master limiter, tested by readback in
`editor-media` on macOS 26.5.2 (arm64, Rust 1.97.1, Homebrew FFmpeg 8.1.2).
Nothing was listened to and no device was opened. Details are in the
ticket. Settled by building it:

- *Ceiling.* Owner decision 12 is unanswered; the default, 0 dBFS, is built.
- *The gain reaches exactly 1.* Near 1 a step of the 200 ms curve is under
  what an `f32` resolves, and the gain stopped at 0.9997, so the limiter
  never let go and every later frame was multiplied. A step is now at least
  one `f32` epsilon, which makes the last stretch a straight line of 200 ms
  at 48 kHz (counted in a test; "about 50 ms" stood here and was wrong).
  The gain is back at 1 only 1.42 s after a peak of twice the ceiling, and
  until then frames within the ceiling are turned down too. Without it "a
  sum within ±1.0 leaves bit for bit" would hold only until the first peak.
- *What is not finite.* A frame with an infinite sample is left to the
  clamp and does not move the gain.
- *The voice-limit message.* The count is taken on the first sample of the
  chunk that has more than sixteen, and the time is that sample's as
  `hh:mm:ss`.
- *Ownership stays by link and media.* The ticket 02 entry left ownership
  by span to this ticket. It is not added: the core's `owned` is the one
  rule, and a picture linked to an audio clip of its media is silent for
  its whole length.
- *Fixtures.* The readback tests decode WAV files the test writes sample by
  sample, not `lavfi` tones, so "bit for bit" is checked against known
  samples.

2026-10-13 (ticket 07, implemented): Fade handles, the inspector's fade
rows, the clip menu rows and the three phrases are built and tested headless
in the desktop crate on macOS 26.5.2 (arm64, Rust 1.97.1). Two native
captures show stored fades drawn; none shows a handle, a held drag or the
inspector rows, which need a pointer, and nothing was listened to. Details
are in the ticket. Settled by building it:

- *Where the handles are.* At the top corners of the tile's sound, under
  the name row, not of the tile, where they would cover the name and the
  chevron of a keyframe lane. On tiles from 40 points wide.
- *Handles are widgets.* Named and claimed before the clip, as a
  transition's handle is; `Hit::Clip` gained nothing.
- *Refusals in words.* An owned picture clip: "Sound is on the linked audio
  clip." A clip without sound: "That clip has no sound to fade." The
  interface refuses these; validation is unchanged.
- *Several clips.* The menu rows and the phrases fade every selected clip
  that takes a fade as one step, each held to its own room; clips that take
  none are passed over, and the reason is said when none is changed.
- *Units.* A handle lands on a frame or a snap; a typed length is stored as
  typed, held to the room.

2026-10-13 (ticket 07, after review): Two rules changed. A fade follows the
travel of its handle, since a handle kept clear of the trim grips is not
where a short fade ends; brought back to where it was taken hold of, it
leaves the fade it has. An inspector fade field reads hundredths of a second
and changes the fade only when what it reads changes, so focusing a field
does not round a frame-exact fade or enter the history.

2026-10-13 (ticket 08a, implemented): The solo cell, the dimmed mute, the
volume field in the head's menu and the three phrases are built and tested
headless in the desktop crate on macOS 26.5.2 (arm64, Rust 1.97.1), with
native captures of the heads at two widths. No capture shows the open menu,
which needs a pointer, and nothing was listened to. Details are in the
ticket. Settled by building it:

- *Four cells do not go in a row.* Beside lock, mute and hide a fourth cell
  leaves a video head's name 20 points of 156, less than "Video 1". A video
  head has two columns of two, hide under lock and solo under mute; an audio
  head has its three in a row. Cells stay 22 points.
- *The solo rule has a name.* `sound::soloed_out(project, track)` is the
  solo part of `sounds`, public for the head. A head is dimmed when its
  track is not muted and is soloed out; a muted one shows its mute.
- *Solo is a pair of headphones,* in the accent when on; the icon is new.
- *The field and the figure read what is kept, to a hundredth of a percent,*
  and a volume set from the field or a phrase is kept to that hundredth
  (changed after review: they read whole percent, so 0.4% said "0%" and
  could not be set to 0). Return takes a typed volume; a drag is one entry
  on release, none when Escape ends it, and an arrow one each. The field holds to 0 to 800; the phrase refuses outside it, and
  refuses a text track, in words.
- *Owner decision 4 is unanswered;* solo is durable and in the history, the
  default.

2026-10-13 (ticket 08b, implemented): The volume line, its drag, Alt-click,
"Key volume" and the empty Volume row are built and tested headless in the
desktop crate on macOS 26.5.2 (arm64, Rust 1.97.1), with native captures of
the line at rest at two widths. No capture shows the empty row or a held
line, which need a pointer, and nothing was listened to. Details are in the
ticket. Settled by building it:

- *The lane goes by the inspector's rule, not by `sounds`.* The empty Volume
  row and "Key volume" are offered for a clip whose media has sound and
  whose sound no linked audio clip owns, as ticket 07 offers the Volume and
  fade rows. A clip that is muted, soloed out or at volume 0 keeps them:
  `sounds` is false for it, and keying its volume is how it is brought back.
- *The scale is of the tile's sound,* under the name row: 100% at two thirds
  of that height, 200% at its top, in two straight pieces.
- *The line is taken hold of on a selected tile only,* within 4 points of
  it and clear of the trim grips, and follows the pointer's travel to a
  whole percent from 0 to 200. A keyed line is not dragged. Alt-click keys
  an unselected clip too.
- *A key is written at the volume the clip has there,* so writing one
  changes nothing heard, and none is written where one already is. After
  review: among other keys it is put on the curve by `AnimTrack::insert` in
  the core, which divides a held or eased stretch as a split does; it was
  linear before and turned a step into a ramp.
- *A line that stands where the stored volume is drawn keeps it* (after
  review), so lifting a line above 200%, or returning one at a finer volume
  to its place, writes nothing.
- *The graph editor is unchanged:* a clip without keyframes has no curve.

2026-10-13 (ticket 09, implemented): The kind filter and "Add as background
music" are built and tested headless in the desktop crate on macOS 26.5.2
(arm64, Rust 1.97.1), with native captures of the filter at rest and of a
bed on the timeline at two widths. No capture shows the menu item or a
filter other than All, which need a pointer, and nothing was listened to.
Details are in the ticket. Settled by building it:

- *What each filter lists.* By the asset's kind: Video is video, Sound is
  audio files, Images is stills and GIFs. A LUT is under All alone, and a
  video with sound is not under Sound.
- *Folders stay under a filter,* to be walked into, whatever they hold; a
  search looks through every folder within the kind. An empty list says
  "No sound" (or video, or images) and whether it means the folder or the
  project.
- *The filter is the person's for the session:* kept across a project
  replace, as the library's tab is, and in no file or setting.
- *A bed is of a sound file.* Media of another kind is refused in words, a
  fourth refusal beside the three listed; the order is an empty project,
  then a sound under a tenth of a second, then more than 256 clips.
- *Where the ids are made.* The action carries the asset's id; the track
  and the clips are made from the project when it is carried out, as
  ticket 02 has it.
- *The phrase without a name* takes the sound selected in the library.
- *The refused row* is shown disabled in the sound's menu and says why on
  hover.

2026-10-13 (ticket 09, after review): One rule changed. *A name that several
media share,* in any case, is the one a bed can be made of, then a sound
file, then the first; before, it was whichever the library held first, so a
video with the name of a sound could refuse the phrase. Three tests were added:
the largest bed (256 clips) applied, validated and undone as one step, the
disabled row's reason drawn on hover, and the shared name. The menu capture
is still a person's to take.

2026-10-13 (ticket 10, implemented): The seven audio kinds, their
validation and the planner's text are built and tested in the core, the
store, the agent and the desktop on macOS 26.5.2 (arm64, Rust 1.97.1). The
mixer does not read them, no provider request was made and nothing was
listened to. Details are in the ticket. Settled by building it:

- *Owner decision 1, by default.* The owner has not answered, so ticket
  01's split is built: instructions under 6 KiB and schema under 23 KiB,
  29 KiB between them as before. One constant each in `planner.rs`
  (`MAX_SYSTEM_BYTES`, `MAX_SCHEMA_BYTES`) is read by the two tests, a
  compile-time assert and the runtime guard; `docs/native-ai.md`,
  `docs/verification.md` and the Acceptance row above say the same. The
  owner may reverse it: at 5 KiB the text does not fit (5,308 bytes).
  Measured: instructions 5,308 (836 left), schema 21,859 (1,693 left).
- *Owner decision 8, by default.* `CenterCut` stays; all seven kinds are in.
- *Refusals.* A picture kind on an audio-track clip is still `Invalid("audio
  clip look")`, and so is an animation track on an audio effect. An audio
  kind on a clip of another track is `Invalid("audio effect on a picture
  clip")`, on a title `"text clip look"`. `mix` other than 1, `points`,
  `asset_id` and a value out of range are `Invalid("effect")`, from
  `Effect::validate`.
- *The guard refuses at the cap.* "Under 23 KiB" in the tests and at run
  time alike; the old guard passed a schema of exactly its cap.
- *The text says where the kinds go.* The seven key lines follow "and the
  audio kinds, for clips on audio tracks only:", which no ticket worded:
  without it nothing told a provider which kinds are audio kinds or that a
  picture clip refuses them.
- *The desktop.* `browser::offered` leaves audio kinds out of the Effects
  tab and of the `add effect` and `remove effect` phrases. They are a
  family of their own, "Audio", for ticket 13 to list. Until then a person
  cannot see or remove one an agent wrote, other than by undo.
- *`look::animatable` is not narrowed:* it is true for an audio kind's
  keys, and the project refuses the track. Ticket 13's rows must not offer
  keyframes.

2026-10-13 (ticket 11, implemented): Audio effects are heard through the
voice chain, the probe runs before the device is opened and the constant
delay is dropped, tested by readback in `editor-media` on macOS 26.5.2
(arm64, Rust 1.97.1, Homebrew FFmpeg 8.1.2). `Pitch` is still inert, no
device was opened and nothing was listened to. Details are in the ticket.
Settled by building it:

- *Constants.* All stand as first written: attack 10 ms, release 150 ms,
  `nr` 6 to 36 dB, floor -40 dB. On the ticket's fixture the noise between
  bursts falls 16.3 dB at the defaults, 11.5 at `Amount` 0.2 and 20.8 at 1,
  and the bursts do not change. None was set by ear.
- *Signatures.* `chain` returns a `Result`: it holds the refusal of a speed
  outside the resampling range. The effects part alone is `effects(clip,
  rate, channels, filters)`, which takes the rate and the channel count for
  the two rules below.
- *A frequency is at most 0.45 of the output rate.* The chain runs at the
  device's rate, and a band above half of it is no filter.
- *No centre reduction into one channel.* The voice is summed to the
  device's channels after the chain, and the sum of the `pan` expression's
  two channels is `1 - amount` of the plain sum: silence at the default for
  every source. On a one-channel output `CenterCut` is left out and said
  once a session; two channels and more keep it.
- *Delays.* Measured through the mixer: `afftdn` 24,979 us at 48 kHz and
  24,965 at 44.1 kHz, `alimiter` 4,979 and 4,965, one frame under the
  table. A voice with both lands 41 and 68 us early once the frames are
  dropped. The end of a source file is short by the same delay.
- *Said once.* A missing kind is said when a voice first goes without it,
  not when the probe ends, so a kind nobody uses is never mentioned; a
  failed control is said when the probe ends. A notice the event channel
  had no room for is said the next time.
- *The mixer assumes nothing is missing until it is given a probe's
  answer,* so its tests and the source monitor's start no probe; the audio
  worker gives it the session's.
- *The sound lane does not probe yet.* It has no `Loudness` job; ticket 14a
  calls `audio::chain::session()` before its first one.
- *The probe took 297 ms* on this host, once, with FFmpeg already read from
  disk.


2026-10-13 (ticket 12, implemented): Pitch and pitch-keeping speed are heard
through the voice chain and set from the inspector and the command panel,
tested in the core, `editor-media` and the desktop on macOS 26.5.2 (arm64,
Rust 1.97.1, Homebrew FFmpeg 8.1.2). No device was opened, no native capture
was taken and nothing was listened to. Details are in the ticket. Settled by
building it:

- *Where the rule lives.* `editor_core::sound` gained `pitch(clip)` (the
  first enabled `Pitch`), `pitch_ratio`, `PITCH_KEPT` (0.5..=2) and
  `pitch_kept`, beside `steady_speed`; the chain's `speed_stage` and the
  inspector both read them. No type, field or command changed.
- *Near 1 the stage goes.* An `atempo` within 0.001 of 1 is left out and
  `k` is the clip speed, not `P`: the length stays exact and the pitch is
  off by under 2 cents.
- *A `Pitch` the probe found missing is not read.* The clip plays as
  without one, speed changing pitch, and that is said once.
- *The switch is in the Playback card.* An audio-track clip has no Speed
  section; the card with its speed row is the constant-speed card. A
  shifted `Pitch` holds the switch on and disabled. The note gives the
  range for the shift: "Pitch is kept from 0.5× to 2×." at 0, "1× to 4×"
  an octave up. A picture clip whose sound its linked audio clip owns has
  no "Detach audio to keep pitch" line.
- *Off leaves no `Pitch` to be read.* Turning "Keep pitch" off removes
  every enabled `Pitch`, all of which must be at 0; a later one that shifts
  is refused in words rather than promoted to the one read. Disabled ones
  stay. (Review fix.)
- *Phrases set the shift.* `pitch up 2` twice is +2, not +4; `pitch down
  0` is keep pitch. They act on one selected clip, as `set speed` does.
- *Agents are told.* The planner text says an enabled `pitch`, 0 included,
  makes the clip's speed keep its pitch, and `docs/agent-protocol.md` says
  audio effects are heard and how keep pitch is written. (Review fix.)
- *Measured lead.* 12.0 to 18.5 ms at 2, 8.0 to 16.0 at 1.5, 27.5 to 28.4
  at 0.84 and 40.0 to 58.5 at 0.5, for bursts at 1, 10 and 30 s of source;
  none late. "About 13 ms at 2" above is the low end of that.

2026-10-13 (ticket 13, implemented): The Audio section, the add menu with
Voice and 3-band EQ, the library's Audio heading and five phrases are built
and tested headless in the desktop crate on macOS 26.5.2 (arm64, Rust
1.97.1). No native capture was taken (none of the states is reachable
without a pointer) and nothing was listened to. Details are in the ticket.
Settled by building it:

- *Where the rules live.* `src/audio.rs` holds the presets, the kinds a
  person adds (`added()`: the audio kinds less `Pitch`), the refusals and
  the noise rule, for the inspector, the library and the panel alike, as
  `src/speed.rs` does for pitch. No type, field or command changed.
- *Refusals in words.* A picture with sound: "Detach audio first: audio
  effects go on clips on audio tracks." A picture whose sound its linked
  audio clip already owns: "Sound is on the linked audio clip: give it
  audio effects there." (fixed in review; it was told to detach again). A
  picture without: "Only a sound on an audio track takes an audio effect."
  A preset past sixteen says how
  many it adds and how much room there is. Of several selected clips, those
  that take it are given it and the rest passed over (ticket 07's rule).
- *The library lists kinds, not presets,* and leaves `Pitch` out, as the
  add menu does: its home is Keep pitch. `add effect` and `remove effect`
  now name the audio kinds, which lifts the ticket 10 limit that a person
  could not remove one an agent wrote.
- *Names.* The section is "Audio" (no section control, it is alone), its
  heading "Audio effects", the library heading "Audio". Centre reduction's
  card is "Reduce centre" and its row "Amount": the ticket's "Reduce
  centre" for the row gave two controls one name. The switch is "Hear
  <name>".
- *Phrases.* `add eq` is the 3-band EQ; `reduce noise 60%` sets the first
  enabled `Denoise` or adds one at the default floor, 40% when no number is
  given.
- *Unavailable kinds are never refused.* The add menu, Voice, a library
  tile, a phrase and an agent's command all add one alike (`editor-agent`
  cannot see the probe, and a project is not held to one machine's
  FFmpeg); once added its card says "Not available in this FFmpeg build"
  and disables its rows, and its switch, moves and removal still work.
  `None` from the probe and an empty answer disable nothing. (Review
  changed this: the menu row alone had been disabled, so the routes
  disagreed.)

2026-10-13 (ticket 14a, implemented): The `Loudness` job and the stderr
reader are built and tested in `editor-media` on macOS 26.5.2 (arm64, Rust
1.97.1, Homebrew FFmpeg 8.1.2). Nothing in the desktop asks for a
measurement yet and nothing was listened to. Details are in the ticket.
Settled by building it:

- *Names.* `SoundService::loudness(LoudnessJob { project, asset, from,
  span, effects })` answers `SoundEvent::Measured { …, result:
  Result<Loudness, String> }`, with `Loudness::{Measured { integrated,
  true_peak }, Silent, TooShort}` and `LOUDNESS_FLOOR` (-70). The job is
  not named `Loudness`, as the peaks job is `PeakJob`.
- *The effects run at 48 kHz.* Ticket 11 holds a filter's frequency under
  0.45 of the rate its chain runs at, so the measurement is not without a
  rate: its effects (`chain::stack_effects`, the effects part of a stack)
  follow an `aresample=48000` and are written for two channels. With no
  effect heard the graph is the meter alone, behind the sum below.
- *Measured as stereo.* A voice is summed to the output's channels after its
  chain (`-ac`), so the meter is behind `aformat=channel_layouts=stereo`:
  a 5.1 or mono source reads as a two-channel output plays it (a 5.1 tone at
  -6 dBFS in left, right and centre: -1.3 LUFS and -1.4 dBFS, as a voice's
  own output, against -4.2 and -6.0 in its own layout). A device of another
  channel count hears other figures.
- *Refusals.* No sound, no length, an empty span, a span that starts at or
  after the media's end, more than 16 effects. A span that runs past the
  end is measured as far as the media goes.
- *Every tool's error waits for its tail,* at most 250 ms once the process
  has exited: `Process::failure` reads `stderr_tail`.
- *Cost.* Nearly all of it is the true peak, and a low rate is not cheaper
  for it: twenty minutes of mono 8 kHz took 2.50 s through `ffmpeg` (0.19 s
  without `peak=true`), ten of stereo 48 kHz noise 2.77 s.

2026-10-13 (ticket 14b, implemented): Normalize loudness is built and
tested headless in the desktop crate on macOS 26.5.2 (arm64, Rust 1.97.1,
Homebrew FFmpeg 8.1.2), through a stand-in lane and once through the real
sound lane; no native capture was taken and nothing was listened to.
Details are in the ticket. Settled by building it:

- *Owner decision 6, by default.* The owner has not answered: one constant,
  -14 LUFS with a -1 dBTP ceiling, is built (`normalize::TARGET`,
  `CEILING`).
- *One span, one request.* Clips of one media over the same source span
  have one lane target, so the ticket's "their targets differ" does not
  hold for them: such clips with the same effects share one measurement,
  and one with other effects waits for that answer before it is sent. A
  full lane holds clips until it has room. At most 64 clips wait at once.
- *A key cannot pass 8 by this rule:* `gain` is held to 8 and no key is
  above the highest, so the refusal the ticket names is a guard, tested
  with a volume above 8 handed in directly.
- *Refusals.* `sounds` as the ticket says, with the reason named (muted,
  soloed out, a track or clip at 0%), and a locked track, before any
  request and again when the figures come.
- *Measured.* A mono 1 kHz tone at -20 dBFS through the lane read -23.0
  LUFS, peak -22.8 dBFS, and was set to 282% (+9.0 dB): the meter hears
  one channel spread over two, as ticket 14a built.

2026-10-14 (ticket 14b, review fixes): Normalize asks `refusal` again when
the figures come, not only the lock: a picture clip whose audio is detached
while it is measured, or a clip no longer heard, is not written to. A
clip's measurement, and whether an answer is stale, rest on its enabled
audio effects alone, so a picture effect changed meanwhile does not throw
the answer away. Each clip over the 64-clip bound is passed over and
counted. Details in the ticket.

2026-10-13 (ticket 15, implemented): Duck under speech is built and tested
headless on macOS 26.5.2 (arm64, Rust 1.97.1, Homebrew FFmpeg 8.1.2),
through a stand-in lane and once through the real sound lane; no native
capture was taken and nothing was listened to. Details are in the ticket.
Settled by building it:

- *Owner decision 7, by default.* The owner has not answered: ducking
  replaces a clip's volume keys after asking ("Replace this clip's volume
  keyframes?", or "… of N clips?"), and Undo brings them back.
- *The level a dip comes back to* is the clip's volume, or its loudest
  volume key when it has keys, since those keys are replaced; a clip
  whose keys are all at 0 is refused in words.
- *Core names.* `sound::duck` returns `Duck { keys, dips, merged }`;
  `duck_keys` is its keys. Spans whose ramps would cross
  (`next.from - attack < prev.to + release`) are one dip; past 256 keys
  the dips with the shortest gap are merged first, and `merge_closest`
  bounds an asset's raw spans to 4,096 the same way.
- *One media file at a time through the held peaks.* `HeldPeaks` has a
  work route: `want_work` asks for a file whatever the room, a work entry
  is never given up for want of a tile and never evicted, and it may take
  the room of on-screen peaks (least lately wanted first) when it comes;
  `failed` says it could not be read; `let_go` forgets it unless a tile
  shows it or it failed (a failure is remembered, as for a tile). The
  voices are the clips not selected, so a selected clip that is passed
  over is no voice; a duck asks the lane from the logic pass, so it reads
  on while the window is hidden (review, 2026-10-14). Without that exception a duck could wait for ever beside
  four long files on screen.
- *A project change* is any change of the editor's instance or revision
  while the sound is read; the run is given up, its read cancelled, and
  nothing written.
- *Routes and depth.* The inspector's Ducking card sits in the Audio
  section (`src/ui/inspector/audio.rs`, not `clip.rs`, where that section
  now lives), with a Depth field in dB (1 to 60, -12 by default) and a
  progress bar with Stop ducking while it reads. The clip menu ducks at
  the default depth; `duck`, `duck under speech` and `duck 18 db` in the
  command panel.

2026-10-13 (ticket 16a, implemented): `beats()` and the beats job are built
and tested in `editor-media` on macOS 26.5.2 (arm64, Rust 1.97.1, Homebrew
FFmpeg 8.1.2), over synthetic peaks and two FFmpeg click tracks; nothing in
the desktop asks for beats yet, no real music was tried and nothing was
listened to. Details are in the ticket. Settled by building it:

- *Names.* `SoundService::beats(PeakJob)` answers `SoundEvent::Beats { …,
  result: Result<Option<BeatGrid>, String> }`; `Ok(None)` is sound with no
  clear period, apart from the errors. `BeatGrid { bpm, beats }`,
  `MAX_BEATS` and the pure `beats(&Peaks)` are exported. A beat is the
  middle of its onset's bucket, in milliseconds of container time.
- *Where it runs.* On the cache-reading thread, in the `PEAK_LOADS` queue
  beside kept-peak reads, with targets (`Kept`, asset) and (`Beats`,
  asset); a newer request for a file's beats gives up the one waiting and
  the one under way, which runs to its end and says nothing. A kept-peak
  read waits behind it: 0.49 s at the bucket bound on the `dev` test
  profile, 4.11 s on `ci`.
- *Missing or stale peaks* are errors in words; the job never reads sound.
- *When a period is clear:* a correlation of at least 0.1 of the
  envelope's power and four standard errors of noise, with four periods in
  the file at least. Chosen, not tuned on music.

2026-10-14 (ticket 16b, implemented): Beat ticks, snapping to beats, Show
and Hide beats, Markers from beats and the phrases are built and tested
headless in the desktop crate on macOS 26.5.2 (arm64, Rust 1.97.1, Homebrew
FFmpeg 8.1.2), through stand-in grids and once through the real lane; no
native capture was taken (showing beats needs the clip menu or the panel,
and a snap guide a held drag), no real music was tried and nothing was
listened to. Details are in the ticket. Settled by building it:

- *Names.* `HeldBeats` in `src/peaks.rs`, held as `HeldPeaks::beats`, with
  `MAX_HELD_GRIDS` (8); `beats_through(clip, grid, from, to)` puts a grid
  on the timeline and `beat_markers` makes the step, "Markers from beats",
  of markers labelled "Beat". `Action::ShowBeats { assets, shown }` and
  `Action::MarkersFromBeats(ClipId)`. The phrases are `show beats`, `hide
  beats` and `markers from beats` (or `mark beats`).
- *Heard means `audible_at`, whole.* Ticks, snap candidates and markers are
  only where the clip is heard, as this file has beat ticks ask the mixer's
  function: a muted, soloed-out or 0% clip shows none, besides none in a
  freeze, a reverse or a ramp. The waveform under them is drawn regardless.
- *Only a move leaves out beats.* A move leaves out the beats of every clip
  that moves with it; a trim, a fade handle and a transition handle land on
  their own clip's beats, which stay where they are.
- *At most 256, for any clip.* Past every fourth beat the step goes on
  halving (every eighth, sixteenth…). A beat that already has a marker at
  that time is passed over; when every one has, nothing is written.
- *A grid is asked for once the peaks are held,* which stands for "the peak
  file exists": peaks read and not kept by the cache answer in words. None
  is asked while eight grids are on screen or on their way, so a ninth
  tile in view waits rather than having the lane find beats without end.
  One held back while a grid that just left the view is still counted
  brings about the next frame itself, as peaks and posters do (review,
  2026-10-14).
- *Too close drops both.* A tick is drawn and snapped to only when it is at
  least 4 points from each neighbour, so ticks do not shift as the view
  scrolls.
- *Shown survives a relink, not a project replace.* Shown again, beats that
  were unclear or not found are asked for anew.

2026-10-13 (ticket 20, implemented): Save audio as… is built and tested
headless in `editor-media` and the desktop crate on macOS 26.5.2 (arm64,
Rust 1.97.1, Homebrew FFmpeg 8.1.2), through FFmpeg fixtures, a stand-in
lane and real egui passes; no native capture was taken and nothing was
listened to. Details are in the ticket. Settled by building it:

- *Owner decision 9 taken as yes by default.* The feature is built; Detach
  audio is unchanged.
- *A file being written outlives a project replace.* `cancel_all` keeps a
  waiting or running `Extract`, its progress and its answer: the file is
  the person's and asked of no project. The answer is said with the whole
  path and nothing is offered to the project that came after. This
  narrows "`cancel_all` on project replace" in the lane above.
- *A file that has claimed its rename is not given up.* The job claims its
  last step under the lane's lock once FFmpeg has ended well;
  `SoundService::cancel` now answers `Cancel`: `Finishing` for a claimed
  file (its answer comes, written or not), `Written` when its `Ok` answer
  waits, else `GivenUp`. `extract` names the requests it actually gave up
  (`Extracting::replaced`), so a claimed older file keeps its card.
  (Amended 2026-10-14 after review.)
- *The `.part` is the job's own.* It is made with `create_new` before
  FFmpeg starts and removed only after FFmpeg has been waited for; a
  `.part` already there is refused and left. The source and the project's
  other media (`ExtractJob::keep`) are refused by path and as the same file
  on disk (device and inode on Unix, the resolved path elsewhere), and a
  folder is refused. The part is flushed through a handle opened for
  writing, as Windows asks.
- *The command* is the ticket's with `-nostats -threads 1`; a whole file
  has no `-ss`/`-t`. A run that reads no sound is an error.
- *Routes.* The library's media menu, the clip menu and the project menu's
  Clip section (one selected clip). No agent command and no panel phrase,
  by principle 8. The written file enters the project only by Add to
  project, the ordinary import.

2026-10-14 (ticket 17, decided on the owner's behalf): Owner decision 5 is
taken, the owner having delegated it. Recordings live in `<document stem>
media/` beside the saved document, made on the first take and never
emptied, moved or deleted by the editor; recording in an untitled project
is refused with "Save the project to record" (amended after review,
below). Paths stay absolute: no store format change, so a take does not
move with the project, and relative paths are a follow-up. Built in
`editor-store` and tested there, on macOS 26.5.2 (arm64, Rust 1.97.1);
nothing in the desktop calls it yet. Settled by building it:

- *Names.* `editor_store::media_folder(document)` and `new_take(document,
  now)`, which makes the folder and returns a free path, `Voiceover
  2026-10-14 15.30.12 UTC.wav`, then ` 2`, ` 3`…; the writer opens it with
  `create_new`. The stamp is UTC and says so, as no time zone database is in
  the workspace.
- *Bounds for ticket 18.* A take ends itself at 2 hours or at the WAV size
  limit, whichever comes first. No free-space check before starting: the
  standard library has none and adding a platform crate to `editor-media`
  would change a manifest. A failed write, a full disk included, stops the
  take and keeps what was written.
- *Save as and Duplicate* leave takes where they are; nothing is copied or
  offered. The folder also holds ticket 21a's stems; ticket 20's "Save audio
  as…" still writes where the person picks.

2026-10-14 (ticket 17, review): Four rules changed. The untitled-project
rule had become "save, then record by itself", which is not the proposed
default the run was to use; it is the refusal again (a Save button beside
it only saves), and the alternative is open in owner decision 5. A name
held by a dangling link is taken (`new_take` used `Path::exists`, which
follows links, and `create_new` then refused the name); a name that cannot
be looked at is an error, not free. The WAV bound is data bytes ≤
`u32::MAX − 36` rounded down to whole frames, not 4 GiB, which wraps the
header's sizes. `new_take` is filesystem work and runs off the frame loop.

2026-10-13 (ticket 18, implemented): `RecordService` is built and tested
headless in `editor-media` on macOS 26.5.2 (arm64, Rust 1.97.1, Homebrew
FFmpeg 8.1.2), with a fake source that calls the callback's own function;
no microphone was opened and nothing was listened to. Details are in the
ticket. Settled by building it:

- *Nothing waits on the caller.* `start` only checks the path has a file
  name and starts `neptune-video-record`; that thread finds the device, builds the
  stream, plays it, and then makes the file with `create_new`. No
  microphone is `RecordEvent::Failed(MediaError::NoMicrophone)` ("No
  microphone") through `poll`, not `start`'s `Err`, and
  `RecordEvent::Started { rate, channels }` is added. `frames()` and
  `rate()` are 0 until the file is made, just before `Started`; `frames()`
  then counts from when the device began to play, so it may be past 0 when
  `Started` is read (corrected after review: this first said 0 until
  `Started`). For ticket 19 this means playback starts on `Started`, and
  the anchor is the pair of `frames()` and the device position read on one
  tick, never `Started`'s arrival as frame 0.
- *No file before the device plays.* A device that will not open or play,
  too many channels or a file at the path answer `Failed`, before any
  `Started`, and leave nothing in the media folder; a file there is refused
  and left as it was (corrected after review: the file was first made
  before `play`, so a refused `play` left an empty WAV and a `Finished`
  take that never started). The one exception is a header that cannot be
  written into the file just made: `Failed`, and that file, holding no
  sound, stays.
- *A take says why it ended.* `Take::ended` is the device's error, a failed
  write or the limit, `None` when stop was asked; a later reason is added
  after an earlier one, not put in its place. After a failed write the
  file is cut to the samples counted, so its length is `Take::bytes`. Frames after `stop` and
  past the limit are not taken and not counted dropped.
- *Checkpoints.* The sizes are rewritten and synced after a second of sound
  written as well as after a second of the clock, so a backlog drained at
  once is not left uncounted.
- *Size.* Stereo 48 kHz float is 1.38 GB an hour, not 690 MB (mono's).

2026-10-13 (ticket 19, implemented): The record control, R, the take's
clock and its placement are built and tested headless in the desktop crate
on macOS 26.5.2 (arm64, Rust 1.97.1), with a stand-in `RecordService`, a
stand-in clock and real egui passes; no microphone was opened, no native
capture was taken and nothing was listened to. Details are in the ticket.
Settled by building it:

- *A take plays the timeline at 1×.* Speed is set to 1× on `Started` and
  cannot be changed until the take ends, so a clip is as long on the
  timeline as it was spoken.
- *The playhead stays with the take.* While a take is asked for, running or
  closing, Seek, Step, the jumps, speed and the source monitor are refused
  in words; Space and Escape (after a held drag) end the take, as R and the
  control do; play and pause by any other route (the play button, K, L, a
  double-click, the panel) are refused in words and do not end it. Record
  pressed while the timeline plays pauses it, so the playhead holds until
  `Started` (corrected after review); edits and undo are allowed, and the
  free track is looked for when the take lands.
- *A replaced project stops the take.* Its file is kept and named and
  nothing is added, as the ticket says; the take does not run on into the
  next project. Closing the window closes the take and places nothing.
- *A take stopped before its anchor* is anchored at the playhead then; one
  given up before `Started` places nothing.
- *The control* sits just before speed on the timeline's transport at every
  width; a running take draws stop, elapsed time and level over record and
  speed. The source monitor has no record control. The optional Save button
  beside "Save the project to record" was not built.

2026-10-14 (ticket 21a, decided on the owner's behalf): Owner decision 10
is taken, the owner having delegated it. The stem separator is the Demucs
command line, which the person installs: `adefossez/demucs`, the maintained
fork of the archived `facebookresearch/demucs`, 4.1.0 on PyPI (11 July
2026). Its code is MIT (the `LICENSE` text was read); no licence text for
its pretrained weights was found, and its author wrote in 2023 that they are
"provided for research purpose", so their status is taken as unsettled and
`docs/installation.md` says so. It passes because the editor bundles,
fetches and recommends no weights for any use. Details in the ticket.

2026-10-13 (ticket 21a, implemented): Separate stems is built and tested
headless in `editor-media` and the desktop crate on macOS 26.5.2 (arm64,
Rust 1.97.1, Homebrew FFmpeg 8.1.2), with a shell-script stand-in for the
tool, and once through the lane with Demucs 4.1.0 over 20 s of FFmpeg tones;
no native capture was taken and nothing was listened to. Details are in the
ticket. Settled by building it:

- *Names.* `Separator` (made only by `Separator::probe`), `SeparateJob {
  project, asset, to, separator }`, `look_for_separator`, `stem_separator`,
  `separator_absent`, `STEMS` (`vocals` as Vocals, `no_vocals` as
  Instruments), `SoundService::separate` answering `Extracting`, and
  `SoundEvent::Separated`. The desktop's rules are in `src/stems.rs`.
- *The probe is `--help`,* which must end well within 60 s and name
  `--two-stems`: Demucs has no version flag. A missing tool is said only
  where Separate stems is asked; one that is there and fails is said once,
  by its last line.
- *The part folder is `<media> (stems).part` inside the media folder,* not
  `<to>.part` beside it: a leftover blocks one media file, not every one,
  and the stems move by a link within one folder. A stem never replaces a
  file: the next free name, ` 2`, ` 3`…, is taken (a dangling link counts
  as taken).
- *What the stems' clips carry.* Besides what the ticket lists, the
  source's fades, audio effects and group, so what was heard is heard from
  the stems; a picture's effects stay on the picture. A stem shorter than
  the span the clip plays cuts a clip of one speed short and refuses a
  clip with a time map, in words, with the files named. Each stem's track
  takes the volume and solo of the source clip's track (review,
  2026-10-14), and stems are not placed for a clip that is no longer the
  one heard when they come: its track muted, or its sound detached to a
  linked audio clip meanwhile; the files are named.
- *A separation outlives a project replace,* as a saved file does:
  `cancel_all` spares it, and its stems are named, not placed.
- *Progress* is the tool's tqdm figure from the tail of its error pipe;
  until it says one the card shows "…" and its bar has no value, and the
  window does not animate. The whole run is held to 15 minutes plus four
  times the media's length.
- *Measured once.* 20 s of tones through the lane in 5.65 to 6.04 s with
  the model held, 15.72 s the run that fetched it; 180 s alone in 26.7 s;
  Apple M1, the tool's default device. Not music, not speech.


2026-10-13 (ticket 05b, recorded by ticket 22): One rule changed in ticket
05b's review and reached no entry here. The checkpoint list reads each
checkpoint through the same version gate as every other entry before it
parses the body, so a version 4 checkpoint that holds a kind this build does
not know is reported as a future file, not a damaged one; and a version 0 or
unversioned checkpoint is now refused by the list (`Invalid`), where one
whose body parsed used to be listed and was refused only when opened. No
file is modified either way.

2026-10-14 (owner decisions, decided on the owner's behalf): The owner
delegated every open decision of the epic. Each is marked under Owner
decisions above, and the build and the documents follow it:

- *1.* Instructions under 6 KiB, schema under 23 KiB, as ticket 10 built.
  Measured on 2026-10-14 by the planner tests: 5,377 and 21,859 bytes.
- *2, 3, 4, 6, 7, 8, 9, 12.* The defaults the tickets built stand: audio
  effects on audio-track clips only; speed changes pitch unless the clip has
  an enabled `Pitch`; solo durable and in the history; -14 LUFS with a
  -1 dBTP ceiling; ducking replaces a clip's volume keys after asking;
  centre reduction kept; Save audio as… built; a 0 dBFS master ceiling.
- *5.* Its last open alternative is closed: an untitled project is refused
  ("Save the project to record") and is never saved and then recorded by
  itself. The optional Save button beside the words stays unbuilt.
- *10.* Demucs, as recorded on 2026-10-14 under ticket 21a.
- *11.* Neural denoise and speech detection are not built; `afftdn` and the
  energy gate stay, and tickets 21b and 21c stay open research with the
  decision written in each.
- *13.* No stock library: the editor ships, fetches and generates no sound.

Ticket 22 wrote ADR 0008 and brought the glossary, usage, design,
architecture, performance, verification, agent-protocol, native-AI and
installation documents, and ADR 0006's caps, to the code as it stands. Where
this file's body is now older than the code, the dated entries above say
what was built; besides those, "Not claimed" still lists that the schema and
instructions fit their caps, which ticket 10 measured and the owner-decision
entry above settles. Ticket 01, whose only open item was decision 1, is
resolved.

2026-10-14 (fixes after native acceptance, decided on the owner's behalf):
The two defects native acceptance left are fixed.

- *The held volume percent* was drawn at the tile's left start
  (`paint_volume`, `first.x + 6`) and overlapped a fade in's curve and
  handle (`normal-05`, `narrow-05`). It is now said beside the point the
  line was taken hold of (`Drag::Volume` keeps that time as `at`), after it
  or before it where the room after is too narrow, inside the tile's sound
  and clear of the grips and of both fades' curves and handles
  (`volume_readout` in `src/ui/timeline.rs`, gap `READOUT_GAP` 6 points).
  Taken over a fade, it is said just past that fade. Decided: where the
  fades leave no room between them, it is said at the trailing end of the
  name's row in a pill like the speed badge, and the name gives way. A tile
  with no name's row (no audio lane is that short: they are 44 points) or
  too narrow for the pill (a few points over the 40 a line takes) then says
  nothing; the inspector's Volume field says the volume once let go. `docs/design.md` says so. Test:
  `the_held_percent_is_said_beside_the_point_taken_and_clear_of_the_fades`
  (`src/ui/volume_tests.rs`) reads the painted text and the fade meshes of
  a real egui pass, over the fade in, between the fades, over the fade out,
  and with fades that meet.
- *A right-click on a library tile's play offer* opened nothing, because the
  offer is its own widget over the tile. A secondary click on it now opens
  the tile's own menu (the tile's response joined with the offer's, so the
  menu, its id and its rows are the tile's); a click still previews and
  opens no menu (`src/ui/library.rs`). Test:
  `a_secondary_click_on_the_play_offer_opens_the_tiles_menu_and_a_click_plays`,
  which fails with the fix taken out.

Checked on macOS 26.5.2 (Apple M1, Rust 1.97.1): `cargo fmt --all --
--check`; `cargo clippy --locked -p neptune-video --all-targets -- -D
warnings`; `cargo test --locked -p neptune-video`, 832 and 12 passed;
`cargo build --release --locked`, `target/release/neptune-video` sha256
`64a0360878cbccb5397168452b50edc666b8f76a8cd03996c78a429fdd56c038`. That
build was relaunched on fresh copies of the acceptance's task-owned project
with fresh `--settings`, driven the same way at 1280x709 and 760x620
points, and captured: `captures/fix-{normal,narrow}-02` the percent held
over the fade in, said just past the fade's handle; `-03` held between the
fades, beside the point; `-04` the play offer under the pointer; `-05` the
tile's menu opened by a right-click on it. Every launched process was
stopped. The name's-row case and the primary click that plays were not
captured natively (the latter would open the output device); both are
tested headless. Ticket 03 was read against the code on the owner's behalf
and resolved (see its Comments).

2026-10-14 (final validation, follow-up fix): Review of the percent fix
found the name's-row pill could still be overlapped: on a tile of about 44
to 50 points the name, truncated to no width, still painted its ellipsis
over the pill. Decided on the owner's behalf: while the held percent is in
the name's row, a name that would not fit beside it is not drawn at all
(`paint_clip`, `said_in_row`). Test:
`on_a_tile_too_narrow_for_its_name_beside_it_the_held_percent_is_said_alone`
(`src/ui/volume_tests.rs`), which fails with the guard taken out. Not
captured natively; the release build named above predates it.

2026-10-14: Merged with titles and captions, which reached main first.
Decided on the owner's behalf. The project file is schema version 4 (titles
took 3): read 1 to 4, write 4, and version 5 is the future refused
everywhere. This design's ADR is 0008 (titles took 0007). The planner text is
the titles text with this work's audio sentences and ticket 01's rewordings
applied to it, 5,793 bytes; the schema is 22,062 bytes. Both caps stay (6 KiB
and 23 KiB), and the titles work's Windows bound also holds: the escaped
schema and instructions stay under 31 KiB. To fit it, the schema no longer
carries descriptions for fades, track volume and solo (the planner text
states them; the Rust comments remain). The icons moved to a Tabler font
subset: track solo uses its `Solo` glyph and the record button its microphone
(`Speech`), red as before; the hand-drawn headphones and record dot are gone.
A title look dropped on the timeline snaps to beats as media does. The tab
strip is the titles work's (icons when the names do not fit); the kind
filter keeps its own segmented control.
