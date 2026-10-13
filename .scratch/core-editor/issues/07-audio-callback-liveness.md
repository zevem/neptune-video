# Release the audio clock when native callbacks stall silently

Status: ready-for-agent
Progress: resolved
Type: task

## Problem

The owned [virtual PipeWire/ALSA experiment](../../../artifacts/production-readiness/audio-device-probe/virtual-failure-round5/report.json)
opened and linked stereo streams, then stopped only the private sound server.
Raw CPAL callbacks stopped at 51,200 frames without an error callback. The
existing public service continued returning `Some(Time(1_045_333))`, keeping
Playback frozen for more than 14 seconds. Its decoder remained until explicit
public stop. The [root verification](../../../artifacts/production-readiness/final/round10-virtual-stall-root-verification-v2.json)
binds this to unchanged R10 source and CI libraries. Generated PCM was entirely
zero; no physical sound device or foreign sound socket was opened. This is
native virtual callback-stall proof, not a fatal callback or physical unplug.

## Acceptance

- Retain the native pre-fix trace and reproduce clock retention deterministically
  at the owning state seam, without sleeping or opening a device.
- Detect stale callback progress while an audio clock is active, with a grace
  period based on the largest observed callback buffer duration for that stream
  lifetime and a conservative minimum.
- Release the clock, coalesce a public failure notice and wake owned teardown.
  A late callback/publication/request cannot reactivate the failed stream.
- Refresh liveness on playback startup and genuine callbacks; paused output
  must not be declared failed just because callbacks are paused deliberately.
- Keep the sample callback bounded and free of locks, allocation, decoding and
  process operations. Record any monotonic clock-read cost accurately.
- Rerun the owned zero-PCM virtual disruption on the new exact libraries and
  require wall-clock video continuation and decoder cleanup before public stop.
- Validate healthy clock behavior and the full Rust/Linux build. Native target
  execution remains deferred; automatic device recreation, DAC/display timing
  and audible quality remain separate.

## Comments

Root owns Cargo and `audio.rs`; no native UI run is active. The accepted R10
editing and healthy clock records remain immutable. Any new source/binary is
a new checkpoint. Earlier virtual setup failures and rejected control routes
are retained; only the successful raw/service route and stronger absolute-path
missing-server control support the observation.

The actual liveness seam reproduced five failures out of eight tests before
expiry enforcement; all eight and all 54 media tests then passed. A preliminary
353-test R11 build passed full Linux/source checks. Independent review found
an additional long-to-short callback-period publication race: a new shorter
period can be observed with the prior heartbeat and manufacture expiry.
R11 is retained as a preliminary checkpoint, without native acceptance. Root
will reproduce that interleaving and retain the largest observed period for the
stream lifetime before recording a new build.

## Answer

The clock now expires after four largest observed callback buffer periods,
with a 500 ms floor. Native start and actual callbacks refresh its heartbeat;
paused epochs do not expire. The active worker observes the same clock, latches
failure, coalesces a notice and exits its owned mixer/stream path. Later PCM,
callbacks and requests cannot resurrect that failed stream. A completed reset
invalidates an old observation; a concurrent reset overlapping an already
observed expiry may lose to failure of the same native stream.

The additional [partial-publication regression](../../../artifacts/production-readiness/audio-device-probe/liveness-period-race-before-fix.log)
failed before retaining the maximum period; [all nine now pass](../../../artifacts/production-readiness/audio-device-probe/liveness-period-race-after-fix.log).
The [independent review response](../../../artifacts/production-readiness/audio-device-probe/callback-liveness-review-response.md)
confirms that correction. The [R12 checkpoint](../../../artifacts/production-readiness/final/round12-build-record.json)
passed 354 workspace tests (55 media), formatting, denied-warning Linux and
three cross-target source/Clippy checks, and the optimized Linux build.

The fresh [virtual native experiment](../../../artifacts/production-readiness/audio-device-probe/virtual-failure-round6/report.json)
still observed zero raw CPAL errors after server loss, but the corrected public
clock became unavailable, Playback continued monotonically, one notice appeared
and the decoder naturally disappeared before public stop. A separate observer
withheld position reads after priming and received the worker's failure notice
before querying position again. Root [verified](../../../artifacts/production-readiness/final/round12-virtual-stall-root-verification.json)
73 source files, exact libraries, unchanged projects, 15 tracked process
identities gone, no forced cleanup, all-zero PCM and private routes without
physical audio access. This establishes the specific silent virtual stall
correction, without claiming a native fatal callback.

The separate [healthy physical zero-PCM clock](../../../artifacts/production-readiness/final/round12-physical-clock-root-verification.json)
passed normal playback, seeking, half/double speed and pause/stop with no missing
clock samples or errors. Its output route stayed unchanged and all five owned
PIDs exited. These CI-profile diagnostics do not measure acoustic quality,
DAC/display timing, A/V synchronization, callback overhead or performance. The
sample callback adds one monotonic clock read; it remains free of locks,
allocation, decoding and process operations. Larger previously observed buffers
retain a longer grace until stream recreation. Automatic output recreation,
physical unplug and the existing 10-second blocked decoder read deadline remain
separate limitations. The subsequent [R12 native UI regression](../../../artifacts/native/post-liveness-round1/report.json)
passed 13 checks plus whole-project undo/redo, source preview and narrow states.
Root verified source/binaries, 18 capture hashes and all 18 tracked processes
gone. That UI run had audio unavailable; backend virtual/healthy proofs remain
separate. The prepared native platform pack is pinned to R12 with 57 offline
guards passing, without actual macOS/Windows execution.
