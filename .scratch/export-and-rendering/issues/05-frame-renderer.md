# Frame renderer and shared layer assembly

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 01, 03

## Scope

`compose::assemble` shared by the viewer and the export; `render_stack` and
non-blocking readback in `compose::gpu`; `FrameRenderer` with its own `Gpu`,
one accumulator, run semantics, titles at the output scale, stabilization and
LUTs; `headless_device`. Contract review rules 12 to 15.

## Comments

2026-10-14: Implemented in [frame.rs](../../../src/export/frame.rs) and
[assemble.rs](../../../src/compose/assemble.rs). 13 frame tests and 5 pure
assembly tests passed on the RTX 4060 (Vulkan) and on llvmpipe, among them:
parity with the viewer's compositor path within ±2 for blended, rotated and
masked layers; a composite run over a title (checked to fail with the run's
own canvas switched off); letterbox and fill; a 3840x2160 frame; stop and
recovery; missing stabilization drawn as is with a warning.

Known limits: plain layers blend in linear light here and in gamma space in
the viewer's mesh path; titles are capped at 4096 a side; the decode size
ignores zoom transitions and distort effects. No export throughput measured
in this ticket.
