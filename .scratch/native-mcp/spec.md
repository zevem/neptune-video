# Native MCP integration

Status: ready-for-human
Progress: implemented; native acceptance on macOS recorded, Linux and Windows not run
Type: task

Phase 8: any MCP client (Claude Code, Codex, others) creates, inspects,
edits, analyses, previews and exports Neptune Video projects through typed tools
that reach the same validated commands, history and project state as the
desktop. A person can open the project the agent made, or the one the agent
is editing in the running desktop, and continue in the window.

This document is the binding design. Code comments, tests and ADR 0010
follow it; a change to a decision is a dated comment below. It was revised
on 2026-10-14 after a three-lens review (synchronization, security,
feasibility); the review's findings are folded into the decisions.

## Decisions

1. **One server program, two transports.** `neptune-video-mcp` (a binary of the
   root package, a thin shim over `neptune_video::mcp::run`) serves MCP with
   the official Rust SDK `rmcp` 3.5 over stdio (default) or Streamable HTTP
   on a loopback address (`--http [ADDR]`). One server process edits one
   project at a time; every HTTP client of one server shares it.
2. **The root package gains a library target.** `src/lib.rs` declares the
   desktop's modules; `src/main.rs` and every binary use it. The MCP server
   lives in `src/mcp/` and reuses the desktop's pure builders (placement
   rules, transitions, audio presets, fades, speed, the title catalog,
   caption builders, speech-to-captions) and its export and frame renderer
   instead of copying them. The headless document code of `neptune-video-agent`
   moves into the library (`src/headless.rs`) and both binaries share it.
   No new crate; the `desktop → core/media/store/agent` edges are
   unchanged. The exported surface stays `app`, `settings`, `headless`,
   `mcp::run`.
3. **The authority owns the project; the server owns nothing durable.** A
   project is edited in exactly one place at a time:
   - **Local**: the server's own `Editor` with a protected
     `DocumentSession`, saved after every changing mutation exactly as
     `neptune-video-agent --project` does (the shared `headless` code). A project
     without a file is in memory until `project_save`. Opening or creating
     a file takes its `RecoveryLease` and refuses `project_busy` when
     another writer holds it. After a `storage_conflict` the project is
     `conflicted` and refuses mutations until it is opened again.
   - **Desktop**: the running desktop's `Editor`, reached through its
     authenticated `AgentHost` session descriptor. Edits apply on the
     window's thread through `editor_agent::handle`, appear at once in the
     window and mark it dirty, as every agent edit already does.
   Every MCP mutation becomes one guarded `AgentRequest` (`apply`, `undo`,
   `redo`, `save`) sent to the authority. Tools never edit a cached copy.
   One thread owns the authority; tools reach it through a bounded queue,
   so concurrent clients are serialized and only one request at a time
   reaches a desktop.
4. **Edits are bound to the project the session chose.** The workspace
   records the `(project_id, context_id, path)` it created, opened or
   attached. A typed tool builds its command from a fresh snapshot, but its
   request carries the recorded project and context as guards, never ones
   it has just read, and the snapshot's revision. A change between the two
   (the person editing) is refused as `stale_revision`; a replaced project
   in the window as `project_changed`, until `desktop` `attach` is called
   again. Every result carries `project_id`, `context_id` and `revision`.
   Callers may pass `expected_revision` to any mutating tool. Selection and
   playhead are never implicit inputs: tools take explicit IDs and times.
5. **The window protects the person's gesture and questions.** While the
   person holds a draft (a drag, a focused field), an agent mutation or
   save is refused `person_editing` (retryable); drafts are never
   force-committed for an agent. While an Unsaved or Conflict sheet is
   open, or a save is being written or prepared, mutations and saves are
   refused `desktop_busy`. `document` reports `editing`, `asking`,
   `saving`, `conflicted`, the last save's outcome and the file version.
   An agent save that conflicts sets `conflicted` and shows a notice, not
   a sheet. An agent's Save As creates a new file only.
