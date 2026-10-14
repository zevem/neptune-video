# Performance contract

This project targets responsive professional editing, not a benchmark inferred
from Rust or GPU dependencies. Record actual results in
[verification](verification.md) with raw artifacts and the tested workload.

## Current performance acceptance

The [measured handover](../artifacts/performance-acceptance/round1/REPORT.md),
[predeclared budgets](../artifacts/performance-acceptance/round1/PLAN.md) and
[recomputed acceptance](../artifacts/performance-acceptance/round1/acceptance.json)
identify source `b09ea17e` and optimized Linux `aster` `9b9ff3f0`.
The [build record](../artifacts/performance-acceptance/round1/final-build.json)
passes 310 focused core/desktop tests, formatting, architecture, denied-warning
core/desktop all-target Clippy, media preflight and locked release linking.
Whole-workspace tests were not rerun for this source.

At 100,000 clips / 1,000 tracks, three release processes measure move/undo
p95 2.862–3.025 ms against 16.667 ms and warm active-query p95 44.644–45.877 µs
against 200 µs. Immutable-root interval bounds avoid expired-prefix scans;
membership-only UUID validation uses a pre-sized hash set. Exact rollback,
overlap order, root replacement and complete undo remain covered. Two thousand
retained edits stay below 98 MiB PSS against 512 MiB. Native text-timeline
input, actual held move/undo, normal/narrow captures and repeated idle samples
are retained; this fixture does not render 1,000 simultaneous text layers.

Two 120-second five-source 1080p30 native controls meet every declared playback
budget: each owner 29.808–29.900 observed changes/s against 29.5; worst trial
exact-scrub p95 198.345 ms against 400 ms; sampled peak PSS below 1,368 MiB
against 2 GiB, GPU process row 314 MiB against 512 MiB, five decoder limit,
finite memory-growth budget and paused CPU below 1% of one core. Visible and
native minimized idle samples also pass. These playback controls and the
large-input control constrain task-owned children to CPUs 0–15 (eight P cores
and their SMT siblings); Aster ships no CPU-affinity policy.

**Default shared-host performance remains below budget.** Completed unpinned
playback observations range from 23.067–25.233 to 29.192–29.733 changes/s;
the slower trial also misses the seek and loop-gap budgets. One unpinned
100,000-clip zoom tail is 211.809 ms against p95 50 / p99 100 ms. CPU placement
and host variation are supported explanations, without attributing each stall
to an observed core. Passing controlled runs do not close these failures.
Physical display/audio, other platforms, codec diversity, large imports,
concurrent storage/agent work and hours-long sessions remain unverified.

## Preceding playback checkpoint

The [playback performance findings](../artifacts/production-readiness/playback-performance-round1/FINDINGS.md)
and [independent worker audit](../artifacts/production-readiness/playback-performance-round1/sustained-audit.md)
belong to source `ebc9da4c` and optimized Linux `aster` `4b3128d8`.
The [final build record](../artifacts/production-readiness/playback-performance-round1/final-build-attempt-03/build-record.json)
passes 610 workspace tests, formatting, architecture checks, warning-denied
workspace Clippy and locked release linking, with explicit FFmpeg 8.0.1.

Three matched eight-second worker pairs use five independently encoded
1080p30 H.264 sources. Median distinct frames per owner improve from
46/43/38/36/35 to 238/237/237/236/236. Median per-trial request-to-poll p95s
improve from approximately 221–244 ms to 5.6–22.6 ms. Updated delivery includes
3,149 of 3,150 steady keys after the first second, with one recorded deadline
miss; cold startup still fails the strict whole-window 240-key diagnostic.
All returned pixel hashes, full-byte fences and cleanup checks pass.
Median sampled peak PSS falls from 1,592 to 1,085 MiB; threads fall from 172 to
54. Demand CPU, including verification, is approximately 2.22 CPU cores.

The exact final native build also runs five full 1920×1080 decoder outputs in
a 2400×1450 window on an owned Weston GL/NVIDIA RTX 4060/Xwayland session.
Three eight-second 120 Hz framebuffer observations show approximately 30
distinct source pictures per second for every owner. Normal and 760×560
windows pass held-frame scrub controls. These are finite native framebuffer
observations with occasional missing keys, not physical presentation callbacks
or a zero-drop guarantee. Full-resolution software Xvfb rendering remains
approximately 15 fps. Exact new-key seek and cached-target confirmation times,
source gaps, CPU/memory, captures and observer limits are retained in the
findings. A [40-second loop follow-up](../artifacts/production-readiness/playback-performance-round1/native-loop-after-accelerated/native.json)
passes three legitimate restarts per owner, exact targets and unchanged history.
Sampled playing PSS stays 1,195–1,224 MiB and decreases in the final ten-second
bucket; peak threads stay 64. This resolves the short-window memory-growth
concern for this finite workload, while preserving explicit cold-loop recovery
and sampling limits. Audio, long sessions, natural-footage/codec diversity and other platforms
remain outside this task's acceptance.

The decoder now retains five pipes, overlaps bounded admitted startup, bounds
raw-video output threads/packet queues, and keeps large frames demand-driven.
Decoded/result caches and frontend viewer/poster budgets remain 64 MiB each.
Aspect-fitted reservations allow five full-HD textures within the existing
viewer budget; seek-time admission, texture reuse and rational repaint pacing
preserve correct frames and paused sleep. Codec/driver working memory remains
separate from logical caches; measured peaks do not impose a hard OS ceiling.

## Core timeline measurement

The [round-nine five-trial record](../artifacts/production-readiness/final/round9-timeline-repeated.json)
uses source `b29ccb69` and a freshly built optimized benchmark executable
(`4a76f329`), with its [build log](../artifacts/production-readiness/final/round9-timeline-build.log).
Median per-trial means were **363.872 µs** move/undo (trial range
**352.091–377.165 µs**) and **2.258 µs** active query (**2.183–2.309 µs**).
Each trial used 5,000 text clips/20 tracks, 1,000 move/undo cycles and 100,000
queries over 0–499.9 seconds. Owned Cargo, UI and device work had stopped before
the five sequential trials; the shared host remained outside CPU isolation.
These are per-trial averages, not individual-operation tails, native interaction
latency or GPU/media throughput. The separate `b10f0422` digest in the record
identifies the unchanged benchmark source file, not the workspace.

The historical optimized [timeline benchmark](../crates/editor-core/benches/timeline.rs)
recorded **421.812 µs** average per move-and-undo cycle and **2.451 µs** average
per active-clip query. The fixture has 5,000 text clips across 20 tracks, 1,000
move/undo cycles and 100,000 queries evenly sampling 0–499.9 seconds of its
timeline; the queries found 1,000,000 total active layers. The raw result is
[timeline.json](../artifacts/performance/timeline.json), with its
[build log](../artifacts/performance/timeline-build.log).

The host was an Intel Core i9-13900HX (32 logical CPUs), approximately 32 GB RAM,
Linux x86_64 / Ubuntu 26.04.1 LTS and Rust 1.97.1. The optimized bench profile
uses the repository's release settings: thin LTO and one codegen unit.

```sh
cargo bench --locked -p editor-core --bench timeline
```

This measures pure editing metadata on a synthetic timeline. It excludes
decoding, audio, texture upload, native input and GPU/display presentation, and
does not exercise growing long-term history. That initial single run has no
repeated-run distribution and is not a CapCut/Palmier Pro or Neptune comparison. It replaces
the earlier preliminary query run that sampled only the first 100 seconds.

An earlier [five-trial sample](../artifacts/production-readiness/portable-fixture/timeline-repeated.json)
used the same workload and benchmark source file, with its own executable hash
recorded. Median per-trial means were **374.132 µs** move/undo
(trial range **363.986–377.497 µs**) and **2.288 µs** active query
(**2.215–2.425 µs**). These are averages within each trial, not individual-operation
p95/p99 or native latency. The host was shared and not CPU-isolated; a brief
Linux lint overlapped part of the direct runs. This is preliminary repeated
metadata evidence; it remains separate from the quiet round-nine sample above.

## Software preview measurement

The accepted [round-thirteen checkpoint](../artifacts/production-readiness/final/round13-build-record.json)
(`5cc20680` source / `6496d42f` Linux binary, 365 tests) corrects
[finite paused-preview delivery loss](../.scratch/core-editor/issues/08-paused-preview-delivery.md).
The [eleven owning regressions](../artifacts/production-readiness/preview-delivery/after-fix.log)
and [review](../artifacts/production-readiness/preview-delivery-review-response.md)
cover protected pending/in-flight identity and cancellable bounded-result
backpressure. Shared pixel storage establishes logical byte accounting, not
independent decode allocations; direct import/late-cleanup interleavings retain
inspection limits. This correction has no new throughput measurement. The
[accepted R13 native report](../artifacts/native/post-preview-delivery-round1/report.json)
shows five paused layers in three fresh launches, stable across seven one-second
reads, with correct seek/Home patterns and unchanged document/history. Native
1920-square request sizes were inferred; publisher parking was unobserved.
The [two independently verified public probes](../artifacts/production-readiness/final/round13-public-preview-root-verification.json)
delivered five distinct 1920-square PNG payloads with exact pixels and distinct
live storage addresses. These CI-library/opt-level-zero probes do not measure
performance, and withholding poll for one second does not prove a completed
fifth decode or parked publisher. Actual wait behavior remains owning-seam proof.
These delivery results belong to R13; current editing-smoke acceptance is R15,
cache/overlap workflows remain R14, audio R12 and measured native performance R9.

The historical [raw preview record](../artifacts/performance/preview-final.json) and
[build log](../artifacts/performance/preview-final-build.log) contain a single
release software-decoder/worker run on 2026-10-12: 120 frames from one synthetic
1080p H.264 source at 30 fps, decoded to 1280×720 RGBA. Throughput was
**517.815 frames/s**; per-request latency was **1.753 ms p50**, **3.241 ms p95**
and **3.803 ms p99**. The 120 requests took 0.231743 seconds. The host was an
Intel Core i9-13900HX (32 logical CPUs), Linux x86_64 / Ubuntu 26.04.1 LTS,
Rust 1.97.1 and FFmpeg 8.0.1.

The run warms import/first-frame decoding before timing and includes subsequent
software decoding and the worker roundtrip. It excludes GPU
uploads, audio, native input, display presentation, random/cold seeks and
multi-layer workloads. It is not a claim of 517.815 fps native editing or a
cross-product performance comparison. Native presentation and A/V drift remain
unmeasured release gates; paused CPU/RSS observations are recorded below.

## Optimized distinct-video worker diagnostic

The completed [R14 findings](../artifacts/production-readiness/multilayer-preview-round1/FINDINGS.md),
[independent root verification](../artifacts/production-readiness/multilayer-preview-round1/root-verification-attempt-01.json)
and [raw-data audit](../artifacts/production-readiness/multilayer-preview-round1/audit-attempt-01.md)
cover 27 public-worker cases on Linux, using the exact optimized R14 media
libraries (`f74d6560` source, release graph matching `8b446cca`). Three trials
rotate request order across 1/4/5 distinct 320×180 H.264 sources on a 30 fps
grid, with fresh processes/services/asset IDs and no same-lane supersession.

Complete-batch milliseconds, **median of three per-trial p50s**:

| Distinct videos | Cold first batch | Sequential new frame keys | Random new frame keys | Same cached frame |
| --- | ---: | ---: | ---: | ---: |
| 1 | 26.113 | 0.257 | 32.104 | 0.257 |
| 4 | 119.718 | 0.764 | 134.510 | 0.256 |
| 5 | 147.947 | 163.942 | 165.827 | 0.256 |

The sequential four-to-five increase was approximately **214.6×**; all 96
measured five-video sequential batches exceeded 33.333 ms. This is an observed
worker-delivery limitation, not an exact decoder-start/dispatch explanation or
native playback fps. A later
[owning-manager regression](../artifacts/production-readiness/decoder-retention-round2/backend-root-verification.json)
reproduces bounded-pipe churn and validates a protected-subset correction;
it does not map exact starts/dispatch onto each older timed batch.

All 2,370 completions across 711 batches (693 measured, 18 warmups) passed
identity and full-byte checks against independent FFmpeg references. Successful
payloads were transient, so root/audit verification checks executed comparisons,
retained reference bytes and raw records rather than replaying every returned
buffer. Root checked 75 source files, both binaries, three release libraries,
206 fixture files, all distributions and no live recorded PID/TID identities.

Roundtrip timing includes 200 µs consumer polling; approximately 0.256 ms cached
results are not exact decode durations. Checks/logging between batches permit
prefetch. Fresh services do not flush OS page caches; power and host activity
were not controlled, and ordinary read-only work could coexist. Three cold
samples do not establish useful tail estimates. These results exclude paced
renewals/poster competition, same-asset different-time consumers, GPU/native
presentation, audio, 1080p/4K workloads and macOS/Windows runtime. Do not turn
timed-batch sums into sustained playback fps.

