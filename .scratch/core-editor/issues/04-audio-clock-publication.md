# Keep decoded audio aligned with its playback generation

Status: ready-for-agent
Progress: resolved
Type: task

## Problem

The audio worker checks its device cursor before decoding, but a decoder read
can block while callbacks advance through underrun silence. Publishing that
completed chunk unconditionally then plays old PCM behind the reported clock.
Cancellation can also race publication and clock activation. The wall-clock
fallback can move ahead before the first decoded audio buffer becomes ready;
switching directly to that earlier audio position moves the playhead backward.

## Acceptance

- Reproduce late and obsolete decoded publication at the private worker/ring
  seam with deterministic device-free tests, preserving the failing run.
- Reject completed audio whose generation is obsolete or whose source cursor
  has passed. Resynchronize on the worker; retain bounded SPSC ownership and
  avoid locks, allocations or decoding in the native callback.
- A late worker cannot activate a newer request's clock. Cursor reset occurs
  after the callback has acknowledged the generation change.
- Playback clock handoff cannot move backward within one play interval; an
  explicit seek still establishes a new position. Test audio catch-up and
  fallback after loss without relying on elapsed real sleeps or hardware.
- Run actual media regressions, native target source checks and optimized
  builds, then fresh native and device measurements for the resulting build.

This closes a reproduced synchronization defect. Callback frame accounting is
not a DAC presentation timestamp or proof of acoustic/display synchronization.
The existing audible voice policy and exact VFR/HDR remain separate work.

## Comments

2026-10-12: Read-only audio trace identified late PCM publication, startup clock
handoff and voice-cap edge cases after the zero-gain decoder fix. Root owns
Cargo; backend agents stage source-only tests first. No source defect is treated
as established native failure until its owning regression or native observation
reproduces it.

2026-10-12 local regression checkpoint: the original publication seam reproduced
four failures out of five cases; the early stale-request guard already passed.
Scheduled per-frame generation/output positions and generation-tagged clock
activation now pass all five. Five additional pure tests cover clock mapping,
whole-expired publication races, mono/stereo ring wrap and alignment/resource
bounds. All 25 media tests passed with real FFmpeg 8.0.1, and media Clippy with
warnings denied passed. The core delayed-clock handoff regression failed before
the monotonic hold correction and all five checkpoint tests passed afterwards.
These are focused source checks, with logs under
`artifacts/production-readiness/audio-device-probe/`; no optimized native or
current-device acceptance is claimed yet. Startup can temporarily hold picture
ahead of sound while it catches up; this correction prevents rewind and does
not establish a shared transport/DAC/display timestamp.

2026-10-12 final source checkpoint: [round nine](../../../artifacts/production-readiness/final/round9-build-record.json)
passed 333 workspace tests, zero failed or ignored, actual FFmpeg 8.0.1,
denied-warning all-target Clippy on Linux and source checks for Mac ARM64,
Windows MSVC and Windows GNU, plus optimized Linux application builds. The
[current physical clock probe](../../../artifacts/production-readiness/audio-device-probe/report.json)
uses the exact recorded CI components and exclusively zero-valued PCM on the
default ALSA/PipeWire HDMI sink; it verifies seek, half/double speed, pause and
stop clock behavior. No capture, route or volume change was used. Fresh native
Linux editing/performance verification of `b29ccb69`/`14435a4a` is in progress;
the issue remains claimed until that report is reviewed. The physical silence
probe is not audible quality, startup A/V alignment or underrun evidence.

## Answer

The actual private publication regressions and core handoff test failed before
the scheduled generation/frame and monotonic handoff corrections and pass
afterwards. The [333-test checkpoint](../../../artifacts/production-readiness/final/round9-build-record.json)
also passed real media tests, source/lint checks for native target code and
optimized Linux builds. Its [fresh native editing/performance report](../../../artifacts/native/performance-round1/report.json)
completed all five bounded cases without a reproduced editing defect; root
[verified the hashes and all 48 owned PIDs](../../../artifacts/production-readiness/final/round9-native-performance-root-verification.json).
The separate current physical zero-PCM probe verifies callback-clock behavior
on its recorded default output. Native UI measurements had audio disabled and
do not verify audible synchronization. Startup can still hold the picture while
delayed sound catches up; DAC/display alignment and underruns remain open.

This resolves the reproduced late/stale publication and rewind defects only.
The source-confirmed [fatal-output clock retention](06-audio-output-failure.md)
is a separate next correction and is not included in this checkpoint.
