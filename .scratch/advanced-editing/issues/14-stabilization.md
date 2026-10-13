# Stabilization

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 13, 08

## Scope

Steady a shaky video from a measured camera path, with the path kept outside
the project.

## Result

`crates/editor-media/src/analysis/steady.rs`: block matching on luma with a
three-level pyramid and outlier rejection gives shift, turn and size a frame
(shift alone when the fit is weak), integrated and smoothed by a gaussian of
Smoothness × 0.5 s. The path of the whole asset is cached in a
`stabilization` folder of the user's cache directory (beside the settings
file when `--settings` is given), 64 MiB and 256 files, least recently read evicted, stamped with file length,
modification time and probe facts. `src/steady.rs` holds eight paths. The
correction is applied in the place pass on source sampling and clamped so no
border is read. The project stores only the `Stabilize` effect; the Motion
section has the Stabilize card with Strength, Crop zoom, Fit crop, Analyse and
Cancel analysis.

## Evidence

`crates/editor-media/src/analysis/steady_tests.rs`, `src/steady.rs` tests,
`src/compose/steady_tests.rs` (readback on NVIDIA),
`src/ui/inspector/steady_tests.rs`. Native on lavapipe at both sizes (session
scratch). Measured: 7.52 to 0.10 px RMS on a 320x240 fixture, 2.379 to 0.165
on synthetic shake, 15 to 16 ms a frame unoptimized.

The [verification matrix](../../../docs/verification.md#advanced-editing)
states the host, adapters and what is unverified on every platform.

## Known limits

- A 2D deshake: no rolling-shutter correction, no parallax, no warp.
- No camera footage was tried. Turn and size estimation is tested
  synthetically only and falls back to shift with large moving subjects.
- The spec's decision 7 says the cache is keyed by source fingerprint and
  algorithm version; the file name is a hash of the media path, and the stamp
  and version are checked on read.
- The whole asset is analysed, not the clip's span; videos over 1,048,576
  frames are refused; one video is measured at a time.
- A media change on disk is noticed when the cache is next read, not while a
  path is held in memory.
- A tracker's region, point and followers go through the steadied reading of
  the frame shown; the path overlay's other points use that same frame's
  reading. A difference matte's plate is not steadied with the layer.
- No agent phrase and no protocol operation starts an analysis. Stabilize is
  not in the effects browser.
- No app-level automated test runs from analysis to canvas; playback pacing of
  a stabilized clip was not measured; most captures predate the estimator's
  second fit guard.

## Comments

2026-10-13: Written by ticket 17 from the code as it stands and the
implementing tasks' reports. Verified on one Linux host only; macOS, Windows,
a hardware GPU in a windowed session, HiDPI and playback rates are unverified.
