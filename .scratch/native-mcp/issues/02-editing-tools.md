# Typed editing tools over the window's rules

Status: ready-for-human
Progress: resolved
Type: task
Blocked by: 01

## Scope

Project, media, timeline, effects, animation, text, templates, captions,
audio, history and checkpoints, and `edit_apply` / `edit_schema` for every
core command. Each tool builds its `Command` with the builders the window
uses (`rules::place_asset`, `rules::split_command`,
`Editor::paste_command`, fades, transitions, presets, title templates,
caption builders).

## Result

`src/mcp/tools/*.rs`. End to end in
`an_agent_makes_edits_looks_at_and_exports_a_video_that_a_person_can_open`;
transcription, captions from speech, templates, masks, blend, retime,
markers, tracks and settings driven by a script against the debug build.

## Comments

2026-10-14: A text style out of range is refused with the ranges; Claude
Code had sent a shadow offset in pixels.

2026-10-14: Review fixes: template looks kept by `text_edit`; ripple trims
bounded by the source; `clip_add` on a new track at `at`; moves keep one
track and the same kind; `text_add` fitted for its length; two-edge fades;
drop-frame timecode refused; the agent's undo steps over several entries
(`an_agent_takes_back_and_redoes_its_own_edits_one_after_another`).
