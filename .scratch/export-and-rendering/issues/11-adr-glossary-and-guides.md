# ADR 0009, glossary, architecture, performance, usage and installation

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 01, 02, 03, 04, 05, 06, 07, 08, 10

## Scope

Record the decision, the vocabulary, where things live, every bound with its
constant, the user guide and FFmpeg encoder availability.

## Comments

2026-10-14: Written:

- [ADR 0009](../../../docs/adr/0009-export-renders-originals-through-the-preview-compositor.md),
  superseding the "no export path" statements of ADR 0006 and 0007 by pointer.
- [Glossary](../../../GLOSSARY.md): an Export and rendering section; Proxy and
  Subtitle file extended.
- [Architecture](../../../docs/architecture.md#export-and-rendering): ownership
  rows, the export path, proxies and preview quality, the codec boundary.
- [Performance](../../../docs/performance.md#export-and-proxy-budgets): every
  bound with its constant, the measurements with their conditions, what was
  not measured.
- [Usage](../../../docs/usage.md#exporting): exporting, the render queue,
  failures, limits, preview quality and proxies.
- [Installation](../../../docs/installation.md#export-encoders): encoders by
  format, runtime detection, licensing pointer. README feature list.
- Dated pointers in the core-editor, advanced-editing and titles-and-captions
  specs and in the core-editor UI dependencies.

Verification (`docs/verification.md#export-and-rendering`) is ticket 12. The
interface paragraphs of the user guide follow ticket 09's scope and must be
checked against the window when it lands.

2026-10-14: Checked against the window (ticket 09) and the review fixes.
The guide had promised a poster the window did not draw (it now does),
left out Shift+Cmd/Ctrl+E, said the Preview section reports proxies
unavailable without FFmpeg (each proxy fails instead), and named the
software encoder "Software" (it reads "Software · libx264"); the Clear
proxies tooltip contradicted the guide (fixed in `src/ui/sheets.rs`). All
are corrected. Also brought in: held jobs and Start all, the replace
question for caption files, the quit guard, the queue's duplicate-file
refusal, the working folder beside the destination, the stale-file sweep,
the finish watchdog, the fallback exclusions, the size estimate and NVENC
ceiling, the proxy changes, and the agent protocol's refusals that now
surface as a failed job. The links to
`docs/verification.md#export-and-rendering` point at ticket 12 and
performance until that section exists.
