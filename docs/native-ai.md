# Native AI editing

Aster's Rust backend invokes an installed Claude or Codex CLI to propose typed
edits to the captured project. The person reviews the proposal before applying
it through the same reducer and history as manual editing. Actual provider
completion and Linux desktop interaction, including unchanged proposals, are
verified below at named checkpoints. Source/build, desktop editing and provider
acceptance retain their recorded identities; native target/provider and broader
production gates remain.

## Runtime contract

The public interface lives in
[editor-agent](../crates/editor-agent/src/lib.rs),
[planner.rs](../crates/editor-agent/src/planner.rs) and
[ai.rs](../crates/editor-agent/src/ai.rs):

| Interface | Contract |
| --- | --- |
| `AiConfig` | Provider, optional executable/model/effort and 10–600 second timeout; no credentials. Default is Claude, CLI-selected model, high effort and 180 seconds. |
| `AiContext::capture` | Retains an immutable project snapshot, context ID, revision, selected clip IDs and playhead without provider or filesystem work. |
| `AiRuntime::request` | Accepts one request, returning its ID; a second active request returns `Busy`. Provider work and metadata serialization run on its worker. |
| `AiRuntime::poll` | Returns `Started`, validated `Proposal`, `Failed` or `Cancelled` events without waiting for provider work. |
| `AiRuntime::cancel` | Signals the owned request; cancellation wins a concurrent validation result. Dropping the runtime signals cancellation without waiting on the GUI thread. |
| `AiProposal` | Summary, typed commands, actual resolved provider/model where available, immutable before/after project states and captured guards. |
| `AiProposal::apply` | Rechecks context, project, revision and captured root, then commits the exact validated state. A changed batch adds one undo entry; a net no-op preserves revision, dirty state, root and redo. An empty proposal has no apply action. |
| `command_schema` / `proposal_schema` | Full external command schema and the narrower closed native-plan schema respectively. |

Selection and playhead are captured targets. Changing them later does not
retarget the request; editing, reopening, recovery or replacing the project
invalidates the proposal through its guards. Before delivery, one command batch
is reduced and validated against the full captured project. Invalid
references, source ranges, transforms or locked-track edits therefore leave the
live document unchanged. The core's `PreparedEdit` with private roots preserves the exact
validated roots, including IDs allocated by split/link/group edits. Apply
installs that root without reducing the commands again, preventing different
IDs from those shown during review. Public proposal display fields do not
confer authority to replace that validated state.
These checks establish editing correctness, not whether the suggested result
matches the person's creative intent; proposal review remains required.

The native-plan schema excludes `AddAssets` and `RestoreProject`, including
inside nested batches. Source import remains a native workflow, and checkpoint
restoration remains an explicit editing operation. The full authenticated
[external protocol](agent-protocol.md) retains those commands for clients that
already have the required metadata and authority. Every look, motion and speed
command, including `SetTracker` and `RemoveTracker`, is available to native
plans; the planner instructions carry their units and the parameter keys of each
effect kind. They state that `apply_preset` gives a look in place of the one a
clip has. A plan cannot start region tracking or a stabilization analysis:
both run in the desktop's analysis lane on the person's request, and the
planner is told to write tracker keys only from positions the person states.
The planner sees no pixels, so it cannot choose a key colour, a region or a
grade from the picture. No real provider request has been made with the
look, motion and speed instructions; their evidence is the schema-size and
planner tests in [planner.rs](../crates/editor-agent/src/planner.rs).

## Planning context and disclosure

The worker sends the person's request, selected IDs, playhead and serialized
project metadata. That metadata includes names, local source paths, text clips,
canvas settings, timing, transforms, track state and available source metadata.
Aster sends no source video, image or audio bytes, decoded frames or waveform
samples to this planner. Requests requiring scene understanding, transcription,
beat detection or semantic media selection are beyond this implementation; ask
for edits expressible from metadata and explicit targets.

[context.rs](../crates/editor-agent/src/context.rs) serializes a complete
project when it fits within 1 MiB. Otherwise it constructs a focused view:

- Selected clips and their transitive linked/grouped relatives remain exact
  and complete, even far from the playhead. Their referenced asset IDs and
  source duration/rate/dimensions/audio metadata remain available.
- Nearby clips are included as complete relationship components when they
  fit. Track state/order, bounded folder inventory and nearby markers are
  included with explicit total and omission counts.
- `context_scope` declares `mode: "focused"`, `complete: false`, completeness
  of selected relatives and referenced assets, omissions and guidance. Oversized
  optional names/paths are null only with an accompanying `omitted_fields`
  declaration; clip data and selected relationships are never silently cut.
