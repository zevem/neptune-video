# Codex planning adapter

Status: ready-for-agent
Progress: resolved
Type: task

Implementation is present in
[provider/codex.rs](../../crates/editor-agent/src/provider/codex.rs).
Compilation, transcript-test execution and installed-provider acceptance remain
root's verification work; this record does not claim those have passed.

## Interface and ownership

`plan(process: &mut super::Process, config: &AiConfig, prompt: &str, schema:
&serde_json::Value) -> Result<super::ProviderOutput, AiError>` uses only the
parent transport's `send`, `next` and `directory` interfaces. Parent `Process`
owns stdout/input bounds, deadline/cancellation and child/process-tree teardown.
The adapter owns pure protocol state and returns final bytes plus the provider's
resolved model; the planner owns proposal decoding, captured project/context/
revision validation, and one reversible guarded batch.

The protocol comes from installed Codex 0.162.1
[stable schemas](../../artifacts/production-readiness/codex-0.162.1-schema/),
with Neptune's driver as the lifecycle reference. No invented thread-start
`tools` field or experimental dynamic-tool dependency is used.

## Implemented behavior

- Initialize/initialized, account readiness, effective config, ephemeral
  thread-start and schema-constrained turn-start.
- Keep configured provider/auth settings inherited; read only effective MCP
  names, disable every one, and fail closed beyond 64 servers or invalid names.
  Reuse the parent tool-feature-disable list, disable web search and request
  never/user approvals with read-only/no-network tool sandbox policy.
- Validate returned policy and resolved model before sending the turn. Account
  details and private RPC error contents are not surfaced in result/errors.
- Match thread/turn identities, accommodate notifications before the start
  reply, and require accepted plus matching successful completion.
- Capture completed messages only; prefer `final_answer`, use unphased legacy
  output as fallback, and exclude commentary/partial/async/question output.
  Empty explicit final answers and oversized answers are failures.
- Deny every unexpected host request with current-schema rejection/error
  responses, then fail the plan. Whitelist harmless items and reject execution
  items/notifications, including foreign-turn snapshots. Failed or interrupted
  turns never return proposals.

## Verification handoff

Thirteen pure transcript tests cover inherited MCP isolation/private fields,
server cap, first-party sign-out/custom-provider readiness, final preference,
early completion, stale turn identity, failure/interruption, tool execution,
request denial, returned policy/private RPC errors, incomplete/async messages,
empty finals and foreign-turn execution snapshots. Only this file was formatted
with the pinned Rust formatter; no Cargo command or provider turn ran in this
implementation task.

Root should run focused tests/Clippy, real large output-schema acceptance, an
actual proposal through the installed CLI, and fake-process cancellation/
deadline/blocked-pipe teardown. Notification rejection prevents accepting a
proposal after tools; actual startup tool suppression needs separate evidence.
