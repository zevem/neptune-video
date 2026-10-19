---
status: accepted
---

# MCP reaches the editor through the agent protocol

Any MCP client (Claude Code, Codex, others) edits Neptune Video projects through
`neptune-video-mcp`, a binary of the root package built on the official Rust MCP
SDK (`rmcp` 3.5), over stdio or Streamable HTTP on a loopback address. The
binding design, with its sixteen decisions, is
[the native MCP spec](../../.scratch/native-mcp/spec.md); this record keeps
the choices that shape the code. The existing contracts hold: every edit is
a validated, reversible `Command`; the core knows no GUI, codec or
filesystem; `editor-agent` depends on neither the store nor the media crate.

**One authority per project, reached by `AgentRequest`.** The server holds no
project state of its own. Its project is either **local**, the server's
own `Editor` with a protected `DocumentSession`, saved after every changing
edit by the same `headless` code `neptune-video-agent --project` uses, or
**desktop**, the `Editor` of a running window, reached through the
window's authenticated `AgentHost` session. Every MCP mutation becomes one
guarded `AgentRequest` (`apply`, `undo`, `redo`, `save`) carrying the
recorded project ID, context ID and the revision the tool built its
command from. Both authorities answer it with `editor_agent::handle`, so
human and agent edits pass the same validation and enter the same history.
One thread owns the authority and tools reach it through a bounded queue,
so clients are serialized and a window receives one request at a time.
Two alternatives were set aside:

- a server-side replica synchronized with the window, which would be a
  second source of truth that can disagree with the window;
- a new IPC channel for MCP, which would duplicate the session, token,
  revision and stale-context rules the agent protocol already enforces.

**Typed tools are thin builders over the window's own rules.** The window's
builders for placement, moves, trims, splits, transitions, fades, audio
presets, speed ramps, title templates, captions and speech-to-captions
moved into the library (`rules::place_asset`, `rules::split_command`,
`Editor::paste_command` and others) when they were not already there. The
MCP tools and the window call the same functions, and each tool returns one
`Command`, a `Batch` when it needs several. `edit_apply` accepts raw core
commands, so an editor feature is reachable over MCP as soon as it is a
command, before it has a typed tool. A tool layer that reimplemented the
rules was set aside: it would drift from what the window does.

**The window stays the person's.** A window refuses agent mutations with
`person_editing` while the person holds a change that is not yet an edit
(a drag, a focused field), and with `desktop_busy` while it is asking
something. The client retries; a refused edit has changed nothing. Agent
undo takes back only the agent's own entries, recognised by
`(context_id, revision)` pairs, unless the client passes `allow_person`.
`project_open` on a file a window has open edits it in that window,
including the person's unsaved work. A local project whose file a window
opens is handed over only when both hold the same `file_version`
(`bytes:sha256`); otherwise the server refuses with `desktop_stale` or
`desktop_conflict`. There are never two writers to one file.

**Jobs never edit.** Transcription, model installs, scene detection,
loudness and exports run as jobs with progress and cancellation (`job`).
Their results are applied by an ordinary edit that names the job
(`captions {from_transcript}`, `audio_edit {normalize}`), so every change
to the project is still one guarded command. Exports keep
[ADR 0009](0009-export-renders-originals-through-the-preview-compositor.md):
a desktop project's export runs in the window's render queue, and a local
project's export runs in a `RenderQueue` the server owns. MCP Tasks, the
protocol's own long-running operations, were set aside while client
support is uneven; `job {op: wait}` waits at most 50 s and sends progress
notifications.

**Pictures come from the export's compositor.** `frame_render` and
`contact_sheet` draw with a headless `FrameRenderer` on the server's own
graphics device. An agent therefore sees the same pixels the export would
write, effects and titles included, without a window. Frames are capped at
2048 pixels a side, 48 and 64 million pixels per call.

**Safety is the server's, not the client's.** The server reads and writes
only inside its roots. Paths are canonicalized and symbolic links
followed, `..` is refused, and nothing is replaced without
`overwrite: true`. FFmpeg probes run with `-protocol_whitelist file`.
Streamable HTTP binds loopback literals only and requires a bearer token,
compared in constant time, from `NEPTUNE_VIDEO_MCP_TOKEN` or 244 random bits. It
validates `Host` and refuses browser `Origin`s, caps bodies at 4 MiB, and
writes its address and token to an owner-only connection file that is
removed on exit. FFmpeg formats that read other files are never media,
and media a raw command adds is probed again. A recovery copy left by an
editor that stopped is never replaced silently: `project_open` refuses
with `recovery_available` until the agent asks for `recover: true`.
Work the server holds that no file has is never deleted: leaving it is
refused, and a forced close or a stopping server keeps it in the
recovery copy.

**A small tool list.** A client reads every tool's schema before its first
call. The server keeps input schemas with their descriptions but drops
output schemas, and the edit result every editing tool returns is
described once in the server's instructions. The tool list stays under
40 KiB, a size an end-to-end test enforces.

## Consequences

- `src/lib.rs` turns the root package into a library shared by `neptune-video`,
  `neptune-video-agent` and `neptune-video-mcp`. `src/headless.rs` holds the headless
  document code the two command-line programs share.
- `AgentOperation` gains `History`, `Document` and `Save`, and `Applied`
  gains `created`. The window answers `Document` even while busy.
- A new editing feature needs a `Command` to be reachable over MCP and a
  typed tool to be convenient. No server change is needed for the first.
- Not offered: forced alignment of arbitrary text, starting tracking,
  stabilization, stems, beats, ducking or recording, clip rename, marker
  move, folder rename or removal, relinking, and MCP Tasks.
