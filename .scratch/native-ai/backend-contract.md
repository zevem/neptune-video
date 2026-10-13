# Native AI integration contract

Root owns `crates/editor-agent`, core JSON schemas, manifests/lockfile,
backend CLI tools and compilation.

## Current public interface

CURRENT FREEZE: R14 cache/culling native acceptance is running.
Root passed 397 tests, format, Linux denied-warning workspace Clippy,
Windows MSVC/GNU and macOS ARM64 source checks, actual FFmpeg preflight and
optimized bins. Source
`f74d6560b7e049856ff6f7fee81f8a80aa28c2d4e93bd68a3775b17c0fa21106`;
Aster `8b446ccacf38ef883fa01f54e65bec852a6c74f8fa9a016ca4c02b4408c7ba6b`;
agent `57c9aff5e138d9b68d2cda70851e43c6f27ade8b7365882ce63d77c93d5bb037`.
See `artifacts/production-readiness/final/round14-build-record.json` and
`round14-backend-root-verification.json`. No source/Cargo/binary replacement
until that native round returns. R14 native behavior is pending; older native
records below remain tied to their own builds. All target-native preparation
plans under `artifacts/native/backend-preparation/round14/` are `not_run`.

The interfaces below are implemented. Root owns Cargo and backend source.
The earlier audio checkpoint is `artifacts/production-readiness/final/round12-build-record.json`
(354 tests, source `2b43797d`, optimized Aster `ec99d455`). Linux formatting,
workspace tests/Clippy and optimized builds pass; three cross-target checks
establish source validity only. Nine callback-liveness regressions, an
independent buffer-publication review and the fresh private native silent-stall
experiment are accepted in their named records. The public clock releases,
Playback continues and the worker naturally reaps its decoder; healthy physical
zero-PCM normal/seek/speed/pause clocks also pass. No acoustic, DAC/display,
A/V, native fatal-callback or performance acceptance follows from these probes.

R12 desktop editing acceptance also passed: 13 native Linux checks,
whole-project undo/redo, source preview and narrow captures. Root verified all
73 source files, both binaries, 18 capture hashes and all 18 tracked processes
gone in `round12-native-editing-root-verification.json`. Audio was unavailable
in that UI run. Its platform pack pin/57 offline guards are historical R12
evidence; the current R13 pin is described below. Actual macOS/Windows execution remains
user-deferred. Performance stays scoped to R9 `b29ccb69`/`14435a4a`, and
real-provider/GTK acceptance to `28259254`/`2c596cb0`.

Root retains Cargo. The current accepted core checkpoint is
`artifacts/production-readiness/final/round13-build-record.json`: 365 passing
tests, 74 source files, source
`5cc2068039f6a8e08e24b6b43114fd840e8f01c2ed3658a40f579d199675014a`,
optimized Aster `6496d42f40d225756796af8908a30fed085ac7fd64761f2055de0eaac94a51fd`
(27,212,040 bytes), and unchanged Aster Agent
`b0ce17857db3a77f503feed5baf832cd551ca720f783e653a940c44a4e300b56`.
Only media `service.rs` and its new private delivery-test module differ from
R12. Eleven delivery regressions pass after eight reproduced failures;
the source correction review is recorded separately. Workspace tests, actual
FFmpeg preflight, formatting, denied-warning Linux/three-target source Clippy
and optimized binary builds pass. R13 native acceptance is recorded below.

R13 native/public acceptance is now independently verified. The optimized
editor passed 13 Linux smoke checks, whole-project editing/source-preview/narrow
checks and three paused five-layer launches; root checked 74 sources, 35 capture
hashes and all 31 tracked processes gone in
`round13-native-editing-root-verification.json`. Two public five-PNG diagnostics
have exact pixel/library/18 process-thread identity verification in
`round13-public-preview-root-verification.json`. Their wait observation and
native request sizes remain separate from exact private seam proof. The
platform pack is pinned to R13; root's 57 inherited-environment guard tests
pass, while all native macOS/Windows cases remain `not_run`.

The R13 acceptance freeze is released. Root retains all Cargo, backend source
and binaries. The next task may author focused frontend
thumbnail-retention/viewport-culling regressions under a fresh task grant.
Product changes are newer than R13 and require a new build/runtime record.
Older performance, provider/GTK and audio proof keeps its original source scope.

The next task is cache/culling phase A: preserve current behavior while
extracting actual private seams and authoring failing retention/overlap tests
in `src/textures.rs`, `src/ui/timeline.rs`, `src/app.rs` and focused test modules.
No Cargo, native acceptance or policy correction in that phase; root runs the
RED tests before the next task. A backend task separately covers only
`model.rs`'s borrowing `Track::clips_overlapping` method and
`crates/editor-core/tests/clip_intervals.rs`. That helper filters half-open
intervals after partitioning by starts; no end-order assumption, I/O or index.
The frontend adopts it only after reproducing the current renderer's behavior.

