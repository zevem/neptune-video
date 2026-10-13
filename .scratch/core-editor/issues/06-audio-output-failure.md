# Release the playback clock after fatal native output failure

Status: ready-for-agent
Progress: resolved
Type: task

## Problem

The current native output error closure in
[audio.rs](../../../crates/editor-media/src/audio.rs) only publishes an error
string and wakes the desktop. `AudioState::clock` continues returning the last
position whenever the playback generation remains active. If the host stops
callbacks after a fatal output error, that position stays frozen.
[Playback](../../../crates/editor-core/src/playback.rs) accepts that `Some`
position each frame and renews its fallback anchor, preventing wall-clock
progress. This contradicts silent video playback continuing after device loss.

Source tracing establishes the missing invalidation path; no native device
failure has been injected or observed. CPAL errors that explicitly retain or
recover the stream must keep working audio alive. Clearing only the active
generation is insufficient: a concurrent decoded publication can activate it
again.

## Acceptance

- Reproduce frozen-clock behavior and a raced late publication through the
  actual private state/error/publication seam without audio devices or sleeps;
  retain the failing run before the correction.
- Classify errors using the pinned CPAL implementation. Preserve notification
  and recovery behavior for DeviceChanged, RealtimeDenied and recovered Xrun;
  fail closed for a fatal or unknown stream error.
- Latch fatal stream failure so clock reads, queued publication, native output
  callbacks and later sync requests cannot resurrect the failed stream.
- Wake the worker for owned decoder/stream teardown. Keep the data callback
  bounded and free of locks, allocations, decoding or process operations.
  The public error remains observable and video can use its existing fallback.
- Test recoverable errors, full error queue, failure/publication ordering,
  callback accounting and fallback after clock loss. Check native-target source
  validity and optimized builds after focused media/core checks pass.
- Record any native/device failure proof separately. This correction does not
  establish automatic device recreation, acoustic quality, A/V alignment or
  underrun-free playback.

## Comments

2026-10-12: Root owns the backend and Cargo. Sources/binaries are frozen at
`b29ccb69`/`14435a4a` for an active native Linux performance
task. A backend subagent is preparing a read-only failure plan; implementation
and tests begin only after that task publishes its unchanged-hash report and
ends its owned runtime work. Native macOS/Windows execution remains deferred
by the user's instruction to prepare tests here for now.

2026-10-12: The [read-only failure plan](../../../artifacts/production-readiness/audio-device-probe/fatal-output-error-plan.md)
confirms the pinned CPAL categories and documents a monotonic per-stream failure
latch, raced-publication guards, typed bounded notices and worker-owned cleanup.
A retained park/unpark token avoids a lost condition-variable notification
without introducing idle polling. Existing blocking decode deadlines can delay
teardown; neither immediate child exit nor native failure timing is claimed.
The plan is ready for a red regression at the actual private error seam after
the frozen performance task completes. No implementation or test is implied
by this investigation.

2026-10-12 regression checkpoint: the actual extracted native error seam
[reproduced six failures](../../../artifacts/production-readiness/audio-device-probe/fatal-output-before-fix.log)
out of seven tests; the recoverable notification case already passed. A
monotonic stream-failure latch now guards the public clock, callback and
publication paths, rejects later requests and wakes worker-owned cleanup
through a retained park token. The [same seven tests pass](../../../artifacts/production-readiness/audio-device-probe/fatal-output-after-fix.log).
Five additional tests cover all 14 pinned error categories, saturated notices,
pre-registration ownership and a controlled notification-before-park race.
[All 12 pass](../../../artifacts/production-readiness/audio-device-probe/fatal-output-expanded-tests.log),
as do all 46 media tests with actual FFmpeg 8.0.1 and denied-warning media
all-target Clippy. A coalesced generic failure remains observable if all eight
notice slots were full. This is device-free regression proof, not injected
native device failure or acoustic output. Whole-workspace, native-target source
and optimized build checks are running; the issue remains claimed.

## Answer

The fatal-output latch, typed/coalesced notice queue and retained worker wake
are implemented at the production callback seam. The six failing regressions
now pass, with 12 fault tests and all 46 media tests green. The immutable
[R10 checkpoint](../../../artifacts/production-readiness/final/round10-build-record.json)
passed all 345 workspace tests, formatting, denied-warning Linux and three
cross-target source/Clippy checks, and the optimized Linux build. Cross-target
checks do not establish native macOS/Windows execution.

The same source and optimized binary also passed the fresh
[13-check Linux editing run](../../../artifacts/native/post-audio-round1/report.json),
including whole-project undo/redo and unchanged source previews. Root verified
72 source files, both binaries, 19 capture hashes and all 18 tracked process
identities gone in the [independent record](../../../artifacts/production-readiness/final/round10-native-editing-root-verification.json).
Audio was unavailable throughout that UI run. The separate
[healthy physical zero-PCM probe](../../../artifacts/production-readiness/final/round10-physical-clock-root-verification.json)
observed advancing clocks, seek/speed invalidation and pause/stop on the R10
libraries; it did not trigger failure or measure acoustic output.

The owning regression is resolved. Native fatal-callback acceptance remains a
separate experiment, as do callback stalls without an error, automatic device
recreation, physical hot-unplug, A/V timing and underruns. Worker cleanup can
still wait for the existing 10-second decoder read deadline; no instantaneous
teardown is claimed. The error callback uses bounded notices and unpark/wake;
only the ordinary sample callback has the lock/allocation-free guarantee.
