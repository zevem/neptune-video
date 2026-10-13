# Establish bounded preview dispatch fairness

Status: ready-for-agent
Progress: claimed
Type: task
Blocked by: 08

## Question

What dispatch policy should preserve latest pending demand within a lane while
preventing continually renewed lanes from indefinitely overtaking others?
The [read-only audit](../../../artifacts/production-readiness/preview-scheduling-audit.md)
gives a three-lane conditional trace and a deterministic regression design.
The current newest-first rule has no dispatch-opportunity bound. No native
starvation frequency or multi-layer throughput was measured.

## Acceptance

- Choose and document a cross-lane policy that accounts for interactive scrub
  freshness and poster work. Requested image size is not caller intent.
- Exercise the actual request/dispatch/publication seam, preserving latest
  time, in-flight rejection, cancellation and the 32-pending-lane bound.
- Retain admission age across pending replacement if fairness depends on it.
  State a dispatch-opportunity bound independently of elapsed decode time.
- Keep the four retained video-decoder resource policy. Five uncached video
  lanes can churn those slots; changing ordering alone does not fix that cost.
- Measure cold and multi-layer delivery separately before claiming real-time
  playback. Fair selection cannot guarantee publication when a slow in-flight
  result is continually superseded.

## Comments

This ticket records a policy gap rather than selecting FIFO silently. Pure FIFO
could put a fresh scrub behind cold posters. Poster admission/retention is
[a separate frontend correction](09-thumbnail-retention.md), which may reduce
that workload but does not itself establish fairness.

## Selected policy for the next implementation round

Root accepts the [source-only recommendation](../../../artifacts/production-readiness/preview-dispatch-policy-proposal.md)
and its independently reviewed rank argument. Preserve bounded first-admission
order across pending replacement; prefer the oldest interactive demand for at
most two consecutive bypasses, then dispatch the global oldest. Both the
pending order and bypass accounting remain private to the media module.
Add compatible `request_poster(asset, time, size)` for declared poster demand;
existing `request_frame` remains interactive. The desktop poster call-site
must use the new purpose. Purpose does not change lane/result identity.

An accepted continuously pending episode is selected within
`3 * (older_pending_count + 1) <= 96` dispatch opportunities. Stable cohorts
with one purpose drain within 32. Selection removes a pending entry and counts
even if later decoding fails or supersession rejects its result. Preserve order
across purpose changes; no enqueue/replacement/publication reset may erase the
bypass debt. A bounded deque avoids ordering the wrapping request sequence as
an admission clock. Cancellation/empty-queue transitions reset the policy.

This supplies no elapsed-time, publication or throughput guarantee. A full
32-poster queue can still return Busy to a new viewer; assigning purpose does
not reserve capacity or authorize discarding another admitted lane. Retain
the current four-decoder policy and every issue08 delivery/currentness guard.
The initial two-bypass allowance needs cold-scrub/multi-layer measurements
before tuning. Equal asset/size consumers still coalesce under the existing
lane contract; independent equal-size delivery is a separate design.

Root [verified the proposal's references and unchanged source hashes](../../../artifacts/production-readiness/preview-dispatch-policy-root-verification.json).
Begin with the three renewed interactive lanes at the actual
request/dispatch/publication/poll seam, retaining meaningful behavioral RED
before correction. Priority tests need a behavior-preserving explicit-purpose
interface scaffold before their RED run. Preserve single-worker in-flight
ownership and revise old newest-first test setups explicitly, without weakening
the finite-delivery assertions. All source and binaries remain frozen for R14
native acceptance, so implementation has not begun and progress remains open.

The later [consumer-identity review](../../../artifacts/production-readiness/preview-consumer-identity-review-response.md)
finds an independent frontend transition gap. Coordinate opaque consumer
identity and result routing with [issue12](12-preview-consumer-identity.md) at
the same media seam. Preserve continuing bindings and same-asset cut handoff;
purpose does not substitute for identity. The rank bound still applies to
accepted pending lanes with explicit consumers. Preserve the already prepared
R14 benchmark and explicitly record any event-reader source migration.

At the R14 freeze release, actual backend RED was retained (2 failures/1 control,
then purpose scaffold 6 failures/2 controls) and bounded scheduling focused
GREEN passed. Full media validation and poster migration/native
confirmation were pending at that stage.

Root subsequently passed all 76 media tests and warning-denied all-target
media Clippy. The [backend proof](../../../artifacts/production-readiness/preview-dispatch-round1/phase-b-backend-verification.json)
retains original/scaffold RED snapshots and logs, exact changed-source hashes
and focused/full-media GREEN. The two history-saturation fixtures now exercise
the pruning seam directly without assuming obsolete newest-first dispatch or
introducing a second in-flight worker. Poster migration and final native
confirmation were pending at that 76-test stage; this is not accepted elapsed-latency or playback
performance proof.

## Comments

R15 now includes the poster migration and bounded scheduling in the
[521-test source/release checkpoint](../../../artifacts/production-readiness/final/round15-build-record.json),
with nine passing architecture/media/fmt/lint/source-target/Linux build checks.
Native dispatch confirmation remains unrun; [three R15 startup captures](../../../artifacts/native/round15-window-captures/root-visual-verification.json)
establish presentation only. The [81-case optimized worker comparison](../../../artifacts/production-readiness/multilayer-preview-round3/root-measurement-verification.json)
passed root verification; its sequential-worker scope excludes poster competition
and native presentation; the [completed independent audit](../../../artifacts/production-readiness/multilayer-preview-round3/audit-attempt-01.md)
agrees with arithmetic and exact build attribution. Earlier pending notes above describe their
own stages; progress stays claimed until required native evidence is complete.
