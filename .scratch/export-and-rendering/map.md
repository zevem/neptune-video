# Export and rendering map

## Notes

Phase 7 on the Linux host (i9-13900HX, RTX 4060 Laptop, Ubuntu 26.04.1,
FFmpeg 8.0.1). The [spec](spec.md) is the binding design;
[ADR 0008](../../docs/adr/0008-export-renders-originals-through-the-preview-compositor.md)
records the decision. Implementation fanned out by module; the window
integration landed last.

## Decisions-so-far

- [01](issues/01-core-export-data-and-validation.md): export data and `plan`
  in the core, 26 tests.
- [02](issues/02-encoding-and-sound-render.md): external FFmpeg probed per
  host; RGBA on stdin, BT.709 limited range; sound from the playback mixer,
  sample-exact.
- [03](issues/03-export-source-reader.md): originals decoded with the
  preview's frame choice, byte-equal to a decode from zero.
- [04](issues/04-proxy-service-and-cache.md): proxies are derived data in a
  bounded cache keyed by source identity.
- [05](issues/05-frame-renderer.md): every export layer through the
  compositor on one accumulator; parity with the viewer's compositor within
  ±2.
- [06](issues/06-render-queue-and-job-runner.md): one job at a time; part
  file, ffprobe check, rename; one software retry under Auto.
- [07](issues/07-export-dialog-and-queue-panel.md): dialog and panel
  modules, headless-tested.
- [08](issues/08-agent-render-operations.md): non-mutating render
  operations on the wire; headless answers `unsupported`.
- [09](issues/09-window-integration.md): the window builds the queue on its
  device; Export and queue buttons, Cmd/Ctrl+E and Shift+Cmd/Ctrl+E, the
  export sheet with its poster, the replace and quit questions, saved
  presets and the agent's render answers.
- [10](issues/10-preview-quality-and-proxy-switching.md): preview quality
  and per-layer proxy switching wired in the window.
- [11](issues/11-adr-glossary-and-guides.md): ADR, glossary, architecture,
  performance, usage, installation and README; checked against the window
  and the review fixes on 2026-10-14.
- [12](issues/12-native-verification.md): verified on the Linux host with
  the release build on 2026-10-14: every format, sync within 0.03 ms
  (WebM within Opus's 7 ms start), original pixels from a proxied asset,
  queue control, quitting and captures at both sizes. Recorded in
  [verification](../../docs/verification.md#export-and-rendering).

## Fog

- Follow-ups from implementation, review and the Linux verification,
  including interface paths driven only through the agent
  ([15](issues/15-implementation-follow-ups.md)).
- Every hardware encoder but NVENC, and macOS and Windows
  ([13](issues/13-other-platforms-and-hardware-encoders.md)).
- Sustained throughput, memory, a thread cap for the export's FFmpeg, the
  cost to the interactive window and the benefit of proxies
  ([14](issues/14-throughput-and-memory.md)).
- Codec licensing for a distributed build, with the open
  [codec and distribution ticket](../core-editor/issues/03-codec-and-distribution.md).
