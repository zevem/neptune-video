# Effect, color, mask, key and transition shaders

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 07

## Scope

Pixel math for every effect kind, mask, blend mode and transition kind, once,
in WGSL.

## Result

`src/compose/shaders/` (`place`, `effect`, `blend`, `transition`, `encode`,
`common`). Grade works in linear light; curves, hue band, keys and grain on
gamma-encoded values. Blur, glow, sharpen and shadow share one separable
gaussian. Lengths are fractions of the canvas height. Masks run in one pass
after the effects. A transition is one pass that joins its two clips, blended
once with the incoming clip's mode; without a device every kind is a
cross-fade. Grain is seeded from `Layer.seed`; shake and glitch move by
`Layer.time`.

## Evidence

`src/compose/effect_tests.rs`, `transition_tests.rs`, `gpu_tests.rs` on NVIDIA
RTX 4060 (Vulkan) and llvmpipe. Native: look, blur, glow, wipe left, dissolve,
chroma key, ellipse mask and screen blend on three adapters (wave 2).

The [verification matrix](../../../docs/verification.md#advanced-editing)
states the host, adapters and what is unverified on every platform.

## Known limits

- Shake, wave, motion blur, grain, sharpen, shadow, vignette, glitch and
  twelve of the fourteen transition kinds were never looked at in the running
  app.
- `Distort` is a wave only; `MotionBlur` streaks one frame and accumulates
  none; effects run in canvas space, so shake and wave can bring in clear
  canvas and a spatial effect does not turn with the layer.
- Slides and wipes cross the whole canvas, not the layer's box.
- Chroma key keeps a one-pixel fringe at defaults on hard 4:2:0 edges.
- Grain and glitch patterns depend on the canvas size and change when the
  window is resized.
- Six live video consumers restart FFmpeg for each next frame on the three
  unprotected ones; the incoming clip is not pre-rolled.
- No performance figure for any pass.

## Comments

2026-10-13: Written by ticket 17 from the code as it stands and the
implementing tasks' reports. Verified on one Linux host only; macOS, Windows,
a hardware GPU in a windowed session, HiDPI and playback rates are unverified.
