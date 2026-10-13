# Keep preview pictures bound to their continuing clips

Status: ready-for-agent
Progress: claimed
Type: task
Blocked by: 08

## Problem

The two [Opus source reviews](../../../artifacts/production-readiness/preview-consumer-identity-review.md)
and [controller-invalidation response](../../../artifacts/production-readiness/preview-consumer-identity-review-response.md)
find that reused-asset clips are disambiguated by their ordinal encoded in
requested image height. Settled clips at different source times have separate
lanes and correct pictures. When a preceding same-asset clip ends, is hidden,
removed or reordered, a continuing clip changes ordinal and reads another
clip's cached texture, or an empty lane, until decoding completes.

Seek, playback and ordinary track/clip commands do not clear viewer layers;
only project replacement or changed asset files do. This is a source-permitted
transition gap, not a native reproduction or measured duration. Holding a
clip's own previous frame is intentional and must remain supported.

## Selected correction

Use explicit opaque preview consumer identity through the media interface and
result routing, keeping it out of the shared decoded-frame cache key. Bind a
continuing clip to the same consumer across active-set changes. Preserve
same-asset contiguous-cut handoff of the old picture and sequential decoder;
a naive raw ClipId key would introduce a blank and restart at every split.
Remove occurrence-dependent image dimensions after consumer routing is ready.

Keep the legacy `request_frame` method's signature and coalescing behavior.
Document any event-shape migration: adding a consumer field breaks exhaustive
FrameReady patterns without `..`, including the prepared R14 benchmark. Preserve
that original benchmark/evidence and prepare a separately identified compatible
entry for the later build. Purpose is scheduling metadata; consumer is identity.
Coordinate this with [issue10](10-preview-dispatch-policy.md), rather than
landing a second temporary height-encoding implementation.

## Acceptance

- Root owns Cargo. Begin only after current R14 native acceptance releases its
  source freeze.
- Retain assertion-level RED at the actual headless viewer paint seam: two
  same-asset clips with distinct source frames, then natural end, hide,
  reorder and appearing-clip transitions. Inspect painted TextureIds for the
  continuing clips. A duplicated occurrence loop or missing-method compile
  failure is not reproduction of this defect.
- Preserve settled independent frames, source-monitor isolation, own-frame
  hold during seek, same-asset contiguous-cut texture/pipe handoff, undo/redo
  and project/source replacement guards.
- Prove new consumers at equal asset/size and different source times retain
  independent latest jobs and unpolled results. Same-consumer replacement,
  cancellation, generation checks and every issue08 finite-delivery guard
  remain intact. Existing legacy callers still coalesce per asset/size.
- Preserve shared raw pixels for equal source-frame/fitted dimensions across
  consumers, including portrait sources. No image-height identity hack or
  increased worker/decoder/mailbox limits.
- Use a real changing-frame source and native capture sequence for transition
  confirmation, with different offsets/rates plus equal-offset control. A
  settled screenshot alone cannot exclude a one-paint routing fault.
- Treat fairness, successful publication, native presentation and measured
  performance separately. Headless correction is included in R15; final native
  acceptance remains open.

At the R14 freeze release, a fresh phase-A task covered actual
headless viewer regression authoring before changing result routing. No
consumer correction was implemented at that stage.

Actual root execution now reproduces all six transition assertion failures
with seven controls passing at the real egui viewer paint seam. The
[root RED proof](../../../artifacts/production-readiness/preview-consumer-phase-a/root-verification.json)
rehashes all 1,517 immutable inputs and preserves the unchanged production
viewer, textures and controller snapshots. This is headless paint evidence,
not native presentation or late-result mailbox proof.

Root implemented the explicit consumer media interface after that RED;
the coordinated frontend followed. Consumer ownership also
routes preview errors, so retirement can discard one consumer's failures
without invalidating another same-asset clip. The initial full media run passed
89 tests; additional capability/retirement controls and final frontend/native
verification were pending at that stage. Current R15 source/build status follows.

## Comments

The [R15 checkpoint](../../../artifacts/production-readiness/final/round15-build-record.json)
includes coordinated consumer routing/bindings, cut continuity and error
ownership in 521 passing workspace tests, including 198 desktop and 112 media
tests. The [issue-16 correction review](../../../artifacts/production-readiness/preview-request-liveness-round1/review-round1.md)
finds no P1/P2 behavior defect. The later [canonical bridge proof](../../../artifacts/production-readiness/frontend-retention-bridge-round1/root-verification.json)
observes the nonempty hint and restart reset at frozen R15; it adds no standing
suite guard and leaves source/cut/reorder/playing and other lifecycle sites
unproved. Native transition captures, slow/in-flight video and final
target execution remain open; source correction does not establish presentation.