## Optimized R15 legacy/explicit/protected comparison

The [root-verified R15 run](../artifacts/production-readiness/multilayer-preview-round3/root-measurement-verification.json)
passed all 81 cases against exact source `dfbb53e9`, release `362ba180` and its
optimized Cargo dependency graph. It compares legacy, explicit-consumer and
protected-three-consumer requests on the same audio-free 320×180 distinct-video
workload. Medians of three per-trial five-video sequential batch p50s were:

| Mode | Complete-batch p50, ms |
| --- | ---: |
| Legacy | 185.490 |
| Explicit | 179.190 |
| Protected | 67.395 |

The ratio of legacy/protected medians is **2.7523×**. All 96 measured five-video
sequential batches in **each** mode exceeded 33.333 ms. Protected sequential
one/four-video medians were 0.254/0.763 ms and cached-five 0.257 ms; protected
cold-five/random-five were 162.488/172.400 ms. No substantive cold/random
benefit, native fps or real-time presentation claim follows.

Across all modes, 7,110 requests and 2,133 batches include 180 requests/54
warmup batches. Root recomputed 324 distribution fields (216 numeric, 108 null),
checked 270 fresh asset IDs/180 explicit consumer IDs and 206 matching fixture
files, and verified 2,387 sampled PID/start and 648 TID/start identities gone.
Sampling is not an exact decoder-start count. Full returned RGBA checks ran
while buffers were alive; records/reference bytes are audited, not replayed
successful payloads. The [completed independent audit](../artifacts/production-readiness/multilayer-preview-round3/audit-attempt-01.md)
agrees with build attribution, arithmetic and runtime comparison; all 3,040
observed process/thread identities, including setup, were gone. Protected mode
reduced this tiny five-video sequential median by approximately 64%, while
every measured five-video sequential batch stayed above the diagnostic budget.

The host remained shared; excluded validation/logging permits prefetch and
200 µs polling sets an observation floor. This excludes GPU/native/audio,
superseding/cohort changes, same-asset different-time streams and poster
competition. It tests public worker modes; the frontend setter/restart bridge
has separate evidence below. The earlier R14 baseline remains immutable.

## GIF regression evidence

A CI-profile diagnostic compared 60 requests for the same GIF before and after
retaining sequential GIF decoder state and reusing unchanged display intervals.
The raw [before](../artifacts/performance/GIF/before-ci.json) and
[after](../artifacts/performance/GIF/after-ci.json) records report approximately
21.15 and 674.22 requests/s respectively. A separate two-second paced worker
regression reproduced frame 8 when frame 19 was expected before the fix and
passed after it. Uniform source delays now expose their rational frame rate;
variable delays still use exact display intervals.

