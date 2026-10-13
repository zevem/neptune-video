# Bound retained viewer textures without losing active composition

Status: ready-for-agent
Progress: claimed
Type: task
Blocked by: 12

## Question

The [Opus controller/identity review](../../../artifacts/production-readiness/preview-consumer-identity-review-response.md)
finds no byte/count cap on Textures.layers. Its 120-painted-frame age limit
releases idle lanes but cannot bound all concurrently drawn lanes or wall-clock
retention while paused. The 32-pending queue and four retained decoders limit
different resources; requests can be accepted serially. Poster retention's
256-item/64 MiB policy does not constrain viewer layers.

This is a source ownership finding, not measured GPU memory. Define and verify
a bounded viewer working set while preserving the complete active composition
and the product's unlimited editable tracks. Silently dropping active pictures
to pass a cache test would not satisfy the editing requirement.

## Acceptance to specify after identity and measurements

- Keep separate logical retained texels, upload staging, driver allocations and
  measured process/GPU resources.
- Exercise actual retention beyond a deliberately small budget, preserving
  RED before correction. Cover late arrivals, replacement, source/project
  change, resize/zoom and idle release.
- Choose a bounded rendering/admission strategy that preserves active
  composition, source-monitor isolation and own-frame/cut continuity. Explicit
  preview quality adaptation or a bounded compositor must be reviewed as
  product behavior; no incidental size-based identity or arbitrary track cap.
- Measure distinct-video cold/multi-layer delivery and current native resource
  behavior before increasing concurrency or claiming real-time playback.
- State remaining native GPU/presentation limits; target execution remains
  deferred while macOS/Windows tests are prepared here.

## Selected policy and preparation

Root selects the [Opus ledger/admission recommendation](../../../artifacts/production-readiness/viewer-working-set-proposal.md):
64 MiB logical RGBA, a common quality cap over the complete active set,
request-time reservation of `max(kept, granted_box)`, and at most eight undrawn
layers in spare room. Preserve cut handover before reclamation and own pictures
while successful shrinks are pending. No CPU staging queue or backend
compositor/API is needed. Existing grants do not grow during playback.

An actual failed shrink may clear that failed oversized preview to release
room; successful active pictures must not be dropped to satisfy the bound.
The mathematical one-texel limit must report an actual nonzero `unplaced`
result to the person, rather than silently hiding it. The proposal's first
four-to-five transition fits 62.5 MiB; the sixth reaches 68.75 MiB and waits
until a shrink releases room.

Root executed the frozen Opus phase A: 173 desktop tests, 160 passing and
13 `ISSUE13` assertion failures, with no compile or fixture failures. All
original 152 tests and eight new controls passed. The texture-manager byte
bounds and actual viewer paint seam reproduced the fault; see
[root verification](../../../artifacts/production-readiness/viewer-working-set-round1/phase-a/root-verification.json).
The fresh phase B implements the selected policy after this RED;
root executed all 177 desktop tests and warning-denied all-target desktop
Clippy successfully. The original 21 regression tests remain byte-identical;
four added controls cover failures, notice edges, an audio source monitor and
actual window resizing. See [GREEN proof](../../../artifacts/production-readiness/viewer-working-set-round1/phase-b/root-green-verification.json).
The read-only Opus review finds no P1/P2 ledger defect; it confirms a
pre-existing error/retry loop and flags playing admission beyond 32. Root
reproduced both under [issue16](16-preview-request-liveness.md). Its correction
and frontend hint choice are now included in R15. The separate
[canonical bridge proof](../../../artifacts/production-readiness/frontend-retention-bridge-round1/root-verification.json)
observes only the nonempty hint and restart reset; native GPU/wake delivery,
video timing and other lifecycle sites remain unverified.

## Comments

Logical 64 MiB `max(kept, granted_box)` reservations/common quality cap/eight
inactive spares are included in the [521-test R15 build](../../../artifacts/production-readiness/final/round15-build-record.json).
The original RED/GREEN records remain intact. The completed review found no
P1/P2 ledger defect; the separate failure/admission correction passes
[21 focused and 198 desktop tests](../../../artifacts/production-readiness/preview-request-liveness-round1/phase-b/root-green-verification.json).
This is logical retained/request accounting, not upload/staging/driver or
measured GPU memory. Native acceptance remains open; progress stays claimed.
