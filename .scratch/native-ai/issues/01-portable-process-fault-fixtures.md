# Native provider process-fault fixtures

Status: ready-for-agent
Progress: claimed
Type: task

## Problem

The provider process adapter contains a Windows suspended-launch/Job Object
implementation; its original process-fault regressions used Unix shell
fixtures. Windows GNU library typechecking does not execute thread discovery,
job assignment, blocked pipes or descendant teardown. The existing Linux
escaped-process regression is useful and remains separately scoped to Linux.

## Implementation boundary

Add a small standard-library Rust example as an owned test helper, excluded
from shipped application binaries. Build it sequentially before process tests
and pass its absolute executable path explicitly. CI must fail if the required
fixture is unavailable; it must not silently count a skipped fault scenario as
proof. Invoke helper modes directly with `Command`, without shell interpolation,
credentials, real providers or user configuration.

Preserve the production `Process` API and lifecycle. Tests belong at that
private owning seam, rather than mirroring Job Object implementation details.
The helper must operate only on descendants it creates, report their identity
through a bounded machine-readable line and retain finite fallback lifetimes.

## First acceptance cases

1. A helper never reads stdin. Write more than pipe capacity through the real
   process adapter, then cancel. Require `Cancelled` and bounded process/reader
   teardown, without waiting on the editor thread.
2. A helper starts itself as a descendant inheriting stdout/stderr, reports its
   PID and exits while the descendant holds the pipes. Acquire a process handle
   for that exact owned descendant before teardown. On Windows, require the
   handle to become signalled after Job Object closure and all reader joins to
   finish within the test deadline. Keep an owned fallback cleanup handle for
   assertion failure; never terminate a process by a reused PID alone.
3. Port output-frame/stderr flooding and full-queue cancellation to helper modes
   after the two ownership cases pass. Keep output and total fixture lifetimes
   bounded even when a test assertion fails.

Run the cases natively on the Windows MSVC CI runner and retain helper/build/
test records. macOS can reuse the portable cases; Linux keeps its additional
escaped-session coverage. Local Linux checks and cross-target typechecking are
preparation evidence until the native target executes these paths.

## Comments

2026-10-12: Read-only backend acceptance audit found no concrete blocking
source defect. This ticket closes a missing target-specific lifecycle proof;
it does not establish desktop GPU/audio, provider sign-in, packaging or general
production readiness. Current final UI acceptance is running against a frozen
release; implementation waited until that task returned source/Cargo ownership.

2026-10-12 implementation checkpoint: the std-only example and private adapter
tests are integrated. Seven transport cases run on every native target; a
Windows/Linux descendant case retains an exact process handle/pidfd before the
parent exits. No production transport code changed. Local Linux focused tests
passed 12/12 and the workspace passed 312 tests. The sequential test runner
builds the example and obtains its absolute path from Cargo's artifact event;
missing fixture configuration fails. Native Windows/MSVC and macOS execution
remain required before resolving this ticket. The Mac cross-target lint found
a leftover Linux-only shell helper and its cfg was corrected; the failure log
is preserved beside the corrected cross-target check.

2026-10-12 later verification: the immutable
[round-eight build record](../../../artifacts/production-readiness/final/round8-build-record.json)
identifies source `3a4cb7d7` and 313 passing Linux workspace tests, zero failed or
ignored, including 81 agent tests. The portable adapter's
[12 focused Linux cases](../../../artifacts/production-readiness/portable-fixture/adapter-tests.log)
passed. Full-workspace/all-target compile/lint with warnings denied also passed
for macOS ARM64, Windows MSVC and Windows GNU on Linux, as recorded in
[round-eight checks](../../../artifacts/production-readiness/final/round8-checks.json).
Those targets were not linked or executed. Native macOS/MSVC CI fixture runs
remain pending; this ticket stays open. Later PCM regression work is outside
this named source/build record.
