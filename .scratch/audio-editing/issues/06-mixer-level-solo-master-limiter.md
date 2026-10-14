# Mixer: `level_at`, admission by `sounds`, master limiter

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 05a

## Scope

Make fades, track volume and solo heard, and stop loud mixes from
hard-clipping. All of it on the `aster-audio` worker; the CPAL callback is
not touched.

- `crates/editor-media/src/audio.rs`, the sum loop (`:1019-1032`): the level
  is `editor_core::level_at(track, clip, at)`. It is evaluated once per
  output frame when `level_varies(clip)`, and once per interval otherwise
  (track gain times clip gain), so a plain clip costs what it costs today.
- `crates/editor-media/src/audio/plan.rs`, `Plan::prepare` (`:130-157`):
  admission is `editor_core::sounds(project, track, clip)`, in place of the
  tests written out here today (mute, volume 0 without keys, the owner set,
  an asset without sound). That adds two skips: a track soloed out and a
  track whose `gain` is 0. The owner set built in `Mixer::new`
  (`audio.rs:836-843`), `gain_animated` and `straight` (`plan.rs:17-20`)
  are removed in favour of the core's; `audible_run` and `steady_speed`
  keep their shape over the core's segment test. Nothing else decides what
  sounds.
- Master: replace the clamp (`:1035-1037`) with a peak limiter. Per frame,
  `peak` is the largest absolute sample across channels; `target =
  min(1, ceiling / peak)` with `ceiling = 1.0`, the level the clamp cut at;
  the held gain drops to `target` at once and rises toward 1 with a 200 ms
  time constant; every channel of the frame is multiplied by it. While the
  held gain is exactly 1 the multiply is skipped, so a frame within ±1.0
  leaves as it does today. The gain is one `f32` on `Mixer`, reset in
  `clear()`. A clamp to ±1 stays after it for non-finite safety.
- The voice-limit message names the time: "Preview audio supports at most 16
  simultaneous audible clips (17 at 00:01:12); mute or solo tracks".

Files: `crates/editor-media/src/audio.rs`, `audio/plan.rs`, their tests,
`docs/architecture.md` (the sound paragraph), `docs/usage.md`.

## Result

Implemented in `editor-media`. Fades, track volume and solo are heard in the
preview and a sum past full scale is limited, not cut flat. Nothing was
listened to.

- `audio.rs`, the sum loop: the level is `sound::level_at(track, clip, at)`
  at every output frame when `sound::level_varies(clip)`, and
  `track.gain * clip.gain` once for the interval otherwise.
- `audio/plan.rs`, `Plan::prepare`: admission is `Sounding::sounds`, the
  core's `sounds` over a `Sounding` the mixer builds once from its snapshot
  (the 05a comment's advice). The tests written out there are gone, and with
  them `gain_animated`, `straight`, `steady_speed`, `audible_run` and
  `audio_owners`; the runs and the decoder speed come from
  `sound::audible_run` and `sound::steady_speed`. `Mixer.audio_owners` is
  replaced by `Mixer.sounding`.
- Master: `Mixer::master`, called where the clamp loop was. `MASTER_CEILING`
  is 1.0 and `LIMITER_RELEASE_SECONDS` 0.2; the gain is `Mixer.limit`, one
  `f32`, reset in `clear()`, and the per-frame rise is worked out once from
  the mixer's rate. A frame is untouched while the gain is 1 and its peak is
  within the ceiling; after a peak the frames that follow are turned down
  too, until the gain is 1 again. The clamp to ±1 follows.
- The voice-limit message: "Preview audio supports at most 16 simultaneous
  audible clips (17 at 00:01:12); mute or solo tracks". The count is the
  number of clips sounding on the first sample that has more than sixteen,
  and the time is that sample's, as hours, minutes and seconds.
- Routes. This ticket adds no route: a fade, a track volume and solo still
  enter only as the serialized commands of ticket 05a, and the mixer reads
  the project those commands leave. Shortcuts, menus, gestures and inspector
  rows are tickets 07 and 08a. Validation, reversal and save/load of the
  fields are 05a's and 05b's and were not touched; the mixer's own failure,
  the voice limit, is refused before any decoder is started or dropped, as
  before.
- `docs/architecture.md` (the sound paragraph) and `docs/usage.md`.

## Evidence

On macOS 26.5.2 (arm64), Rust 1.97.1, Homebrew FFmpeg 8.1.2 on `PATH`
(`scripts/check-media-tools.py` passed), 2026-10-13, `dev` profile.

`cargo test -p editor-media --locked`: 235 passed, none ignored, 13 of them
new. The readback tests are in the new
`crates/editor-media/src/audio/level_tests.rs`. Their sound is a 32-bit
float WAV the test writes sample by sample and FFmpeg decodes, not a `lavfi`
tone, so every expected sample is known exactly; a 997 Hz full-scale tone
where a tone is needed.

- Fade in of 100 ms: frame 0 is 0, the frame at 50 ms is the tone times
  `sin(π/4)`, every frame of the fade is on the curve within 1e-4 across
  four chunk edges, and frames past 100 ms are the tone bit for bit. Fade
  out: the mirror, the last frame all but silent, silence after the clip.
- Track volume 0.5 halves a clip exactly; with volume keys from 0 to 2 the
  product holds, and with a fade in as well.
- Solo: only the soloed track's samples and only its decoder (the audit's
  count equals the voices held); a soloed and muted track is silent and
  starts nothing; a soloed picture whose linked audio clip is soloed out is
  silent, and soloing the owner's track brings the sound back.
