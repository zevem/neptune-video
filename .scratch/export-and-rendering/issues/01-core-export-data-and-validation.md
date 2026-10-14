# Core export data, validation and presets

Status: ready-for-agent
Progress: resolved
Type: task

## Scope

`editor_core::export`: settings, containers and codecs, `Capabilities` data
and the `VIDEO_ENCODERS` table, `ExportSettings::plan` into an `ExportPlan`,
`FrameSpan`, `sample_count`, `Geometry`, `render_project`, `estimate_bytes`,
`range_cues`, `builtin_presets`, `RenderJobInfo`. `source_frame` moved from
the viewer into `editor_core::render`.

## Comments

2026-10-14: Implemented in
[export.rs](../../../crates/editor-core/src/export.rs) with 26 tests in
[tests/export.rs](../../../crates/editor-core/tests/export.rs) (none needs
FFmpeg): frame spans at 30000/1001, 24 and 60; sample counts exact over about
9.3 hours of NTSC frames; sizes, letterbox and fill; encoder choice with
host-like capabilities; every plan error; all 18 presets planning on full,
host-like and software-only capabilities; serde and schema round trips.
Reported on Linux: the tests, clippy with `-D warnings` and the architecture
check passed.

Differences from the spec: `MAX_OUTPUT_PIXELS` is public; `plan` also refuses
MP3 and Opus rates outside `SAMPLE_RATES`; a `subtitles.track` that is not a
caption track has its own message; `range_cues` clamps word times into the
cut cue rather than dropping them, so the word count holds.