6. **Handover, never two writers.** Before each call on a Local project
   with a file, the server looks (cached, at most once a second, 500 ms
   connect) for a live desktop session with the same canonical file open.
   Ownership is decided by file version (`bytes:sha256` of the document
   baseline), not by comparing projects:
   - same version as the server's last save → the server removes its
     recovery copy while it holds the lease, releases the lease and
     continues on the Desktop authority; the result says that Local undo
     history ended there;
   - the window is clean but has another version → `desktop_stale`;
   - the window is dirty with another version → `desktop_conflict`;
   - more than one live window has the file → `desktop_conflict`.
   A desktop that exits makes the next call answer `desktop_closed`; the
   server never reopens the file behind the person's back. `desktop`
   `open` saves, releases the Local project and launches the `neptune-video` beside
   the server with null standard streams, detached, with only
   `--agent-session` and the canonical file, at most one at a time.
7. **The protocol grows three operations.** `history` (past and future
   labels, answered by `handle`, so both hosts serve it), `document` (path,
   file version, dirty, editing, asking, saving, conflicted, last save,
   undo/redo, playhead and selection) and `save` (save, or Save As to a new
   file). The window publishes its `document` answer into the host after
   every frame's agent calls and saves, so `document` is answered on the
   host thread without waking the window. `OPERATIONS` keeps its order and
   appends them. `apply` answers also carry `created`, the IDs the edit
   added (tracks, clips, assets, folders, markers, trackers), computed by
   `handle` from the before and after snapshots, so a split's right half or
   a pasted clip is known to the caller the moment it exists. A dry run
   reports what would change, not IDs.
8. **Agent undo is the agent's own.** `history` `undo` and `redo` act only
   on the entry this server applied last (its own stack of
   `(context_id, revision)`), sent as `expected_revision`; otherwise they
   are refused `not_agent_entry` unless `allow_person: true`.
9. **Long work is a job, and jobs never edit.** Export, preview renders,
   transcription, model download, scene detection and loudness run as
   server jobs with an id, kind, state, progress, message and result.
   `*_start` tools answer at once; `job` `status`, `wait` (at most 50 s,
   default 20 s, sends MCP progress notifications, stops when the request
   is cancelled) and `cancel` work for every kind. Results that would
   change the project (words, loudness) are returned to the agent, which
   applies them with an ordinary tool (`captions` `from_transcript`,
   `audio` `normalize`), so a job can never apply against a project that
   changed under it. A Desktop project's exports go to the window's render
   queue (`queue_export`), so the person sees them; a Local project's go to
   a `RenderQueue` the server owns, answered by the same
   `export::wiring::render_result` the desktop uses. Local exports end
   with the server process; the result says so.
