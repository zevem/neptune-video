# Conversation runtime and providers

Status: ready-for-human
Progress: resolved
Type: task
Blocked by: none

## Scope

`editor_agent::Conversation`, `ToolServer`, `TurnEvent` and `INSTRUCTIONS`
in `crates/editor-agent/src/conversation.rs`: one worker thread per window
that owns the provider process, `ask`, `stop`, `end` and `poll`, the
conversation's limits, and when a request continues the conversation or
starts fresh. The stream readers `provider/claude_agent.rs` and
`provider/codex_agent.rs`, `provider::converse`, and the conversation
`Limits` in `provider/process.rs`. The planner stays as it is.

## Result

- A request is 1 to 65,536 bytes without NUL; another is refused as busy
  while a turn runs. Events: `Started {turn, fresh}`, `Said`, `Called
  {call, tool, op}`, `Returned {call, ok, detail}`, then exactly one of
  `Finished {model}`, `Failed {message}` or `Cancelled`.
- Limits: a provider process lives at most 12 hours, 600 seconds without a
  word from the provider fails the turn, and at most 4096 unread events are
  kept, oldest steps first and never a turn's end. Conversation output
  lines may be up to 256 MiB; stream and standard error have no total
  limit.
- A conversation continues while the provider, executable, model, effort
  and the tool server's command and arguments are the same.
- Claude Code 2.1.300 runs with built-in tools off, one strict MCP server
  in a private config and `--permission-mode dontAsk`, without
  `--safe-mode`. Codex 0.162.1 runs `codex app-server` with
  `code_mode_host` on and the shell tool off.
- Tests with `ASTER_PROVIDER_TEST_FIXTURE`: 84 passed in the
  `editor-agent` library, among them three conversation tests (a fake
  `claude` script: several turns keep one process, `end` starts fresh,
  `stop` ends within 3 s, a missing provider fails), three for
  `claude_agent` and three for `codex_agent` (the shell stays off and
  `code_mode_host` stays on). One failed:
  `provider::tests::an_unrepresentable_native_path_is_refused_before_creating_an_instruction_file`
  (os error 92), a failure on this host that predates this work.
- Real providers: [native runs](../evidence/native-runs-2026-10-14.md).

## Comments

2026-10-14: Resolved on macOS 26.5.2 with debug builds. Two probes changed
the providers' arguments for conversations: Claude Code 2.1.300's
`--safe-mode` drops every MCP server, Aster's included, so conversations do
not use it; Codex 0.162.1 with `code_mode_host` off answered "I couldn't
call aster_window.project_status because the tool host is disabled", so
conversations keep it. Other Codex versions are unverified.
