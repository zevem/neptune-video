# GPU compositor backbone, budget, viewer consumes the plan

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 03

## Scope

An off-screen wgpu compositor for non-plain plans with the mesh path kept for
plain ones.

## Result

`src/compose/` splits a plan into runs (`runs`), books a frame against
`ComposeBudget` (96 MiB logical, `Ledger`), and draws place, chain, blend and
encode passes (`gpu.rs`, `pass.rs`). Intermediate canvases are
`Rgba8UnormSrgb` and the published canvas `Rgba8Unorm`. A canvas is composited
at the screen pixels it covers and steps down through `CANVAS_STEPS` only to
fit the budget. A layer a canvas shows keeps its decoded frame and no texture.
Everything is released three seconds after the last composited frame, after
120 painted frames without a job, and on project replacement. Whatever the
device refuses is caught and the mesh path draws with "Effects not shown".

## Evidence

`src/compose/tests.rs` (ledger), `src/compose/gpu_tests.rs` (readback on
NVIDIA RTX 4060 Vulkan and llvmpipe), `src/ui/plan_viewer_tests.rs`,
`src/ui/working_set_tests.rs`. Wave 2 captures on lavapipe, NVIDIA and GL.

The [verification matrix](../../../docs/verification.md#advanced-editing)
states the host, adapters and what is unverified on every platform.

## Known limits

- The spec's rendering bullet still says `Rgba8Unorm` intermediates; the
  ticket 07 comment replaces it.
- "Effects not shown" was never seen in a running app: the window always has a
  device. The refusal path is tested with a validation error only.
- Text is not composited and parts the plan into runs (four canvases a frame
  at most); a blended layer above a title blends only with the media between
  the two.
- The mesh path blends in gamma space, so a half-transparent layer changes
  shade when its plan stops being plain.
- No pass cost, upload cost, pipeline-build hitch or GPU memory was measured.
  `ComposeBudget` is logical bytes, not a measured GPU bound.
- Device tests opening one device each in parallel crashed on the NVIDIA
  driver in 2 of 4 early runs; they now share one device. The cause was not
  investigated.

## Comments

2026-10-13: Written by ticket 17 from the code as it stands and the
implementing tasks' reports. Verified on one Linux host only; macOS, Windows,
a hardware GPU in a windowed session, HiDPI and playback rates are unverified.
