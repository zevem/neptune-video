# Native AI editing

Status: ready-for-agent
Progress: claimed
Type: task

The original problem was the missing model runtime in the desktop: the phrase
interpreter and external editing protocol provided command foundations. The
native runtime is now implemented; acceptance and platform scope are recorded
below.

## Contract

- Rust owns bounded asynchronous provider processes. Use the installed Claude
  or Codex CLI and its existing sign-in without reading or copying credentials.
- Adapt Neptune's provider/process discipline, including cancellation and
  output limits. Keep platform support independent of its Unix adapter shims.
- A request captures project, editing context, revision, selection and playhead.
  Generate typed command proposals and validate them against that snapshot.
- Validate an accepted proposal through one ordinary reversible Batch command
  once, then commit that exact prepared state, including generated identities.
  Changed/reopened/replaced projects reject stale proposals atomically.
- The model receives editing metadata, not source-media bytes. Provider tools,
  unrelated MCP servers, shell execution and project instruction discovery are
  disabled for planning. A provider failure never becomes a fake success.
- Provider/model/effort configuration, progress, cancellation and proposal
  review are reachable from the native interface.
- Verify actual signed-in providers on generated task-owned project data,
  alongside deterministic process, protocol, validation and cancellation tests.

Hosted HTTP credentials and local-model adapters may be added later without
changing the command/proposal interface. Native CLI integration is the first
real provider adapter, not proof of every platform's runtime acceptance.

## Current acceptance work

The runtime and protected document/recovery APIs are implemented. Prepared-edit
tests prove exact reviewed split/link/group identities, one changed-batch
undo/redo and stale-root rejection. Claude safe mode and Codex owned nonempty
instructions isolate customization while retaining each CLI's sign-in.
On the db30/9f checkpoint, actual native Ask requests applied correct splits
with exact undo/redo, and GTK Save As refused existing files for legacy/private
unbound recovery before adopting and subsequently saving a fresh destination.
One Codex proposal described a split but proposed an unchanged transform and
was discarded; review of intent remains necessary.

The historical native-verified source `28259254` / release `2c596cb0` adds
explicit "No changes" handling and passes 309 workspace tests. Its native acceptance passed all four
required gates plus file/folder import, with both real providers completing a
correct split on their first request and exact undo/redo. The labeled owned
no-op fixture preserved clean state/revision/redo and reported only the quiet
"No changes" card, without an Apply toast. Historical reports retain their own
source/binary scope.

