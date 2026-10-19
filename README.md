# Neptune Video

<img src="assets/branding/neptune-video-icon.png" alt="Neptune Video logo" width="96" height="96">

A free, open-source native video editor built in Rust for the Neptune
ecosystem, with a shared editing command model for people and AI agents.

The desktop follows Neptune Terminal's `egui`/`wgpu` stack. Project rules live in an
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
- An Agent panel that talks with the person's signed-in Claude Code or Codex.
  By default the agent edits the open project as it works, through Neptune Video's MCP
  tools started in that window: it can look at frames, each edit appears at
  once as one undo step shared with the person, and like any MCP client it can
  also make, open and export other projects. Set to Proposed for review, a
  planner that uses project metadata returns one proposal that changes nothing
  until it is applied; unchanged proposals preserve history. Debug builds were
  run with both providers on macOS only.
- `neptune-video-mcp`, an MCP server through which an agent in an MCP client creates,
  inspects, edits, analyses, previews and exports projects with the window's
  own commands, on its own or in a running Neptune Video window.
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
required. Video/audio processing runs FFmpeg, which Neptune Video ships with itself:
fetch the pinned, checksum-verified build for this computer once, and a build
in this checkout finds it in `.tools/media/` with nothing installed. Still-image
decoding is native to the Rust media adapter. Speech models are downloaded on
request with the system's `curl`, or chosen as files.

```sh
python3 scripts/fetch-media-tools.py
cargo run --release --locked --bin neptune-video
```

The local launcher runs the existing optimized binary and fetches FFmpeg first
when it is missing:

```sh
./scripts/run-local.sh
```

Explicit `NEPTUNE_VIDEO_FFMPEG` / `NEPTUNE_VIDEO_FFPROBE` overrides still win. It does not
rebuild or install anything. After editing source, rebuild with
`cargo build --release --locked --bins` before using it.

Open a project or import sources at launch:

```sh
cargo run --release --locked --bin neptune-video -- /absolute/film.nvp /absolute/media
```

See [installation](docs/installation.md) for platform prerequisites and media
tools, and [development](docs/development.md) for focused checks. The deliverable
is a source build; packages carry FFmpeg beside `neptune-video`, and signed installers
remain release work.

## Project structure

| Component | Responsibility |
| --- | --- |
| `crates/editor-core` | Typed project/timeline model, titles and captions, validated commands, undo/redo, frame timing and export plans |
| `crates/editor-media` | Media probing, decoding, the text renderer, speech recognition, export encoding, proxies and bounded worker services |
| `crates/editor-store` | Versioned save/load, autosave, recovery and checkpoints |
| `crates/editor-agent` | Native provider planning, the Agent panel's provider conversation, exact validated proposals and local authenticated editing transport |
| `src/` | Native desktop UI, compositor, render queue, `neptune-video-agent` command interface and `neptune-video-mcp` MCP server |

## Documentation

- [Core-feature spec](.scratch/core-editor/spec.md) and [backend contract](.scratch/core-editor/backend-contract.md)
- [Export and rendering spec](.scratch/export-and-rendering/spec.md) and [ADR 0009](docs/adr/0009-export-renders-originals-through-the-preview-compositor.md)
- [Installation](docs/installation.md) and [user guide](docs/usage.md)
- [Agent protocol](docs/agent-protocol.md)
- [Native AI editing](docs/native-ai.md) and [ADR 0011](docs/adr/0011-the-agent-tab-edits-through-neptune-video-mcp-started-in-its-window.md)
- [MCP server](docs/mcp.md) and [ADR 0010](docs/adr/0010-mcp-reaches-the-editor-through-the-agent-protocol.md)
- [Architecture](docs/architecture.md) and [domain vocabulary](GLOSSARY.md)
- [Interface design](docs/design.md) and [performance](docs/performance.md)
- [Development](docs/development.md) and [verification/release gates](docs/verification.md)
- [Agent guidance](AGENTS.md) and [local issue tracker](docs/agents/issue-tracker.md)

## License

Neptune Video is [MIT licensed](LICENSE). Bundled fonts retain their SIL Open
Font License notices; Tabler icons and vendored skills retain their upstream
licenses and attribution. See [third-party notices](THIRD-PARTY-NOTICES.md).
