# Commands and split/trim/rate/paste rebasing

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 01

## Scope

The ten new commands and the changes to split, trim, rate change and paste.

## Result

`SetEffects`, `SetMasks`, `SetBlendMode`, `SetKeyframes`, `SetRetime`,
`SetTransition`, `ApplyPreset`, `SetTracker`, `RemoveTracker` and `SetFollow`
are whole-value setters through the path that refuses a locked track.
`SplitClip` partitions animation and time-map keys at the cut and may
synthesize more than one key so each part keeps its curve; `TrimClip` rebases
keys on a start trim and crops a time map; `SetPlaybackRate` rescales keys.
`SetRetime` moves keyframes to where the new map shows their source moment
(`Retime::shows`). `ApplyPreset` replaces the clip's derived look
(`look::applied_look`, `look::with_look`). Copy and paste remap effect and
mask IDs; the clipboard text is version 2 and version 1 is still read.

## Evidence

`crates/editor-core/tests/advanced_commands.rs` (reversal, atomic failure,
split invariance of sampled values, key following for freeze, reverse and
ramp, look replacement).

The [verification matrix](../../../docs/verification.md#advanced-editing)
states the host, adapters and what is unverified on every platform.

## Known limits

- The spec's command table still says `ApplyPreset` appends; the dated
  Comments entry for the wave 3 defects replaces that.
- A look applied at strength 0 cannot be read back from the effects, so the
  next apply appends.
- A moment a new time map shows twice takes the showing nearest the key's old
  time; one never shown takes the nearest map key in source.
- Reverse and ramp key-following are core tests only; natively only the freeze
  case was driven.
- `SetSettings` lengthens a transition shorter than a new frame to one frame;
  no test covers that on a locked track.

## Comments

2026-10-13: Written by ticket 17 from the code as it stands and the
implementing tasks' reports. Verified on one Linux host only; macOS, Windows,
a hardware GPU in a windowed session, HiDPI and playback rates are unverified.
