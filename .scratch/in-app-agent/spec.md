# In-app agent edits through neptune-video-mcp

Status: ready-for-human
Progress: implemented; first round accepted natively on macOS with debug builds; the second round ([issue 06](issues/06-same-reach-as-an-outside-agent.md)) accepted natively on macOS with release builds on 2026-10-15; no Linux or Windows window run
Type: task

The Agent tab's Ask mode becomes a live conversation with the Claude Code
or Codex command line the person has signed in to. Its only tools are those
of `neptune-video-mcp`, the same binary and the same 39 tools external MCP clients
use, started in the window. Every edit of the window's project goes
through the window's authenticated agent session, appears at once and is
one undo step in the history the person uses. Like an outside agent, it
may also make, open and close projects of its own and come back.

This document is the binding design.
[ADR 0011](../../docs/adr/0011-the-agent-tab-edits-through-neptune-video-mcp-started-in-its-window.md)
records the decision; [the native MCP spec](../native-mcp/spec.md) and
[ADR 0010](../../docs/adr/0010-mcp-reaches-the-editor-through-the-agent-protocol.md)
stay as they are. A change to a decision is a dated comment below.

## Problem

Before this work, Ask ran Claude or Codex with every tool and MCP server
disabled. The provider returned one typed proposal from project metadata,
with no pixels; the person reviewed it and applied it as one undo step
([ADR 0004](../../docs/adr/0004-native-ai-produces-reviewed-edit-proposals.md),
[native AI](../../docs/native-ai.md)). That planner cannot look at frames,
transcribe, export or check its own work. An external MCP client connected
to the same window could do all of that, so the agent inside Neptune Video could do
less than one outside it.

## Decisions

1. **One tool layer.** The panel's agent uses `neptune-video-mcp`, not a second,
   in-process tool layer. A new MCP tool reaches the Agent tab with no
   change to the window; the panel names its step by the tool's name until
   `ai::step_words` has words for it.
2. **Started in its window.** The window starts `neptune-video-mcp --window <its
   session file> --tag neptune-video-tab-<instance> --roots-file <its session
   file with extension .roots> --desktop-settings <settings file> --models <models
   dir>`. The server attaches to that window's `AgentHost` session at once,
   or at the first call if the window is not answering yet, and goes back
   to it when `desktop {op: attach}` names no window. Each edit of the
   window's project is one guarded `AgentRequest` answered by
   `editor_agent::handle`, the path external MCP clients take.
3. **The same reach as an outside agent.** Decided on the owner's behalf
   (2026-10-15): `project_create`, `project_open`, `project_close` and
   `desktop {op: open}` work exactly as for any MCP client, because an
   agent locked to one project could do less than one outside Neptune Video. A
   project no window shows is edited by the server itself and saved after
   every edit; `desktop {op: open}` shows a project in a new window;
   `desktop {op: attach}` with an `instance_id` reaches another window, and
   with none brings the agent back to its own window. `project_save
   {destination}` saves a copy, which a window writes on its own time
   (`project_status.last_save`). While the server follows its window, a
   gone window answers `desktop_closed` and a server with no project
   attaches again on the next call. Once the agent closes, opens or
   launches a project by choice, the server stops following the window
   until it attaches back. `history {op: undo}` takes back only the
   agent's own entries since it last attached, as for every client.
4. **Made as it works is the default; review stays a setting.** Decided on
   the owner's behalf (2026-10-14): new and existing settings start in Made
   as it works (`AiSettings.review`, serde default false), because the
   reviewed proposal cannot look at frames, transcribe or export.
   Proposed for review is ADR 0004's planner unchanged. Caption translation
   always uses the planner. Command mode is unchanged.
5. **Roots are the project's and the person's media places.** Decided on
   the owner's behalf (2026-10-14): the project file's folder first, the
   folder of every asset, then the person's Videos or Movies, Desktop,
   Documents, Downloads, Music and Pictures folders, rather than the
   project folder alone, so the agent can import media from the person's
   usual folders. `usable_roots` keeps the order, canonicalizes, keeps
   directories, drops roots that are too wide (home, a drive or a system
   folder, by the server's own rule) and drops a root inside another.
   Decided on the owner's behalf (2026-10-15): the folders the person adds
   in Settings › Agent › Folders are roots too, and the roots are live.
   The window rewrites its roots file whenever the wanted folders change
   while the agent talks, and the server reads it again when it changes,
   so importing media from a new folder or adding a folder keeps the
   conversation.
6. **Stop kills the provider.** Decided on the owner's behalf
   (2026-10-14): Stop and Cancel end the provider process at once and end
   the conversation, at the cost of the conversation. There is no graceful
   interrupt.
7. **A conversation is its provider, model and tool server.** A request
   continues the conversation while the provider, executable, model,
   effort and the tool server's command and arguments are the same.
   Otherwise the next request starts fresh and the panel says so. The
   roots are not among the arguments (2026-10-15); they live in the roots
   file.
8. **The window tells the agent what the person sees.** Decided on the
   owner's behalf (2026-10-14): each request is prefixed with the playhead
   and the selected clips ("[In the window, the playhead is at X s (frame
   N) and the selected clips are … / no clips are selected.]"), since tools
   never take selection or playhead implicitly.
