# Types, defaults and validation

Status: ready-for-agent
Progress: resolved
Type: task

## Scope

The durable types of the epic, their serde defaults and every bound, in
`crates/editor-core`.

## Result

`animate.rs`, `look.rs` and `retime.rs` hold `ParamKey`, `Ease`, `Keyframe`,
`AnimTrack`, `EffectKind` (20 kinds), `Effect`, `Mask`, `BlendMode`,
`Transition`, `Tracker`, `Follow`, `LookPreset`, `Retime` and `TimeKey`.
`Clip` gained `blend`, `effects`, `masks`, `animation`, `retime`,
`transition_in`, `transition_out` and `follow`; `Project` gained `trackers`.
Every new field is defaulted and skipped when empty, so a version 1 clip
decodes and a default clip emits no new key. `look::spec`, `mask_spec`,
`clip_spec` and `transition_spec` are the parameter tables. All bounds are in
`Project::validate`. `MediaKind::Lut` was added later by ticket 12.

## Evidence

`crates/editor-core/tests/advanced_model.rs` (bounds, NaN and infinity, v1
decode, round trip).

The [verification matrix](../../../docs/verification.md#advanced-editing)
states the host, adapters and what is unverified on every platform.

## Known limits

- The ranges and defaults in the tables were chosen during implementation, not
  specified.
- Clips on audio tracks accept gain keyframes and a time map only; a sound
  cannot be an effect asset or a tracker asset. Text clips refuse effects,
  masks, a time map and a non-normal blend.
- A difference matte's plate must be a still image; a project from an earlier
  build of this epic that used a video plate fails validation, and no
  migration exists.

## Comments

2026-10-13: Written by ticket 17 from the code as it stands and the
implementing tasks' reports. Verified on one Linux host only; macOS, Windows,
a hardware GPU in a windowed session, HiDPI and playback rates are unverified.
