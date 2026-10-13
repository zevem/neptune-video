# Keep overlapping clips visible in a scrolled timeline

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 08

## Question

Can viewport culling skip a long visible clip when later clips on the same
track end before the viewport? `timeline.rs` uses `partition_point` on clip
ends although core validation orders clips only by starts and allows overlap.
For intervals [0,100], [1,2], [3,4] and viewport start 50, the predicate is
false/true/true and is not partitioned. The Opus R13
[read-only source observation](../../../artifacts/native/post-preview-delivery-round1/report.json)
confirms this mismatch; native reproduction remains to be executed.

## Acceptance

- Extract/exercise the actual current culling seam without changing behavior
  first, retaining the failing overlapping/nested-interval regression.
- Query start-sorted intervals without assuming end monotonicity. Preserve
  stable order, correct boundary/empty-range behavior and viewport filtering.
- Use the same correct visible interval selection for thumbnail admission and
  painting; retain overlapping long clips after scrolling past short clips.
- Verify actual native visibility and selection of the long clip on an owned
  project, with document/history unchanged by scrolling or selection. Source
  arithmetic alone is not native defect reproduction or acceptance.
- Keep scans limited to start-qualified candidates; document any remaining
  cost before introducing an index or claiming logarithmic queries.

## Comments

The model's active-clip query already partitions by starts and filters ends.
The renderer's end predicate is the separate concern. The R13 source is released
after root's native/public delivery verification; subsequent changes require a
new build and acceptance record.

R13 root native/public verification passes and the freeze is released. The
frontend task extracts the actual renderer's culling seam without changing
behavior and adds a failing overlap regression. A backend task covers a borrowing,
allocation-free `Track::clips_overlapping(start, end)` query and core tests;
the frontend may adopt it in the subsequent correction round. Root owns all
Cargo checks. Neither the source observation nor the new core helper alone
will be called native timeline acceptance.

The new core query now passes all 52 core tests and denied-warning all-target
Clippy on Linux. Its two new tests use real editing commands, 21 range cases,
borrowed stable-order clip references and retained snapshot/move/undo/redo
behavior. Root [records the exact core source and log hashes](../../../artifacts/production-readiness/preview-delivery/core-clip-intervals-verification.json).
The old renderer's actual-seam RED and corrected native visibility remain
pending; the core query is ready for subsequent frontend adoption.

The actual renderer culling helper is now exercised in root's
[phase-A timeline run](../../../artifacts/native/cache-culling-round1/phase-a-timeline-red.log).
Three long/nested/stable-order overlap cases fail, alongside the offscreen-poster
admission case; six timeline controls pass. The
[root record and original source byte copies](../../../artifacts/native/cache-culling-round1/phase-a-root-verification.json)
preserve all seven combined cache/culling failures and 13 controls. The next
task covers the correction; native visibility remains a later required check.

The [concurrent design review](../../../artifacts/native/cache-culling-round1/phase-b-design-review.md)
requires shared drawn row/clip selection for painting and poster admission,
including the actual height gate rather than a video-track-only assumption.
Source was moving during that review, so final implementation validation remains
pending root's frozen-source tests and native verification.

The correction now uses the shared core interval query and the same drawn
row/clip iterator for painting and poster admission. Root's corrected runs pass
all 14 timeline tests and 18 texture tests; the
[frozen Opus review](../../../artifacts/native/cache-culling-round1/phase-b-review-response.md)
found no blocking defect. Geometry-only lane fixtures are not proof of valid
audio-track runtime behavior, and no logarithmic interval index is claimed.

R13's actual native baseline reproduces the defect: at viewport 49.97–59.83 s
the [0,100] long text clip is absent, though clicking the empty lane selects
that clip. Revision, history and document/file bytes remain unchanged. Root
independently verified the
[baseline captures, fixtures, process identities and old binary](../../../artifacts/native/cache-culling-round1/baseline-root-verification.json),
preserving the first failed harness attempt. Full R14 validation passes
397 tests, formatting, Linux/three cross-target source Clippy checks and
optimized builds. Root independently verified
[the exact source/binaries and retained logs](../../../artifacts/production-readiness/final/round14-backend-root-verification.json).
Actual corrected visibility/selection is now under native verification.

## Answer

The renderer now uses the borrowing half-open Track::clips_overlapping query
and shared drawn row/clip selection for painting and poster demand. The actual
old renderer's long/nested/stable-order regressions are preserved RED, the
corrected focused tests pass, and the full R14 build passes 397 tests.
The query scans start-qualified candidates after the start partition;
O(log n + prefix) remains explicit and no interval index is claimed.

The [current native report](../../../artifacts/native/cache-culling-acceptance-round1/report.json)
shows the long [0,100] clip filling the 49.97–59.83 s view (1.0 versus old
R13's 0.0016) and correctly selected. Normal and 760×560 captures show the
selection outline through the visible lane. Scrolling/selection leave
revision zero, document/history and file bytes unchanged. Root viewed both
captures and [verified their hashes with the exact source/binaries and cleanup](../../../artifacts/production-readiness/final/round14-native-editing-root-verification.json).
The exact inclusive-hit/half-open-paint trim-edge behavior remains unobserved
and is not implied by this interval-culling resolution.
