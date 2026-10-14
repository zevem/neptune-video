# Window integration

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 06, 07, 08, 10

## Scope

`App` builds the `RenderQueue` with the window's device; toolbar Export
button and queue toggle; Project menu "Export…" (Command/Ctrl+E) and "Render
queue"; `Overlay::Export` and `Overlay::RenderQueue`; asking before replacing
an existing file; `export-presets.json`; last-used settings and folder;
finished notices; quit guard and `shutdown`; the desktop's answers to the
agent render operations.

## Comments

2026-10-14: Under way at the time the docs were written. The user guide
describes the interface as the spec and the dialog modules define it; the
docs owner could not find the toolbar button, shortcut, overlays, preset file
or quit guard in `src/app.rs` yet. Check the guide against the window once
this lands.

2026-10-14: Integration landed in `src/app.rs`, `src/ui/toolbar.rs`,
`src/ui/sheets.rs`, `src/agent.rs` and `src/export/{wiring,presets,poster}.rs`,
and the review fixes after it. The docs owner checked the user guide, the
agent protocol and the architecture against it; see ticket 11.

## Answer

The window owns a `RenderQueue` on its device. The toolbar has the primary
Export button and the Render queue button (list icon idle, progress ring
while a job renders or waits). The Project menu has "Export…" (Cmd+E) and
"Render queue" (Shift+Cmd+E) after Checkpoints; each chord also opens its
sheet over the other, and the command panel takes "export" and "render
queue". The export sheet draws a poster from the originals on
`aster-poster`, keeps its state when closed unqueued, and remembers the last
settings, folder and preset (`export_last`, `export_folder`,
`export_preset`) on a successful queue. Replacing the file, or the file and
its caption file, is asked first. Closing the window while jobs render, wait
or are held asks "Rendering will stop" before unsaved changes, and exit shuts
the queue and the proxy lane down. Saved presets live in
`export-presets.json`. The desktop answers the agent's render operations
from the queue.
