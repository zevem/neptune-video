# Avoid cyclic video-decoder churn without unbounded native processes

Status: ready-for-agent
Progress: claimed
Type: task

## Evidence

The independently verified [optimized R14 worker diagnostic](../../../artifacts/production-readiness/multilayer-preview-round1/FINDINGS.md)
shows sequential complete-batch p50 increasing from 0.764 ms at four distinct
videos to 163.942 ms at five. Cached frames remain fast. That public experiment
does not observe exact private decoder starts.

The new decoder-manager characterization executed in the initial 89-test media
run: two same-order uncached rounds of five distinct videos require ten starts;
the four-video control requires four, and exact cached repeats require no extra
starts. Every call checks full lossless RGBA and at most four retained pipes.
This is separate structural evidence at the unchanged production retention
algorithm, not a native playback or same-codec causal timing experiment.

## Selected policy

The [retention proposal](../../../artifacts/production-readiness/decoder-retention-proposal.md)
recommends three protected active streams and one overflow slot, reclaiming
inactive owners first. It needs an honest retention-activity signal distinct
from consumer identity/currentness. Blind MRU or permanent pinning can trap old
inactive sources and regress changing working sets. Root selected the optional
[three-consumer retention hint](../../../artifacts/production-readiness/decoder-retention-hint-proposal.md).
The caller replaces a bounded protected subset; the four-pipe manager evicts
the oldest unprotected pipe. Empty hints retain the existing LRU behavior.
Hints do not authorize results or register all active owners. Normalize the
subset, protect its existing latest tags, and reclaim against latest lanes
union hints (at most 67). Apply new-generation hints after clearing the pool,
and carry the same snapshot through idle and blocked-publication maintenance.
The backend interface and policy are implemented after actual decoder-manager
RED. Preserve same-consumer continuous-source reuse across cuts and the
separate pixel cache.

Do not raise worker/pipe limits or silently drop active pictures to pass a
test. Bound native resources, exercise larger and changing cohorts, source
replacement, cancellation/retirement and exact cleanup. Observe meaningful
structural RED before correction, then repeat the optimized public workload
against the exact corrected build. Distinguish process counts, worker latency
and actual native presentation.

## Partial verification

Root reproduced two manager assertion failures (five videos: ten starts
instead of seven; eight: sixteen instead of thirteen), with nine controls
passing. The corrected eleven manager controls passed. Current full media
tests pass 112/112 and all-target warning-denied media Clippy passes.
[Root proof](../../../artifacts/production-readiness/decoder-retention-round2/backend-root-verification.json)
records exact source/log hashes and the preserved RED.

The actual worker applies new hints after a generation change (one old start,
five new initial starts and two next-frame starts), with full lossless pixels.
After 65 image completions prune a warm consumer's latest tag, installing its
hint and awaiting unrelated completion preserves the pipe for its uncached
next frame. Off-mutex count/byte pressure, retirement and currentness controls
also pass. The generation counts confirm hint adoption; clearing itself is
separately source-reviewed. At this backend-only stage, frontend adoption and
optimized/native comparison were pending; these counts establish no FPS or
real-time playback claim.

## Comments

The backend API and frontend stable at-most-three granted/drawn VIDEO chooser
are included in the [R15 checkpoint](../../../artifacts/production-readiness/final/round15-build-record.json)
(521 tests, nine source/build stages). Seven selection controls pass, but the
[completed source review](../../../artifacts/production-readiness/preview-request-liveness-round1/review-round1.md)
identified G1. Six prepared bookkeeping tests were declined, not integrated or
run. The later [canonical bridge proof](../../../artifacts/production-readiness/frontend-retention-bridge-round1/root-verification.json)
and [independent review](../../../artifacts/production-readiness/frontend-retention-bridge-round1/review-round1.md)
observe the nonempty hint and restart reset at frozen R15: four cases measured
7/10 and 14/20 starts, 60 transient full-pixel checks and all 51 logged decoders
gone. This adds no standing suite guard; retire/clear/Closed, source/cut/reorder
and playing remain unproved. The 81-case
legacy/explicit/protected [optimized run](../../../artifacts/production-readiness/multilayer-preview-round3/root-measurement-verification.json)
passed all 81 cases: five-video sequential medians 185.490/179.190/67.395 ms,
ratio of legacy/protected medians 2.7523×. All 96 measured five-video sequential
batches in each mode still exceed 33.333 ms. Cold/random benefit, native FPS,
residual frontend lifecycle sites and paced slow-video evidence are not proved;
the [completed independent audit](../../../artifacts/production-readiness/multilayer-preview-round3/audit-attempt-01.md)
agrees with arithmetic/build attribution, while final native acceptance stays open.

The [independent-source read-ahead experiment](../../../artifacts/production-readiness/video-read-ahead-round1/FINDINGS.md)
rebuilt matched private R16 variants and verified exact continuously indexed
pixels. Five-owner source progression increased from 45/45/45/23/23 distinct
frames to 60/60/60/60/53 during three seconds of 30 Hz demand at 320 bounds.
Request-to-poll p95 regressed for four owners in this single trial. Delayed
publication, speculative ten-second reads and missing Playback-only eligibility
also prevent production selection. The current app is unchanged; sustained
presentation throughput remains open.

The subsequent [published-first correction](../../../artifacts/production-readiness/publish-first-read-ahead-round1/FINDINGS.md)
is integrated after 132 media tests, six matched independent-source optimized
trials and the 549-test workspace pass. Read-ahead is explicitly playback-only,
occurs after requested-frame publication, fits the existing cache, and uses one
absolute four-millisecond deadline with irreversible process expiry. Native
all-five advancement and thirteen editing checks pass on the rebuilt release.
Overflow-owner tails improve in the measured low-resolution preview workload,
with a modest protected-owner latency tradeoff. Sustained full-resolution/native
presentation throughput remains an unresolved release gate; this ticket stays
claimed rather than resolved.

The [larger-preview feedback](../../../artifacts/production-readiness/high-resolution-preview-round1/FINDINGS.md)
confirms exact pixels across twelve matched 1280/1920 trials, but gains remain
modest and five-video full-resolution source-rate playback is not met. Next work
must investigate decoder capacity, scheduling and RGBA transport rather than
repeat or generalize the small-preview result. No additional resource limit was
raised by this feedback pass.