Phase A is now executed: seven expected cache/culling/admission failures and
13 controls, with exact source byte copies in
`artifacts/native/cache-culling-round1/phase-a-root-verification.json`.
The fresh phase-B grant permits production correction and meaningful
controls in the four frontend modules above. Core/model, manifests, Cargo,
binaries, documentation and platform repinning stay with their existing owners.
No frontend Cargo/format/native-current-build claim until root validates and
builds the corrected source. A bounded optional native baseline may use the
still-unchanged R13 executable with its own provenance, despite newer working
source. The next accepted build/runtime will have a new named checkpoint.

The quiet no-op proposal card is the intended feedback. Historical compilation
grants below have expired; later grants must name source/binary and scope.

`AiProvider::{Claude, Codex}`: installed native CLI providers.

`AiConfig { provider: AiProvider, executable: Option<PathBuf>, model: Option<String>,
effort: Option<String>, timeout_seconds: u64 }`: serde configuration, with no
credential fields. Defaults choose Claude, CLI default model, high effort and
a bounded request deadline. The runtime resolves/starts the executable off-thread.

`AiContext::capture(editor: &Editor, selected: &[ClipId], playhead: Time) -> Self`:
owned immutable snapshot plus project/context/revision guard; no I/O.

`AiRuntime::new(wake: impl Fn() + Send + Sync + 'static) -> Result<Self,AiError>`;
`request(&self, config: AiConfig, context: AiContext, prompt: String)
-> Result<Uuid,AiError>`; `poll() -> Option<AiEvent>`; `cancel()`. At most one
active planning request; GUI submission/poll/cancellation never wait for a
provider process. No request is sent until explicitly submitted by the person.

`AiEvent::{Started { id }, Proposal { id, proposal: Box<AiProposal> },
Failed { id, message: String }, Cancelled { id }}`. Events are bounded and wake
the GUI. Each terminal event belongs to its captured run id.

`AiProposal` exposes `id`, `summary`, `commands: Vec<Command>`, `provider`,
`model: Option<String>`, and `before/after: Arc<Project>` for review. Methods:
`is_current(&Editor) -> bool`,
`apply(&mut Editor) -> Result<editor_core::EditOutcome,AiError>`.
Apply requires the original project/context/revision/root and commits the exact
state validated through one Batch. The runtime reduces and validates once on
its worker; generated clip/relationship IDs remain identical to the preview.

Use the existing command interpreter for explicitly local commands. The native
Agent surface should make real model requests, provider/model configuration,
pending/cancel/error states and concrete proposal apply/discard reachable.
Maintain quiet minimal controls and meaningful tooltips. Keep credentials out
of settings/project files. Do not claim a provider connected until it actually
responds successfully.

## Storage integration (parallel backend implementation)

`editor_store::DocumentSession::{prepare(path),create(path)} -> Result<Arc<Self>,StoreError>`;
`path() -> &Path` returns canonical destination. `load_project_with_session(path)
-> Result<(Project,Arc<DocumentSession>),StoreError>`; save through
`SaveWorker::request_document(Arc<DocumentSession>,&Editor)` after `register_save`.
The session baseline is shared across ordered queued writes and never refreshed
automatically after an external conflict. `DocumentConflict(PathBuf)` and
`DocumentBusy(PathBuf)` leave edits dirty; let the person save to another file
or explicitly reopen. SaveAs prepares its destination on a worker, duplicate
creates a missing destination session; recovery ownership remains separate.

The desktop must hold the current ordinary project's session, keep it through
checkpoint restore, renew it only when a document/destination is deliberately
opened or changed, and match session/preparation completions to the captured
context/revision. Headless CLI migration is root's work.

Recovery ownership contention is now typed `StoreError::RecoveryInUse(PathBuf)`.
Choose private recovery only for that variant; generic IO failures must surface
as errors. Read-only backend review also found that any asynchronous recovery
deletion must retain the exact `Arc<RecoveryLease>` through the delete job,
or acquire it inside the job (busy means leave the copy). A stale removal job
must never delete a newer window's archive after the original lease was dropped.

On recovery-lease contention, a second editor of the same ordinary project
must retain a private instance-specific recovery destination under the settings
recovery directory. Keep the ordinary save destination and DocumentSession
separate. A manual-save conflict must leave that window's unsaved history
recoverable. Never overwrite/remove another window's owned sidecar. Verify
two-window conflicting-save/crash/recovery behavior.

## Compilation