- Admission equals `sounds`: nine tracks (open, muted, soloed, at volume 0,
  a detached pair, a video without sound, a clip at volume 0, one at volume
  0 with keys) under five choices of soloed tracks; the clips whose decoders
  start are those for which `sound::sounds` is true, and both equal a list
  written out in the test. In `plan.rs`, without FFmpeg: a track soloed out
  or at volume 0 is not planned, and seventeen overlapping clips fit once
  one track is soloed.
- Limiter: two full-scale tones in phase never pass 1.0, and from the first
  peak on the output is within 0.01 of the single tone at every frame (the
  clamp left it flat at 1 for two thirds of each cycle); under a tenth of
  the samples sit on the ceiling. A full-scale single tone leaves bit for
  bit with the gain still 1. The gain is 1 after `clear()`. On `master`
  alone: a mix within ±1.0, the values 1.0 and -1.0 included, is unchanged
  bit for bit; the loudest channel sets the gain of the whole frame; a
  louder frame drops it further at once; after 200 ms the gain has covered
  `1 - 1/e` of the way back; it then reaches exactly 1 and stops touching
  the mix; an infinite sample is clamped and moves no gain.
- A project with no fades, unit track volume and no solo, and the source
  monitor's document (`Project::asset_preview`), each return the written
  tone bit for bit. The mixer tests that were there before pass unchanged
  but for the message's text.
- The message: 18 clips at 72.003 s read "(18 at 00:01:12)" from a window
  that starts on that second and from one that starts before it; an hour
  reads "01:02:03"; a window with more intervals than the scratch keeps
  still names its count.

Also run: `cargo clippy -p editor-media --all-targets --locked -- -D
warnings`, `cargo check --workspace --all-targets --locked`, `cargo fmt
--all -- --check` and `python3 scripts/check-architecture.py`: clean.

Not done: no native build was run, no device was opened and nothing was
listened to. The other crates' tests were not run again; none of their
sources changed.

## Known limits

- The limiter has no lookahead: it cannot overshoot, and it distorts hard
  transients that cross the ceiling. It is a safety stage, not mastering.
- It limits sample peaks, not true peaks.
- A mix that never passes ±1.0 is as it was. A sum past it was flat-topped
  and is now turned down, and so is what follows it, within the ceiling or
  not, until the gain is 1 again: 200 ms is the time constant, not the
  length. Counted in a test at 48 kHz, the gain is back at 1 after 1.42 s
  for a peak of twice the ceiling, 1.08 s for one 10% over and 0.64 s for
  one 1% over; after the peak of twice the ceiling it is more than 0.1 dB
  down for 0.76 s. Inter-sample peaks above 0 dBFS are not prevented; a
  -1 dBFS ceiling is owner decision 12.
- Sixteen voices remains the bound and the 17th still fails the whole mix.
- Audible quality and CPU cost are unmeasured; nothing was listened to.
- The gain's last stretch back to 1 is a straight line of one `f32` epsilon
  a frame, 200 ms long at 48 kHz as counted in a test (the floor takes over
  0.0011 below 1): the 200 ms curve's own steps fall under what an `f32`
  near 1 resolves, and without the floor the gain stopped at 0.9997 and
  never let go.
- A frame holding a sample that is not finite is not limited; the clamp
  takes it. A NaN in the sum stays a NaN, as before.
- The message's time is to the second and its count is taken on the first
  crowded sample of the chunk, not the worst one.
- A clip whose level is fixed costs one multiply a sample, as before; a
  clip with a fade pays `level_at` at every frame for its whole length, not
  only inside the fade.
- Ownership is by link and media, as the core defines it. Ownership by
  span, which the spec's ticket 02 comment left to this ticket, is not
  added: a picture linked to an audio clip of its media is silent over its
  whole length, whatever the two clips' spans.
- Verified on one macOS host only.

## Comments

2026-10-13: Written with the spec. Nothing implemented or measured.

2026-10-13 (review): The ceiling was -1 dBFS (0.891). The stage it replaces
is `sample.clamp(-1.0, 1.0)` (`audio.rs:1035-1037`), an identity within
±1.0, and the same `Mixer` serves the source monitor (`app.rs:1051-1061`):
a lower ceiling would have turned down every mastered music file on each
peak, against the spec's "exactly as before". Admission now goes through
the core's `sounds`, so the mixer and the UI cannot disagree.

2026-10-13: Implemented as above. Owner decision 12 is not answered; the
spec's default, a ceiling of 0 dBFS, is what is built. The spec gained an
entry for what building it settled.

2026-10-13 (review of the implementation): `docs/architecture.md` and
`docs/usage.md` said a mix within the ceiling is not altered. That holds
only while the gain is 1: after a peak the frames that follow are turned
down too. Both are reworded, and so is the comment on `Mixer::master`. The
review proposed "roughly a quarter second" for how long; counting frames
showed that wrong as well, and the "about 50 ms" tail written here before
with it. The tail is 200 ms and the gain is back at 1 only 1.42 s after a
peak of twice the ceiling. The figures are in Known limits, and
`the_master_turns_down_what_follows_a_peak_until_the_gain_is_back` holds
them. No rule and no code of the limiter changed. Whether so long a
recovery is wanted is a matter for listening, which has not been done.
With the new test `cargo test -p editor-media --locked` has 236 tests. Of
four full runs after the change three passed all 236 and one failed
`sound::tests::a_reading_given_up_or_replaced_answers_nothing`, a test of
the waveform reader outside this ticket, which then passed six times alone;
its cause was not looked into.