The latest immutable
[round-fifteen build checkpoint](../../artifacts/production-readiness/final/round15-build-record.json)
is source `dfbb53e9` / Linux release `362ba180`: 521 tests passed, zero failed or
ignored, including 81 agent tests. Full workspace/all-target source checking and
denied-warning Clippy passed on Linux and for Mac ARM64, Windows MSVC and GNU;
those cross-target checks did not link or run native applications.
[Independent backend verification](../../artifacts/production-readiness/final/round15-backend-root-verification.json)
checked 83 source files, binaries, the release graph and nine passing stages.
R15's [thirteen native editing/input checks](../../artifacts/native/round15-editing-smoke/root-verification.json)
pass, including exact reversal, command-panel rename, durable save and stale-plan
refusal. Root viewed five normal captures and checked eleven sampled process
identities gone. No inference was requested; wake/cache/multi-owner acceptance
remains unrun. [Three startup captures](../../artifacts/native/round15-window-captures/root-visual-verification.json)
verify only normal/compact presentation, with unavailable audio and no provider
requests. Earlier cache/overlap/preview/narrow evidence is R14 `f74d6560` / `8b446cca`, recorded in the
[accepted build record](../../artifacts/production-readiness/final/round14-native-editing-build-record.json)
and [Linux editing/preview/narrow/cache/overlap acceptance](../../artifacts/native/cache-culling-acceptance-round1/report.json).
That run passed with audio unavailable, unchanged
identities, 144 verified captures and all 75 recorded process identities gone in
[root verification](../../artifacts/production-readiness/final/round14-native-editing-root-verification.json).
Earlier R13 three-launch paused five-layer acceptance preserved the document and history.
[Two public five-PNG probes](../../artifacts/production-readiness/final/round13-public-preview-root-verification.json)
independently delivered exact distinct pixel payloads. Native request dimensions
and publisher parking were not observed, and the probes provide no throughput or general
scheduling guarantee.
The desktop run made no provider requests and repeated no GTK-dialog acceptance or performance
measurements. Native interface performance remains round nine's `b29ccb69` / `14435a4a`, 333-test
checkpoint. Native real-provider AI evidence above remains `28259254` / `2c596cb0`.
The separate [optimized R14 worker diagnostic](../../docs/performance.md#optimized-distinct-video-worker-diagnostic)
records distinct-video latency, not native presentation or new dispatch/consumer
acceptance. Follow-up source work is outside the accepted R14 checkpoint.
Earlier viewer/liveness RED remains preserved. The
[correction GREEN](../../artifacts/production-readiness/preview-request-liveness-round1/phase-b/root-green-verification.json)
has 21 focused/198 desktop tests and is included with 112 media tests in R15.
The [completed review](../../artifacts/production-readiness/preview-request-liveness-round1/review-round1.md)
finds no P1/P2 behavior defect. The later [canonical bridge proof](../../artifacts/production-readiness/frontend-retention-bridge-round1/root-verification.json)
and [independent review](../../artifacts/production-readiness/frontend-retention-bridge-round1/review-round1.md)
close the nonempty call and `cancel`/`ask_again` reset sites at frozen R15:
four cases measured 7/10 and 14/20 starts, with 60 full-pixel checks. This adds
no standing suite guard; retire/clear/Closed, source/cut/reorder and playing
remain unproved. The separate
[81-case optimized worker comparison](../../docs/performance.md#optimized-r15-legacyexplicitprotected-comparison)
passed root verification, with native/GPU/audio and frontend setter/reset scope
excluded. Complete R15 input, wake/cache/multi-owner and platform-pack integration remain
pending; no new provider result follows. Six prepared bookkeeping tests were
declined, not integrated or executed; the 198 desktop/521 workspace counts
are unchanged.
Round twelve's nine liveness regressions and independently verified
[virtual silent-stall recovery](../../artifacts/production-readiness/final/round12-virtual-stall-root-verification.json)
establish public clock release, continued Playback and natural decoder cleanup
on owned zero-PCM routes. Its separate
[healthy physical silence-clock probe](../../artifacts/production-readiness/final/round12-physical-clock-root-verification.json)
checks the actual inherited output clock at 1×, seek, 0.5× and 2×. Neither is
native fatal error-callback, acoustic, A/V or performance acceptance; the UI run
opened no output stream.

Windows/macOS native execution is deferred by the user's instruction to prepare
tests locally for now. The [native pack](../../docs/native-platform-tests.md)
now has 64 cases, every native case `not_run`, and [106 offline guards passed on the pinned pack](../../artifacts/native/platform-regressions-integration-round1/root-pinned-attempt-01/root-verification.json).
Fixture bytes and ten Linux native startup openings are verified, without
catalog passes; pins are set and [final independent review](../../artifacts/native/platform-regressions-integration-round1/review-round2/REVIEW.md)
closed all four preparation findings with no P1/P2. Provider/process behavior on those hosts,
distribution and broader production gates remain open in
[verification](../../docs/verification.md).
The R14 51-case/57-guard baseline and [eleven-case draft](../../artifacts/native/platform-regressions-round1/README.md)
remain historical. No native platform execution follows from local preparation.

## Comments

2026-10-14: The Agent tab's Ask mode now defaults to Made as it works, a
live conversation whose only tools are `aster-mcp` pinned to the window
([in-app agent spec](../in-app-agent/spec.md),
[ADR 0011](../../docs/adr/0011-the-agent-tab-edits-through-aster-mcp-started-in-its-window.md)).
The planner this spec describes stays as Proposed for review
(`AiSettings.review`) and still serves caption translation; its contract
above is unchanged. "Provider tools, unrelated MCP servers, shell
execution and project instruction discovery are disabled" and "the model
receives editing metadata" now hold for the planner only: the live agent
reaches Aster's MCP tools, and rendered frames, contact sheets,
transcripts, measurements, and file names and paths inside its roots reach
the provider. Decided on the
owner's behalf (2026-10-14): Made as it works is the default for new and
existing settings. Verified on one macOS host with debug builds
([native runs](../in-app-agent/evidence/native-runs-2026-10-14.md));
Linux, Windows and release builds are unverified.

2026-10-15: The live agent's `aster-mcp` is no longer pinned to the
window. It starts in the window and, like an outside MCP agent, may create,
open and close projects of its own and come back; its roots are live
through `--roots-file` and include folders the person adds in Settings ›
Agent › Folders ([in-app agent spec](../in-app-agent/spec.md),
[issue 06](../in-app-agent/issues/06-same-reach-as-an-outside-agent.md)).
Decided on the owner's behalf (2026-10-15): the in-app agent has the same
reach as an outside agent. The planner and its contract above are
unchanged. Covered by tests on one macOS host; native runs are pending and
will be recorded in [verification](../../docs/verification.md#in-app-agent).
