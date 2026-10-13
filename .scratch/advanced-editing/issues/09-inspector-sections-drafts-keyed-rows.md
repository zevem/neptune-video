# Inspector tabs, drafts and keyed rows

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 02, 07

## Scope

The clip inspector for the epic: sections, effect stack, masks, blend, looks,
colour, keyframe toggles.

## Result

`src/ui/inspector/`: Video (transform, crop, Layout presets, Compositing
blend, Masks), Look (effect stack by family, Looks with Strength and Remove
look), Color (grade, hue bands, curves, LUT, Remove background with chroma key
eyedropper and clean plate, Background helper, canvas colour), Speed and
Motion. Rows are generated from the core tables. A draft is a `ClipOverride`;
a drag, scrub or typed value is one history entry and Escape gives it up.
Keyed rows carry a diamond with previous/next steps, Enter toggles a keyframe
and Shift+Enter opens its menu.

## Evidence

`src/ui/inspector/tests.rs`, `color_tests.rs`, `src/ui/plate_viewer_tests.rs`,
`lut_viewer_tests.rs`. Native on lavapipe at 1440x900, sections at 760x560
(wave 3); focus order, labels and menu focus re-captured in wave 4.

The [verification matrix](../../../docs/verification.md#advanced-editing)
states the host, adapters and what is unverified on every platform.

## Known limits

- A mask is outlined on the picture but has no on-canvas handles.
- The Color section edits the first effect of each of its kinds only.
- At 760x560 the inspector shows about 105 points of rows, less than the curve
  editor is tall.
- Not captured natively: effect reorder and remove, mix, rectangle mask and
  its ops, the canvas colour picker, LUT and plate import buttons, the Looks
  library tab, locked tracks, the light theme.
- The `App` wiring of Escape and draft commit has no headless test; `App`
  cannot be built in a test.
- Inspector scroll-into-view of a keyboard-focused row was not separately
  captured.

## Comments

2026-10-13: Written by ticket 17 from the code as it stands and the
implementing tasks' reports. Verified on one Linux host only; macOS, Windows,
a hardware GPU in a windowed session, HiDPI and playback rates are unverified.
