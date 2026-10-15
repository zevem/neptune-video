# MCP acceptance on Linux and Windows

Status: needs-triage
Progress: not started
Type: task
Blocked by: 01, 02, 03, 04

## Scope

Run `tests/mcp.rs` (with `ASTER_TEST_DESKTOP=1` under a display), a real
Claude Code or Codex session and the hand-over script on Linux (X11 and
Wayland) and Windows. On Windows check stdio without the descriptor move
(`protocol_output` is Unix-only), the connection file's permissions (no
0600 there), and named-pipe-free loopback HTTP. Check whether a minimized
window answers within the host's 5 s on each platform.
