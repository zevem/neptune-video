# Agent tab panel and settings

Status: ready-for-human
Progress: resolved
Type: task
Blocked by: 01, 02

## Scope

The panel in `src/ui/agent.rs`, `App::agent_converse` and `window_note` in
`src/app.rs`, `ai::Agent` and `ai::step_words`, the Settings › Agent
"Edits" row in `src/ui/sheets.rs`, and `Action::SetAiReview` and
`Action::AiNewConversation` in `src/ui/mod.rs`.

## Result

- Ask defaults to Made as it works. Settings › Agent has "Edits" with As
  it works and For review (`AiSettings.review`, false by default). The
  panel's menu has Answered by, Effort, "Edits: Made as it works /
  Proposed for review", "New conversation" and Model….
- Each request is prefixed with the playhead and the selected clips.
- Steps show running, done with the edit's label, refused with the reason,
  or stopped; `step_words` names them ("Looked at frames", "Trimmed
  clips", "Undid an edit", "Waiting for a job"). Replies render bold,
  code, headings and bullets. The working row reads "Working… Ns · N
  changes" with Cancel; the hint is "Reply, or ask for more".
- Notes: "{Provider} · {model} · N changes; undo and redo take them back"
  (one: "1 change, which undo or redo takes back"; none: "no changes");
  "Stopped. The N changes already made stay; undo and redo take them back."
  (one: "Stopped. The change already made stays; undo or redo takes it
  back."; none: "Stopped. Nothing was changed."); "Another project is open,
  so the agent's conversation ended.";
  and the restart note.
- Tests: `markdown_marks_are_read_not_shown` and
  `a_step_ends_once_and_steps_left_running_stop_with_the_request`.
- Captures 01 to 15 in [`../captures/`](../captures/), wide and
  820x640 pt narrow, described in the
  [native runs](../evidence/native-runs-2026-10-14.md).

## Comments

2026-10-14: Resolved on macOS 26.5.2 with debug builds. Fixed during the
native runs: the working row truncated its count ("3 edi…",
`06-final-claude-working.png`), the note said "edits" although the agent's
own undo is counted too, and a restart gave no notice. Captures 10 to 13
show the fixed row, note, menu and restart note. Decided on the owner's
behalf (2026-10-14): new and existing settings start in Made as it works.
