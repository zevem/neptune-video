# Preserve admitted paused preview demand until delivery or explicit failure

Status: ready-for-agent
Progress: resolved
Type: task

## Question

Can the backend silently discard an admitted preview request/result while the
paused desktop keeps that exact demand marked as pending and never retries?
The [read-only source audit](../../../artifacts/production-readiness/multi-layer-preview-source-audit.md)
identifies two finite conditional traces: eviction from the 64-entry latest
history while a lane is still queued, and eviction of undelivered FrameReady
events from the 64 MiB result budget. These are source-derived, not executed
or native failure proof. The four retained decoder lanes are a separate cache
policy and must not be raised to mask delivery loss.

## Acceptance

- Reproduce each finite trace at the actual private queue/current/publication
  seam, with deterministic scheduling and retained failing results.
- Protect queued/in-flight request identity while keeping bounded metadata.
- Preserve bounded frame/result bytes through cancellable backpressure or an
  explicit delivery-failure path that lets the existing desktop retry.
- Prove latest-wins replacement, project cancellation and shutdown still reject
  stale results and unblock any waiting worker.
- Verify a finite paused multi-layer demand settles with all expected results;
  performance and native presentation claims require separate measured runs.

## Comments

Root completed callback liveness and independently verified the R12 native UI
task's source, captures and cleanup. Source ownership is released; root now
owns the actual queue/publication red regressions and correction. The original
source audit alone does not claim either loss path reproduced or fixed.

The [actual before-fix log](../../../artifacts/production-readiness/preview-delivery/before-fix.log)
now reproduces eight failures with three controls passing. The corrected
production seams pass all [11 regressions](../../../artifacts/production-readiness/preview-delivery/after-fix.log)
and [66 media tests](../../../artifacts/production-readiness/preview-delivery/all-media-tests.log).
The [independent correction review](../../../artifacts/production-readiness/preview-delivery-review-response.md)
finds no further concrete defect and records direct-interleaving proof limits.
The [R13 checkpoint](../../../artifacts/production-readiness/final/round13-build-record.json)
has 365 passing workspace tests, passing actual FFmpeg preflight, formatting,
Linux/three-target source Clippy and optimized binaries. Root independently
checked its [74-file inventory, binary/library hashes and retained logs](../../../artifacts/production-readiness/final/round13-backend-root-verification.json).
The source/binaries are frozen for native editing/five-layer acceptance
and a separate public five-PNG diagnostic. These runtime checks remain pending;
the private shared-Arc fixture alone is not native composition or five distinct
decode-allocation evidence.

## Answer

Admitted pending/in-flight lanes retain their identity within the existing
64-entry latest history. Publication removes only an obsolete result for the
same lane; other current results wait for byte/count capacity with the mutex
released. Poll, supersession, project cancellation and shutdown wake that wait.
The importer now observes both the four-import and 64-total-outcome limits.
Decoder concurrency and cache budgets were not increased.

The corrected actual seams pass 11 tests after eight reproduced failures.
The 365-test R13 checkpoint and independent correction review pass. Two public
diagnostics decode five distinct 1920×1920 PNGs, retain five separate live
payloads and deliver every current identity after polling is withheld. Root
[verified their exact pixels, hashes, libraries and 18 process/thread identities](../../../artifacts/production-readiness/final/round13-public-preview-root-verification.json).
The second run adds two recorder assertions; both runs and the first compiled
source remain preserved. The public observation does not prove when the fifth
decode finished or that its publisher had parked.

The optimized native R13 editor passes all 13 smoke checks, exact whole-project
gesture undo/redo, source preview and narrow selection. Three fresh paused
launches show all five image regions without input, remain stable, then show
the second five-region pattern after a seek and restore the first with Home.
Root [checked 74 sources, both binaries, 35 captures and all 31 tracked processes gone](../../../artifacts/production-readiness/final/round13-native-editing-root-verification.json).
Two earlier harness failures are retained; no product failure was reproduced.
Native request dimensions and mailbox waiting were inferred/unobserved, while
the private regressions prove the actual wait seam. This closes finite paused
delivery correctness, not sustained multi-layer throughput or dispatch fairness.
