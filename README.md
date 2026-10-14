# Aster

A native Rust desktop video editor with a shared editing command model for
people and AI agents. Aster is an initial product name.

The desktop follows Neptune's `egui`/`wgpu` stack. Project rules live in an
independent Rust core; media and persistence work run behind separate boundaries.
The current development build includes project save/recovery, a media library,
video/audio/text tracks, reversible clip edits, playback, transforms and canvas
settings. Linux native editing has been exercised; macOS/Windows acceptance and
production release gates remain in [verification](docs/verification.md).

## Core features

- Create/open/save/autosave/duplicate projects, project settings, checkpoints
  and undo/redo.
- Import video, images, GIFs and audio; folder organization, thumbnails, search
  and sorting.
- Video/audio/text tracks, snapping, zoom, markers, trim, split, crop, move,
  copy/paste, duplicate and delete.
- Ripple edits, linked/grouped clips and track lock/hide/mute controls.
- Source/timeline preview, scrubbing, rational frame stepping, playback speed,
  position/scale/rotation/opacity/reflection and canvas presets/backgrounds.
- Local authenticated agent access with project/context/revision guards, shared
  edit validation and undo history.
- Native Claude/Codex requests, reviewable editing proposals, cancellation and
  guarded edits as one undo step; unchanged proposals preserve history. This
  first planner uses project metadata.
- Titles with fonts, colours, outlines, shadows and backgrounds; fade, pop,
  bounce, typewriter, slide and wipe animations by line, word or letter; lower
  thirds, title cards, callouts, stickers and emoji with editable texts and
  colours; kept looks, templates and brand colours.
- Captions: made locally from the timeline's speech by a built-in whisper.cpp
  with word times and karaoke looks, edited and timed in a list, imported and
  exported as SRT or VTT, one track per language, and translated through the
  native AI provider for review.
- Export from the original media through the preview's compositor: MP4, MOV
  and WebM with H.264, H.265, AV1, VP9 or ProRes, or sound-only M4A, MP3, WAV
  and FLAC; presets for common destinations, a range, a size and frame rate,
  captions burned in, beside the file or inside it, and hardware encoders
  where this computer's FFmpeg has a working one. Files are written whole or
  not at all.
- A background render queue with pause, resume, cancel, retry and reordering;
  agents in a running editor can queue, list, pause, resume, cancel, retry and
  remove jobs.
- Preview quality settings and proxies: smaller copies of large videos for the
  preview only, kept in a bounded cache outside the project.

See the [user guide](docs/usage.md) for controls and current limits, and the
[feature acceptance record](docs/verification.md) for what was actually tested.
The verification record links the current tests, builds and native captures,
including actual provider requests and protected save/recovery behavior.

## Build and run

Use the pinned Rust 1.97.1 toolchain, plus CMake, a C/C++ compiler and libclang
to build the speech recognizer. A graphical desktop and graphics driver are
required. FFmpeg/ffprobe must be installed for video/audio processing; still-image
decoding is native to the Rust media adapter. Speech models are downloaded on
request with the system's `curl`, or chosen as files.

```sh
cargo run --release --locked --bin aster
```

For this prepared Linux checkout, the local launcher uses the existing optimized
binary and task-local FFmpeg tools when available:

```sh
./scripts/run-local.sh
```

It preserves explicit `ASTER_FFMPEG` / `ASTER_FFPROBE` overrides and does not
download, rebuild or install anything. After editing source, rebuild with
`cargo build --release --locked --bins` before using it.

Open a project or import sources at launch:

```sh
cargo run --release --locked --bin aster -- /absolute/film.aster /absolute/media
```

See [installation](docs/installation.md) for platform prerequisites and media
tools, and [development](docs/development.md) for focused checks. The deliverable
is a source build; signed installers and codec bundling remain release work.

## Project structure

| Component | Responsibility |
| --- | --- |
| `crates/editor-core` | Typed project/timeline model, titles and captions, validated commands, undo/redo, frame timing and export plans |
| `crates/editor-media` | Media probing, decoding, the text renderer, speech recognition, export encoding, proxies and bounded worker services |
| `crates/editor-store` | Versioned save/load, autosave, recovery and checkpoints |
| `crates/editor-agent` | Native provider planning, exact validated proposals and local authenticated editing transport |
| `src/` | Native desktop UI, compositor, render queue and `aster-agent` command interface |

## Documentation

- [Core-feature spec](.scratch/core-editor/spec.md) and [backend contract](.scratch/core-editor/backend-contract.md)
- [Export and rendering spec](.scratch/export-and-rendering/spec.md) and [ADR 0009](docs/adr/0009-export-renders-originals-through-the-preview-compositor.md)
- [Installation](docs/installation.md) and [user guide](docs/usage.md)
- [Agent protocol](docs/agent-protocol.md)
- [Native AI editing](docs/native-ai.md)
- [Architecture](docs/architecture.md) and [domain vocabulary](GLOSSARY.md)
- [Interface design](docs/design.md) and [performance](docs/performance.md)
- [Development](docs/development.md) and [verification/release gates](docs/verification.md)
- [Agent guidance](AGENTS.md) and [local issue tracker](docs/agents/issue-tracker.md)