9. **The panel counts changes, not edits.** Decided on the owner's behalf
   (2026-10-14): the turn's note says "changes" and names both undo and
   redo, because the agent's own undo is counted too.

## Contract

- `editor-agent` owns a `Conversation`: one worker thread per window, which
  owns the provider process. `ask(config, server, prompt)` takes 1 to
  65,536 bytes without NUL and is refused as busy while a turn runs;
  `stop`, `end` and `poll` do no provider or filesystem I/O. Events are
  `Started`, `Said`, `Called`, `Returned`, and exactly one of `Finished`,
  `Failed` or `Cancelled`. At most 4096 unread events are kept, oldest
  steps first and never a turn's end. A provider process lives at most
  12 hours; 600 seconds without a word from it fails the turn.
  Conversation output lines may be up to 256 MiB; the stream and standard
  error have no total limit.
- The window finds the program in `NEPTUNE_VIDEO_MCP`, else `neptune-video-mcp` beside the
  running `neptune-video`, else the request fails with a message. Request IDs
  start with `neptune-video-tab-<instance>-`, `<instance>` being the window's agent
  session instance ID in 32 hex digits (`ai::window_tag`), and a window
  claims only its own (`ai::own_request`); for them the window counts the turn's changes
  and does not add the "Agent: …" rows it shows for outside clients. Tool
  results that report a change (`changed: true` with a label) are counted
  too; those beyond this window's are changes in other projects.
  `person_editing` and `desktop_busy` refuse the in-app agent as they
  refuse every client.
- The roots file is the window's agent session file with extension
  `.roots`: `{"roots": [DIR, …]}`, 0600 on Unix, written whole (a part
  file renamed over it) by a thread of its own, `neptune-video-agent-roots`. While
  the agent talks, the window compares the project's revision, its path
  and the person's folders each frame and writes the file again only when they change. It
  never waits on the disk while drawing, except up to one second for the
  write when a request is sent while no conversation is going on and the
  folders changed since the file was last written; with the folders
  unchanged it never waits, and while a conversation goes on the file
  follows changes without a wait. The file is removed
  when the window's agent is dropped. The window sends the folders raw;
  the server filters them with `usable_roots`, reads the file again when
  its modified time, length or, on Unix, inode changes, and takes a
  missing, unreadable or over-256 KiB file to name no folder. With a roots
  file and no `--root`, the working directory is not a root; `--root`
  folders stay and come first. `project_status.roots` lists the current
  roots. `outside_roots` ends "the person can add one with --root, or for
  Neptune Video's own agent in Settings › Agent › Folders".
- Settings › Agent lists the person's folders below "Edits" (name, the
  full path on hover, Remove) and a row "Folders" with "Add folder…", a
  native dialog titled "A folder the agent may use". A folder too wide is
  refused with "… is too wide for the agent; choose a folder inside it".
  `AiSettings.folders` is serde default empty.
- Claude Code (verified 2.1.300) runs `claude --print --verbose` with
  stream-json input and output, `--tools ""`, `--strict-mcp-config`, a
  private `neptune-video-tools.json` (0600 on Unix) naming one server `neptune-video`,
  `--allowedTools "mcp__neptune-video__*"`, `--permission-mode dontAsk`,
  `--setting-sources ""`, `--disable-slash-commands`,
  `--no-session-persistence`, `--max-turns 400` and Neptune Video's
  `INSTRUCTIONS` as the system prompt. It does not use `--safe-mode`,
  which in 2.1.300 drops every MCP server. A turn fails unless the init
  line reports `neptune-video` connected. The person's words go on stdin.
- Codex (verified codex-cli 0.162.1) runs `codex app-server` with the
  planner's disabled features except `code_mode_host`. The shell tool, web
  search, project docs and notify are off, the person's own MCP servers
  are disabled for the thread beside one server `neptune_video_window`
  (`startup_timeout_sec` 30, `tool_timeout_sec` 300,
  `default_tools_approval_mode` "approve"), the approval policy is never
  and the sandbox is read-only. An approval request, a tool of another
  server or a failed server start ends the turn as an error. Follow-ups
  are new turns on the same thread.
- The conversation ends on Stop or Cancel, New conversation, the person
  replacing the window's project, a failure, 12 hours or a request that goes
  600 seconds without a word from the provider. Changes already made stay.
  An export the agent queued in a window keeps running in that window's
  render queue. The agent's work on other projects does not end it.
- Disclosure: the planner sends metadata and no pixels. The person's
  words, with the window note, and the live agent's tool results reach the
  provider under the person's own sign-in and terms: the project's editing
  data, rendered frames, contact sheets, transcripts, level and loudness
  measurements, and file names and paths inside the roots. Neptune Video uploads no source media file.
  The agent cannot read outside the roots, run shell commands, browse the
  web or use other MCP servers.
