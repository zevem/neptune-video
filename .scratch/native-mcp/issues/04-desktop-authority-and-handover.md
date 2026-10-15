# Editing the project open in a window

Status: ready-for-human
Progress: resolved
Type: task
Blocked by: 01

## Scope

`desktop {list, attach, open}`, `project_open` routing to a window,
handover by file version, `person_editing` / `desktop_busy` refusals, the
agent's own undo, saving through the window, and recovery copies.

## Result

`an_agent_edits_the_project_open_in_a_window` (with `ASTER_TEST_DESKTOP=1`)
passed against the debug `aster` on macOS. A person's Cmd+Z and M (sent as
key presses) reached the agent, whose undo refused the person's entry;
the agent's transition, look and title appeared in the window at once.
Captures are in `../captures/`.

## Comments

2026-10-14: `project_open` refused `project_busy` when the window had
unsaved work; it now attaches to the window first.

2026-10-14: Review fixes: the window answers reads while the person edits;
work in no file is never released; a refused save's renewed context is
followed. Re-run on the final build with new captures in `../captures/`.
