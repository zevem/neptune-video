# Production readiness audit

Date: 2026-10-12. Scope: read-only source/evidence review after the 169-test
core checkpoint, before the provider and manual-writer work now in progress.
The original production-ready, agent-native, Linux/macOS/Windows objective
remains open. The resolved core implementation phase is progress toward it.
Existing acceptance records are unchanged by this audit.

## Requirement states

`Implemented` means a source path exists; `locally verified` names the actual
Linux observation. Neither establishes acceptance on another OS or on untested
hardware. The [source/build record](../../artifacts/verification/build-record.json),
[feature matrix](../../docs/verification.md#implemented-feature-matrix) and
[native evidence](../../docs/verification.md#native-linux-evidence) own the
baseline below.

| Requirement | State at this checkpoint | Concrete remaining work |
| --- | --- | --- |
| Projects | Implemented; Linux save/duplicate/settings and recovery verified | Concurrent explicit saves need conflict detection; prove interruption, full-disk and permission behavior on each OS |
| Media import | Implemented native image/GIF and FFmpeg video/audio/folders; Weston URI drops verified | Normal chooser/file-manager and sandbox portal transfers; supported-format/clean-machine codec matrix |
| Media library | Implemented thumbnails, folders, search, sorting and source monitor; Linux real input verified | Offline source relinking/portable-media workflow for dependable reopened projects; very large libraries remain unmeasured |
| Timeline | Implemented arbitrary model track count, snapping, zoom, markers and visible-row rendering | Native interaction at large scale; index behavior must be profiled over late and overlapping clip ranges |
| Clip editing | Implemented requested operations; core reversals and selected Linux gestures verified | Complete pointer/keyboard/clipboard acceptance on all targets, including failure and inverse actions |
| Advanced editing | Implemented ripple/link/group/track states; invariant and lock tests exist | Complete native mixed linked/ripple/group scenarios and OS acceptance |
| Playback | Software preview/source monitor and native audio service implemented | Actual sound-device quality and A/V drift; cold/random seeks, sustained source-rate presentation and drop counts |
| Transform and canvas | Implemented requested transforms, crop, presets/custom canvas/background | Native inspector/DPI/device acceptance across all OSes |
| History | Complete recovery chain, checkpoints and reversible restore implemented; Linux exit/relaunch verified | Long editing-session memory/storage policy, interrupted writes, manual-writer conflicts and platform replacement semantics |
| Native AI | Shared guarded edit protocol exists; in-app command line is an exact phrase parser | Native provider-backed planning/tool execution, cancellation, error states and real provider acceptance are missing at this baseline |
| Performance | Optimized metadata/worker benchmarks and two native idle samples exist | Repeated input-to-display, seek, layered preview, audio and GPU-memory measurements with enforceable budgets |
| Cross-platform product | Source and three-OS headless CI configured; Linux native records exist | Actual macOS/Windows builds and runtime acceptance, broad Linux desktop behavior, distributions and signing |

Sources for additional implementation limits: the
[exact command parser](../../src/agent.rs),
[guarded protocol](../../crates/editor-agent/src/lib.rs),
[asset/timeline model](../../crates/editor-core/src/model.rs),
[editor history](../../crates/editor-core/src/editor.rs),
[audio mixer](../../crates/editor-media/src/audio.rs),
[software video decoder](../../crates/editor-media/src/decode.rs) and
[configured CI](../../.github/workflows/ci.yml).

The mixer currently rejects more than 16 simultaneous audible clips and rate
changes alter pitch (`audio.rs:25,518,558`). The decoder uses software RGBA and
a fixed-rate FFmpeg `fps` filter (`decode.rs:332`), so exact VFR/HDR and
hardware/zero-copy are additional implementation work. The asset model stores
source paths (`model.rs:138`); there is no dedicated relink command in the
[command enum](../../crates/editor-core/src/command.rs). These are practical
limits, separate from the existence of unlimited model tracks.

## Highest-priority next work

1. **Native AI planning and execution.** Add an owned Rust provider service
   behind the existing reducer. Capture project/context/revision, selection and
   playhead before planning; expose bounded states/events to the native UI;
   validate a returned plan and execute it as one atomic `Batch`. Recheck all
   identities at execution and cancel results after project replacement.
   Completion proof: actual natural-language request through a configured
   provider creates the intended edits, exact undo/redo succeeds, stale plans
   fail, cancellation cannot apply a late result, and missing/auth/limit/protocol
   errors remain usable. The current phrase parser at `src/agent.rs:1` cannot
   satisfy that proof. Root is implementing this work; this audit does not claim
   the new integration exists yet.
2. **Manual project-save conflict detection.** Recovery leases protect sidecars,
   but current ordinary saves use unconditional atomic replacement. Track the
   loaded/saved file fingerprint and compare under an OS-held write lock before
   replacing it; return a structured conflict without losing either document.
   Save As/duplicate must handle occupied destinations deliberately. Completion
   proof: two independent writers of the same file cannot silently overwrite
   each other's edits; future/corrupt targets remain protected. See
   [atomic write](../../crates/editor-store/src/lib.rs),
   [recovery lease](../../crates/editor-store/src/lease.rs) and
   [ADR 0003](../../docs/adr/0003-recovery-ownership-uses-os-locks.md).
   The architecture agent is implementing this seam.
3. **Real media correctness and sustained preview.** Test actual CPAL devices
   and measure A/V drift, underruns and presented frames under edits/seeks.
   Resolve source timing for VFR and define supported color/HDR behavior before
   advertising those sources. Measure layered video and more than 16 audible
   clips, then choose a mixer/decoder scheduling policy from those observations.
   PCM fixtures and worker throughput alone do not close this requirement.
4. **Native platform acceptance and packages.** Build/run on actual macOS and
   Windows; verify GPU/audio, DPI/IME, clipboard/dialogs/drop, paths and atomic
   replacement. For Linux, drive normal GNOME/KDE chooser/file-manager routes,
   portal-only drops and accessibility. Produce local packages with dependency
   and license manifests, then clean-machine test them before signing/promoting.
   [Neptune packaging findings](neptune-packaging.md) identify reusable machinery.
5. **Long-session and interactive performance budgets.** Measure long growing
   undo histories, saves under slow storage, 100,000-clip/1,000-track interaction,
   repeated cold/warm seek and presentation latency. `Track::active_clips` uses
   a start partition followed by a prefix scan (`model.rs:209`); this is a
   concrete profiling seam rather than an interval-index guarantee.
   `Editor::dispatch` retains history roots (`editor.rs:200`) while store output
   is bounded to 256 MiB (`editor-store/src/lib.rs:139`). Establish an explicit
   history/recovery resource policy that preserves the advertised undo depth
   and reports exhausted capacity. Keep [worker and idle measurements](../../docs/performance.md)
   distinct from end-to-end guarantees.

Export was outside the enumerated first core-feature phase. It remains absent
and should receive its own spec before a broader professional-editor release;
this audit does not silently add it to the original core acceptance list.

## How Neptune integrates AI

Neptune's [root guidance](/home/biggabo/Documents/Projects/rust-terminal/rust-terminal/AGENTS.md)
requires native ownership, bounded off-frame work, stable-target mutations,
private diagnostics and actual platform evidence. Its
[runtime guidance](/home/biggabo/Documents/Projects/rust-terminal/rust-terminal/src/runtime/AGENTS.md)
uses worker ownership, exact completion generations and bounded shutdown.
Those are reusable engineering constraints; terminal/PTY and release-specific
semantics do not transfer automatically.

Neptune runs the person's installed, signed-in Claude Code or Codex CLI instead
of holding a provider account/key. Its
[Projects requirements](/home/biggabo/Documents/Projects/rust-terminal/rust-terminal/docs/projects.md:28)
explicitly restrict the feature to local Unix workspaces: Windows adapters are
unavailable, and most non-X11 native acceptance remains open. Its
[verification](/home/biggabo/Documents/Projects/rust-terminal/rust-terminal/docs/verification.md:477)
records actual Claude turns/delegation/resume and one Codex READY turn; Codex
tool/interrupt/resume failures still need native proof.

| Neptune boundary | Source and reusable behavior |
| --- | --- |
| Process service | [project_lead.rs](/home/biggabo/Documents/Projects/rust-terminal/rust-terminal/src/runtime/project_lead.rs:1): pure `Driver`, worker-owned child/stdin/stdout/stderr, 64 events, 8 inputs, bounded frames/text, 50 ms stream repaint coalescing; `Lead::start/send_turn/interrupt/try_events` |
| Lifecycle | [Lead](/home/biggabo/Documents/Projects/rust-terminal/rust-terminal/src/runtime/project_lead.rs:493): Starting/Ready/Busy/Dead, typed auth/tool/limit/transport failures, ordered turn events, nonblocking drop; handshake and interruption deadlines followed by owned-process teardown |
| Claude driver | [claude.rs](/home/biggabo/Documents/Projects/rust-terminal/rust-terminal/src/runtime/project_lead/claude.rs:291): bidirectional stream-JSON; initialize and MCP-ready handshake; explicit tool/settings isolation; model/effort/resume; token deltas, tool results, turn status and interrupt |
| Codex driver | [codex.rs](/home/biggabo/Documents/Projects/rust-terminal/rust-terminal/src/runtime/project_lead/codex.rs:137): app-server JSON-RPC initialize/account/config/thread start or resume; only product MCP enabled, native execution/search/delegation disabled, read-only sandbox; streamed turns and interrupts |
| Product tools | [agent_mcp.rs](/home/biggabo/Documents/Projects/rust-terminal/rust-terminal/src/runtime/agent_mcp.rs:1586): bounded newline MCP stdio server and cancellation; role-scoped tool discovery/calls; tools delegate to the app rather than editing UI selection |
| Bridge/discovery | [agents.rs](/home/biggabo/Documents/Projects/rust-terminal/rust-terminal/src/runtime/agents.rs:1002): authenticated loopback TCP, Unix-only shell adapters; [discovery](/home/biggabo/Documents/Projects/rust-terminal/rust-terminal/src/runtime/project_lead.rs:1298) searches PATH/known directories/login shell off-frame |

Claude runs with `--tools ''`, strict product MCP, `dontAsk`, empty setting
sources and disabled slash commands. It initializes over stdin and waits for
the product MCP server before accepting a turn (`claude.rs:309,348`). Codex
starts `app-server`, then supplies MCP configuration over stdin; product tool
credentials avoid process arguments (`codex.rs:137,569`). Neptune's private
temporary MCP config and secret-tail redaction are at
[project_lead.rs:608,1225](/home/biggabo/Documents/Projects/rust-terminal/rust-terminal/src/runtime/project_lead.rs:608).
These citations describe mechanisms, not actual token values.

Its bridge is TCP, not a Unix socket. The Unix limitation comes from adapter
creation and `open_lead` gating (`agents.rs:990,1002,1075`). Reuse provider stdio
and Aster's [authenticated TCP host](../../crates/editor-agent/src/host.rs),
without copying terminal shell shims or Unix gating. Windows also needs its own
owned process-tree teardown: Neptune's Unix process-group signal helper is a
no-op on other systems (`project_lead.rs:1039`).

## Concrete provider-runtime options

Local read-only discovery found `claude` and `codex` in the user's local binary
directory. `claude --version` reported **2.1.296**; `codex --version` reported
**0.162.1**. `--help`/`app-server --help` were inspected only; no inference,
account files, credentials or token values were read. Version availability
does not prove account/model access.

| Option | Concrete interface | Scope and tradeoff |
| --- | --- | --- |
| Claude plan-only child | `claude -p --output-format json --json-schema SCHEMA --tools '' --no-session-persistence`, isolated app-owned cwd/settings/MCP; request/context through bounded stdin | Small Rust process adapter; parse `structured_output`, deserialize/validate a bounded plan, present/apply through Aster guards; process cancellation must invalidate late output |
| Claude persistent native agent | Neptune's stream-JSON driver plus Aster MCP stdio adapter over its existing local protocol | Streaming/multiple turns and product tool calls; requires protocol fixtures, MCP-ready handshake, permissions and bounded lifecycle, rather than copying terminal-specific tools |
| Codex app-server native agent | `codex app-server --listen stdio://`; initialize/initialized, thread/start or resume, turn/start, delta/completed events, turn/interrupt | JSON-RPC harness reusable from Rust; version-specific schema and explicit product-only tool restrictions; MCP works through a product stdio adapter |

Official [Claude programmatic docs](https://code.claude.com/docs/en/headless)
confirm JSON/stream-JSON, schema output in `structured_output`, and stdin use.
They also state that `--bare` ignores subscription OAuth/keychain, so it is
unsuitable for reusing installed subscription sign-in. `dontAsk` alone still
permits some tools: explicitly disable native tools for a plan-only child.
An app-owned cwd avoids loading source-project instructions; inherited
settings/MCP require explicit isolation. These are adapter-design constraints,
not a claim that every helper flag forms an OS security boundary.

Official [Codex app-server docs](https://learn.chatgpt.com/docs/app-server)
confirm initialization, persistent threads, streaming and interruption.
`turn/start.outputSchema` can constrain a plan. Dynamic tools require the
experimental API; a product MCP adapter avoids that dependency. Generate
schemas from the installed version with
`codex app-server generate-json-schema --out PATH` before implementing protocol
fields. The installed stable and experimental schemas were generated after
this initial review; exact findings are below. No provider turn was run for
this audit.

Recommended first increment: bounded plan-only service with stable context and
one guarded reversible batch, followed by native streaming/product tools if the
workflow needs them. Keep provider credentials/session handles out of project
documents, use explicit provider/version errors, and prove a real turn before
advertising native AI. This recommendation is an inference from the existing
reducer and installed interfaces; root owns the implementation decision.

## Installed Codex 0.162.1 wire verification

The read-only CLI generated its own primary schemas successfully into
[stable](../../artifacts/production-readiness/codex-0.162.1-schema/) and
[experimental](../../artifacts/production-readiness/codex-0.162.1-schema-experimental/)
task-owned artifact directories. This operation emitted protocol shapes only;
it sent no account/inference request and exposed no credentials. Existing
source and acceptance records remain unchanged.

```sh
codex app-server generate-json-schema --out artifacts/production-readiness/codex-0.162.1-schema
codex app-server generate-json-schema --experimental --out artifacts/production-readiness/codex-0.162.1-schema-experimental
```

| Wire step | Installed primary schema and minimum fields |
| --- | --- |
| Initialize | [InitializeParams](../../artifacts/production-readiness/codex-0.162.1-schema/v1/InitializeParams.json): `clientInfo` required; its `name`/`version` required, `title` optional; send `initialized` notification after success |
| Start thread | [ThreadStartParams](../../artifacts/production-readiness/codex-0.162.1-schema/v2/ThreadStartParams.json): no required params; `cwd`, `developerInstructions`, `model`, `approvalPolicy`, `sandbox`, `ephemeral`, `config` are optional; no stable top-level `tools` field |
| Start turn | [TurnStartParams](../../artifacts/production-readiness/codex-0.162.1-schema/v2/TurnStartParams.json): `threadId` and `input` required; text input requires `{type:"text",text:"..."}`, `text_elements` defaults to `[]`; `outputSchema` is a stable optional JSON Schema constraining the final assistant message |
| Restrict sandbox | Thread `sandbox:"read-only"`; turn `sandboxPolicy:{type:"readOnly",networkAccess:false}`. Stable approval strings are `untrusted`, `on-request`, `never`; use `never` for a planner |
| Receive final result | [TurnCompletedNotification](../../artifacts/production-readiness/codex-0.162.1-schema/v2/TurnCompletedNotification.json): `threadId` and `turn` required; turn requires `id`,`items`,`status`; accept a proposal only on `status:"completed"`, not `failed`/`interrupted`/`inProgress`; final `agentMessage` carries `text` |
| Interrupt | [TurnInterruptParams](../../artifacts/production-readiness/codex-0.162.1-schema/v2/TurnInterruptParams.json): both `threadId` and `turnId` required; preserve their identities while planning/cancelling |
| Inspect effective configuration | [ConfigReadParams](../../artifacts/production-readiness/codex-0.162.1-schema/v2/ConfigReadParams.json): optional `cwd`/`includeLayers`; inspect only necessary server names internally and keep config contents out of diagnostics |

Every request also requires envelope `id`, `method`, `params` in
[ClientRequest](../../artifacts/production-readiness/codex-0.162.1-schema/ClientRequest.json).
`dynamicTools` exists only in the experimental
[ThreadStartParams](../../artifacts/production-readiness/codex-0.162.1-schema-experimental/v2/ThreadStartParams.json);
it is not needed for typed plan-only output. Do not invent `tools:[]` as a stable
thread-start restriction. The stable `outputSchema` field does not require
experimental API opt-in.

Tool restrictions belong to effective configuration. Installed `config` is an
open JSON map, so that schema alone cannot prove each feature key has the
intended effect. Official [configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference)
documents `features.shell_tool`, `features.unified_exec`, `features.apps`,
top-level `web_search` and per-server `mcp_servers.<name>.enabled`. Disable native
execution/apps/search for a planner and disable every inherited MCP by name;
an empty MCP object is not proof that merged inherited configuration is cleared.
Neptune's driver discovers effective server names before disabling them
(`codex.rs:250,137`). Read-only sandbox and `never` approvals still allow tools
that require no approval, so they do not replace tool disabling. Treat unknown
or managed-policy-incompatible configuration and unexpected execution as a
provider error; real installed-provider acceptance remains necessary.
