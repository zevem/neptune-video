---
status: accepted
---

# Sound is levels in the mixer and filters in the decoder

Waveforms, fades, track volume and solo, detached audio, background music,
voiceover recording, audio effects, pitch-keeping speed, loudness
normalization, ducking, beats and stem separation are added without a new
audio engine. The contracts of [ADR 0006](0006-advanced-editing-is-generic-data-resolved-once.md)
hold: every edit is a validated reversible command, the core knows no codec
or filesystem, workers are bounded and stale-safe, and the device callback
only copies.

**Two places shape sound.** Whatever is a level is arithmetic in the mixer's
sum loop on the `neptune-video-audio` worker: clip volume and its keyframes, the edge
fades, track volume, and the master limiter that replaces the hard clamp.
Whatever is spectral or dynamic is an FFmpeg filter in the `-af` of the
clip's own decoder process, which already existed: high-pass, equalizer
band, compressor, limiter, noise reduction (`afftdn`), pitch (one `atempo`
stage beside `asetrate`) and centre reduction (`pan`).
[`audio/chain.rs`](../../crates/editor-media/src/audio/chain.rs) is the one
place filter syntax is written, and a clip without audio effects gets the
string it always had. No DSP crate was added, nothing runs in the callback,
and no second process is started for a clip. The master limiter's ceiling
is 0 dBFS, the level the clamp cut at, so a mix that never passed full scale
plays bit for bit as before.

**Audio effects are rows of the one effect table.** Seven kinds join the flat
`EffectKind` with rows in [`look::spec`](../../crates/editor-core/src/look.rs)
built from parameter keys that already existed; `SetEffects` carries them.
They are legal only on clips on audio tracks, at `mix` 1, without points, an
asset or keyframes, because FFmpeg reads a filter's values once, when the
decoder starts; a picture clip is one Detach audio away. Loosening either rule
later invalidates no file. Presets (Voice, 3-band EQ) write ordinary effects
and are not stored. "Audio effect" names these kinds; "sound effect" stays
the name of an imported sound clip.

**Fades are edge fields, not keyframes.** `fade_in` and `fade_out` are
timeline lengths held to the clip's edges by trim, split, rate and retime,
with one equal-power curve, set by the one new command, `SetFades`. A
keyframed fade would be cropped away from the edge by the next split or
trim. Track `gain` and `solo` are track fields beside `muted`, set by
`SetTrackState`; solo is durable project state, so that what is heard is what
the project says, and enters undo history.

**One definition of what sounds, in core.** [`editor_core::sound`](../../crates/editor-core/src/sound.rs)
answers whether a clip is in the mix (`sounds`), heard at a time
(`audible_at`), how loud (`level_at`) and whether its speed keeps its pitch
(`pitch_kept`). Link ownership and the straight-segment test of a time map
moved there from `editor-media`. The mixer's admission, the inspector's rows,
the track head's dimmed mute, Normalize, the ducking gate and beat ticks all
ask it, so the interface never offers a control the mix does not hear.

**Derived sound data stays outside the project.** Peaks are one file per media
path in a bounded cache under the user's cache directory, read by a sound lane
of its own beside the analysis lane, and feed the waveform, the ducking gate
and the beat grid. Loudness is measured when asked and kept nowhere; the beat
grid lives in memory. The project holds only what the person decided: a
volume, keyframes, markers.

**Analysis and recording are not commands.** A person starts a measurement, a
duck, a beat search, a recording, a saved file or a separation; their results
enter as ordinary commands (`SetGain`, `SetKeyframes`, `AddMarker`, a `Batch`
of `AddAssets`, `AddTrack`, `AddClip` and `LinkClips`) through the reducer,
one history entry each, after their request, project and media have been
checked. An agent can write any of those results and can start none of the
work. A take and stems are the person's files in `<document stem> media/`
beside the saved document; the editor never moves or deletes them, also not on
undo, and paths stay absolute.

**Filters are probed by running them.** FFmpeg stays an external host tool
with no minimum version. Once a session each audio kind's own string at its
defaults runs over 50 ms of silence after a control run; a kind whose run
fails is left out of every chain, said once and shown as unavailable, but is
never refused as data, since a project is not held to one machine's FFmpeg.
The stem separator (the Demucs command line) is found and probed the same
way; the editor ships no model and fetches nothing.

**Set aside.** An in-process DSP graph (five crates and a per-track bus) would
have given live parameter changes and keyframed effect parameters, neither of
which was asked for. A live sidechain would have made ducking invisible and
unrepeatable; written `Gain` keyframes can be seen, edited and undone. A
parallel `audio_effects` field or a struct per effect would have doubled the
schema, the inspector and the validation that the effect table already gives.
Baked audio files (rendered noise reduction, pitch, normalization) would have
been derived media to store, evict and keep in step with every edit; the only
sound files the editor writes are a person's copy (Save audio as…), takes and
stems.
Parsing `ffmpeg -filters` was set aside because a filter that is listed can
still refuse an option.

The file is schema version 4: the new fields read through serde defaults, and
a build that knows only version 3 or older refuses a file that may hold an
audio kind through its future-schema protection instead of reading it as
damaged. Versions 1 to 3 are read and never written. (This work was built as
version 3; titles and captions, [ADR 0007](0007-titles-and-captions-are-clips-drawn-off-the-frame.md),
reached main first with that number, so sound took 4 when the two were merged
on 2026-10-14.)

The binding rules, the owner decisions taken on the owner's behalf and the
dated changes made during implementation are in
[the audio-editing design](../../.scratch/audio-editing/spec.md). Everything
here is preview: there is no export, and nothing has been listened to.
(2026-10-14: an export now renders this mix offline through the same mixer;
see [ADR 0009](0009-export-renders-originals-through-the-preview-compositor.md).)
[Verification](../verification.md#audio-editing) lists what was measured and
on which host.
