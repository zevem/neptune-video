# Performance acceptance

Status: ready-for-agent
Progress: resolved
Type: task

Finish performance acceptance on the current Linux host: large timelines,
sustained five-video playback, random/held scrubbing, bounded resource
observations and visible/minimized idle CPU. Fix measured budget failures,
record exact optimized build identities and raw evidence, then stop.

The predeclared [budget plan](../../artifacts/performance-acceptance/round1/PLAN.md)
uses 100,000 clips / 1,000 tracks, two 120-second five-source 1080p30 loops,
seeded random seeks, process-tree/GPU observations and repeated idle samples.
Full physical presentation/A-V, other native platforms, format diversity,
huge imports and hours-long soaks remain separate gates.

Two metadata failures were observed: expired-prefix query tails exceed 200 us;
one rebuilt BTree validation checkpoint exceeds 16.667 ms move/undo p95.
Immutable-root interval bounds and membership-only UUID validation address
those owning seams. Preserve overlap order, exact rollback, undo/redo and
serialized project shape. The [findings](../../artifacts/performance-acceptance/round1/REPORT.md)
carry the final pass/fail verdict and limitations.

## Answer

Performance measurements and owning-seam fixes are complete for source
`b09ea17e` / optimized Linux `aster` `9b9ff3f0`. The
[audited results](../../artifacts/performance-acceptance/round1/acceptance.json)
pass metadata/query, exact history/gesture reversal, finite memory/decoder/GPU
budgets and repeated visible/minimized idle. Two 120-second playback runs and
one large-input repeat pass with task-owned affinity to CPUs 0–15.

Overall default-host acceptance remains open: unpinned shared-host playback
misses cadence (and one seek/loop trial); an unpinned large zoom tail is
211.809 ms. CPU control supports the placement/host-variation hypothesis and
does not ship as a product fix. Keep these failures in the
[handover](../../artifacts/performance-acceptance/round1/REPORT.md), alongside
physical/audio/platform/format/concurrency/hours-long limitations. No release
claim or platform pack changes. Stop after final evidence and cleanup checks.
