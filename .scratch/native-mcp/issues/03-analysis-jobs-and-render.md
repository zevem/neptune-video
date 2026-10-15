# Pictures, analysis jobs and exports

Status: ready-for-human
Progress: resolved
Type: task
Blocked by: 01

## Scope

`frame_render` and `contact_sheet` through the export's compositor;
`scenes_start`, `loudness_start`, `transcription_models`,
`transcribe_start` as jobs; `render_capabilities`, `render_export_start`
and `job` (wait with progress, cancel, pause, resume, retry, remove).

## Result

`src/mcp/{frames,jobs}.rs`, `src/mcp/tools/{analysis,render,jobs}.rs`,
`editor-media::inspect` (scene cuts, ffprobe details, `import_now`, with
three unit tests). Whisper `tiny` installed and transcribed macOS `say`
speech; an export cancelled mid-render ended `cancelled` and its part file
was removed.

## Comments

2026-10-14: Review fixes: measurements and transcripts apply only to the
project state they were made on; local exports survive project switches;
speech jobs one at a time; jobs cancelled at shutdown; frames encoded off
the protocol threads with a pixel budget and cancellation; contact sheets
fit 2048 px; notifications at most twice a second.
