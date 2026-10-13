# Speed UI and reverse preview buffering

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 06, 09

## Scope

Edit speed, reverse, freeze and ramps, and make stepping backwards through
video cheaper.

## Result

`src/speed.rs` models a time map as a speed curve; `src/ui/inspector/speed.rs`
has the speed field, 0.25× to 4× presets, Reverse, Hold for and Freeze at
playhead, seven ramp presets and the Speed curve graph (linear to 2, 4, 8 or
16 times, holding its top while a point is held). Freeze is F, the inspector
button and the clip menu. Retimed tiles carry a badge. A step back behind a
decoder's pipe starts the replacement up to 24 frames early
(`REVERSE_GROUP_FRAMES` in `crates/editor-media/src/decode.rs`) and keeps what
it reads in the frame cache.

## Evidence

`src/speed.rs` tests, `src/ui/inspector/speed_tests.rs`, the `MEASURE:` test
in `decode.rs`. Native on lavapipe at both sizes (wave 3; the curve drag
re-captured in wave 4). Measured at `DecoderPool` on 640x360 H.264: 30 steps
back went from 30 decoder starts and 55.5 to 60.8 ms a frame to 3 starts and
6.6 ms.

The [verification matrix](../../../docs/verification.md#advanced-editing)
states the host, adapters and what is unverified on every platform.

## Known limits

- Reverse preview still costs a process start for every 24 frames and is not
  measured through the service, the viewer or the window. Not real time.
- A speed-curve drag is not previewed on the canvas or the timeline until
  release.
- A speed change is cut to the room on the track; later clips are not moved.
- The graph draws how fast, not which way; Reverse turns the whole clip.
- A ramp preset replaces the clip's existing curve, including a freeze.
- Clip lengths after a ramp are not quantized to whole frames.
- The refusal note and the silent-sound note were not captured natively.

## Comments

2026-10-13: Written by ticket 17 from the code as it stands and the
implementing tasks' reports. Verified on one Linux host only; macOS, Windows,
a hardware GPU in a windowed session, HiDPI and playback rates are unverified.
