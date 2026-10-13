# Keep viewer transport controls separate in a compact central pane

Status: ready-for-agent
Progress: open
Type: task

## Problem

With both library and inspector shown at 800×600, the central viewer is narrow
and its play button overlaps the speed control. Opus observed this outside the
cache/culling scope; root confirmed it in the
[native capture](../../../artifacts/native/cache-culling-acceptance-round1/shots/e2-library-narrow-800x600.png).
The capture is included in the independently verified R14 native inventory.

## Acceptance

- Root owns builds. Begin after the current R14 measurement freeze is released.
- Fit the transport to actual central-pane width with clear accessible
  actions and separate hit regions. Keep play/pause and playback speed
  reachable, including compact layouts; ordinary wide layout remains usable.
- Verify 800×600 with both panels and 760×560, normal width, actual play/pause
  and speed interactions, source-monitor and timeline transport. Preserve
  keyboard controls and document/history nonmutation from transport use.
- Capture the corrected final native build. No playback math, decoder or
  unrelated project mutation belongs to this layout correction.

## Comments

The transport source/pointer correction is included in the
[R15 521-test build](../../../artifacts/production-readiness/final/round15-build-record.json)
and its 198 desktop tests; earlier font-fixture and two-pixel-slot failures
are preserved in the consumer-stage evidence. [Root viewed three R15 startup
captures](../../../artifacts/native/round15-window-captures/root-visual-verification.json):
Play and 48×28 Speed are visibly separate at 800×600 with both panels; 760×560
collapses the library while keeping preview/inspector/timeline visible. The
isolated unavailable-audio notice covers the upper inspector. Actual play/speed
and source-monitor input, transitions and history nonmutation remain unrun;
the R14 overlap retains its historical scope and this ticket stays open.
