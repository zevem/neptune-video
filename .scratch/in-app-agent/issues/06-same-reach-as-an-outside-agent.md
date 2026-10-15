# The in-app agent reaches as far as an outside agent

Status: done
Progress: done; native runs recorded in [native runs 2026-10-15](../evidence/native-runs-2026-10-15.md)
Type: task
Blocked by: 02, 03

## Scope

- Leave and return to the window. `aster-mcp --window FILE` starts in the
  window and goes back to it when `desktop {op: attach}` names no window.
  `project_create`, `project_open`, `project_close` and `desktop {op:
  open}` work as for any MCP client; `check_unpinned` is removed.
  `Workspace.following` in `src/mcp/workspace.rs` decides whether the
  server looks for its window again.
- Live roots file. `aster-mcp --roots-file FILE` reads `{"roots": [DIR,
  …]}` again whenever the file changes (modified time, length and, on
  Unix, inode), at most 256 KiB, filtered by `usable_roots`. The window
  writes its session file with extension `.roots` from the thread
  `aster-agent-roots` when the project's revision, path or the person's
  folders change, and no longer passes `--root`. Roots are no longer part
  of the conversation's identity.
- Settings folders. Settings › Agent › Folders with "Add folder…" and
  Remove, `AiSettings.folders`, and the too-wide notice.
- Notes for other projects. The panel counts tool results that report a
  change; those beyond this window's are named as changes in other
  projects in the turn's note and in Stop's note.
- Tests in `tests/mcp.rs` and `src/ai.rs`.

## Result

- `--window FILE` attaches to the window at once, or at the first call if
  it is not answering yet. A project no window shows is edited by the
  server itself and saved after every edit; `desktop {op: open}` shows a
  project in a new window; `desktop {op: attach}` with an `instance_id`
  reaches another window, and with none comes back. While the server
  follows its window, a gone window answers `desktop_closed`. Closing,
  opening or launching by choice ends following; attaching back starts it
  again. `history {op: undo}` takes back only the agent's own entries
  since it last attached.
- With a roots file and no `--root`, the working directory is not a root;
  with no roots at all every path is refused `outside_roots`, whose
  message now ends "the person can add one with --root, or for Aster's own
  agent in Settings › Agent › Folders". `project_status.roots` lists the
  current roots.
- The roots file is 0600 on Unix, written whole (a part file renamed over
  it) and removed when the window's agent is dropped. The window waits for
  the write at most one second only when a request is sent while no
  conversation is going on and the folders changed since the file was last
  written; with the folders unchanged it never waits, and while a
  conversation goes on the file follows changes without a wait.
- Importing media from a new folder or adding a folder no longer restarts
  the conversation. The restart note reads "A new conversation started,
  because the agent's provider, model or effort changed. It remembers
  nothing of the last one."
- Notes: " · 1 change in another project" or " · N changes in other
  projects" after the turn's note, "no changes here" when only other
  projects changed; Stop: "Stopped. … · N changes in other projects" or
  "Stopped. Nothing was changed here.". The window still ends the
  conversation when the person replaces its project.
- Tests on macOS 26.5.2 arm64: `ASTER_TEST_DESKTOP=1 cargo test --locked
  --test mcp`, 12 passed.
  `a_server_started_for_a_closed_window_still_makes_projects_of_its_own`
  replaces
  `a_server_pinned_to_a_closed_window_edits_nothing_and_opens_no_project`;
  `the_agent_tab_server_starts_in_its_window_and_may_leave_and_come_back`
  replaces `the_agent_tab_server_edits_only_its_own_window`; new
  `the_folders_in_a_roots_file_are_read_again_when_it_changes`. New unit
  test in `src/ai.rs`:
  `the_agents_folders_are_written_whole_for_its_owner_only_and_replaced_when_they_change`.
  `cargo test --locked --lib`: 1125 passed.
- Native runs on macOS with release builds E and F
  ([evidence](../evidence/native-runs-2026-10-15.md), summarized under
  [In-app agent](../../../docs/verification.md#in-app-agent)): Claude made
  and exported another project and came back to its window; a folder added
  in Settings › Agent › Folders was used in the same conversation; Codex
  exported and saved in a narrow window; Stop with Codex mid-request and
  early; Codex and then Claude edited a project in a second window and came
  back. Defect 12, the shared `aster-tab` tag that hid the second window's
  "Agent: Edit text" row, was found and fixed: each window now tags its
  agent's requests `aster-tab-<instance>`.

## Comments

2026-10-15: Opened and implemented. A goal check found that the first
round's agent, pinned to its window with roots fixed for the conversation,
could do less than an outside MCP agent. Decided on the owner's behalf
(2026-10-15): the in-app agent gets the same reach as an outside agent,
starting in the person's window and saying so when it leaves and comes
back, and the person can add folders in Settings › Agent › Folders. This
issue closes once the native runs are recorded. Linux, Windows and release
builds are unverified, as in
[issue 04](04-linux-and-windows-acceptance.md).

2026-10-15: Closed after the native runs on one macOS host with release
builds E and F, both providers
([evidence](../evidence/native-runs-2026-10-15.md)). Defect 12 was found
and fixed in the runs: each window now starts its agent's server with its
own tag, `aster-tab-<instance>`, and claims only its own requests. Linux
and Windows windows remain unverified
([issue 04](04-linux-and-windows-acceptance.md)).
