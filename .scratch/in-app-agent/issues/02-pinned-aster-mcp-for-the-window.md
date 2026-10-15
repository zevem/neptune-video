# aster-mcp started in its window

Status: ready-for-human
Progress: resolved
Type: task
Blocked by: none

## Scope

`aster-mcp --window FILE` and `--tag TAG` in `src/mcp/mod.rs`, the home
window in `src/mcp/workspace.rs` (`pin` and `following`), `usable_roots`,
and the window side in `src/app.rs` and `src/ai.rs`: `tool_program`,
`tool_server`, `agent_roots`, the per-window tag (`window_tag`) and
`own_request`.

## Result

- `--window FILE` starts the server in the window whose session file is
  FILE: it attaches at once, or at the first call if the window is not
  answering yet, and goes back to it when `desktop {op: attach}` names no
  window. `--tag` takes 1 to 64 ASCII letters, digits and dashes; request
  IDs become `TAG-N`, `mcp` by default.
- The server is not locked to its window. `project_create`,
  `project_open`, `project_close` and `desktop {op: open}` work as for any
  MCP client ([issue 06](06-same-reach-as-an-outside-agent.md)). While it
  follows its window, a gone window answers `desktop_closed` and, with no
  project attached, the next call attaches again. Closing, opening or
  launching a project by choice ends following; attaching back to the
  window starts it again.
- The window starts the program named by `ASTER_MCP`, else `aster-mcp`
  beside the running `aster`, with `--window`,
  `--tag aster-tab-<instance>` (the window's agent session instance ID in
  32 hex digits, from `ai::window_tag`), `--roots-file` (its session file with extension `.roots`),
  `--desktop-settings` and `--models`. It no longer passes `--root`.
- Roots: the project file's folder, the folder of every asset, the
  folders the person added in Settings › Agent › Folders, then Videos or
  Movies, Desktop, Documents, Downloads, Music and Pictures from
  `directories::UserDirs`. The window writes them raw to the roots file;
  the server keeps them in order with `usable_roots` as canonical
  directories without too-wide or nested roots, and reads the file again
  when it changes.
- For requests with its own tag, `aster-tab-<instance>-N`, the window
  counts the turn's changes and adds no "Agent: …" rows; outside clients,
  another window's agent included, keep theirs.
- First round, `ASTER_TEST_DESKTOP=1 cargo test --locked --test mcp`: 11
  passed, three runs in a row. Its two pinned-server tests were replaced
  in [issue 06](06-same-reach-as-an-outside-agent.md), where the file now
  has 12 cases.

## Comments

2026-10-14: Resolved on macOS 26.5.2. Both desktop tests now close their
window even when they fail; a failed run had left one open. Decided on the
owner's behalf (2026-10-14): the roots include the person's usual media
folders, not only the project folder.

2026-10-15: Changed. This issue first made a server pinned to its window:
`check_unpinned` refused `project_create`, `project_open`, `project_close`
and `desktop {op: open}` with `unavailable`, and the roots were `--root`
arguments fixed for the conversation. That agent could do less than an
outside MCP agent. Decided on the owner's behalf (2026-10-15): the server
starts in its window and may leave and come back, `check_unpinned` is
gone, and the roots come from a live `--roots-file`. The file name stays
for history; the text above describes the new behaviour. The work and its
tests are tracked in [issue 06](06-same-reach-as-an-outside-agent.md).
