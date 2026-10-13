# Let failed paused previews settle and serve active owners beyond the queue

Status: ready-for-agent
Progress: claimed
Type: task

## Findings and scope

The [Opus budget review](../../../artifacts/production-readiness/viewer-working-set-round1/review-round1.md)
finds no P1/P2 ledger defect but confirms a source-level P2 failure loop.
A missing image/video can repeatedly fail while paused: the worker publishes
an error and wakes the app, `Textures::failed` clears the remembered request,
and the next composition reissues the identical request. The notice is also
renewed. The cycle rate and native resource cost are not measured.

The same review identifies an admission gap during playback. Requests enter in
paint order; the first pending owners can reclaim freed lanes while owners
beyond the 32-pending limit remain unadmitted. Accepted-lane dispatch fairness
does not establish fairness for rejected owners. Root subsequently reproduced
the gap with the real public worker; the controlled results are below.

## Acceptance

- Author actual public-worker regressions before any correction;
  root owns all execution. Missing image files need no external codec. A
  controlled wake callback may park the real publisher between complete frames,
  without replacing the service or simulating its queue.
- An unchanged failed paused request settles. Changed time, bounds or source,
  explicit `ask_again`, forgetting the asset and replacing a layer/project
  permit appropriate retry. Keep ordinary held pictures and release only a
  failed oversized shrink as the selected budget policy allows.
- A finite controlled playback cohort of 40 granted owners gets every owned
  picture despite the 32-pending queue, with exact identities and full pixels.
  Rotate admission or use another reviewed bounded strategy without changing
  paint order, increasing queue/worker limits or dropping active tracks.
- Preserve 64 MiB logical reservations, eight spares, cut/source/project
  identity, all current 177 desktop controls and 112 media controls. Add no
  idle polling timer and do not treat `Busy` as a decode failure.
- Verify real RED/controls, then correction; final native wake/idle/presentation
  and optimized video timings remain separate. Playing in-flight supersession
  and external wake delivery are not established by an image admission test.

## Work

Root executed the frozen Opus phase A: fourteen real public-worker tests,
eleven passing controls and three `ISSUE16` assertion failures, with no compile,
fixture or timeout failures. Both missing-media views retried in four unchanged
paused frames. Playing layers 32–39 received no picture in 80 controlled
publication results; the paused 40-layer control and exact pixel/identity,
valid-retry and failed-shrink controls passed. Task-owned temporary storage
was empty afterward. See [RED proof](../../../artifacts/production-readiness/preview-request-liveness-round1/phase-a/root-red-verification.json).

Opus phase B corrected failure suppression and fair request admission and added
frontend choice of the validated three-consumer decoder hint. The
[root GREEN proof](../../../artifacts/production-readiness/preview-request-liveness-round1/phase-b/root-green-verification.json)
passes all 21 focused/198 desktop tests and denied-warning Clippy, preserving
6,410 immutable artifacts and original test blocks before formatting. The
[R15 build](../../../artifacts/production-readiness/final/round15-build-record.json)
then passed all 521 tests and nine architecture/media/fmt/lint/source-target/
Linux release stages. Native failure/admission acceptance is unrun; [three R15
startup captures](../../../artifacts/native/round15-window-captures/root-visual-verification.json)
establish normal/compact presentation only.

## Comments

The [completed review](../../../artifacts/production-readiness/preview-request-liveness-round1/review-round1.md)
finds no P1/P2 behavior defect and identified G1. Six prepared bookkeeping tests
were declined without integration/execution. The later [canonical bridge proof](../../../artifacts/production-readiness/frontend-retention-bridge-round1/root-verification.json)
and [independent review](../../../artifacts/production-readiness/frontend-retention-bridge-round1/review-round1.md)
close the nonempty call and restart-reset sites at frozen R15 (7/10 and 14/20
decoder starts; 60 full-pixel checks). This artifact probe adds no suite guard;
retire/clear/Closed, source/cut/reorder and playing remain unproved. Time-less
errors can permit one bounded extra retry. A same-path restored file while
paused needs seek/resize retry, and playing offline video can fail/renew notices
for each new time. Forty-owner PNG controls do not prove slow/in-flight video,
native wake/idle, GPU or paced throughput. Progress stays claimed.
