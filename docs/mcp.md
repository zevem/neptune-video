# Aster for MCP clients

`aster-mcp` lets an AI agent in any MCP client (Claude Code, Codex and others)
create, inspect, edit, analyse, preview and export Aster projects. It edits
with the same validated, undoable commands as the window, so a person can open
the project an agent made, or watch and continue the one an agent is editing
in a running Aster window. Aster's own Agent tab edits through it too by
default, starting in its window.

The server is a native binary built beside `aster` and `aster-agent`
(`cargo build --release --locked --bins`). It uses the official Rust MCP SDK,
`rmcp` 3.5, over stdio or Streamable HTTP. [ADR 0010](adr/0010-mcp-reaches-the-editor-through-the-agent-protocol.md)
records the design; [verification](verification.md#native-mcp) records what
was verified and on which host.

## Setting it up

The server reads and writes only inside its **roots**: the folders named with
`--root` (repeatable) or `ASTER_MCP_ROOTS` (separated as `PATH` is). Without
either it uses the folder it starts in, unless that is a home, drive or system
folder, which it refuses. Give it the folder that holds the footage and where
projects and exports should go. `--roots-file` names more folders in a file
the server reads again whenever it changes.

### Claude Code

```bash
claude mcp add --transport stdio aster -- /path/to/aster-mcp --root ~/Videos/promo
```

Or in a project's `.mcp.json`:

```json
{
  "mcpServers": {
    "aster": {
      "type": "stdio",
      "command": "/path/to/aster-mcp",
      "args": ["--root", "${HOME}/Videos/promo"]
    }
  }
}
```

The tools appear as `mcp__aster__project_create` and so on. A headless run
can allow them all: `claude -p "…" --allowedTools "mcp__aster__*"`.

### Codex

```bash
codex mcp add aster -- /path/to/aster-mcp --root ~/Videos/promo
```

Or in `~/.codex/config.toml`:

```toml
[mcp_servers.aster]
command = "/path/to/aster-mcp"
args = ["--root", "/Users/me/Videos/promo"]
# Exports and transcription answer within 50 s; a longer timeout leaves room.
tool_timeout_sec = 120
# `codex exec` never asks: without this its editing calls are refused.
default_tools_approval_mode = "approve"
```

### Other clients over Streamable HTTP

```bash
ASTER_MCP_TOKEN=$(openssl rand -hex 32) aster-mcp --root ~/Videos/promo --http
```

The server listens on `http://127.0.0.1:47420/mcp` by default
(`--http 127.0.0.1:PORT`; port 0 picks a free one). It refuses any address
that is not a loopback address. Every request must carry
`Authorization: Bearer TOKEN`. Without `ASTER_MCP_TOKEN` the server makes a
random 244-bit token. The address and token are written to a connection
file, `mcp/PID.json` beside the settings, that only its owner can read. The
file is removed when the server stops (Ctrl-C, SIGTERM or SIGHUP).

```bash
claude mcp add --transport http aster http://127.0.0.1:47420/mcp \
  --header "Authorization: Bearer $ASTER_MCP_TOKEN"
codex mcp add aster --url http://127.0.0.1:47420/mcp --bearer-token-env-var ASTER_MCP_TOKEN
```

All HTTP clients of one server share its project. Requests whose `Host` is
not the loopback get 403, and so do requests that carry a browser `Origin`.
Bodies over 4 MiB get 413. The token is never logged, and the server removes
it from its own environment so that FFmpeg and windows it starts never see it.

### Options

| Option | Meaning |
| --- | --- |
| `--root DIR` | A folder the server may use; repeat for more |
| `--http [ADDR]` | Serve Streamable HTTP instead of stdio |
| `--desktop-settings PATH` | The settings file of the windows to work with; sessions, presets, models and connection files are beside it |
| `--sessions DIR` | Where windows publish their live sessions |
| `--models DIR` | Speech models (also `ASTER_MODELS_DIR`) |
| `--aster PATH` | The window program `desktop {op: open}` starts; the `aster` beside the server by default |
| `--window FILE` | Start in the window whose session descriptor is FILE: attach to it at once, and go back to it when `desktop {op: attach}` names no window. See [Aster's Agent tab](#asters-agent-tab) |
| `--roots-file FILE` | A JSON file, `{"roots": [DIR, …]}`, naming more folders the server may use; see below |
| `--tag TAG` | Begin request ids with TAG, as `TAG-1`, `TAG-2` and so on, so a window can tell this server's requests apart; 1–64 ASCII letters, digits and dashes; `mcp` by default |

A roots file is read again whenever it changes, so a folder added to it can
be used in the same session and a folder taken out is refused again. Replace
the file whole rather than writing it in place. A file over 256 KiB, missing,
unreadable or not understood names no folder. Each folder it names must exist
and must not be too wide (a home, drive or system folder) or inside another
folder the file names; any other is left out. `--root` folders still apply and come first. With
a roots file and no `--root`, the working directory is not a root, and while
there are no roots every path is refused with `outside_roots`.
`project_status` lists the current roots.

Over stdio, standard output carries only the protocol. Anything else that
would print there goes to standard error.

## How the server edits

**One project per server.** `project_create` and `project_open` choose it
(a server started with `--window` starts on its window's project);
`project_status` says where it is:

- **local**: the server edits the file itself. It holds the file's lock, saves
  after every edit and keeps a recovery copy, exactly as `aster-agent
  --project` does. A project made without a path stays in memory until
  `project_save {destination}`.
- **desktop**: the project is open in an Aster window and the server edits it
  there. Edits appear in the window at once, are in the window's undo
  history, and are saved when the window saves (`project_save`).
  `project_open` on a file a window has open edits it in that window,
  including the person's unsaved work. `desktop {op: list|attach}` finds and
  chooses windows, and `desktop {op: open}` opens the project in a new
  window. A window the server started is the person's: the server never
  closes it.

If a window opens the file while the server is editing it locally, the
server hands over to the window when both have the same version. If they
differ it refuses with `desktop_stale` or `desktop_conflict`; it never has
two writers. While the person is in the middle of a gesture or a dialog the
window refuses with `person_editing` or `desktop_busy`: wait and try again.

**Every edit is one undoable step.** Typed tools build the same commands the
window builds, from the editor's own rules (placement, trimming, fades,
transitions, title templates, captions), and every result names what it did:

```json
{"project_id":"…","context_id":"…","revision":12,"authority":"local",
 "label":"Add clip","changed":true,"created":["<new clip id>"]}
```

- `expected_revision` refuses an edit with `stale_revision` when the project
  changed since the agent looked.
- `dry_run: true` validates an edit and applies nothing.
- `allow_overlap: true` lets clips overlap on a track, which the window never
  does.
- `history {op: undo}` takes back only the agent's own last edit and refuses
  `not_agent_entry` for anyone else's unless `allow_person: true`.
- `checkpoint` keeps named copies beside the project file and restores one as
  a single undoable edit.
- IDs are stable: an ID the agent captured keeps naming the same asset, track
  or clip, whatever is selected in the window.

**Times are frame-accurate.** Typed tools take a time as exactly one of
`{"frame": 120}`, `{"seconds": 4.0}`, `{"us": 4000000}` or
`{"timecode": "00:00:04:00"}` (non-drop, at the project's rate). A time is
snapped to the nearest frame start, as the window snaps, and times are reported in all
four forms. `edit_apply` takes core commands, whose times are integer
microseconds.

## Tools

| Area | Tools |
| --- | --- |
| Project | `project_status`, `project_create`, `project_open`, `project_save`, `project_inspect` (summary, timeline, media, full), `project_settings`, `project_close`, `desktop` |
| Media | `media_import` (files and folders, deduplicated), `media_inspect` (with every stream ffprobe reports), `media_remove`, `media_folder` |
| Timeline | `track_edit`, `clip_add`, `clip_edit` (move, trim, split, delete with ripple, speed, transform, link, group, duplicate), `marker_edit` |
| Effects | `effects_catalog`, `effect_edit` (effects, audio presets, masks, blend modes, looks), `transition_set`, `retime_set` (ramps, reverse, freeze, pitch) |
| Animation | `keyframes_set` (any animatable clip, effect or mask parameter, with easing) |
| Text | `text_add`, `text_edit` (style, colour, entrance and exit animations), `captions` (from a transcript, `.srt`/`.vtt` import and export, add, retime, split, merge, retype, language, translate) |
| Templates | `templates` (list, preview as a picture, apply a look), then `text_add {template_id}` |
| Audio | `audio_edit` (volume, fades, track mix, background music, detach, noise reduction, loudness normalization) |
| Transcription | `transcription_models` (list, install), `transcribe_start` (Whisper words with timeline times) |
| Analysis | `frame_render`, `contact_sheet`, `scenes_start`, `loudness_start`, `media_inspect` |
| Render | `render_capabilities`, `render_export_start` (presets, settings, quick preview, a range) |
| Jobs | `job` (list, status, wait with progress, cancel; pause, resume, retry and remove for exports) |
| History | `history`, `checkpoint` |
| Everything else | `edit_apply` (any core command, in an all-or-nothing batch), `edit_schema` |

Resources: `aster://project`, `aster://history` and `aster://jobs`. A
subscribed client gets `notifications/resources/updated` when they change.

**Pictures.** `frame_render` draws the timeline, or one media file, at given
times through the export's compositor: effects, masks, titles, captions and
transitions included. It returns PNG or JPEG images, at most 48 frames of at
most 2048 pixels a side and 64 million pixels in all, and can also save
them inside the roots. `contact_sheet` draws up to 48 evenly spaced frames
as one picture of at most 2048 pixels a side.

**Jobs.** Transcription, model installs, scene detection, loudness and
exports run as jobs. A start tool answers at once with a `job_id`. `job {op:
wait}` waits at most 50 s and sends progress notifications when the client
asked for them. Jobs never edit: apply their results with `captions {op:
from_transcript, job_id}` or `audio_edit {op: normalize, job_id}`. An export
from a local project runs in the server, through project switches, and
stops with it. An export from a
window's project runs in the window's render queue, where the person sees
it.

**Everything else.** `edit_apply` accepts every editing command the editor
has: `edit_schema` lists them and gives one command's JSON schema. A new
editor feature is reachable here as soon as it is a command, before it has
a typed tool.

## A session

1. `project_create {path: "…/promo.aster", width: 1280, height: 720, frame_rate: 30}`
2. `media_import {paths: ["…/footage"]}`, then `contact_sheet {asset_id}` to
   see what each file holds and `scenes_start` to find its cuts.
3. `clip_add`, `clip_edit {op: trim|split|move}`, `transition_set`,
   `effect_edit`, `keyframes_set`.
4. `transcribe_start` (after `transcription_models {op: install, model:
   "base"}`), then `captions {op: from_transcript, job_id}`.
5. `text_add`, `audio_edit {op: music, asset_id}`, `loudness_start`, then
   `audio_edit {op: normalize, job_id}`.
6. `frame_render` at the moments that matter, fix and look again;
   `render_export_start {preview: true}` for a quick, small file.
7. `render_export_start {destination, preset}`, then `job {op: wait}` until
   it ends.
8. The person opens the `.aster` file in Aster, or watches it all happen in
   the window when the project was opened there.

## Aster's Agent tab

The Agent tab's live agent ("Edits: Made as it works") is this same server.
The window holds a conversation with the installed Claude Code or Codex CLI
whose only tools are this server's: no shell, no web and none of the
person's own MCP servers. The agent's edits enter the window's agent session
as an outside client's do, and each is one step in the window's undo
history. [ADR 0011](adr/0011-the-agent-tab-edits-through-aster-mcp-started-in-its-window.md)
records the design.

The window starts the program `ASTER_MCP` names, or the `aster-mcp` beside
its own `aster`, as:

```bash
aster-mcp --window SESSION.json --tag aster-tab-INSTANCE \
  --roots-file SESSION.roots --desktop-settings SETTINGS --models DIR
```

`SESSION.json` is the window's own session descriptor and `INSTANCE` its
agent session's instance id, 32 hex digits. `SESSION.roots`, beside
it, is a roots file only its owner can read. The window writes it from a
thread of its own and rewrites it whenever the folders it wants change while
the agent talks: the project file's folder, the folder of every asset, the
person's Videos (Movies), Desktop, Documents, Downloads, Music and Pictures
folders, and the folders the person added in Settings › Agent › Folders. The
server leaves out a folder that is gone, too wide or inside another one the
file names, and refuses a file anywhere else with `outside_roots`. The file is removed when
the window closes. Importing media from a new folder or adding a folder does
not restart the conversation; a new provider, model or effort does.

A server started with `--window` starts in that window and can leave it, as
any MCP client can:

- It attaches to the window's project at once, or at its first call when
  the window is not answering yet. While it follows its window and has no
  project, it attaches again on its next call.
- `project_create`, `project_open`, `project_close` and `desktop {op: open}`
  work as for any client. A project no window shows is edited by the server
  itself and saved after every edit; `desktop {op: open}` shows it in a new
  window, and `desktop {op: attach}` with an `instance_id` moves to another
  window. Creating, opening or closing a project, or opening a window,
  stops it following its window.
- `desktop {op: attach}` naming no window brings it back to its own window,
  and it follows the window again. `history {op: undo}` takes back only the
  edits it made since it last attached, so after leaving and coming back
  its earlier edits need `allow_person: true` or the person's own undo.
- `project_save {destination}` saves a copy, which a window writes on its
  own time; `project_status` reports `last_save`.
- While it follows its window and the window is gone, it refuses with
  `desktop_closed`, as does `desktop {op: attach}` naming no window. A
  window that stops answering after an edit was sent gives `indeterminate`,
  as for any client.
- The window ends the conversation when the person opens or makes another
  project in it.

With `--tag aster-tab-INSTANCE` its request ids are `aster-tab-INSTANCE-1`,
`aster-tab-INSTANCE-2` and so on; an outside server's are `mcp-N` unless it
is given another tag. Each window claims only requests with its own tag, so
another window's agent is an outside client to it. The
window counts its own agent's edits for the turn, and the changes the agent
reports in other projects, and shows its "Agent: …" rows only for outside
clients. `person_editing` and `desktop_busy` refuse the Agent tab as they
refuse any client.

External MCP clients and the Agent tab can edit the same window at the same
time. The window takes their requests one at a time, and a typed tool given
`expected_revision` refuses with `stale_revision` when the other changed
the project first.

The Agent tab was verified on one macOS host in debug builds with Claude
Code 2.1.300 and codex-cli 0.162.1 while its server was locked to its window.
Starting in the window and leaving it, and the roots file, are covered by
`tests/mcp.rs` and by unit tests; native runs of them are recorded in
[verification](verification.md#in-app-agent) once made. Codex 0.162.1 calls MCP
tools only with its `code_mode_host` feature on, so the window leaves it
on; other Codex versions are unverified. Linux, Windows and release builds
are unverified. See [verification](verification.md#in-app-agent).

## Safety

- Paths are resolved through symbolic links and must stay inside the roots.
  A path containing `..` is refused. The `outside_roots` message lists the
  roots and says that the person can add one with `--root`, or for Aster's
  own agent in Settings › Agent › Folders. A refusal does not say whether
  the file exists.
- Nothing is replaced without `overwrite: true`: projects, exports, pictures
  and caption files alike. Exports are written to a part file and renamed
  when they are complete.
- Media is probed with FFmpeg restricted to local files
  (`-protocol_whitelist file`), so a file cannot make it fetch a URL.
  Formats that read other files (playlists, concat lists, manifests) are
  never media, whatever their name. Media a raw command adds is probed
  again, and a link inside an imported folder that leads outside the
  roots is not read.
- Work that is in no file (recovered work, or an edit whose save was
  refused) is never let go silently. `project_close`, `project_open` and
  `project_create` refuse until it is saved. `discard: true` and a
  stopping server leave it in the recovery copy.
- Every edit is validated before it is applied. A refused edit or batch
  leaves the project exactly as it was.
- A recovery copy left by an editor that stopped is never replaced
  silently. `project_open` refuses with `recovery_available` until the agent
  opens it with `recover: true` or the person deals with it.
- Errors are structured: `{code, message, revision?, detail?}`. The codes
  are `outside_roots`, `not_found`, `invalid`, `stale_revision`, `no_project`,
  `project_changed`, `project_busy`, `recovery_available`, `person_editing`,
  `desktop_busy`, `desktop_conflict`, `desktop_stale`, `desktop_closed`,
  `not_agent_entry`, `destination_exists`, `too_large`, `busy`, `job_failed`,
  `unavailable`, `indeterminate` (a window stopped answering mid-edit: look
  before retrying) and the editor's own validation codes.

## Limits

These are not offered, or not verified:

- Forced alignment of a script to speech (transcription times every word it
  hears, but a given text is not aligned to the audio).
- Starting motion tracking, stabilization, stem separation, beat detection,
  ducking or voiceover recording. Their results already in a project are kept
  and rendered.
- Renaming clips, moving markers, renaming or removing library folders, and
  relinking media.
- MCP Tasks, the protocol's own long-running operations. Jobs and `job` stand
  in for them.
- Whether a minimized or hidden window answers on every platform, and
  acceptance on Linux and Windows. See [verification](verification.md#native-mcp).
