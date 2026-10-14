# Export dialog and render queue panel

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 01

## Scope

`src/ui/export_sheet.rs` and `src/ui/render_queue.rs` as headless-tested
modules: presets, settings, range, summary, disabled reasons, save and delete
preset; queue rows, actions by state, empty state, summary ring.

## Comments

2026-10-14: 26 headless tests passed at 1440x900, 760x560 and 640x560 with
hand-built capabilities. The sheet is `min(760, screen − 64)` wide with two
columns from 640 pt. "Add to queue" queues the job paused; "Export" queues it
to run. Showing them in the window is ticket 09; no native capture exists yet.
