# LUT asset (`.cube`)

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 08

## Scope

Import a `.cube` table and grade a layer with it.

## Result

`crates/editor-media/src/lut.rs` parses a 3D table of 2 to 65 points an edge
and 32 MiB at most; import makes a `MediaKind::Lut` asset (duration one
microsecond, `width` the edge, never a clip, a tracker source or a preview).
`MediaService::request_lut` reads it on the worker. `src/compose/lut.rs` keeps
at most four tables as tiled `Rgba16Float` textures counted in `ComposeBudget`
at eight bytes a texel. A missing table changes nothing and is said.

## Evidence

`crates/editor-media/src/lut.rs` tests, `src/compose/lut_tests.rs` (NVIDIA and
llvmpipe), `src/ui/lut_viewer_tests.rs`, store round trip. Native: red/blue
swap on three adapters (wave 2); LUT row in the Color section at 1440x900
(wave 3).

The [verification matrix](../../../docs/verification.md#advanced-editing)
states the host, adapters and what is unverified on every platform.

## Known limits

- 1D tables are refused.
- The wgpu GL backend was not reached by the test harness; the GL native
  capture is the only GL evidence.
- A table read waits behind an import in progress on the same worker; not
  measured.
- The import button opens a file dialog and was not driven natively.

## Comments

2026-10-13: Written by ticket 17 from the code as it stands and the
implementing tasks' reports. Verified on one Linux host only; macOS, Windows,
a hardware GPU in a windowed session, HiDPI and playback rates are unverified.