- An animation track or tracker with more than 32 keys is sent as `keys: null`
  with `key_count` and `omitted_fields: ["keys"]`, in complete and focused
  documents alike. A focused view lists only the trackers its included clips
  follow, with the assets those trackers and the clips' effects reference.
- If the required selection and its relationship/source context cannot fit,
  planning fails with a request for a smaller independent selection. An omitted
  object still exists; its absence cannot justify deletion or imply an empty
  track/library.

The immutable project remains complete for validation even when the provider
receives a focused view. Five-thousand-clip regression fixtures cover exact
source limits, distant relationship closure, explicit omissions, complete
small-project shape and rejection of an oversized required component in
[context tests](../crates/editor-agent/src/context.rs).

## Provider boundary and process ownership

Install and sign in with the selected provider CLI separately. Aster resolves
its executable on the planning worker before entering a private temporary
working directory; an explicit configured path is relative to the application's
launch directory. `AiConfig` stores no credential values, and Aster does not
open or copy credential files. The provider CLI retains responsibility for its
own sign-in and account configuration. Requests use that provider's configured
model unless a model override is supplied; recorded resolved models are
observations, not promises of availability on another account.

The [Claude adapter](../crates/editor-agent/src/provider/mod.rs) uses
`--print --safe-mode --output-format json --json-schema`, with an empty tool set, strict
empty MCP configuration, empty settings sources, disabled slash commands and
no session persistence. Safe mode disables instruction/skill/plugin/hook
discovery while retaining CLI authentication. It requires a successful result with
`structured_output`. It deliberately preserves CLI subscription sign-in rather
than using `--bare`, which disables OAuth/keychain authentication according to
the [official Claude CLI documentation](https://code.claude.com/docs/en/headless).

The [Codex adapter](../crates/editor-agent/src/provider/codex.rs) implements
initialize/initialized, account readiness, effective configuration, ephemeral
thread creation and schema-constrained turn completion. It disables inherited
MCP names individually, native tools, search, plugins, hooks and delegation;
also sets `project_doc_max_bytes=0`, empty instruction fallback names and
`notify=[]` at process and thread levels. Supplied instruction sources must be
empty. Before starting, it points `model_instructions_file` to an owned file
containing only Aster's planner instructions inside the private working
directory, then supplies Aster's explicit
`baseInstructions` and `developerInstructions` for the thread. This keeps an
inherited custom model-instruction file out of startup and the model prompt.
It preserves configured provider/authentication settings; verifies the returned
read-only/never-approval policy; and accepts only the matching successfully
completed turn. Unexpected tools or approval requests are denied and fail the
request. Commentary, partial, asynchronous, failed and interrupted output cannot
become a proposal. Typed failures map to fixed messages without raw private
provider payloads. Native tagged command unions use `anyOf` and singleton
`enum` tags, matching the
[structured-output subset](https://developers.openai.com/api/docs/guides/structured-outputs?api-mode=responses);
the full external Command schema retains its ordinary JSON Schema shape.
The wire contract was checked against the installed Codex
0.162.1 [primary schemas](../artifacts/production-readiness/codex-0.162.1-schema)
and [official app-server documentation](https://learn.chatgpt.com/docs/app-server).

[Process ownership](../crates/editor-agent/src/provider/process.rs) uses a
watchdog and bounded stdout/stderr readers. Unix children enter an owned process
group; nonblocking pipes keep cancellation from waiting on an escaped
descendant's held pipe. Windows children start suspended, enter a configured
kill-on-close Job Object and resume only after assignment through
[the stable Windows adapter](../crates/editor-agent/src/provider/windows.rs).
Only owned process groups/jobs are terminated. No tools-disabled CLI setting is
a general sandbox for an arbitrary untrusted replacement executable.

| Resource | Current bound |
| --- | --- |
| User request / serialized planning context | 32 KiB / 1 MiB |
| Active requests / unread runtime events | 1 / at most 8 |
| Proposal / summary | 2 MiB / 1–2048 bytes |
| Edits | 128 top-level, 256 total including nested batches; depth at most 8 |
| Native-plan schema | Under 24 KiB encoded; it shares the Claude command line with the planner instructions |
| Keys sent per animation track or tracker | 32; longer lists are sent as a count |
| Effects / masks / animation tracks per clip | 16 / 8 / 64 |
| Keys per animation track / retime map / tracker | 256 / 2–64 / 1–2048 |
| Curve points / trackers per project | 2–16 / 64 |
| Transition length | One canvas frame to the shorter of the clip and 10 s |
| Process JSON frame / total stdout / stderr | 2 MiB / 32 MiB / 256 KiB |
| Inherited Codex MCP names | At most 64, each explicitly disabled |

Oversized, missing-executable, sign-in, protocol, model-access, deadline and
cancelled requests surface an error or cancellation event. They do not produce
a fallback edit or an invented successful response. Raw provider error/account
payloads and stderr are not passed through to the UI.

## Evidence and remaining acceptance

The immutable [round-fifteen source/build checkpoint](../artifacts/production-readiness/final/round15-build-record.json)
(`dfbb53e9` source, `362ba180` Linux release) passed 521 Linux tests, including 81 agent
tests, and full-workspace/all-target denied-warning Clippy for Linux, macOS
ARM64, Windows MSVC and Windows GNU, with
[independent backend identity/log verification](../artifacts/production-readiness/final/round15-backend-root-verification.json)
and [exact release dependency graph](../artifacts/production-readiness/final/round15-release-libraries.json).
R15's [thirteen native editing/input checks](../artifacts/native/round15-editing-smoke/root-verification.json)
pass, including exact reversals, command-panel rename, durable save and stale-plan
refusal, with five viewed captures and eleven sampled process identities gone.
No inference was requested; wake/cache/multi-owner acceptance remains unrun.
Its [three startup captures](../artifacts/native/round15-window-captures/root-visual-verification.json)
verify normal/compact presentation only, with unavailable audio; they made no
provider requests. Earlier cache/overlap/preview/narrow workflows remain
R14 `f74d6560` / `8b446cca`, recorded in the
[accepted native build record](../artifacts/production-readiness/final/round14-native-editing-build-record.json)
and [native editing/preview/narrow/cache/overlap checks](../artifacts/native/cache-culling-acceptance-round1/report.json), with
[independent identity/cleanup verification](../artifacts/production-readiness/final/round14-native-editing-root-verification.json).
Finite paused five-layer native composition and
[public PNG delivery](../artifacts/production-readiness/final/round13-public-preview-root-verification.json)
remain R13 evidence; these records do not establish provider, audio or performance acceptance.
Audio was unavailable; no model requests, GTK dialogs or performance were
retested. Native interface performance remains `b29ccb69` / `14435a4a` at the
333-test round-nine checkpoint. Latest actual
native real-provider AI/GTK evidence remains source `28259254`, release
`2c596cb0`, as scoped below. Separate round-twelve
[virtual silent-stall recovery](../artifacts/production-readiness/final/round12-virtual-stall-root-verification.json)
and [healthy physical silence-clock verification](../artifacts/production-readiness/final/round12-physical-clock-root-verification.json)
establish clock release/Playback fallback and healthy zero-PCM clock behavior
respectively. Neither establishes a native fatal error callback, acoustic quality
or A/V synchronization; they were not exercised by the UI run.
The separate [optimized R14 worker diagnostic](performance.md#optimized-distinct-video-worker-diagnostic)
now records distinct-video batch latency; it is not AI, native presentation or
follow-up dispatch/consumer acceptance.
The [liveness correction](../artifacts/production-readiness/preview-request-liveness-round1/phase-b/root-green-verification.json)
passes 21 focused/198 desktop tests and is now included with 112 media tests
in R15's 521-test checkpoint. The [completed review](../artifacts/production-readiness/preview-request-liveness-round1/review-round1.md)
finds no P1/P2 behavior defect. The later [canonical bridge proof](../artifacts/production-readiness/frontend-retention-bridge-round1/root-verification.json)
and [independent review](../artifacts/production-readiness/frontend-retention-bridge-round1/review-round1.md)
observe the nonempty retention call and restart reset (7/10 and 14/20 starts,
60 full-pixel checks). This is a four-case artifact probe at frozen R15, not a
standing suite guard; retire/clear/Closed, source/cut/reorder and playing remain
unproved. The separate
[81-case R15 worker comparison](performance.md#optimized-r15-legacyexplicitprotected-comparison)
passed with root verification; it establishes no native or provider result.
Complete native R15 input, wake/cache/multi-owner, platform-pack integration and new provider results
remain unverified. The six prepared bookkeeping tests were declined without
integration or execution; the 198 desktop/521 workspace counts are unchanged.

On 2026-10-12, an earlier backend checkpoint's public `AiRuntime` using default
PATH-resolved CLIs completed a generated title-split request with both real providers:

| Provider | Recorded resolved model | One-run elapsed time | Evidence |
| --- | --- | --- | --- |
| Claude | `claude-opus-5-5` | 3.3215 s | [Claude split](../artifacts/production-readiness/final/claude-split-explicit-semantics.json) |
| Codex | `gpt-6.1-sol` | 6.9987 s | [Codex split](../artifacts/production-readiness/final/codex-split-explicit-semantics.json) |

Each record verifies completed provider output, the requested split, preserved
settings, read-only review, committed state identical to review including new
IDs, one undo entry and exact undo/redo. Earlier
[Claude](../artifacts/production-readiness/ai/claude-final.json) and
[Codex](../artifacts/production-readiness/ai/codex-final.json) records verify
rename/move/portrait canvas edits on a prior backend checkpoint. These are
single backend-runtime
observations on generated metadata; native interaction has its own records below.
They do not establish comparative model performance or visual media understanding.
Deterministic protocol,
context, proposal and process regressions are in the
[round-fifteen 81-test agent/workspace record](../artifacts/production-readiness/final/round15-tests.log).

Acceptance caught and corrected two integration problems: Codex rejected the
original `oneOf` schema on some requests, and its loader rejected a zero-byte
instruction file. The
[owned no-model startup comparison](../artifacts/production-readiness/codex-owned-prompt-startup-fq9txewt/classification.json)
confirms the latter; the override now contains only Aster's known nonempty
instructions. Two subsequent model responses described a split but proposed
an unchanged transform; explicit operation semantics then produced the correct
split above. These observations reinforce review of command intent as well as
typed validity. In the later
[native GTK/Ask round](../artifacts/native/ai-round8/report.json), on db30 source
and the preserved `9f721c8b` release, both providers applied a correct text split
with exact undo/redo. Claude completed two proposals and one cancellation from
three requests; Codex completed three proposals, including one unchanged
transform with a split summary that was discarded before a correct retry.
That failure occurred despite the explicit split wording. Claude's observed
default there was `claude-haiku-5-5`; the command line controls defaults.

The native-verified `2c596cb0` release reports a nonempty net-no-op as "No changes",
preserving existing history and clean state without an undo promise. Four new
desktop regressions passed in the 309-test source record. The
[round-nine native checkpoint](../artifacts/native/ai-round9/report.json) verifies the
actual Ask/apply path with a labeled owned no-op fixture: clean state, revision,
full project and redo stay unchanged; the card gives quiet feedback without an
Apply toast. Real Claude and Codex each produced the correct split on their first
request, with one exact undo/redo and no failed/bad plans in this round. This
does not erase the earlier bad responses. Narrow/normal captures, a stale
fixture, import dialogs and protected unbound GTK recovery also passed.

The round-fifteen full workspace, including desktop/media and test targets,
passed Rust 1.97.1 cross-target compilation and denied-warning Clippy for
[macOS ARM64](../artifacts/production-readiness/final/round15-macos-arm64-clippy.log),
[Windows MSVC](../artifacts/production-readiness/final/round15-windows-msvc-clippy.log) and
[Windows GNU](../artifacts/production-readiness/final/round15-windows-gnu-clippy.log).
These checks neither linked nor ran a native application.
[Portable process-fault fixtures](../.scratch/native-ai/issues/01-portable-process-fault-fixtures.md)
are integrated; [12 focused adapter cases](../artifacts/production-readiness/portable-fixture/adapter-tests.log)
ran on Linux, and the round-fifteen workspace passed 521 tests. Native macOS/Windows
execution is deferred by the user while tests are prepared here. The
[native acceptance pack](native-platform-tests.md) now has 64 cases, all
`not_run`, and [106 offline guards passed on the pinned pack](../artifacts/native/platform-regressions-integration-round1/root-pinned-attempt-01/root-verification.json).
Fixture bytes and ten Linux native startup openings are verified, without
catalog passes. Source/fixture pins are set; [final independent review](../artifacts/native/platform-regressions-integration-round1/review-round2/REVIEW.md)
found no P1/P2 and closed all four preparation findings.
The R14 51-case/57-guard baseline and separate eleven-case draft remain
historical. These checks establish no native target execution.
[Verification](verification.md#platform-matrix)
keeps that work separate from the historical baseline and target execution.
Actual provider/job behavior, packages and
signed-in review/failure/cancel/apply on those platforms remain acceptance work.
Linux native review/stale/cancel/exact apply passed at db30, followed by
`2c596cb0` no-op and both-provider split/undo/redo acceptance. See
[verification](verification.md) for broader editor/platform gates and
[the Native AI contract](../.scratch/native-ai/spec.md) for the active feature.
