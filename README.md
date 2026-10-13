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

See the [user guide](docs/usage.md) for controls and current limits, and the
[feature acceptance record](docs/verification.md) for what was actually tested.
The verification record links the current tests, builds and native captures,
including actual provider requests and protected save/recovery behavior.

## Build and run

Use the pinned Rust 1.97.1 toolchain. A graphical desktop and graphics driver are
required. FFmpeg/ffprobe must be installed for video/audio processing; still-image
decoding is native to the Rust media adapter.

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
| `crates/editor-core` | Typed project/timeline model, validated commands, undo/redo and frame timing |
| `crates/editor-media` | Media probing, decoding and bounded worker services |
| `crates/editor-store` | Versioned save/load, autosave, recovery and checkpoints |
| `crates/editor-agent` | Native provider planning, exact validated proposals and local authenticated editing transport |
| `src/` | Native desktop UI and `aster-agent` command interface |

## Documentation

- [Core-feature spec](.scratch/core-editor/spec.md) and [backend contract](.scratch/core-editor/backend-contract.md)
- [Installation](docs/installation.md) and [user guide](docs/usage.md)
- [Agent protocol](docs/agent-protocol.md)
- [Native AI editing](docs/native-ai.md)
- [Architecture](docs/architecture.md) and [domain vocabulary](GLOSSARY.md)
- [Interface design](docs/design.md) and [performance](docs/performance.md)
- [Development](docs/development.md) and [verification/release gates](docs/verification.md)
- [Agent guidance](AGENTS.md) and [local issue tracker](docs/agents/issue-tracker.md)