ROOT CARGO OWNERSHIP: the first UI round and its watcher have been stopped after
native captures and13/13 regression artifacts were retained. Root now owns
Cargo for finalized backend integration and validation before the next UI round.
Backend APIs are implemented and compile; 48 agent tests and store54/bin9 pass.
Root is running actual provider acceptance using the already-built ci ai-smoke
example; no additional Cargo is required for those calls. Both CLI migrations
and typed RecoveryInUse are ready. Coordinate before broad workspace formatting;
do not edit backend/manifests.

Use fresh task-owned native storage/display. Real-provider tests may follow
root's reports; do not send additional real requests until the first adapter
acceptance is resolved. Tests, captures and deterministic fault fixtures may run.

Root real provider update: Claude via public AiRuntime passed all seven checks
on generated metadata in3.915s (its CLI default resolved claude-haiku-5-5).
Codex first run failed at a terminal error notification; a separate isolated
schema-only app-server probe then completed successfully, so schema transport
is accepted. Root is diagnosing/retrying before the desktop Codex acceptance.
Do not infer native Windows runtime acceptance from the new Windows source.

Root assigned a backend store agent to your reported crash-recovery baseline
gap. It will add optional archived DocumentSession baseline binding; legacy and
private unbound archives must restore unsaved rather than take a fresh current
disk baseline. API will be written to recovery-contract.md. Please continue
current UI tests/captures; this integration will require a later UI revision,
and root will rebuild latest backend once Cargo returns. Also clip summary
history labels in UI is sufficient for now; keep current proposal API stable.

ROOT REQUESTS CARGO RETURN NOW: captures/report.json and13/13 smoke are present;
finish any current command, record focused check/test/build outcomes, then release
Cargo and complete this delegated round (stop the file watcher). Root needs to
compile the wired large-context projection, explicit executable resolution and
bound-recovery APIs/tests. Frontend's existing binary captures are useful but
not final evidence for these newer backend changes. The next UI task will be a
new delegated round after backend gates pass, integrating recovery-contract.md
and driving real providers/portal choosers. Please do not start another rebuild.

Root provider finding: explicit real Codex executable path passes seven checks
on the public runtime in12.22s (gpt-6.1-sol); bare-name direct launches failed
consistently in this embedded environment. Runtime now resolves executable paths
on its worker. Native adapters are real; no Python wrapper is part of product.
The new backend validates closed proposal fields and rejects invented AddAssets.

Current short Cargo window: native Codex neptune_docs runs WindowsGNU+macOSARM64
headless core/agent/store lib typechecks. Root/backend Linux tests passed
core43, agent60, store66 and clippy; context47unit includes exacttyped5000clip
focus and link/group closure. Root built latest ai-smoke and is running two real
providers through defaultresolvedexecutables. ClaudeUIround2 source-ready bound
recovery integration received; noUICargo untilrootnewgrant/newdelegatedround3.

FRONTEND CARGO GRANT FOR ROUND3: root and native backend agents have returned
Cargo. The frontend task covers focused desktop tests/clippy and optimized bins
build, then all remaining native acceptance listed by round2. BackendAPIfinalized, tests
core43+agent60+store66+CLI12 pass; CLI8 real independentprocess checks pass;
both headless WindowsGNU/macOSARM64 core/agent/store typechecks pass. The latest
publicRuntime real providers BOTH pass via defaults: Claude3.947s/Codex10.129s,
allseven checks. Realnativeprovider requests nowauthorized. Root noCargo until
round3 returnsownership. UseactualpinnedCargo/RUSTC/RUSTDOC forproxyavoidance.
Do not run a persistent watcher or nested waiting loop; complete this round.

NEW FRONTEND SAFETY FINDING: ADR reviewer observed prepare_save unconditionally
uses DocumentSession::prepare, so an UNBOUND RESTORED legacy/private/moved copy
can SaveAs back over an existing project and acquire a new baseline. For that
recovery origin only, use DocumentSession::create (freshdestination refusal)
and verify chosen existing ordinary bytes unchanged. Preserve normal deliberate
SaveAs for other editing contexts. Track unbound-restored origin explicitly,
not every new unsaved project; clear it on deliberateopen/new/destinationadoption.
CLI --recover --save-as already has createonly policy. ADR0005 records required
behavior; do not call native recovery gates complete without this check.

BACKEND ISOLATION FIX INCOMING: independent audit found Claude --setting-sources
'' does not itselfdisable CLAUDE.md/customization discovery. InstalledCLI2.1.296
--safe-mode explicitlydisables those whilepreservingauth/model/permissions.
Backendreviewer will add --safe-mode to Claude launcher and focusedfixturetest,
withoutCargo. Please rebuildlatestbackend afterthatfix before FINALnative real
Claude evidence; earliercapturescanexerciseotherfeatures. No providercredentials
are copied/read. Root willrunrequiredbackendfault/wholeworkspacechecksafterreturn.