- Panel texts: steps (running, done with the edit's label, refused with
  the reason, stopped), replies with a small Markdown subset (bold, code,
  headings, bullets), "Working… Ns · N changes" with Cancel, the hint
  "Reply, or ask for more", the turn's note "{Provider} · {model} · N
  changes; undo and redo take them back", followed by " · 1 change in
  another project" or " · N changes in other projects" when there are
  some ("no changes here" when only other projects changed), Stop's
  "Stopped. … · N changes in other projects" or "Stopped. Nothing was
  changed here.", and the restart note "A new conversation started,
  because the agent's provider, model or effort changed. It remembers
  nothing of the last one."

## Out of scope

- A graceful interrupt that keeps the conversation
  ([issue 05](issues/05-graceful-interrupt-and-per-request-roots.md)).
- Listening to sound. The agent judges audio by levels and loudness
  measurements only.
- Hosted HTTP credentials and local models. Sign-in, model access and
  usage limits are the person's own.
- Hiding Neptune Video's `INSTRUCTIONS` from the process list. They are Neptune Video's
  own text and hold no person data.
- Codex versions other than 0.162.1 and Claude Code versions other than
  2.1.300.

## Acceptance

- Library, MCP and provider tests pass on the host, including the
  `tests/mcp.rs` cases for the agent's own server
  ([issue 02](issues/02-pinned-neptune-video-mcp-for-the-window.md),
  [issue 06](issues/06-same-reach-as-an-outside-agent.md)).
- With each provider, in a running debug window: a multi-step request
  edits the timeline, looks at frames, exports and saves; a follow-up in
  the same conversation remembers the previous turn; the agent undoes only
  its own step when asked; the person's Undo takes back the agent's edit;
  Stop leaves edits and a queued export in place and no provider or server
  process behind; Proposed for review still returns a reviewed proposal
  and applies nothing by itself.
- With each provider, in a running window: the agent creates a project
  of its own, edits it, shows it with `desktop {op: open}` and comes back
  to the person's window; a folder added in Settings › Agent › Folders is
  usable in the same conversation; importing media from a new folder
  keeps the conversation. The panel's note counts changes in other
  projects.
- Captures of the wide and the narrow window (820x640 pt) in working,
  done, stopped, menu and restart states, and of Settings › Agent with
  folders.
- Linux and Windows: the same, not yet run
  ([issue 04](issues/04-linux-and-windows-acceptance.md)).

Recorded on macOS for the first round:
[native runs](evidence/native-runs-2026-10-14.md) and
[captures](captures/). Not every item was run with both providers: Codex did
not export or save in any run, Stop was run with Claude only, and Claude was
not run in the narrow window. The stopped state was captured in the wide
window only. The second round (leaving and coming back, live folders,
Settings folders, notes for other projects) is covered by tests and was
run with release builds on 2026-10-15
([native runs](evidence/native-runs-2026-10-15.md),
[In-app agent](../../docs/verification.md#in-app-agent) in verification):
with both providers, including Codex exporting and saving in a narrow
window and Stop with Codex. Those runs found defect 12, the shared
`neptune-video-tab` request tag, now fixed with a tag per window.

## Comments

2026-10-14: Implemented on base commit `1d25fc7` plus uncommitted work.
Probes changed two provider choices: Claude conversations run without
`--safe-mode`, which dropped Neptune Video's server in 2.1.300, and Codex keeps
`code_mode_host`, without which 0.162.1 answered that its tool host was
disabled. The planner keeps both as before.

2026-10-14: Native acceptance recorded on one macOS host with debug builds,
eleven runs with both providers
([evidence](evidence/native-runs-2026-10-14.md)). Fixed during the runs:
the working row truncated its count ("3 edi…"), the notes now say
"changes" and name undo and redo because the agent's own undo counts,
Codex could not reach its tools until `code_mode_host` was kept on, and
the restart note was added. Linux, Windows and release builds are unverified.

2026-10-15: Decisions 2, 3, 5 and 7 changed. A goal check found that a
server pinned to its window, refusing `project_create`, `project_open`,
`project_close` and `desktop {op: open}`, with roots fixed for the
conversation, could do less than an outside MCP agent. Decided on the
owner's behalf (2026-10-15): the server starts in its window and may leave
it and come back; the roots are live through `--roots-file`, and the person
adds folders in Settings › Agent › Folders. Roots no longer restart the
conversation, and the restart note names only the provider, model or
effort. ADR 0011's file is now
[0011-the-agent-tab-edits-through-neptune-video-mcp-started-in-its-window.md](../../docs/adr/0011-the-agent-tab-edits-through-neptune-video-mcp-started-in-its-window.md).
`tests/mcp.rs` has 12 cases, all passing on macOS 26.5.2 arm64, and a new
`src/ai.rs` unit test covers the roots file; native runs followed the
same day ([issue 06](issues/06-same-reach-as-an-outside-agent.md)).
