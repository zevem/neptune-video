# Bound retained thumbnail textures and admit visible demand

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 08

## Question

Can a large library/timeline retain every accepted poster indefinitely despite
the backend's bounded decode queues and caches? The read-only
[scheduling/resource audit](../../../artifacts/production-readiness/preview-scheduling-audit.md)
confirms that `Textures.posters` has no byte/count eviction, and the desktop
requests posters for all timeline clips. Library row virtualization does not
bound that separate producer. This is a source finding, not measured GPU usage.
The historical 5,000-clip native fixture has only one distinct video asset.

## Acceptance

- Begin only after root releases the R13 frozen acceptance grant.
- Reproduce retention beyond an explicit small test budget through the actual
  texture implementation, preserving the failing result before correction.
- Bound retained poster logical bytes and count, with a documented production
  policy. Distinguish retained texels from upload staging, driver allocations,
  viewer-layer textures and measured process/GPU memory.
- Admit visible/overscan library and timeline demand without visiting every
  offscreen clip solely to decode its poster. Preserve timeline thumbnails
  when their library rows are outside the library viewport.
- Prefer recently used visible posters during eviction. Late completions
  cannot bypass retention limits. Evicted posters can be requested again;
  reconcile `poster_requested` so a revisit is not permanently suppressed.
- Preserve Busy retry, failed-poster icons, cancellation retry, project/source
  replacement and the current editing/undo/redo behavior.
- Verify a bounded native workload with many distinct assets, scrolling and
  revisiting, with normal/narrow captures and unchanged project/history from
  browsing. State cache observations separately from physical GPU performance.

## Comments

The 64 MiB backend frame cache and mailbox do not constrain caller-owned GPU
texture retention. Raising decoder concurrency would not fix this ownership
gap. No production source change is authorized under the current frozen R13
test grant; the existing user request authorizes the subsequent correction
once acceptance ownership is released.

R13 root verification is complete and its source freeze is released. The first
round covers actual-seam regression/extraction; root
will run its failing tests before the correction round. Backend compilation,
binaries and all non-frontend source remain under root ownership.

Root executed Opus's actual-seam tests: three retained-poster count/byte/use-order
tests fail, while seven texture controls pass. Four timeline tests also fail
with six controls passing, including actual all-project admission versus four
visible assets. The [phase-A verification](../../../artifacts/native/cache-culling-round1/phase-a-root-verification.json)
retains seven expected failures, 13 controls, source hashes and byte copies.
This establishes the logical cache/admission defects before correction; no
native GPU-memory measurement or policy correction is implied.

The concurrent [Opus design review](../../../artifacts/native/cache-culling-round1/phase-b-design-review.md)
adds selected-asset thumbnail demand, shared actual row/height admission and
paused progress-versus-sleep controls. It explicitly does not validate the
moving implementation. Root's [backend response](../../../artifacts/native/cache-culling-round1/backend-delivery-review-response.md)
confirms R13's finite protected identity/backpressure paths, while keeping
supersession/cancellation and infinite dispatch fairness separate. The findings
were delivered to the active correction task before root compilation.

Root's corrected focused runs pass 18 texture and 14 timeline tests, including
the original seven regressions, late/replacement arrivals, budget-preserving
clear, Busy claim retry, oversubscription sleep and departed-view wake controls.
The [frozen Opus review](../../../artifacts/native/cache-culling-round1/phase-b-review-response.md)
found no blocking defect. The [correction record](../../../artifacts/native/cache-culling-round1/phase-b-correction.json)
sets 256 retained posters and 64 MiB logical RGBA texels, whichever binds first;
this does not bound GPU/driver/process memory. Application wake and selected
asset demand still need corrected-build native verification.

The old R13 binary was also run against 480 distinct still assets. All 480
posters appeared in its fitted timeline. Root independently checked the
[baseline evidence](../../../artifacts/native/cache-culling-round1/baseline-root-verification.json):
all 16 captures, 13 terminated process identities, 480 complete solid-image
fixtures and the existing R13 binary identity matched. Two failed harness
attempts remain preserved. These observations establish the old behavior,
not acceptance or performance of the corrected source. Full R14 validation
passes 397 tests, formatting, Linux/three cross-target source Clippy checks
and optimized builds. Root independently verified
[the exact source/binaries and retained logs](../../../artifacts/production-readiness/final/round14-backend-root-verification.json).
Current native acceptance is running; this ticket remains claimed.

## Answer

The corrected production policy retains at most 256 posters and 64 MiB of
logical RGBA texels, whichever binds first. The actual retention/admission
seams pass their preserved RED/GREEN regressions and controls; the immutable
R14 build passes all 397 tests. Visible library/timeline and inspected/source
demand replace the old whole-project producer.

The [current native report](../../../artifacts/native/cache-culling-acceptance-round1/report.json)
and [independent root verification](../../../artifacts/production-readiness/final/round14-native-editing-root-verification.json)
confirm 256 own posters at the normal fitted 480-asset view, all 480 while
paging, revisits, no-input progress after a single notch, and selected-asset
inspector empty-to-poster retrieval after browsing 464/424 other assets.
Immediate reopening and held-inspector controls retain the picture. Source
monitor poster demand is indirect evidence from the subsequent inspector;
the solid still cannot distinguish viewer/poster lanes. Narrow paging reads
320 assets on two lanes; the third is cut off and the fitted narrow centre
count is not exact. Valid video-with-sound media also shows its poster on
the 44-point audio lane and undoes exactly.

Root checked all 144 captures, 75 tracked process identities gone, 19 retained
attempts, unchanged source/binaries and original fixtures, and 13 platform
snapshot copies. Five harness failures remain preserved. Whole-edit undo/redo,
source preview and the unchanged 13-check smoke pass on this exact build.
Initial notice-dismissal CPU and later idle ticks are diagnostics; neither
these nor the logical retention policy prove GPU/driver/process memory bounds
or real-time performance. Viewer-layer retention is separate
[issue13](13-viewer-texture-retention.md); this resolution covers posters.