BACKEND EXACT-PROPOSAL FIX INCOMING: dry-running and then re-dispatching a split,
link or group generates different UUIDs from the reviewed after-state. Core now
has a dedicated agent implementing private validated PreparedEdit authority;
AiProposal's public frontend interface stays unchanged, and Apply will commit
the exact prepared immutable state as one ordinary history entry with the same
context/revision guards. No Cargo until frontend returns it. Codex inherited
AGENTS.md/notify isolation and Windows Claude batch-launcher guard are also being
closed in separate backend files. Finish native dialog/recovery checks on your
current build; report its exact source/build and return Cargo. Root will compile
and verify these changes before the next final native build/acceptance round.

ROUND3 CARGO RETURN RECEIVED: desktop83tests/clippy, 13/13 native editing smoke,
14 real GTK dialog checks, both actual providers eventually applied with exact
undo/redo, and bound/legacy/private recovery native flows have records. Three
Codex failures were classified as schema/oneOf/notpermitted; root has normalized
native proposal tagged unions to anyOf and const tags to singleton enum without
changing the public command schema or validation. Root owns compilation now;
core49+agent74 focused tests pass against exact prepared state and new isolation.
Round4 covers source-only unbound-recovery create-only integration, noCargo.
After that source is ready root runs final workspace and real-provider checks,
then grants a fresh native acceptance round. Persistent empty lock files
are intentional lock anchors; do not remove them based on existence or a PID.

FINAL BACKEND BUILD READY FOR NATIVE ROUND5: source db30b8bdb7018890e7b692d7152b1672c0b9d92fff740aea80fe97023b373016;
release aster 9f721c8b97b6f3500e283544afd89bfd0122653753240560354b62065f17131c;
305 workspace tests, fmt, all-target denied-warning Clippy, optimized bins and
fresh core/agent/store WindowsGNU/macOSARM64 lib typechecks pass. Build record:
artifacts/production-readiness/final/db30-native-round5-build-record.json. Desktop Origin policy
from round4 is compiled, with86 desktop tests passing. Native acceptance now
uses these existing binaries, NO Cargo or source edits unless an actual defect
is reported and compilation coordinated with root.

Codex instruction override is a private0600 file containing only Aster's SYSTEM
before launch, plus explicit base/developer prompts. An empty file was rejected
by the actual CLI; that regression is fixed. Two subsequent split proposals
were no-op transformations despite their summary; explicit split semantics in
SYSTEM/schema now produce the correct split. Latest realpublicRuntime split
passes8checks for BOTH: Claude default observedclaude-opus-5-5 in3.322s, Codex
observedgpt-6.1-sol in6.999s. Earlier3edit request alsopasses afterownedprompt.
Review stillcheckscreative intent, notjustschema validity. Do not hide prior
failed requests or count them as successful intended editing.

CONCRETE UI NO-OP FINDING / PARALLEL SOURCE ROUND6: src/ai.rs apply maps
EditOutcome through .map(drop); src/app.rs ai_apply settles Applied unconditionally
on Ok, even for a validated nonempty net-no-op whose revision/history stay
unchanged. That falsely says Undo takes it back. New round6 covers only this
source-level result handling (no Cargo/native input). Round5 continues its
already-built db30/9f native checkpoint; root preserves that build record at
artifacts/production-readiness/final/db30-native-round5-build-record.json and
WILL NOT replace release binaries while round5 runs. Report round5 as that
specific checkpoint, not the upcoming no-op-status build. No source edits or
Cargo in round5. Root validates source round6 on ci, rebuilds after round5 returns,
then requires final native no-op/changed proposal captures with the new binary.

ROUND5 COMPLETE / ROUND7 FINAL NATIVE ACCEPTANCE GRANT: round5's db30/9f
release passed real GTK legacy/private create-only recovery, fresh-file adoption
and second saves, real Claude/Codex split/one-undo/exact-redo, and 13/13 native
smoke. One Codex no-op Transform with a split summary was discarded; keep that
failure in the report. All owned helpers exited and root checked recorded PIDs.

Source round6 is compiled: 309 tests (90 desktop), fmt/all-target Clippy and
optimized bins pass. Current source28259254abdf0c6a59f6facdc13f9067db3f9353ce4da7a3e9e58ed5c9b1edf1;
release aster2c596cb0f4a41bebde8e28e8eaf41e2111f4b7d6038ffdd511096e1e32560b52.
Immutable record artifacts/production-readiness/final/round7-build-record.json.
Only src/ai.rs/app.rs/ui/agent.rs changed since db30: preserve EditOutcome.changed,
document_changed only on Changed, quiet Unchanged card/status without Undo claim.
New round7 covers current-build native no-op fixture, real providers,
13-check smoke and a protected-recovery spot under artifacts/native/ai-round9.
NO source/Cargo or release replacement during round7. Root writes docs only.