10. **Analysis runs in the server's process.** Frames and contact sheets
    are drawn by the export's `FrameRenderer` on one headless device owned
    by one frames thread, from the authority's snapshot; transcription by
    `SpeechService` (models in the desktop's default folder); loudness by
    `SoundService`; scene cuts and full metadata by new `editor-media`
    functions over FFmpeg. This supersedes ADR 0008's "an agent can start
    none of the work" and ADR 0009's "headless render jobs are
    unsupported" for MCP clients (ADR 0010); the desktop's interactive
    frames stay independent of all of it because it runs in another
    process. Images returned inline are at most 2048 px on the long edge;
    a contact sheet has at most 48 cells.
11. **Every command stays reachable.** `edit_apply` takes any list of core
    `Command`s (JSON, schema from `edit_schema`), applies them as one batch
    and history entry, optionally as a dry run, and reports the IDs created.
    Before it reaches the authority, every command (recursing into `Batch`)
    is checked against decision 12 and the resulting project against the
    desktop's placement rules (no overlapping clips on a track) unless
    `allow_overlap` is passed. A new core command is MCP-accessible without
    new server code; typed tools are conveniences over it. `edit_apply`
    takes core microseconds; typed tools take `TimeSpec`.
12. **Files stay inside allowed roots.** The server reads and writes only
    under its roots: `--root DIR` (repeatable) or `NEPTUNE_VIDEO_MCP_ROOTS`; with
    neither, the working directory, unless it is `/`, a drive root, the home
    folder or a system folder, in which case it refuses to start. Paths are
    resolved by canonicalizing the deepest existing ancestor and refusing
    `..`, before any store call; `outside_roots` does not reveal whether a
    path exists. Reads need a regular file. Media outside the roots is
    listed as such and never read: renders, frames and analysis refuse a
    project that uses it until the person adds a root. `AddAssets` and
    `RestoreProject` are refused when an asset path is outside the roots
    and not already in the project; imported media is probed by the server,
    its container format checked (playlists, concat lists and text formats
    are refused), never taken from the caller. Nothing existing is replaced
    without `overwrite: true`; project files go through `DocumentSession`
    (create refuses an existing file). Roots guard this server's tools;
    they are not a sandbox for a client that also has a shell.
13. **Loopback HTTP is authenticated.** `--http` accepts only a loopback IP
    literal. Every request (POST, GET stream, DELETE) needs
    `Authorization: Bearer <token>`, compared in constant time; the token
    is `NEPTUNE_VIDEO_MCP_TOKEN` or 256 random bits. rmcp's Host check stays on,
    Origin validation is enforced with no allowed browser origin, bodies
    are capped at 4 MiB. The address and token are written to a connection
    file in the per-user data folder, created owner-only (0600 on Unix),
    removed on exit, and never logged. The token is removed from the
    environment of child processes.
14. **Errors are structured.** A refused tool returns `isError` with
    structured content `{code, message, ...}`; codes reuse the protocol's
    (`stale_revision`, `invalid_edit`, `stale_context`, ...) and add
    `outside_roots`, `not_found`, `no_project`, `project_changed`,
    `project_busy`, `person_editing`, `desktop_busy`, `desktop_conflict`,
    `desktop_stale`, `desktop_closed`, `not_agent_entry`, `too_large`,
    `busy`, `job_failed`, `unavailable`, `indeterminate`. A transport
    failure after a mutation was sent is `indeterminate`: the agent reads
    `history` to learn whether it landed. JSON-RPC errors are kept for
    malformed requests.
15. **Changes are announced.** Resources `neptune-video://project`,
    `neptune-video://history` and `neptune-video://jobs` can be read and subscribed. While
    a client is subscribed, the server polls `document` (at most twice a
    second, answered without waking the window) and sends
    `notifications/resources/updated` when the change key moves:
    `(context_id, project_id, path, revision, file_version, dirty, saving,
    conflicted)`. Playhead and selection are not part of the key.
16. **Stdout is the protocol.** In stdio mode the server moves the
    process's standard output to a private descriptor for rmcp and points
    descriptor 1 at standard error, so anything a library prints cannot
    corrupt the stream. Logs go to standard error.

## Tools

Forty tools; several take an `op`.

| Namespace | Tools |
| --- | --- |
| project | `project_create`, `project_open`, `project_save`, `project_status`, `project_inspect`, `project_settings`, `project_close`, `desktop` (`list`, `attach`, `open`) |
| media | `media_import`, `media_inspect`, `media_remove`, `media_folder` |
| timeline | `track_edit`, `clip_add`, `clip_edit` (`move`, `trim`, `split`, `delete`, `speed`, `transform`, `link`, `unlink`, `group`, `ungroup`, `duplicate`), `marker_edit` |
| effects | `effects_catalog`, `effect_edit` (`add`, `update`, `remove`, `masks`, `blend`, `look`), `transition_set` |
| animation | `keyframes_set`, `retime_set` |
| text | `text_add`, `text_edit`, `captions` (`from_transcript`, `import`, `export`, `edit`) |
| audio | `audio_edit` (`volume`, `fades`, `track`, `effect`, `music`, `detach`, `normalize`) |
| transcription | `transcription_models` (`list`, `install`), `transcribe_start` |
| analysis | `frame_render`, `contact_sheet`, `scenes_start`, `loudness_start` |
| templates | `templates` (`list`, `preview`, `apply`) |
| render | `render_capabilities`, `render_export_start` |
| history | `history` (`list`, `undo`, `redo`), `checkpoint` (`create`, `list`, `restore`) |
| edit | `edit_apply`, `edit_schema` |
| jobs | `job` (`list`, `status`, `wait`, `cancel`) |

## Not claimed

- Forced alignment of arbitrary text to audio: words come from the speech
  model's own timing; `text_edit` re-times corrected text over those words
  with the reducer's `realign`, as `set_text` does.
- Starting tracking, stabilization, stem separation, beat finding,
  ducking or voiceover recording from MCP. Their results stay writable
  through `edit_apply` (`set_tracker`, `set_keyframes`, ...).
- Commands the core does not have: clip rename, marker move, folder rename
  or removal, relinking media.
- MCP Tasks (SEP-2663): jobs are ordinary tools because the clients in
  scope do not declare the extension.
- Re-verifying the path of an opened handle against the roots (a link
  swapped between check and open); FFmpeg protocol restriction on the
  desktop's own decode paths.
- macOS and Windows desktop acceptance beyond what is recorded in
  verification; GPU frame rendering where no adapter opens; whether a
  minimized window answers within the host's 5 s.

## Comments

2026-10-14: Decided on the owner's behalf: single active project per
server process; Desktop authority wins over Local; HTTP clients share the
process's workspace.

2026-10-14: Decided on the owner's behalf after review: an agent mutation
during the person's gesture is refused (`person_editing`) rather than
merged field by field into the draft, which keeps the one-undo-step rule
and needs no rebase; jobs return results instead of applying them, so no
job can apply against a project that changed; tools are merged to forty
with `op` fields to bound the `tools/list` payload.

2026-10-14: Implemented. Where the code departs from the decisions above,
or settles what they left open (decided on the owner's behalf):

- 39 tools, not 40: `audio_edit` has no `effect` op; audio effects are
  `effect_edit` on an audio clip, the same command the window uses.
- Output schemas are dropped from `tools/list` and the edit result is
  described once in the instructions. With them the list was 102 KB; the
  40 KiB budget of decision 1's review is now enforced by an end-to-end
  test (40.7 KB measured).
- The bearer token, when the server makes one, is two UUIDv4s: 244 random
  bits, not 256. Both come from the system's generator; no new dependency.
- The connection file is `mcp/PID.json` beside the settings in use (the
  per-user config folder, or `--desktop-settings`'s folder), so a test or
  verification run keeps its own.
- Media probes and the scene and details commands run FFmpeg with
  `-protocol_whitelist file`. A format sniff was not added: `supported()`
  already admits only media extensions, so playlists, concat lists and
  text formats never reach a probe.
- `project_open` on a file a Neptune Video window has open attaches to that
  window, whatever the window's unsaved state, before any local open is
  tried. A failed open keeps the project that was open.
- A recovery copy that differs from its file (an editor stopped with
  unsaved work) makes `project_open` refuse `recovery_available` without
  touching the copy; `recover: true` opens it. A copy equal to the file
  (this server saves after every edit) is taken over as before.
- A server stopped by SIGTERM, SIGHUP or Ctrl-C lets go of its project
  (lock and recovery copy) as when its client closes stdin; Claude Code
  and Codex stop their servers with a signal.
- No `mcp` cargo feature: `neptune-video-mcp` is always built, as `neptune-video-agent`
  is.
- Model installs rely on the download's SHA-256 check; two servers
  installing the same model at once are not serialised across processes.
- `job {op: cancel}` waits up to 2 s for the job to end and answers its
  final state.

2026-10-14: A review in four parts confirmed 29 defects, all fixed the same
day. The record is in [verification](../../docs/verification.md#native-mcp).
Decided on the owner's behalf while fixing:

- Work in no file (recovered work, or an edit whose save was refused) is
  never deleted. Leaving it is refused, and `discard` and shutdown keep it
  in the recovery copy. This overrides "a copy equal to the file is taken
  over" for any editor state that is dirty.
- Local exports belong to the server, not the project. A switch or a
  handover leaves them running, and `job` still reaches them.
- Probes refuse FFmpeg formats that read other files (`hls`, `concat`,
  `dash`, `imf`, text-as-video formats). This replaces the format sniff
  that decision 12 named: those formats are recognised by content
  whatever the extension. Media that `AddAssets` and `RestoreProject`
  bring in is probed again and must be a kind the editor imports.
- Times snap to the nearest frame start, as the window snaps, and
  drop-frame timecode is refused rather than read.
- Speech jobs run one at a time in a server.
- A call's frames are capped at 64 million pixels.

