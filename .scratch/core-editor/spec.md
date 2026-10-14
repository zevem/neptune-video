# Core desktop video editor

Status: ready-for-agent
Progress: resolved
Type: task

Build a native Rust desktop editor for Linux, macOS and Windows, using Neptune's
stack and engineering patterns where applicable. The initial core provides the
editing foundation below. AI operations use the same validated, reversible
commands as human editing; provider integration can follow after the core is
usable.

## Acceptance criteria

| Area | Required behavior | Proof |
| --- | --- | --- |
| Projects | Create, open, save, autosave, duplicate and change project settings; independent project identity and dirty state | Native create/edit/save/reopen/duplicate flow plus revision and failure tests |
| Import | Videos, images, GIFs and audio through chooser, file drop and recursive folder import | Real supported-format fixtures, native drop tests, invalid/offline media and cancelled import |
| Media library | Thumbnails, library folders, search, sort and asset preview | Native large-library flow, state persistence and bounded cache observation |
| Timeline | Video, audio and text tracks without an arbitrary track-count product limit; snapping, zoom and markers | Command tests plus native multi-track scroll, zoom, snapping and marker navigation |
| Clip editing | Trim, split, crop, move, copy, paste, duplicate and delete | Valid/invalid boundary tests, native gestures, clipboard flow and undo for each operation |
| Advanced editing | Ripple edits, linked clips, groups and track lock/hide/mute | Native editing scenarios plus offset, relationship and locked-track invariants |
| Playback | Real source preview, scrubbing, frame stepping and playback-speed controls | Decoded fixture frames, native latency observation, actual audio/video timing where applicable |
| Transform | Position, scale, rotation, opacity and horizontal/vertical reflection | Command validation plus final preview pixels and undo |
| Canvas | 16:9, 9:16, 1:1, custom dimensions and backgrounds | Saved settings, native preview geometry and invalid-dimension tests |
| History | Undo/redo for all durable edits, named checkpoints and recovery | Mixed edit sequences, redo-branch behavior, save/load history, interrupted/failing write scenarios |

## Cross-cutting requirements

- Stable IDs and typed serialized commands make edits independent of UI focus.
- Invalid commands and failed atomic batches leave the document unchanged.
- Integer timeline time and rational frame rate preserve frame boundaries over
  long projects, including fractional rates.
- Media and filesystem work remain off the interactive thread. Queues, decoder
  concurrency and caches have explicit resource budgets.
- Source media remains untouched by non-destructive editing. Missing sources
  keep asset identities and clips and can be presented as offline.
- Future, unreadable and damaged project documents are protected from silent
  replacement. Recovery preserves the original bytes.
- The interface is minimal, sophisticated and original, using rounded icons
  with meaningful tooltips and accessible names where text is unnecessary.
- Native runtime, input, GPU and persistence acceptance is recorded separately
  for Linux X11/Wayland, macOS and Windows.

## Completion boundary

A command implemented in the core is not complete UI acceptance. A preview of
a still thumbnail is not real-time video playback. CI matrix configuration is
not native platform verification. Update [verification](../../docs/verification.md)
with implemented scope, checks actually run and remaining gates before calling
this production-ready or comparable to an established editor.

Export, effects, transitions, captions, collaboration and cloud AI credentials
are outside this first core-feature request. A later deliverable should specify
export and full audio/codec deployment explicitly before release.

## Comments

2026-10-12: Initial spec transcribes the user's requested feature set. Neptune
is a read-only reference; no performance numbers or release evidence carry over.

2026-10-12 implementation checkpoint: the native Rust core/UI contains the
requested categories, with Linux XTest editing evidence and passing backend
fixtures. The [implementation/evidence matrix](../../docs/verification.md#implemented-feature-matrix)
tracks exact coverage and limits. Linux source-monitor input passed in round
three; the initial core source checkpoint passed 169 tests with real FFmpeg, formatting, Clippy
and optimized builds. Native Weston file-drop/timeout/close and GIF cadence
also passed. The final optimized binaries passed all 13 X11 smoke checks,
native exit/recovery-history retention and a fresh Weston URI drop; two paused
CPU/RSS observations are retained with their limits. This resolves the
implementation phase; macOS/Windows native acceptance and the explicit
production-gate tickets remain open.

2026-10-12 native-AI/recovery checkpoint: 309 workspace tests now pass, including
90 desktop tests. Core PreparedEdit commits the exact validated proposal state;
native providers and protected GTK legacy/private recovery passed on db30/9f.
Source28259254/release2c596cb0 adds truthful "No changes" feedback and passed
its own native acceptance, including both real providers and GTK import/recovery.
See the current verification record;
the implementation resolution does not close the broader production gates.

2026-10-14: Export is now specified and implemented in
[export and rendering](../export-and-rendering/spec.md), with
[ADR 0009](../../docs/adr/0009-export-renders-originals-through-the-preview-compositor.md).
The "Export … outside this first core-feature request" sentence above stays as
the record of this phase. Codec deployment and licensing remain with
[ticket 03](issues/03-codec-and-distribution.md).
