# Agent render operations

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 01

## Scope

`AgentOperation`s `export_capabilities`, `queue_export`, `render_jobs`,
`cancel_render`, `pause_render`, `resume_render`, `retry_render`,
`remove_render`; results, error codes, `check_target`,
`check_queue_export`, `AgentCall::claim`; `unsupported` from headless
`handle`. Contract review rule 17.

## Comments

2026-10-14: Implemented in [editor-agent](../../../crates/editor-agent/src/lib.rs)
and documented in [agent protocol](../../../docs/agent-protocol.md#render-jobs).
72 lib and 25 protocol tests (11 new) and 13 `aster-agent` tests passed.
The desktop's answers from its queue belong to ticket 09; until then the
window answers render operations `unsupported`.