Both diagnostic records explicitly have `optimized: false` and exclude GPU
presentation. They establish the reproduced software-worker regression and its
fix, rather than a release/native playback rate or cross-product comparison.
The rebuilt CI-profile native GIF monitor was also captured at 2.000 seconds
showing its own frame-20 timestamp and 10 fps metadata; see
[native evidence](verification.md#native-linux-evidence).

The historical optimized [GIF record](../artifacts/performance/gif-release.json) and
[build log](../artifacts/performance/gif-release-build.log) measured 60 requests
for a 320×180 GIF with a native 10 fps display grid, requested at 30 fps. The
warm worker handled **2,755.856 requests/s** in 0.021772 seconds, with
**0.254 ms p50**, **0.744 ms p95** and **0.757 ms p99**. Requests inside the same
native display interval reuse pixels: request throughput is therefore not a
count of unique decoded GIF frames, native presentation fps or a GPU/audio result.

## Physical-output callback clock evidence

The [round-twelve healthy silence-only report](../artifacts/production-readiness/audio-device-probe/round12/report.json)
and [source/library identities](../artifacts/production-readiness/audio-device-probe/round12/build.json)
match source `2b43797d` and exact CI-profile components throughout the run;
[root verification](../artifacts/production-readiness/final/round12-physical-clock-root-verification.json)
independently checked zero PCM, phase results and exact owned PID cleanup.
Direct pinned rustc compiled the artifact with `opt-level=0`; this is callback
clock evidence, not an optimized audio performance benchmark. The public
`AudioService` opened inherited CPAL ALSA default output, stereo F32 at 48 kHz,
and read-only PipeWire 1.6.2 discovery attributed its stream to the default
AD107 HDMI hardware sink. The 20-second generated source had only verified zero
PCM. No route, volume or device configuration changed, and no audio was captured.

After priming, each of four phases sampled 21 monotonic clock positions over
about two seconds: **1.000387** timeline seconds/wall second at 1×,
**0.999697** after seek, **0.500121** at 0.5× and **1.999157** at 2×. Sync
immediately invalidated position, pause/stop kept clock position `None`, no
public runtime errors appeared, and the diagnostic and four observed FFmpeg
children stopped. Source, libraries and fingerprints remained unchanged.

The clock advances even when underrun silence is inserted. These measurements
establish callback-clock availability and slope on one actual default route;
they establish no native fatal output callback, audible quality, sustained
PCM/underrun correctness, hardware/DAC presentation timestamp, latency,
pitch correctness or A/V sync.

The historical [before-zero-gain report](../artifacts/production-readiness/audio-device-probe/before-zero-gain/report.json)
and [build identities](../artifacts/production-readiness/audio-device-probe/before-zero-gain/build.json)
belong to source `28259254` before the zero-gain fix. They measured the same
default route with mathematical silence: **0.999873**, **0.999276**,
**0.500545** and **1.998499** for the four respective phases. That earlier
report remains immutable, with its own source/library scope and the same
acoustic, underrun and A/V limitations.

Latest accepted editing-smoke evidence is [thirteen R15 native input checks](../artifacts/native/round15-editing-smoke/root-verification.json),
source `dfbb53e9` / binary `362ba180`: exact gesture reversals, clipboard, durable
save, stale-plan refusal and transport/settings, with five viewed 1440×900
captures and eleven sampled process identities gone. It establishes no
multi-owner/cache/wake timing, hardware GPU, audio or display-latency result.
Earlier R14 `f74d6560` / `8b446cca` [whole-project gesture/preview/narrow/cache checks](../artifacts/native/cache-culling-acceptance-round1/report.json)
passed with audio unavailable, with [independent identity/cleanup verification](../artifacts/production-readiness/final/round14-native-editing-root-verification.json).
No performance measurement or native fatal callback was exercised there.
The measurements below belong exclusively to
round nine's `b29ccb69` / `14435a4a`, 333-test checkpoint. Real-provider AI/GTK
remains historical `28259254` / `2c596cb0`. R15's [three native startup captures](../artifacts/native/round15-window-captures/root-visual-verification.json)
establish normal/compact presentation and separate Play/Speed controls, without
input, wake/cache, performance or audio proof. The user deferred macOS/Windows
execution. Their last verified [pack baseline](native-platform-tests.md) was R14:
51 native cases `not_run` and [57 offline guards passed](../artifacts/native/backend-preparation/platform-pack-round8-root-tests.log).
The pinned 64-case catalog passed 106 offline guards; all native cases remain
`not_run`, with [independent preparation review complete](../artifacts/native/platform-regressions-integration-round1/review-round2/REVIEW.md).
Ten fixture
projects have verified Linux startup observations, without timings,
as scoped in [verification](verification.md#platform-matrix). The
[renewed paced-demand diagnostic](../artifacts/production-readiness/renewed-preview-demand-round1/FINDINGS.md)
now has eighteen actual optimized R15 cases. All 3,301 returned demand frames
and 60 final fences validated, but continuously renewed five-owner demand
published nothing for two owners. A matched private four-versus-five-slot
experiment changed only one source byte: distinct advancing publications
changed from 13/13/12/0/0 to 82 per owner in three seconds. This identifies
capacity-sensitive decoder churn; it does not select a larger product budget.
The native R15 viewer reproduced the symptom with real Play/Pause input:
two tiles stayed on frame zero throughout forty screen reads, then all five
reached frame 119 after pause. Software Xvfb, ten reads per second and no
audio device establish neither presentation frame rate nor A/V acceptance.
The subsequent [playback-admission correction](../artifacts/production-readiness/playback-admission-round1/FINDINGS.md)
keeps four pipes and distinguishes advancing playback from immediate seeks.
The updated optimized build passes 538 workspace tests. Under the same
three-second five-owner demand, the immediate control returned 10/12/12/0/0
distinct frames and playback admission returned 23/23/22/12/12, with exact
pixel validation and all final fences. The updated native window showed
21/21/21/17/17 distinct stamped frames at forty finite screen reads, compared
with two entirely frozen tiles before. No tile went backwards or lacked a
readable frame code; all five reached frame 119 after pause. This establishes
delivery progress in those runs, not 30 fps or sustained multi-layer throughput.

The [earlier published-first read-ahead correction](../artifacts/production-readiness/publish-first-read-ahead-round1/FINDINGS.md)
passes 549 workspace tests and keeps the same resource budgets. Three matched
optimized trials use five independently encoded 1080p30 sources and 320×180
previews, with exact continuous per-owner pixel oracles. Median distinct frames
per owner during three seconds increase from **46/46/46/24/23** to
**63/63/63/50/39**. Overflow-owner request-to-poll p95 improves from
**140.46/130.18 ms** to **81.12/118.42 ms**; two protected-owner tails increase
about 4–5%. Later cold pictures incur additional queue time. The unchanged
native harness observes all five advancing on the rebuilt editor, without
establishing improved native presentation rate. Sustained full-resolution
performance, physical audio and hardware GPU acceptance remain open.

The [subsequent larger-preview comparison](../artifacts/production-readiness/high-resolution-preview-round1/FINDINGS.md)
passes all twelve exact-pixel cases. At 1280×720, median distinct source frames
per owner during three seconds change from **24/20/19/15/15** to
**26/24/24/16/15**. At 1920×1080 they change from **13/12/12/11/11** to
**15/14/13/12/11**. Full-resolution p95 ranges approximately 241–350 ms under
the R17 policy, with regressions on two owners. These outcomes do not meet
source-rate playback; the stronger small-preview gain cannot establish the
required sustained high-resolution performance.

## Virtual callback-stall recovery

The [round-ten reproduction](../artifacts/production-readiness/final/round10-virtual-stall-root-verification-v2.json)
froze raw callbacks at 51,200 frames with zero errors; public audio and Playback
remained at 1,045,333 µs for over 14 seconds. Failed earlier setups/controls are
retained, and the final absolute-route missing-server control refused startup
without a physical/default fallback.

On exact round-twelve CI components, the [new virtual report](../artifacts/production-readiness/audio-device-probe/virtual-failure-round6/report.json)
and [independent verification](../artifacts/production-readiness/final/round12-virtual-stall-root-verification.json)
show raw CPAL still stalled without an error callback, but `AudioService`
released its public clock, Playback continued monotonically and one unavailable
notice was delivered. The decoder disappeared naturally at approximately
1.81 seconds from session start, about 0.60 seconds after observed server loss,
before explicit stop at approximately 17.1 seconds. A second service session
also detected failure with five position reads omitted. All 15 recorded process
identities stopped without forced cleanup; owned absolute routes, zero PCM and
no `/dev/snd`/foreign sound access were independently checked. This single
virtual experiment is correctness evidence, not acoustic output, physical
hot-unplug, native fatal-callback, A/V or a performance guarantee.

Active-clock expiry retains four **largest observed** callback periods per
stream, with a 500 ms floor; later smaller buffers do not shorten that grace.
Paused clocks are excluded. An already-observed genuine expiry may win against
an overlapping reset, which does not recreate native output. The ordinary
sample callback adds one `Instant::elapsed` monotonic read and bounded atomics;
it adds no application lock, allocation, decoding or process operation. Its
optimized cost is unmeasured, so it is not a strictly atomic-only callback claim.
Heartbeat grace is not a universal cleanup bound: a blocked decoder still has
its separate ten-second read watchdog.

The [initial review](../artifacts/production-readiness/audio-device-probe/callback-liveness-source-review.md)
found an actual period/heartbeat publication race after the preliminary
353-test round-eleven build; it received no native acceptance. The ninth test
exercised the real between-stores seam and failed with eight others passing.
The [corrected nine-test log](../artifacts/production-readiness/audio-device-probe/liveness-period-race-after-fix.log)
and [review response](../artifacts/production-readiness/audio-device-probe/callback-liveness-review-response.md)
preserve the correction. Earlier red/green and failed setup/control evidence
remain diagnostic records rather than successful native proof.

## Native paused-window observations

### Accepted Linux round one

The [native report](../artifacts/native/performance-round1/report.json) and
[independent verification](../artifacts/production-readiness/final/round9-native-performance-root-verification.json)
identify source `b29ccb69`, optimized `aster` `14435a4a` and unchanged source/
binaries throughout. All 48 recorded process entries were verified stopped.
The host was an i9-13900HX, approximately 31 GiB RAM and NVIDIA RTX 4060 Laptop
GPU/driver 595.99.02. It remained shared with the person's desktop; the task
used owned fixtures/windows. Audio was disabled, so playback used wall time.
Later rounds added device-free fatal-clock/wake/notice/liveness regressions and
virtual-stall recovery. These performance figures were not repeated on round
twelve and do not measure the added callback clock-read cost.

Two rigs were measured: software Xvfb and NVIDIA-EGL headless Weston→Xwayland
with DRI3/Present. Neither had physical scan-out/vblank. Forced NVIDIA-only,
lavapipe-only and GL-only loader comparisons, per-PID GPU rows and driver thread
signatures strongly support NVIDIA Vulkan rendering in the default runs;
Aster did not log its chosen adapter. This does not establish hardware decoding.

Paused trials used six seconds of warmup followed by about 15.2 seconds, three
trials per case. No input, agent calls or screen polling occurred during those
samples. CPU sums editor/decoder process time as percent of one core; RSS sums
can double-count shared mappings and exclude GPU memory. PSS and the observed
210–220 MiB per-editor GPU rows remain in the raw report.

| Rig / paused workload | CPU median, % one core | Process-tree RSS range, MiB |
| --- | --- | --- |
| Xvfb, empty | 0.525 | 267.33–269.29 |
| Xvfb, one 1080p clip + text | 0.525 | 540.75–541.50 |
| Xvfb, 5,000 clips / 20 tracks | 0.460 | 556.90–566.86 |
| Headless Xwayland, empty | 0.394 | 237.19–237.75 |
| Headless Xwayland, one 1080p clip + text | 0.525 | 497.60–498.72 |
| Headless Xwayland, 5,000 clips / 20 tracks | 0.591 | 506.76–515.93 |

Three Openbox/Xvfb minimize trials confirmed hidden/unmapped state, unchanged
document/resources and an identical restored capture. Minimized CPU was
0.466–0.533% of one core, comparable to paused visible CPU. Post-restore click
acceptance was not recorded. Paused NVIDIA-path voluntary context switches
remained approximately 115/s versus approximately 2/s in a forced-lavapipe
diagnostic; attributing these recurring wakeups remains follow-up work.

The 5,000-clip project had 18 text and two video tracks; snapping/autosave were
off. XTest events were timed to the first differing XGetImage timeline read.
These are medians of observed framebuffer feedback, not physical-display
latency, GPU fences, per-operation tail guarantees or frame-time p99.

| Action | Xvfb median, ms | Headless Xwayland median, ms |
| --- | --- | --- |
| Zoom in | 40.66 | 14.54 |
| Scroll right | 39.81 | 12.86 |
| Selection press | 23.37 | 9.88 |
| Drag step | 23.63 | 10.11 |
| Drag release | 25.34 | 13.53 |
| Undo key | 40.25 | 16.80 |

Each successful held drag committed once and exact undo/redo restored the full
project. Smoothed wheel changes settled around 183–244 ms; first-change timings
do not represent completion. Limits/no-change counts and sample sizes remain in
the report. Live 5,000-clip inspections cost about 0.23 s each outside timed spans.

Seek timing read generated frame codes and matching timecode pixels. First
visits opened new FFmpeg processes; cached revisits did not. Timeline first-visit
medians were 204.02/186.88 ms on Xvfb/headless Xwayland; cached revisit medians
were 54.34/9.22 ms. Fresh-position held-scrub medians were 188.89/165.05 ms.
All 16 latest-wins bursts ended at the requested frame/label without a later
older frame. Source-preview/editing state remained unchanged.

Thirty-second framebuffer playback observations fitted approximately 30 frames/s
against wall time. They cannot identify dropped frames or physical presentation;
capture polling can miss replacements and itself consumed substantial CPU.
Follow-up source playback without the image observer used 119.4%/70.8% of one
core for the editor/decoder tree on Xvfb/headless Xwayland. Early playback CPU
figures that omitted a newly started decoder must not be used. The report keeps
observer/no-observer controls and raw samples, not an A/V or source-rate display
guarantee.

Other retained caveats: no creation-time hash was stored for the generated video,
although in-run hashes matched; the narrow "selected" image is actually a ruler
seek; Shift+Right harness auto-repeat prevents a step-size claim from those
samples. Real-provider AI/GTK was not repeated. Physical display/audio, long
sessions, other drivers and target-platform execution remain outside this run.

### Historical core checkpoint

The original core checkpoint's optimized [idle record](../artifacts/performance/native/idle.json)
contains two Linux X11 observations on the same i9-13900HX/Ubuntu host, each
with a 1440×900 visible paused window, six seconds of warmup and about twelve
seconds of sampling. Process sets were stable during each sample.
These observations predate the native AI and protected-recovery changes; they
do not identify the current binary or measure an active model request.

| Paused case | CPU, percent of one core | Peak process-tree RSS | Processes | Sample |
| --- | --- | --- | --- | --- |
| Empty project | 0.980% | 226.10 MiB | 1 | 12.241 s |
| Loaded 1080p fixture | 0.901% | 503.54 MiB | 3 | 12.206 s |

The loaded figure includes retained decoder children. Summed process RSS can
count shared mappings more than once and excludes GPU memory. The graphics
driver was not established for these samples, so the separate Wayland lavapipe
run does not identify their driver. Captures are
[empty](../artifacts/performance/native/empty/paused.png) and
[loaded](../artifacts/performance/native/loaded/paused.png).

The agent session descriptor became ready in approximately 0.24 seconds in each
run; that measures descriptor availability, not display-ready startup latency.
These are two host-local observations, not repeated-run CPU/memory distributions
or proof of minimized behavior, wakeup budgets, GPU activity or native A/V timing.

## Interactive path

The [R13 native baseline](../artifacts/native/cache-culling-round1/baseline-root-verification.json)
confirmed the long overlapping clip was not drawn in the scrolled view, though
clicking the empty lane selected it,
and observed all 480 distinct owned posters while browsing. It did not measure
texture retention or GPU memory. The subsequent
[R14 source/build checkpoint](../artifacts/production-readiness/final/round14-build-record.json)
(`f74d6560` / `8b446cca`, 397 tests) corrects
[retention/admission](../.scratch/core-editor/issues/09-thumbnail-retention.md) and
[overlap culling](../.scratch/core-editor/issues/11-overlapping-timeline-culling.md).
[Seven failing regressions/thirteen controls](../artifacts/native/cache-culling-round1/phase-a-root-verification.json)
became [32 passing cache/timeline tests](../artifacts/native/cache-culling-round1/phase-b-executed-tests.json);
the [frozen review](../artifacts/native/cache-culling-round1/phase-b-review-response.md)
found no blocking defect. The [accepted R14 native report](../artifacts/native/cache-culling-acceptance-round1/report.json)
now shows long-clip fill 1.0 normally and narrowly, versus R13's 0.0016 while
remaining selectable in both. Its fitted normal timeline showed 256/480 own
posters, with all 480 paged/revisited; narrow readable lanes covered 320 while
the third was clipped and fitted centre pixels were inconclusive. One notch
freed 43 normal/21 narrow new pictures without more input, not an exact repaint
count. Five failed harness attempts remain retained. These observations do not
measure private texture counts or GPU memory. After a startup notice dismissed
itself, five-second idle diagnostics read 1–3 CPU ticks; initial 25–28-tick
samples included that dismissal. Neither is a repeated performance benchmark.

The current poster policy retains at most 256 posters and 64 MiB of logical
RGBA texels, whichever binds first. It excludes upload staging, driver memory
and viewer-layer textures. Demand follows visible library rows, the shared
timeline interval/row/height predicates, and inspected/source assets. Unit
controls cover one paused repaint when room returns and settling under steady
oversubscription. Native inspector retrieval after eviction passed; source-monitor
poster retrieval remains indirect because its solid still also matches a layer.
The borrowing,
allocation-free `Track::clips_overlapping` query partitions starts and filters
ends for half-open ranges. Consumption costs O(log n + k), where k is the
start-qualified prefix, not just returned clips; no interval index was added.

The [dispatch-policy task](../.scratch/core-editor/issues/10-preview-dispatch-policy.md)
now has a selected design: explicit poster demand, retained first-admission
order, and at most two interactive bypasses before global-oldest dispatch.
R14 does not contain the selected correction. The earlier 22-focused/76-media
checks and the 96-test consumer stage remain historical. Follow-up source now
passes [112 media tests and current package-only denied-warning Clippy](../artifacts/production-readiness/decoder-retention-round2/backend-root-verification.json).
The [second backend review](../artifacts/production-readiness/preview-consumer-backend/review-round2.md)
confirms corrected GIF-resize accounting and unrelated byte-blocked retirement:
cleanup releases the queue mutex and publication rechecks currentness. Shared
portrait pixels and same-consumer pipe continuity retain owning controls.
These changes now belong to the [521-test R15 source/release checkpoint](../artifacts/production-readiness/final/round15-build-record.json),
with whole-workspace/source-target checks and [exact optimized dependency attribution](../artifacts/production-readiness/final/round15-release-libraries.json).
Native R15 dispatch/multi-owner/cache acceptance remains pending; the verified worker comparison above
does not establish native timing. The changes are
outside the measured R14 build. [R14 binaries/libraries were retained](../artifacts/production-readiness/final/round14-retained-release/record.json).
The conditional bound
of 96 dispatch opportunities establishes neither elapsed latency, successful
publication nor throughput. The [optimized release-library graph](../artifacts/production-readiness/final/round14-release-libraries.json)
and [original distinct-asset preparation record](../artifacts/production-readiness/multilayer-preview-round1/prepared-root-verification.json)
remain historical provenance for the completed diagnostic above. The R14
measurements do not verify or measure the follow-up scheduling correction.

R15 introduced an optional normalized three-consumer protected subset
within its four retained video pipes. Two actual manager failures (five streams
making ten instead of seven total starts; eight making sixteen instead of thirteen)
and nine controls became eleven passes. A real worker's generation/full-pixel
control counted eight starts; a later warm-tag-pruning/maintenance extension
kept that total after an uncached next frame. The
[production review](../artifacts/production-readiness/decoder-retention-round2/backend-review-round1.md)
found no P1/P2 blocker; that later test extension was separately executed,
not included in the review. `clear()` is source-reviewed rather than isolated
by the counter alone. R15 adds frontend selection controls; the later bridge
below observes the nonempty call and restart reset, without native timing.
The optimized worker comparison above does not exercise the frontend call.
That four-pipe limit and 67-lane hint/latest union describe R15 resource policy,
not throughput. The latest playback checkpoint raises the pipe limit to five
while preserving the three-consumer hint and queue/cache limits.

The [compatible legacy round-two probe](../artifacts/production-readiness/multilayer-preview-round2/PLAN.md),
[static audit](../artifacts/production-readiness/multilayer-preview-round2/root-static-verification.json)
and [driver hardening](../artifacts/production-readiness/multilayer-preview-round2/root-driver-hardening.json)
remain prepared/unrun. The [three-mode comparison](../artifacts/production-readiness/multilayer-preview-round3/PLAN.md)
prepared 81 `not_run` cases; its [static audit](../artifacts/production-readiness/multilayer-preview-round3/root-static-verification.json)
checked byte copies, hashes, Python syntax and inventory only. Its later actual
81-case execution passed root verification and the completed independent audit
above. Those earlier static records remain preparation evidence, not runtime proof.

The [consumer/controller source review](../artifacts/production-readiness/preview-consumer-identity-review-response.md)
identified a transition identity gap in R14. [Its follow-up correction](../.scratch/core-editor/issues/12-preview-consumer-identity.md)
now has [152 passing desktop tests](../artifacts/production-readiness/preview-consumer-phase-b/root-green-verification.json),
including paint identity and transport pointer controls. Fixture-font failures
and a real two-pixel Speed-slot failure are retained before correction;
[formatting followed the green run](../artifacts/production-readiness/preview-consumer-phase-b/root-format-source-record.json).
The later 177-test desktop proof records package-only denied-warning Clippy;
final transition/compact-input/performance acceptance remains pending. R14's known 800×600
Play/Speed overlap therefore retains its scope.

[Viewer textures](../.scratch/core-editor/issues/13-viewer-texture-retention.md) in R14
still lack a byte/count budget; 120 painted frames is not a paused wall-time
bound, and poster limits do not constrain them. The selected next policy uses
64 MiB logical RGBA, request-time `max(kept, granted_box)` reservations, a common
active-set quality cap and eight inactive layers in spare room. The
[173-test phase-A run](../artifacts/production-readiness/viewer-working-set-round1/phase-a/root-verification.json)
reproduced thirteen budget assertions, with 160 controls passing (all prior
152 plus eight new controls), no compile/fixture failures and no ignored tests.
The [phase-B proof](../artifacts/production-readiness/viewer-working-set-round1/phase-b/root-green-verification.json)
has 177 passing desktop tests and clean all-target package Clippy at that source, including
twenty-five resource tests with the original twenty-one unchanged and added
resize/audio/notice/failure controls. Logical budget accounting is implemented;
GPU/upload/staging bounds and native idle behavior are not measured. The
[completed review](../artifacts/production-readiness/viewer-working-set-round1/review-round1.md)
found no P1/P2 ledger defect but confirmed a paused persistent-error cycle.
The new [public-worker liveness filter](../artifacts/production-readiness/preview-request-liveness-round1/phase-a/root-red-verification.json)
passed eleven controls and failed three assertions; all earlier 177 tests were
filtered out. Missing clip/source demands reasked across four unchanged paused
frames, and playing owners 32–39 received no picture in eighty controlled
results. Exact PNG identity/pixels, retries, held picture/shrink and paused
forty-owner controls passed without fixture/compile/timeout failures; owned
temporary storage was empty. This proves the controlled request behavior,
not native eframe wake delivery, cycle rate, video/GPU timing or presentation.
[Failure/admission correction](../.scratch/core-editor/issues/16-preview-request-liveness.md)
then passed [21 focused and all 198 desktop tests plus package lint](../artifacts/production-readiness/preview-request-liveness-round1/phase-b/root-green-verification.json).
Paused unchanged failed time/box is suppressed; changed time/bounds/source and
recovery retry, held/oversized failure and finite forty-playing-owner PNG
admission have controls. The full R15 checkpoint above has 521 passing tests,
source-target lint and optimized linking. The advancing-demand and native
five-video diagnosis above now observes slow/in-flight delivery failure;
corrected native wake/idle, poster bursts, GPU and real-time delivery remain
unverified.

The [completed correction review](../artifacts/production-readiness/preview-request-liveness-round1/review-round1.md)
found no P1/P2 behavior defect but identified P2 evidence gap G1. Seven stable
at-most-three VIDEO granted/drawn selection controls pass; six use no media
service, and the real still-service control sends an empty set. Nonempty
frontend effects were not observed by those tests. Six prepared bookkeeping
tests were declined, not integrated or run. The [actual canonical frontend/service bridge](../artifacts/production-readiness/frontend-retention-bridge-round1/root-verification.json)
and [independent review](../artifacts/production-readiness/frontend-retention-bridge-round1/review-round1.md)
now verify a nonempty hint and its reset after `cancel`/`ask_again` at the frozen
R15 source. Four controlled cases measured 7/10 and restart 14/20 decoder starts,
with 60 transient full-pixel checks across twelve frames; all 51 logged decoders
were gone. These are process-start observations, not timing or GPU/texture
readback; the worker is deliberately held through composition. This artifact
probe adds no standing guard or suite tests. Retire/clear/Closed, source/cut/
reorder and playing remain unproved. Backend hint behavior has its separate
112-test proof. Time-less errors can permit
one extra retry; playing offline video still fails for new times and renews
notices, and restored same-path media while paused needs a seek/resize retry.
No complete missing-media UX or native performance is claimed; the optimized
public-worker comparison above does not exercise the frontend setter.

Measure command validation, timeline hit testing, preview scheduling, decode,
texture upload, composition, audio and display latency separately. A fast pure
model test does not establish scrubbing latency; an FFmpeg decode rate does not
establish native playback throughput.

Keep filesystem, codec and network calls outside the frame loop. Worker queues
are bounded, frequent requests coalesce and superseded seeks cancel. Use
request generations so a slow older frame cannot replace a newer scrub result.
Publish owned results instead of holding decoder locks during rendering.

Virtualize media-library entries and timeline rows. Query clips intersecting the
visible time range rather than visiting every clip for every frame. Reuse
thumbnail textures and layout; invalidate them by asset revision, size or
display scale. Decode at preview resolution unless a workflow requires the
original, and retain originals for final-quality processing.

## Resource budgets

Assign a byte budget to decoded frames, thumbnails, waveform storage and GPU
textures, not just an item count. A single 8K frame can otherwise defeat a
reasonable-looking cache length. Bound worker/subprocess concurrency and retain
reservations until cancellation or teardown actually completes.

Budget history separately from render caches. Repeated edits on a large project
must not clone all media buffers or grow unchecked. Referenced source files and
immutable metadata are shared where appropriate; undo policy preserves the
documented history and checkpoint contract.

Idle native windows should sleep between input and relevant deadlines. Pausing
or minimizing a preview stops unnecessary decode and redraw work; queued saves
and essential import work may finish. Repaint notifications coalesce and
unchanged worker polls do not wake the window.

Motion analysis has a lane of its own (`editor-media/src/analysis.rs`): one
thread, started when a region is first tracked, with one FFmpeg process at a
time and at most four jobs waiting. It reads luma no larger than 384 pixels a
side, holds one such frame and one patch, and keeps a centre per frame until
the path is thinned to 2048 keys at most. A job is given up by a flag and by
killing its process; its answer, and the one progress figure that waits to be
read, are dropped with it. The window is woken once for each whole percent and
once for the answer, and an idle lane wakes nobody. How long a job takes has
not been measured on an optimized build.

Stabilization measures on the same lane and thread, queued with tracking: the
whole video is read once as luma no larger than 384 pixels a side, two frames
and their three-level pyramids are held, and four numbers a frame are kept
(refused past 1,048,576 frames). The path is derived data: it is written to
one file a media path under the user's cache directory, bounded to 64 MiB and
256 files with the least lately read removed first, and read back on a second
thread of the lane (sixteen questions waiting at most) so that a reading never
waits behind a measuring. The window holds eight paths and about two million
frames of them at most. A layer whose path is not held is drawn uncorrected in
the same frame; a held path costs one interpolation and six floats in the
place pass's uniform, and no further pass. On the unoptimized test build a
320x240 fixture of 60 frames took 15 ms a frame, FFmpeg included; an optimized
build and real footage have not been measured.

Sound analysis has a lane of its own beside that one
(`editor-media/src/sound.rs`), so a long tracking or stabilization run holds
up no waveform: one thread reading sound with one FFmpeg process at a time
and sixteen jobs waiting at most, and a second thread that only reads the
peak cache, sixteen questions waiting at most. A file's first sound stream is
read whole, once, as one channel at 8 kHz in pieces of 32 KiB, and each 80
samples become four bytes: a hundred buckets a second, about 1.4 MB an hour,
refused before any process past 2,097,152 buckets (5.8 hours, 8 MiB). The
peaks are derived data: one file a media path under the user's cache
directory, bounded to 64 MiB and 512 files with the least lately read removed
first. A newer request takes the place of an older only when both are of the
same kind for the same media; a job is given up by a flag and by killing its
process. The window is woken once for each whole percent and once for the
answer, and an idle lane wakes nobody. The lane is started by the first tile
on an audio lane that is drawn. Only the tiles drawn ask: the cache first,
then the sound, each once for a media file, and a failure is remembered. The
window holds 32 MiB of peaks at most (`src/peaks.rs`); those least lately
wanted make room and never those of a tile on screen, so media with no room
beside what is on screen is not asked about until it has. The one exception
is work: Duck under speech wants one media file's peaks at a time whether a
tile draws them or not, they are asked for whatever the room, and when they
come they may take the room of on-screen peaks, least lately wanted first.
Those tiles wait while the work holds its peaks and ask again once it lets
them go, so a duck always finishes with more voice files than the map holds.
A duck asks the lane from the window's logic pass, which also runs while the
window is hidden, so it reads on unseen. A file whose sound a duck could not
read stays failed when the duck lets it go, as one a tile asked for does: it
is not read or reported again. A sound read asked for by a tile that has not been drawn for two frames is given up, so the lane
reads for what is in view; what it had read of the file is lost, and the file
is read from its start when the tile is drawn again. Telling held peaks from
those of a relinked file compares the path in place and builds nothing for a
tile drawn. A waveform is one mesh a tile, a column for each
point of the tile in view; a column reads the least and greatest of the
buckets its stretch of source touches, so a frame at the widest zoom reads
every bucket of the media in view. That cost is not measured.

## Advanced editing budgets

These are the bounds the [advanced-editing](../.scratch/advanced-editing/spec.md)
code enforces and the few figures measured for it. Every figure is from one
Linux host (i9-13900HX, RTX 4060 Laptop, Ubuntu 26.04.1) on the unoptimized
`ci` profile; none is an optimized-build, macOS, Windows or playback-rate
measurement. Logical bytes count what was asked of the device or kept in main
memory at four bytes a texel; what a graphics driver allocates for it was not
measured.

### Compositor

A plan of plain layers costs what it did before: one quad a layer and no
compositor state. Any other run of layers is drawn off screen by
[`src/compose`](../src/compose/mod.rs).

| Bound | Value | Where |
| --- | --- | --- |
| `ComposeBudget::PRODUCTION` | 96 MiB logical | Intermediate and published canvases and uploaded pictures at 4 bytes a texel, lookup tables at 8 bytes a texel (the table's colours and its texture), and the uniform records. Separate from the 64 MiB `ViewerBudget` and the poster budget |
| Canvases a frame | 4 (`MAX_CANVASES`) | The runs that text clips part; a run past the fourth keeps the mesh path |
| Passes a canvas | 1024 (`MAX_PASSES`) | A job of more is refused and drawn by the mesh path |
| Lookup tables a frame | 4 (`MAX_LUTS`) | A frame naming more refuses all of its jobs |
| Intermediate targets | 2 to blend between, plus 1 to 3 a layer's chain goes between; the two clips of a transition take the sum of both chains, 3 at least | All canvases of a frame share one size |
| Longest canvas side | The device's `max_texture_dimension_2d`, and 16384 at most | |

**Canvas sizing.** A canvas is composited at the screen pixels it covers, a
texel to a pixel in the canvas's shape (`target_size`), so a composited layer
is resampled once, as a quad is. It steps down only when the pictures, the
targets and the outputs of the frame do not fit the budget at that size: to
the largest of 1920, 1280, 960, 640, 480, 320 and 160 pixels on the longer
side that fits (`CANVAS_STEPS`). If not even 160 fits, every job of the frame
is refused, the mesh path draws its layers and the viewer says "Effects not
shown". The price of exact sizing is that a window dragged through sizes makes
and draws its canvases again at each one; that cost was not measured.

**Nothing to redraw.** A canvas whose size, layers, placements, picture
serials, plates, tables and stabilization sampling are what they were is
skipped, and the compositor never asks for a repaint. `resolve` reads no
clock, so a paused plan with grain, shake or glitch composes once.

**Kept frames.** A layer a canvas shows keeps the decoded frame the worker
produced, shared and not copied, and no texture of its own: it is uploaded
once, by the compositor. The viewer asks for frames to be kept only while the
plan has something to composite; a plan that stops being plain while paused
asks for the frames it shows once more. Kept frames live in main memory
outside `ViewerBudget`, which counts each layer's texels once, and their
upload is counted in `ComposeBudget`. A clean plate is asked for in the same
viewer budget as the layers and is kept as a decoded frame only.

**Idle release.** Canvases, uploaded pictures, lookup tables and kept frames
are given up three seconds (`COMPOSE_IDLE`) after the last frame that had
something to composite, on the clock, with one scheduled wake to do it and
none after. They are also given up after 120 painted frames without a job
(`LAYER_IDLE_FRAMES`) and when the project is replaced. Whatever the device
refuses while pipelines are built or a canvas is drawn (validation, an
internal failure, memory) is caught: everything is released and the mesh path
draws from then on.

**Lookup tables.** A `.cube` file is read by the media worker, never in a
frame: 2 to 65 points an edge and 32 MiB of file at most, eight reads waiting
and four answers unread at most. Until a table has come the effect changes
nothing.

**Transitions and decoders.** The two clips of a transition each keep a layer
and a decoder. Six live video consumers (a transition on each of three stacked
tracks) are one more than the five retained decoder pipes and three more than
the protected subset: the unprotected three start an FFmpeg process for each
next frame until the transitions end. On 64x32 lossless fixtures every frame
was pixel-exact with 6, 9, 10 and 10 process starts a round; no frame rate was
measured at a real size. The incoming clip is not decoded ahead of its
transition.

Paused CPU of the editor and its decoder children over five seconds, after a
composited frame was parked:

| Run | State | CPU, % of one core |
| --- | --- | --- |
| [Wave 2](../artifacts/advanced-editing/wave2/README.md), one sample each | Masked, blended, keyed layer at 1440x900 | 0.2 lavapipe, 0.2 GL on llvmpipe, 0.8 NVIDIA |
| [Wave 3](../artifacts/advanced-editing/wave3/README.md), `idle.jsonl` | Lane, graph editor, library tabs, clean plate, 760x560 | 0.0 to 0.2 in six samples; 3.18 in one sample taken seconds after a run of agent commands and not repeated |
| Wave 4, session captures | Tracker paths drawn; stabilized clip | 0.0; 0.17 and 0.0 |

These are single samples of process CPU on a virtual display. They do not
count wakeups or repaints, and the three-second release with its one wake was
not observed in the running app.

Not measured: the cost of any pass or effect stack, the per-frame upload
during playback, the one-time pipeline build on the first frame that
composites, the scan of each track `resolve` makes every painted frame on a
large project, GPU memory, and the frame rate of any effect, transition or
stabilized clip in playback.

### Speed and reverse

A decoder pipe runs forwards only. A request a little behind its lane's pipe
starts the replacement up to 24 frames early (`REVERSE_GROUP_FRAMES`), or two
frames short of what half the frame cache holds, and keeps what it reads on the
way in the 64 MiB frame cache: runs of 24 at 640x360, 14 at 960x540, 2 at
1920x1080. A frame too large for a run of two is stepped one at a time as
before.

While one lane steps backwards, the preview worker starts the run before the
one being shown after it has published a frame (`DecoderPool::look_behind`).
An `aster-look-behind` thread owns that FFmpeg process and reads only as many
frames as the pool has room to take, so the frames it holds, the pending
frames and the cache together stay inside `FRAME_CACHE_BYTES`. It takes a
decoder slot only when one of the five is free and gives it up when a request
needs one. A step forwards, a jump, another source or size, a project change
or `retain_lanes` ends it and drops what it read; a frame request that its
run covers waits for it instead of starting a decoder, and stops waiting when
the request is no longer current. A walk that is paused keeps one blocked
FFmpeg process until one of those happens.

| Workload | Before | After |
| --- | --- | --- |
| 30 single steps back through 640x360 H.264 with one keyframe, from frame 60, at `DecoderPool` | 30 decoder starts, 55.5 to 60.8 ms a frame | 3 starts, 6.4 to 6.8 ms a frame (no look-behind in this test) |
| 90 thirtieths of a second walked back at 30 a second through 1920x1080 H.264 with one keyframe, previewed at 960x540, at `DecoderPool`, unoptimized | No look-behind: 45 or 46 frames shown, 7 starts on the frame path, longest step 242 to 269 ms | 72 to 75 frames shown, 2 starts on the frame path (the seek and the first run) and 6 behind it, longest step after those 35 to 63 ms |
| Reversed 10 s clip of 1920x1080 30 fps H.264 (keyframe every 15 frames, `hd1080.mp4`) playing at 1x in the release build, 1440x900, RTX 4060 under headless Weston, screen sampled at 60 Hz for 6 s, three windows each | 144, 145 and 147 distinct pictures; 10 or 11 holds of 133 to 167 ms, one every 0.533 s | 180 distinct pictures in each window; longest time on one picture 50, 50 and 33 ms; no hold over 60 ms |

The tests in [decode.rs](../crates/editor-media/src/decode.rs) print the first
two rows as `MEASURE:` and assert the start counts, the five-decoder limit and
the cache bound; the times are from a few runs on one machine. A third test
walks 140 frames of 64x32 back in runs of 24, letting each look-behind finish
before the next step, and asserts that no step after the first two starts a
decoder on the frame path and that at least five runs were decoded behind it. Those are not
measurements through `MediaService`, the viewer or the window.

The last row is: the pictures and holds are counted from hashes of the viewer
area ([figures](../artifacts/advanced-editing/final-fixes/reverse-pacing.txt));
a paused control window gave one picture. It is one fixture, one adapter, one
canvas size and a plain clip. Not measured: reversed playback with an effect
stack or a transition, sources with distant keyframes (a run that takes longer
to decode than its frames last on screen still holds the picture, as the
second row's longest step shows), a preview frame of 1920x1080 or larger
(runs of two, or single steps), more than one reversed lane at once (one lane
has the look-behind at a time), other adapters, a windowed session and a
physical display. Reverse preview still costs one process start for each run.
Nine stills of a reversed clip playing on lavapipe showed the expected source
frame at each timecode
([wave 3](../artifacts/advanced-editing/wave3/README.md#reversed-playback)).

Preview sound follows gain keyframes and splits a clip's chunk at the time
map's segment boundaries. It plays straight forward stretches and is silent on
a freeze, a reverse and a ramp.

### Analysis lane and stabilization cache

The two paragraphs under [Resource budgets](#resource-budgets) state the
lane's bounds. In one table:

| Bound | Value |
| --- | --- |
| Lane threads / FFmpeg processes at a time | 1 measuring thread and 1 cache-reading thread / 1 |
| Jobs waiting / cache reads waiting | 4 (`ANALYSIS_QUEUE`) / 16 (`STEADY_LOADS`) |
| Luma read | 384 pixels a side at most (`ANALYSIS_SIDE`) |
| Tracker path | Thinned to 2048 keys at most; one `SetTracker` |
| Stabilization, frames a video | 1,048,576 at most (`MAX_STEADY_FRAMES`), four numbers a frame |
| Stabilization cache on disk | 64 MiB and 256 files, least lately read removed first; `<user cache>/stabilization`, or beside the settings file when `--settings` names one |
| Stabilization paths held by the window | 8 paths and about two million frames |
| Crop suggestion | Samples 2048 frames of a span at most |

The desktop runs one tracking and one stabilization measurement at a time; a
newer request gives the older up, although the lane itself queues four. A job
is given up by a flag and by killing its process.

| Measurement | Result | Conditions |
| --- | --- | --- |
| Tracking accuracy | 125 keys within 1.8 source pixels of the fixture's box | FFmpeg-generated moving box, MPEG-4, 1280x720, 30 fps, 5 s |
| Tracking cost | About 220 to 240 % of one core while tracking | Native run on lavapipe, FFmpeg included |
| Stabilization residual | 7.52 px RMS before, 0.10 px after | FFmpeg fixture, 320x240, 60 frames |
| Stabilization residual | 2.379 px RMS before, 0.165 px after; the test asserts under 0.2 px | Synthetic shake |
| Stabilization cost | 15 to 16 ms a frame, FFmpeg included | The 320x240 fixture |
| Native marker step | 15.4 / 12.7 screen px RMS before, 0.58 / 0.00 after | Xvfb 1440x900 on lavapipe; the remainder is the fixture's pan |

No real camera footage, optimized build or other host was measured, and the
native stabilization figures predate the estimator's second fit guard.

## Audio editing budgets

The bounds the [audio-editing](../.scratch/audio-editing/spec.md) code
enforces. The figures are from one macOS host (Apple M1, macOS 26.5.2, Rust
1.97.1, Homebrew FFmpeg 8.1.2) through each feature's own tests on the `dev`
test profile (opt-level 1, dependencies at 2) unless a row says `ci`, which is
unoptimized; none is a release-build, Linux, Windows, real-device or
real-recording measurement, and nothing was listened to. No CPU cost of a
voice, a filter, the level stage or the limiter has been measured, nor
underruns.

### Agent text

| Bound | Value |
| --- | --- |
| Planner instructions / provider schema | Under 6 KiB (`MAX_SYSTEM_BYTES`) / under 23 KiB (`MAX_SCHEMA_BYTES`), 29 KiB between them on the Claude command line and under 31 KiB once Windows escapes the schema's quotes; the runtime guard refuses a schema at its cap before launch |

| Measurement | Result | Conditions |
| --- | --- | --- |
| Sizes | Instructions 5,377 bytes (767 left), schema 21,859 bytes (1,693 left) | Printed by `cargo test -p editor-agent --locked --lib -- --nocapture planner::tests` on 2026-10-14; Windows command-line headroom not measured |

### Sound lane and peak cache

| Bound | Value |
| --- | --- |
| Lane threads / FFmpeg processes at a time | 1 reading sound (`aster-sound`) and 1 reading the cache (`aster-sound-load`) / 1 |
| Jobs waiting / cache reads waiting | 16 (`SOUND_QUEUE`) / 16 (`PEAK_LOADS`) |
| Sound read | The first sound stream, one channel, 8 kHz, 16-bit, 32 KiB a read under the 10 s read deadline |
| Peaks | 100 buckets a second (`PEAKS_PER_SECOND`) of 4 bytes; 2,097,152 a file at most (`MAX_PEAK_BUCKETS`, 5.8 hours, 8 MiB) |
| Peak cache on disk | 64 MiB and 512 files (`PEAK_CACHE_BYTES`, `PEAK_CACHE_ENTRIES`), least lately read removed first |
| Peaks held by the window | 32 MiB (`MAX_HELD_PEAK_BYTES`): four files at the bucket bound; least lately wanted evicted, never a tile on screen except for a duck's one file of work |
| Duck under speech | One voice file's peaks held at a time, let go when its spans are taken; at most 4,096 spans a file (closest merged); 256 volume keys a clip (closest dips merged). The gate walks each voice clip over the ducked clips' time in 10 ms steps on the UI thread, one file a frame; that cost is not measured |
| Waveform drawn | One mesh a tile, four vertices a point of the tile's width in view |
| Beats | Found in kept peaks on the cache-reading thread, in its queue (`PEAK_LOADS`), with no process; a cache read waits behind it. 16,384 beats a grid at most (`MAX_BEATS`). About 13 bytes a bucket while it runs, the peaks included: 26 MiB at the bucket bound, counted and not measured |
| Beat grids held by the window | 8 media files (`MAX_HELD_GRIDS`), 64 KiB each at most; the least lately shown dropped, and none asked for while 8 are on screen or on their way. Asked for only of media whose beats are shown, by a tile drawn, once its peaks are held. Ticks and snap candidates are the beats in view, recomputed each frame by binary search in the grid; that cost is not measured |

| Measurement | Result | Conditions |
| --- | --- | --- |
| Reading a file's peaks | 0.98 s on the `dev` test profile, 1.64 s on the unoptimized `ci` profile, FFmpeg included | Two hours of silent mono 8 kHz FLAC (1.3 MB, 720,000 buckets), one run each |
| A late sound stream | First loud bucket 100 of 301 | 3 s MP4, AAC tone given a 1 s offset; ffprobe reports the stream at 0.978 s |
| Finding beats at the bucket bound | 0.49 s on the `dev` test profile, 4.11 s on the unoptimized `ci` profile | 2,097,152 buckets of synthetic clicks at 120 BPM, the peaks already in memory, one run each |

Silence in FLAC is the cheapest sound there is to decode: the first row says
what the reduction and the pipe cost, not how long a real recording of that
length takes. Not measured: compressed speech or music, a file at the bucket
bound, a slow disk, memory while reading, and the time to a first waveform.

### Loudness measurement

A span of a file is measured when asked, on the sound lane, by one FFmpeg
process through the clip's audio effects and the `ebur128` filter; it writes
nothing and nothing is kept. It holds the lane while it runs, so waveforms
wait behind it, and the first one in a session waits for the filter probe.

| Bound | Value |
| --- | --- |
| Process | 1, `-threads 1`, output to `null`; each read of its `-progress` lines (twice a second) under the 10 s read deadline |
| Replacement | A newer request for the same media and the same span stops the older; other spans and the media's peaks are left alone |
| Effects | Run at 48 kHz behind an `aresample`, written for two channels, as a voice's chain at that rate; none and no `aresample` when the clip has none to run |
| Channels | Summed to stereo (`aformat`, FFmpeg's default matrix, as a voice's `-ac 2`) before the meter, so a 5.1 or mono source reads as a two-channel output plays it; checked against a voice's own output for 5.1 and mono tones on the design host |
| Report | The last 8 KiB of FFmpeg's error pipe (`STDERR_TAIL`), waited for at most 250 ms once the process has ended |
| Desktop | Normalize loudness keeps at most 64 clips waiting (`normalize::MOST_WAITING`); clips of one span with the same enabled audio effects share one request, one of the same span with other audio effects waits for that answer, and clips the full lane turned away are asked again in the frames after, which the lane's own answers wake |

| Measurement | Result | Conditions |
| --- | --- | --- |
| Twenty minutes of sound | 2.54 s through the lane, FFmpeg included and the probe already done; 2.50 s for `ffmpeg` alone, 0.19 s without `peak=true` | Silent mono 8 kHz FLAC, `dev` test profile, one run each |
| Ten minutes of sound | 2.77 s for `ffmpeg` alone | Stereo 48 kHz FLAC of white noise, one run |

Nearly all of the time is the true peak, and it does not fall with the
source's rate: twenty minutes of mono 8 kHz cost about what ten of stereo
48 kHz do. Not measured: compressed speech or music, a span through several
effects, and a measurement on another host.

### Save audio as a file

A span of a file's sound is written when asked, on the sound lane, by one
FFmpeg process; it holds the lane while it runs, so waveforms and loudness
measurements wait behind it. Its cost is the decoder's and the disk's: the
WAV it writes is as large as the span at its own rate and channels.

| Bound | Value |
| --- | --- |
| Process | 1, `-threads 1`, `-nostats`; each read of its `-progress` lines (twice a second) under the 10 s read deadline |
| Files | One `.part` beside the target, made with `create_new` before the process and the only file ever removed; renamed over the target after `sync_all` through a handle opened for writing |
| Replacement | A newer request for the same media and the same span stops the older, unless it has claimed its rename, and names what it stopped; the media's peaks, beats and loudness and other spans are left alone |
| Project replace | `cancel_all` keeps a file being written or waiting, and its answer; the desktop offers it only to the project that asked |
| Cancel | Too late once the job has claimed its rename (`Finishing`) or its written answer waits (`Written`); a waiting failure is given up with it |
| Desktop | Cards for files being written and written ones, each offered for 12 s (`extract::OFFER_SECONDS`); no file is read on the window's thread, the dialog's thread looks whether a name with `.wav` added is taken |

| Measurement | Result | Conditions |
| --- | --- | --- |
| A video's whole sound | 3.008 s written of 3 s of video; a 1000.0 Hz tone at -9.02 dBFS RMS, as made | MP4 of `mpeg4` picture and AAC 1 kHz tone at half of full scale, 48 kHz mono, one run |
| A span | The tone begins 24,001 frames into a 1 s span asked from 1.5 s of a file whose tone starts at 2 s (24,000 expected) | Stereo 48 kHz PCM WAV, one run |

Not measured: how long a long or compressed source takes, the disk's cost,
memory while writing, a span from a seek far into a long file, and any other
host. A WAV past 4 GiB (about 6.2 hours of 48 kHz stereo) is written without
RF64 and its header cannot say its length.

### Stem separation

A separation runs one external process, the person's stem separator
(Demucs), on the sound lane over a whole media file. It holds the lane for
as long as the tool runs, so waveforms, loudness and beats wait behind it;
the tool's own threads, memory, GPU use and model download are its own and
are not bounded by Aster.

| Bound | Value |
| --- | --- |
| Probe | Once a session on `aster-separator-probe`, `<tool> --help` within 60 s and 256 KiB of output; the window reads the answer and never waits for it |
| Process | 1; its output drained and dropped on a thread of its own; its progress read from the last 8 KiB of its error pipe every 0.5 s; looked at every 0.1 s for cancellation and its end |
| Deadline | The whole run, 15 minutes plus 4 times the media's length, enforced by the process watchdog |
| Files | One `<media> (stems).part` folder in the media folder, made with `create_dir` before the process (one already there is refused and left) and removed with what is in it however the job ends; stems moved out by hard link to a free name, or a rename where links are refused; never over a file |
| Replacement | A newer separation of the same media stops the older unless it has claimed its last step; other jobs of the media are left alone |
| Project replace | `cancel_all` keeps it and its answer; the desktop places stems only for the project and clip that asked and otherwise names the files |
| Voices | Each stem is one more voice where its clip plays, within the 16-voice bound |

| Measurement | Result | Conditions |
| --- | --- | --- |
| Through the lane | Probe 1.24 to 1.87 s; 20 s of source separated in 5.65 and 6.04 s with the model held, 15.72 s on the run that fetched it; two 20.000 s stems of 3,528,044 bytes; progress said at 25, 50, 75 (and 100) % | Demucs 4.1.0 (torch 2.14.1, Python 3.12, through `uvx`), model `htdemucs` (`955717e8`), its default device; two tones FFmpeg made, stereo 44.1 kHz WAV; macOS 26.5.2, Apple M1, `dev` test profile; three runs |
| The tool alone | 180 s of source in 26.7 s wall (6.4 s user) | The same command line and host, one run |

Not measured: speech or music, memory, the GPU or CPU the tool chose, a
host without a GPU, other models, Linux and Windows. Quality is not
claimed: nothing was listened to, and two tones are no test of it.

### Mixer level stage and master limiter

The level stage and the limiter are arithmetic in the mixer's sum loop on the
`aster-audio` worker; neither starts a process or allocates per frame, and the
device callback still only copies.

| Bound | Value |
| --- | --- |
| Voices | 16 sounding clips at once, as before; the 17th stops the mix with a message naming the count and the time |
| Level | `track.gain × clip.gain` once for an interval when the clip's level does not vary; `sound::level_at` at every output frame for a clip with a fade or volume keys, over its whole length and not only inside the fade |
| Master limiter | One `f32` of state (`Mixer.limit`), reset in `clear()`; ceiling 1.0 (`MASTER_CEILING`, 0 dBFS), release time constant 0.2 s (`LIMITER_RELEASE_SECONDS`), no lookahead, sample peaks only; a step back towards 1 is at least one `f32` epsilon; the clamp to ±1 follows |
| Admission | One `Sounding` built from the mixer's snapshot answers `sounds` for every clip; its cost over a large project is not measured |

| Measurement | Result | Conditions |
| --- | --- | --- |
| Fades | Every frame of a 100 ms fade within 1e-4 of the curve across four chunk edges; frames past the fade bit for bit | 997 Hz full-scale tone in a 32-bit float WAV the test writes, through `Mixer::chunk` |
| A mix within ±1.0 | Bit for bit, the values 1.0 and -1.0 included; a full-scale single tone bit for bit with the gain still 1 | The same |
| Two full-scale tones in phase | Never above 1.0; from the first peak on within 0.01 of the single tone; under a tenth of the samples on the ceiling | The same |
| The gain's return to 1 | 1.42 s after a peak of twice the ceiling, 1.08 s after 10 % over, 0.64 s after 1 % over; more than 0.1 dB down for 0.76 s after the first | Counted in a test at 48 kHz |

### Pitch and pitch-keeping speed

Keeping or shifting pitch adds one `atempo` stage to the chain a voice already
has, never two; it starts no process.

| Measurement | Result | Conditions |
| --- | --- | --- |
| Pitch read back | 440.0 Hz at speed 2 with `Pitch` 0, ending at 399.9 ms of 400; 880.0 Hz with `Pitch` +12 at speed 1, ending at 790.8 ms of 800; 877.5 Hz at speed 2 without a `Pitch` | 0.8 s 440 Hz tone and silence through `Mixer::chunk`, three runs alike |
| Sound early against the picture | `atempo` 0.5: 40.0, 58.5 and 58.5 ms; 0.84: 27.5, 28.2 and 28.4 ms; 1.5: 8.0, 16.0 and 16.0 ms; 2: 12.0, 13.3 and 18.5 ms; none late. Not compensated | Onsets of 5 ms 1 kHz bursts at 1, 10 and 30 s of source |
| Two stages of 0.5 | 151.5 ms early | FFmpeg 8.1.2 alone, design review; why one stage is the rule |

### Voice chain and filter probe

An audio effect costs no process: it is a filter in the decoder a voice
already has, one thread each (`-threads 1`, `-filter_threads 1`), sixteen
voices at most. The probe is eight FFmpeg runs over 50 ms of silence, once a
session, on the audio worker before the first sound.

| Bound | Value |
| --- | --- |
| Probe | 1 control and 7 kinds, one process at a time, each under the 10 s read deadline, its own `Processes` |
| Delay dropped at a voice's start | 25,000 us for `Denoise`, 5,000 us for `Limiter`, summed and rounded to frames at the device's rate (1,440 frames for both at 48 kHz) |
| Filter frequency | At most 0.45 of the output rate |
| Centre reduction | Left out of a chain for an output of one channel |

| Measurement | Result | Conditions |
| --- | --- | --- |
| The probe's eight runs | 297 ms, one run | `dev` test profile, FFmpeg already in the file cache |
| Delay of `afftdn` | 24,979 us at 48 kHz, 24,965 us at 44.1 kHz | Onset of a 5 ms 1 kHz burst through `Mixer::chunk`, against the same clip without effects |
| Delay of `alimiter` | 4,979 us at 48 kHz, 4,965 us at 44.1 kHz | The same |
| Both, after the drop | The burst lands 41 us early at 48 kHz and 68 us early at 44.1 kHz | The same; the test allows 1 ms |
| `highpass`, `equalizer`, `acompressor` | No delay | The same |
| Noise between bursts with `Denoise` | 16.3 dB lower at the defaults, 11.5 at amount 0.2, 20.8 at amount 1, unchanged with the floor at -80 dB; the bursts unchanged | White noise at -45 dBFS RMS under 300 Hz and 1 kHz in bursts of 0.5 s |

Not measured: the CPU of a filtered voice or of sixteen, underruns, the time a
decoder with a chain takes to start, the probe on a cold disk or another
host, and how any of it sounds. The last 25 ms of a source file are not heard
through noise reduction (5 ms through the limiter): FFmpeg ends the stream as
long as it came in, and the delay is taken off its start.

### Voiceover recording

`RecordService` captures the default input device to a WAV file the desktop
named. The device's callback only copies into a ring and counts what found it
full; one thread opens and plays the device, then makes the file (the ring
holds what comes meanwhile), writes and closes them, and no call waits for it
except `Drop`, which joins it.

| Bound | Value |
| --- | --- |
| Threads / processes | 1 (`aster-record`) beside the device's own callback / none |
| Ring | Two seconds at the device's rate and channels (`RECORD_RING_SECONDS`), 32-bit samples in atomics: 375 KiB for mono 48 kHz, 750 KiB for stereo, 5.9 MiB at the highest rate taken (384 kHz stereo). A full ring drops the frames and counts them (`Take::dropped`); the callback never waits |
| Callback | Copies whole frames into the ring with atomics: no lock, allocation, file work or wake. After `stop` it takes nothing |
| Channels and rate | One or two channels as the device delivers them, more refused; 1 Hz to 384 kHz |
| Writer | Drains every 20 ms, 4,096 frames at a write, from a buffer allocated once |
| Crash tolerance | The header's two sizes are rewritten and the file synced (`sync_data`) after each second of sound written and after a second of the clock with sound unsynced; `sync_all` when the take is closed |
| Longest take | 2 hours (`MAX_TAKE_SECONDS`), or data bytes ≤ `u32::MAX − 36` rounded down to whole frames (4,294,967,256 for mono or stereo), whichever comes first; the take ends itself there as if stopped. 1.38 GB an hour for stereo 48 kHz, 0.69 GB for mono |
| Events | `Started`, `Finished` and `Failed` wake once each; `Level` is the loudest sample since the last poll, kept as one value and never a wake |
| Free space | Not checked before a take; a failed write, a full disk included, ends the take and keeps what was written; part of a piece the failed write left is cut off on close, so the file's length is `Take::bytes`. Only a test's simulated full disk was run |

| Measurement | Result | Conditions |
| --- | --- | --- |
| Longest callback into a stalled writer | 0.3 to 12.2 us over three runs | 300 calls of 80 mono frames into a ring that filled after 200, fake source on the test thread, `dev` test profile; the test allows 5 ms |
| `stop` with the writer stalled | 1.3 to 3.5 us over three runs | The same; the test allows 20 ms |
| A writer ended without closing | Its header counts 19,096 of 20,000 frames written; ffprobe reads 2.387 s of 2.5 s | 8 kHz mono, fed under the ring's length and drained, then ended as a crash would; the test allows one second short |
| ffprobe's length of a closed take | 2 s for 96,000 frames at 48 kHz stereo, within one frame | The same fake source |

Not measured: a real microphone on any host, a device's own latency, the cost
of `sync_data` on a slow or full disk, the writer under load, and a take of
any length beyond these fixtures. The 44-byte header has no `fact` chunk;
FFmpeg 8.1.2 reads it.

In the desktop the frame loop does no file work for a take: the take's path
and media folder are made on a short worker thread (`aster-take`), and the
service is started from its answer. While a take runs the window is drawn
ten times a second (`record::REPAINT`) for its elapsed time and level, and
not at all for the take once it is placed; starting and closing are woken by
the service. A take placed is one asset built from `Take`, with no ffprobe.
Closing the window during a take stops it and joins the service's thread,
which writes what the ring holds and closes the file. The time a stop takes
on a real device was not measured.

## Titles, captions and speech budgets

These are the bounds the [titles and captions](../.scratch/titles-and-captions/spec.md)
code enforces and the few figures measured for it. Every figure is from one
Linux container (4 vCPU Intel Xeon at 2.8 GHz, 15 GiB, Ubuntu 24.04.5) on the
`dev` profile, whose dependencies are built at `opt-level` 2, except the one
row marked as the release build. None is a macOS, Windows or playback-rate
measurement.

### Title lane and textures

A title costs the frame one quad. Its picture is drawn on the `aster-titles`
thread, never on the frame, and an animated title asks for a new picture on
each frame its layer changes. How long a picture takes to draw, and so whether
an animated caption keeps up with 30 or 60 fps playback, has not been
measured: a slow picture shows the previous one for longer, it does not slow
the frame.

| Bound | Value |
| --- | --- |
| Lane threads | 1 (`aster-titles`), started with the window |
| Titles waiting for a picture | 64 (`TEXT_QUEUE`); past it the oldest is let go. Only the newest request per title is kept |
| Pictures drawn but not yet taken | 64 MiB (`TITLE_BUDGET`); the lane draws no more until the window takes some |
| Picture size | 4096 pixels a side at most (`MAX_TITLE_SIDE`); scale 1/64 to 16, quantized to eighth octaves so a zoom does not redraw every frame |
| Textures the window holds | One per title drawn on screen; a title not drawn for two frames gives its texture up, and a replaced project clears them all |
| Font files read and held | 192 MiB beyond the bundled faces (`FONT_CACHE_BYTES`), least lately used let go; the faces of the title being drawn are always kept. One file is read only up to 128 MiB |
| System font search | Once a lane, when first needed: 8192 files and 8 directory levels at most; 48 faces read per picture to find a missing character, and 65,536 characters' fallbacks remembered |
| Text tab tiles | Drawn by the same lane at tile size; a tile's entrance plays only while the pointer rests on it, and the tab asks for no repaint while nothing is pointed at |

### Speech lane

| Bound | Value |
| --- | --- |
| Jobs at a time | 1 on the `aster-speech` thread, started the first time captions are asked for: a model download or a transcription |
| Stretch heard | 6 hours of timeline at most (`MAX_SPEECH`) |
| Sound held | The timeline mix at 16 kHz mono, rendered offline and taken in pieces of about 5 minutes cut at a quiet moment; one piece and its speech-only copy are held, not the whole stretch |
| Recognition threads | All processors but one, from 1 to 8 |
| Progress wakes | One every 100 ms at most; 16 events kept for the window |
| Model files | Tiny 32.2 MB, Base 59.7 MB, Small 190.1 MB, Medium 539.2 MB, Large v3 Turbo 574.0 MB; the speech detector 0.9 MB. Kept in `models/` beside the settings file when `--settings` names one, otherwise in the platform's local data directory |

| Measurement | Result | Conditions |
| --- | --- | --- |
| Model load | 140 ms | Tiny q5_1 |
| Offline 16 kHz mix | About 300 ms | 7 s of speech in an 11.55 s timeline |
| Speech detector | About 180 ms | Same |
| Recognition, language detected | 3.7 s | Same, Tiny |
| Recognition, language given | 2.0 s greedy, 2.3 s beam search of 5 | Same, Tiny; beam search is what ships |
| Click to captions | About 5.5 s for six captions | Native run of the debug build, Tiny, 8.5 s of speech |
| Click to captions, release build | 4.2 to 4.9 s for six captions | Native run of `cargo build --release`, Tiny, language detected, same speech; the captions were absent from a capture 4.2 s after the click and present in the next, at 4.9 s |
| Cancel during recognition | 3.34 s | Tiny; whisper.cpp checks for a cancel only between passes and its language detection cannot be stopped |
| Base model download | 59,707,625 bytes, checksum matched | Through this container's network proxy; time not recorded |

Recognition for under 30 s of speech reports no progress until it ends,
because whisper.cpp reports per 30 s window. The larger models' speed, the
memory used while recognizing, GPU acceleration and a real recording longer
than one five-minute piece were not measured.

## Export and proxy budgets

These are the bounds the [export and rendering](../.scratch/export-and-rendering/spec.md)
code enforces and the figures measured for it. Every figure is from one
Linux host (i9-13900HX, RTX 4060 Laptop on Vulkan, Ubuntu 26.04.1) with
FFmpeg 8.0.1. Those under [Measurements](#measurements) come from test runs
whose build profile was not recorded; those under
[release-build export measurements](#release-build-export-measurements) come
from the release build of the native verification; those under
[export FFmpeg thread caps](#export-ffmpeg-thread-caps) from the export's own
FFmpeg commands run outside the editor. None is from macOS or Windows.

### Render queue and frame pipeline

| Bound | Value |
| --- | --- |
| Jobs listed | 64, finished ones included (`MAX_JOBS`); queueing more is refused as `queue_full` |
| Jobs rendering | 1, on the `aster-render` thread; a paused running job holds the queue |
| Output frames in existence | 3: the one being drawn and 2 handed to the `aster-render-writer` thread (`IN_FLIGHT`), in recycled buffers. At 3840x2160 that is about 95 MiB; at 8192x8192 about 768 MiB |
| One frame written to FFmpeg | 60 s (`WRITE_DEADLINE`); a write past it kills FFmpeg, which is an encoder failure (see the software fallback in [architecture](architecture.md#export-and-rendering)) |
| FFmpeg finishing the file | At least 600 s (`FINISH_DEADLINE`) plus the part file's size at 8 MiB/s (`FINISH_PACE`), on an `aster-render-finish` thread; past that, as long as the part file's length or modified time changes; 120 s without a change (`FINISH_STALL`) kills FFmpeg and fails the job, with no software fallback |
| One frame read back from the device | 60 s (`READ_DEADLINE` in `src/export/frame.rs`), polled without blocking, with short sleeps |
| Progress wakes | At most one every 100 ms (`WAKE_EVERY`); start, stage changes, the end and the probe result wake at once |
| Finished events waiting | 32 (`MAX_EVENTS`); the oldest go first |
| Speed shown | Measured over the last 2 s (`SPEED_WINDOW`) |
| Shutdown | Cancels every job and waits 750 ms at most for the running one to tidy up (`SHUTDOWN_GRACE`); joins no thread |
| Sound file in the job's folder | Raw 32-bit float: sample rate x channels x 4 bytes a second, in `.aster-render-<job>` beside the destination, or `<system temp>/aster-render-<job>` when that folder cannot be made. 48 kHz stereo is 384,000 bytes a second, about 1.4 GB an hour (computed, not measured). No free space is checked first |
| Left-over part files and working folders | Removed once unchanged for 1 h (`STALE_AFTER`) and named by no listed job: in the destination folder when a job starts there, and in the system's temporary folder when the queue starts |
| Part file name | `.<name>.<job>.part`, the name cut at a character boundary so the whole stays within 255 bytes |
| Encoder FFmpeg threads | libx264: `-threads:v` x264's own count, 1.5 times the logical processors (4 when they cannot be counted), from 1 to 24 (`X264_MAX_THREADS`): 2 processors give 3, 8 give 12, 16 or more give 24. libsvtav1: `-svtav1-params lp=4` (`SVT_AV1_PARALLELISM`), as it ignores `-threads`. libx265, libvpx-vp9, prores_ks, libaom-av1 and every hardware encoder get no thread option. The encoder probe uses the same arguments. Memory is under [export FFmpeg thread caps](#export-ffmpeg-thread-caps) |

### Output and the frame renderer

| Bound | Value |
| --- | --- |
| Output side | 16 to 8192 pixels (`MIN_OUTPUT_SIDE`, `MAX_OUTPUT_SIDE`); a custom size at most 134,217,728 pixels (`MAX_OUTPUT_PIXELS`). Fill is refused when the canvas it would draw is over 8192 a side |
| Hardware encoder limits | H.264 hardware encoders 4096 pixels a side, others 8192; NVENC's smallest pictures (146x50 for H.264, 130x34 otherwise) are applied to every hardware family. `Auto` picks software for sizes outside them |
| Video bitrate | 100 to 800,000 kbit/s (`MIN_VIDEO_KBPS`, `MAX_VIDEO_KBPS`) |
| Audio | 1 or 2 channels; 44.1, 48 or 96 kHz (`SAMPLE_RATES`); 32 kbit/s to the codec's ceiling, at most 512 (`MIN_AUDIO_KBPS`, `MAX_AUDIO_KBPS`) |
| Compositor passes a frame | 1024 (`MAX_PASSES`); a frame needing more fails the job. The viewer's `ComposeBudget`, canvas steps and four-canvas limit do not apply |
| Decode size a clip | The most it needs over its keyframed scale and crop on the drawn canvas, never above the source, 16 to 8192 a side (`MIN_DECODE_SIDE`, `MAX_DECODE_SIDE`) |
| Title pictures | 4096 pixels a side (`MAX_TITLE_SIDE`), drawn on the render thread at the output scale |
| Sound | The playback mixer's 16 simultaneous audible clips (`MAX_VOICES`); more fails the job with "Export sound supports at most 16 simultaneous audible clips; mute extra tracks" |

### Rate control and the size estimate

| Bound | Value |
| --- | --- |
| Quality value on hardware | libx264/libx265's CRF for the level plus an offset (`hardware_quality`): NVENC `-cq` +5 for H.264 and +3 for HEVC; AMF and VA-API constant QP +2 and +1; QSV and every AV1 and VP9 mapping unchanged |
| NVENC quality ceiling | `-b:v 0 -maxrate` 4 times the suggested rate (`NVENC_CEILING`) and `-bufsize` 8 times. Without it NVENC held Higher and Maximum to the rate its level allows, about 16 Mbit/s for H.264 and 10 for HEVC at 1080p |
| Suggested rate | Pixels a second times bits per pixel (H.264 0.14, H.265 0.11, AV1 0.055, VP9 0.07, ProRes 1.9) times the level's factor (Lower 0.55, Recommended 1, Higher 1.8, Maximum 4), within 100 to 800,000 kbit/s. 1080p30 H.264 Recommended is 8,709 kbit/s. Bitrate mode starts from it, and Media Foundation's quality mode holds it |
| Quality-mode estimate | The suggested rate times 0.85 for libx264 at Faster, 1 otherwise, plus the sound and 1 % for the container |

### Export dialog poster

| Bound | Value |
| --- | --- |
| Thread | One `aster-poster` thread with its own `FrameRenderer` and `SourceReader`, while the dialog is open |
| Picture | The output's shape at most 640 pixels on the long side (`POSTER_SIDE`) |
| Requests | Sent once unchanged for 150 ms (`SETTLE`); only the latest waits; 10 s to read the picture back (`READ_DEADLINE` in `poster.rs`) |

### Source reader

The `SourceReader` decodes the export's pictures with FFmpeg processes of its
own, apart from the preview's decoder pool.

| Bound | Value |
| --- | --- |
| Decode pipes | 16 open at once (`MAX_PIPES`), one per clip, asset and size; the least recently read closes first |
| One decoded picture | 8192 pixels a side and 256 MiB (`MAX_READ_SIDE`, `MAX_READ_BYTES`); the preview's 4096-pixel, 64 MiB limit does not apply |
| One picture arriving | 60 s, seeking included (`READ_DEADLINE` in `reader.rs`) |
| Reading on instead of seeking | Up to 12 frames or one second of source, whichever is more (`MIN_GAP`) |
| Decoded stills held | 512 MiB (`STILL_BUDGET`); the one just used always stays |
| GIF decoders | 8 (`MAX_GIFS`) |
| FFmpeg threads a pipe | 4 decoder threads (`-threads 4` before the input, `DECODE_THREADS`), 2 filter threads (`-filter_threads`) and 1 for the raw output (`-threads:v 1`) |

There is no prefetch thread; FFmpeg's own threads decode ahead.

### Encoder probe

| Bound | Value |
| --- | --- |
| When | Once on an `aster-render-probe` thread when the queue starts, and again on request; queued jobs wait for it |
| Probes at once | 4 (`PROBES_AT_ONCE`), each with a 10 s deadline (`PROBE_DEADLINE`) |
| Test | Video: 2 frames of a 256x144 black picture (`PROBE_PICTURE`) through the export's own argument builder. Sound: 0.1 s of silence. VA-API tries each `/dev/dri/renderD*` |
| Listings read | `-encoders` and `-muxers`, 4 MiB of output each at most (`LISTING_BYTES`) |

### Proxy lane and cache

| Bound | Value |
| --- | --- |
| Lane threads / FFmpeg processes | 1 (`aster-proxy`) / 1; one proxy written at a time |
| Proxies waiting | 64 (`PROXY_QUEUE`); past it a request is refused and asked again at the next change of the library, project or mode |
| Proxy picture | 1280 pixels on the long side at most (`PROXY_SIDE`), never larger than the source; H.264 `veryfast`, CRF 23, a keyframe every 15 frames and no B-frames, or MJPEG without libx264; no sound; frame grid from the file's start (`fps=N/D:start_time=0`); none for a source with an alpha pixel format |
| FFmpeg threads | Half the processors, from 2 to 8, and 2 filter threads |
| Which videos at Auto | Longer than 1920 pixels on a side (`PROXY_ABOVE_SIDE`), or above 50,000,000 bit/s averaged over the file (`PROXY_ABOVE_BITRATE`) |
| Cache budget | 20 GiB unless changed (`PROXY_BUDGET`, `PROXY_BUDGET_GIB`); the settings sheet offers 5 to 500 GB and a stored value is held to 1 to 1000. Least recently used first; proxies open in a decoder and the one written last are kept |
| Cache place | `proxies/` beside the settings file when `--settings` names one, otherwise the user's cache directory |
| Deadlines | 120 s without progress from FFmpeg (`SILENCE`), 30 s to exit (`EXIT`) |
| Left-over files | Part files and proxies without a manifest are removed once over an hour old (`LEFT_OVER`) |
| Manifests | 64 KiB each at most (`MANIFEST_BYTES`); read once, on the lane, when it starts |
| Remembered failures / waiting answers | 256 (`FAILURES`) / 1024 (`EVENTS`); one progress figure an asset |
| Recipe | `PROXY_PROFILE` 2; a proxy written with another is not used |
| Closing the window | `ProxyService::shutdown` waits at most 750 ms (`SHUTDOWN_GRACE`) for the proxy being written to stop |

### Preview quality

| Bound | Value |
| --- | --- |
| Divisors | Full 1, Half 2, Quarter 4, applied to the long side of each decode box and of the compositor canvas |
| Auto | Full, until 4 (`LATE_STEP`) of the last 16 (`LATE_WINDOW`) frame advances of playing video layers were late; then Half until playback stops |

### Measurements

| Measurement | Result | Conditions |
| --- | --- | --- |
| Encoder probe | About 0.9 s | The probe test on this host. Offered: libx264, libx265, libsvtav1, libaom-av1, libvpx-vp9, prores_ks, h264_nvenc, hevc_nvenc, av1_nvenc, and every audio encoder. QSV ("Error creating a MFX session") and VA-API ("Failed to initialise VAAPI connection") failed on both render nodes |
| Whole job, 320x180, NVENC | 60 frames in 0.42 s | Engine test, FFmpeg start and sound included; fixture is the test's generated project |
| Whole job, 1920x1080, NVENC | 60 frames in 0.60 s on the RTX 4060, 1.17 s on llvmpipe | Same |
| Proxy cancel | A paced proxy encode stopped, its watchdogs ended and no part file left, within the test's 5 s bound | Proxy test, an encode paced with `-readrate 1` |
| Quality-level size, software and NVENC | 520 encodes scored on size, SSIM and PSNR. At Recommended, x264 / NVENC wrote 2.83 / 3.00 MB (clean 720p), 5.83 / 6.40 (clean 1080p), 3.44 / 3.01 (grain 720p), 7.49 / 6.47 (grain 1080p); x265 / NVENC HEVC 2.49 / 2.49, 5.69 / 5.43, 2.17 / 2.49, 4.56 / 5.51. SSIM matched within 0.002 | testsrc2 with and without `noise=alls=4:allf=t`, 1280x720 and 1920x1080, 30 fps, 6 s; libx264, libx265 and NVENC with the quality offsets above |
| Size against the estimate | Lower and Recommended: 0.89 to 1.29 times the estimate across these fixtures and encoders (1.24 to 3.06 before). Higher and Maximum: 0.42 to 0.9 on clean pictures; x264 on grainy pictures 3 to 4 times | Same fixtures |
| Real-encode size test | x264 wrote 1.01 and x265 1.04 times their estimate; NVENC wrote 1.16 (H.264) and 1.24 (HEVC) times what x264 and x265 did. Heavy grain at Maximum reached 16.7 and 16.5 Mbit/s on NVENC, against 13.6 and 9.5 without the ceiling | `a_quality_level_writes_about_the_estimate_on_software_and_nvenc_alike` |
| Sound alignment | A sound stream starting late in MOV (0, 0.04 and 0.5 s seeks) and Matroska PCM seeks (0, 0.5, 0.51, 0.7 s) land within 4 samples; WebM VP9 with Opus within 2 ms | `editor-media` export tests |

The whole-job figures are two-second jobs and say little about a long render:
FFmpeg's start and the sound render are a large part of them.

### Release-build export measurements

From the [native verification](verification.md#export-and-rendering) on
2026-10-14: `cargo build --release --locked --bins`, `aster` SHA-256
`3452f930…f2e572`; Dell G15 5530, i9-13900HX (32 threads), 30 GiB RAM,
RTX 4060 Laptop driver 595.99.02 on Vulkan, Linux 7.0.0-38-generic; FFmpeg
8.0.1-3ubuntu2. The window ran on Xvfb with no window manager, with other
desktop load on the host. Fixtures are synthetic `lavfi` clips with H.264
sources. A job's time is its own `elapsed_seconds`: FFmpeg start, the sound
render and the ffprobe check included. The speed shown is the queue's 2 s
figure (`SPEED_WINDOW`), polled during the job.

| Job | Whole-job time | Speed shown while rendering |
| --- | --- | --- |
| 1080p 29.97 fps, 240 frames (8 s), libx264 | 9.24, 9.22, 5.30, 5.20 s (26 to 46 fps); identical runs differed widely | Highest 29 to 86 fps a run |
| Same, h264_nvenc | 3.67, 3.55, 3.74, 3.65 s (64 to 68 fps) | Highest 79 to 86 fps a run |
| 4K 25 fps, 100 frames (4 s) from the 4K source, h264_nvenc | 4.75, 4.59 s (21 to 22 fps) | Highest 27 to 29 fps a run |
| Same, libx264 | 7.32, 6.42 s (14 to 16 fps) | Highest 21 to 25 fps a run |
| Same, hevc_nvenc | 4.65 s | Highest 27 fps |
| The whole 14 s timeline (420 frames) at 3840x2160, libx264 | 19 to 20 s (about 21 fps) | 16.7 to 32 fps |
| Sound only, 8 s: AAC, WAV, MP3, FLAC | 0.83, 0.10, 0.21, 0.13 s | n/a |

| Memory | Result | How |
| --- | --- | --- |
| `aster` | Highest VmRSS 396 to 446 MiB on 1080p and smaller jobs, 570 MiB on 4K jobs | Sampled every 100 ms |
| All of `aster`'s child processes, the viewer's decoders included | Highest summed VmRSS: about 1,150 to 1,190 MiB for 1080p NVENC, 1,720 to 1,780 MiB for 1080p libx264, 2,810 to 3,020 MiB for 4K NVENC, 5,990 to 6,040 MiB for 4K libx264. RSS counts shared pages in each process | Same |
| 4K libx264 job, per process | Highest PSS: encoder FFmpeg 4,942 MiB, export source reader 1,004 MiB, `aster` 402 MiB. Highest PSS of `aster` and all its children together: 6,294 MiB | Sampled PSS |

These figures are from the build before the export FFmpeg thread caps; no
job through the editor was measured with them.

### Export FFmpeg thread caps

Measured on 2026-10-14 on the same host (i9-13900HX, 32 logical processors,
30 GiB RAM), FFmpeg 8.0.1 from `.tools`, SVT-AV1 2.3.0, by running the
export's source reader command into its encoder command, with the export's
own arguments, at Recommended and Balanced, outside the editor and without
its compositor. Another task's `aster` kept the load average between 2 and
21, so each figure is the median of 3 interleaved runs, and speed varied up
to ±15 % between runs. Sources were synthetic: `testsrc2` with noise, H.264 at
about 300 Mbit/s, and HEVC at 4K. Peaks are the higher of VmHWM and
ru_maxrss.

| Job | Speed before → after | Encoder peak MiB, before → after | Reader peak MiB, before → after | Both, before → after |
| --- | --- | --- | --- | --- |
| libx264 1080p, 300 frames | 72.4 → 69.6 fps | 1,197–1,205 → 1,018–1,022 | 261–269 → 178–181 | 1,460–1,474 → 1,199–1,201 |
| libx264 4K, 180 frames | 17.8 → 17.7 fps | 5,442–5,466 → 3,685–3,971 | 814–816 → 462–497 | 6,258–6,282 → 4,147–4,467 |
| libx265 1080p | 55.1 → 54.9 fps | 982–1,123, arguments unchanged | 259–340 → 181–244 | 1,322–1,444 → 1,163–1,367 |
| libx265 4K | 15.5 → 14.9 fps | 2,466–2,504, arguments unchanged | 784–816 → 494 | 3,261–3,294 → 2,959–2,998 |
| libsvtav1 1080p, preset 8 | 62.9 → 62.2 fps | 2,400–2,426 → 1,465–1,521 | 340 → 252 | 2,740–2,766 → 1,718–1,774 |
| libsvtav1 4K, preset 8 | 17.3 → 18.4 fps | 7,771–8,064 → 4,452–4,645 | 1,099–1,101 → 619–765 | 8,873–9,163 → 5,178–5,344 |
| libx264 encoder alone, 1080p | 93.2 → 90.6 fps | 1,353–1,375 → 1,160–1,170 | – | – |
| libx264 encoder alone, 4K | 24.3 → 20.6 fps | 6,206–6,319 → 4,305–4,368 | – | – |

Why these values:

- **x264 at 24, not 16.** 16 threads saved about 0.4 GiB more at 4K but cost
  the encoder alone about 16 % of its 1080p speed. With 1 thread the 4K
  encoder still used 2.3 GiB; a minimal x264 left about 1.1 GiB, which points
  to FFmpeg's own queues of 4K RGBA frames, and the rest is the medium
  preset's 40-frame lookahead, about 26 MiB a frame at 4K. `-rc-lookahead 20`
  with about 8 threads would give about 2.5 GiB at 4K at 14.6 fps for the
  encoder alone, against 22 to 29; it changes the encoded output and is not
  applied.
- **SVT-AV1 at `lp=4`.** Left alone it chose level 6 even with the process
  held to 2 CPUs. Level 4 was 9 % slower at preset 5 at 1080p and as fast or
  faster at presets 8 and 10; level 3 lost 13 to 18 % at 1080p. SVT-AV1
  releases before 2.0, which may read `lp` as a processor count, and preset
  5 at 4K were not measured.
- **x265 left alone.** FFmpeg passes `-threads` to x265 as frame threads:
  `-threads 16` reached 4.29 GiB at 4K against 3.16 GiB with its defaults,
  which choose about 6 frame threads at most.
- **libvpx-vp9 and prores_ks** already stop at 16 threads (1.7 and 2.05 GiB
  at 4K); **libaom-av1** used 2.30 GiB at 4K with or without `-threads 16`.
  These were spot checks at 4K only.
- **Reader at 4 decoder threads.** Paced at 16 fps, as the editor reads,
  4K went from 1,099–1,147 MiB to 779–790 MiB and 1080p from 332 to 242 MiB.
  4 threads read as fast as FFmpeg's own choice (33 to 35 fps at 4K, set by
  the pipe's scaler, and 134 fps at 1080p); 2 threads were about 40 % slower
  on the 4K H.264 source and at 1080p.

Not measured: sustained throughput of any long render, at 1080p or 4K;
GPU memory of a running job, and process memory beyond the short jobs above;
how much a running export slows the
interactive window, with which it may share the GPU; the cost of effect
stacks, stabilized clips or many layers per output frame; source reader
throughput for 8K media, and for 4K beyond the synthetic sources above; how long a proxy takes to write; whether
proxies or a lower preview quality make large media play smoothly; any
hardware encoder other than NVENC, whose quality offsets (AMF, VA-API) are
NVENC's stand-ins; AV1 and VP9 quality and size; camera footage, as every
size figure above is from synthetic pictures; macOS and Windows. The export
thread caps were measured outside the editor only; their effect on a whole
export through the editor is not measured.

## Acceptance targets

These are investigation targets, not achieved guarantees. Fix a host and
workload before using them as regression budgets.

| Workload | Target to investigate | Required observation |
| --- | --- | --- |
| Paused visible editor | Event-driven idle with negligible recurring work | Process CPU, wakeups, RSS and GPU activity after warmup |
| Timeline interaction | Responsive at the display refresh rate | Native input-to-feedback latency and frame-time p50/p95/p99 |
| 1080p30 preview | Sustained source-rate presentation | Actual presented frames, late/dropped frames and A/V drift |
| Rapid random scrubbing | Latest requested frame wins without input stalls | Cold/warm seek latency distribution and cancelled resource cleanup |
| 1,000 tracks / 100,000 clips | Visible-area cost rather than total project cost | Scroll/zoom latency, frame CPU and retained memory |
| Large folder import | Responsive editing and bounded growth | Import throughput, queue depth, UI latency and peak process-tree RSS |
| Long fractional-frame timeline | Deterministic frame positions | Frame-index timing at 30000/1001 and 60000/1001 |
| Save during editing | Interactive input independent of storage | Slow-disk frame latency and exact saved-revision acknowledgement |

Hardware decoding, waveform pyramids, GPU-native composition, zero-copy upload
and indexed timeline lookup are opportunities to assess against profiles. They
are not features merely because the architecture permits them. Proxy
generation now exists (see [export and proxy budgets](#export-and-proxy-budgets)),
but whether it makes large media play at its rate has not been measured.

## Reproducible measurements

Use optimized release builds with the pinned toolchain and lockfile. Record the
source revision, build/features, OS/session type, CPU, RAM, GPU/backend, driver,
display refresh/scale, window/preview size, power mode and competing workload.
Fixtures record codec, resolution, frame rate, audio rate, duration and source
storage; include at least still images, GIF, interframe video and audio.

Separate startup and cache warmup from timed intervals, and retain cold-cache
measurements where they represent the user flow. Repeat measurements and report
distributions. Include decoder/subprocess CPU and memory when applicable;
process RSS excludes GPU memory and can include shared mappings. Observe actual
native input and presented output when claiming latency.

Failed measurements remain diagnostic evidence. Neptune's parser/PTY/idle
results belong to Neptune and are not an editor baseline. Cross-product
comparisons require the same media, hardware, operation and observation method.
