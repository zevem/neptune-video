---
status: accepted
---

# The Agent tab edits through neptune-video-mcp started in its window

By default, the Agent tab's Ask mode is a live conversation with the Claude
Code or Codex command line the person has already signed in to. Its only
tools are those of `neptune-video-mcp`, the same binary and the same 39 tools
external MCP clients use, started by the window in that window. The agent
has what an outside MCP agent has: it starts on the window's project and
may make, open, save, close and export others. It narrows
[ADR 0004](0004-native-ai-produces-reviewed-edit-proposals.md) to the
review setting and caption translation, and keeps
[ADR 0010](0010-mcp-reaches-the-editor-through-the-agent-protocol.md)
unchanged. The work, its evidence and its captures are in
[the in-app agent folder](../../.scratch/in-app-agent/). The existing
contracts hold: every edit is a validated, reversible `Command`; the core
knows no GUI, codec or filesystem; nothing in an interactive frame waits for
a provider. This record was revised on 2026-10-15; see
[Revised 2026-10-15](#revised-2026-10-15).

**One command boundary and one history for the person and every agent.**
The window starts `neptune-video-mcp --window <its session file> --roots-file <its
folders file> --tag neptune-video-tab-<instance>`, where `<instance>` is the
window's agent session instance ID in 32 hex digits. The server attaches at once to that
window's authenticated `AgentHost` session, or at its first call if the
window is not answering yet, so each edit to the window's project is one
guarded `AgentRequest` answered by `editor_agent::handle`, the path
external MCP clients already take. Edits appear in the window at once, each
one undo step in the history the person uses; the person's Undo takes back
an agent edit like any other. Request IDs start with the window's own
tag, and a window claims only those, so it counts the turn's changes and
does not add the "Agent: …" rows it shows for outside clients. The tag is
per window because one shared `neptune-video-tab` let a second window, opened by
another window's agent with `desktop {op: open}`, take that agent's edits
for its own agent's and hide their rows (found in the 2026-10-15 native
runs). `person_editing` and `desktop_busy` refuse the
in-app agent as they refuse every client. An external MCP client can edit
the same window at the same time; the window serializes requests one at a
time, and typed tools that pass `expected_revision` are guarded by it. The
window finds the program in `NEPTUNE_VIDEO_MCP`, else beside its own executable,
and otherwise refuses the request with a message.

**"Made as it works" is the default; review stays a setting.** Settings ›
Agent has an Edits row, As it works or For review, and the panel's menu
offers "Edits: Made as it works / Proposed for review". Proposed for review
is ADR 0004's planner unchanged: one typed proposal from project metadata,
reviewed and applied as one undo step. Caption translation (Captions ›
Translate into…) always uses it, whichever setting is chosen. Command
mode, the local command language, is unchanged. Decided on the owner's
behalf (2026-10-14): new and existing settings start
in Made as it works (`AiSettings.review` defaults to false), because the
reviewed proposal cannot look at frames, transcribe or export.

**The window's project is the start, not a limit.** `project_create`,
`project_open`, `project_close` and `desktop {op: open}` work as for any
MCP client. A project no window shows is edited by the server itself and
saved after every edit; `desktop {op: open}` shows a project in a new
window; `desktop {op: attach}` with an `instance_id` moves to another
window, and with no window named brings the agent back to its own. While
the server follows its window, it attaches again when it has no project, so
a project the person opens later in an empty window is the one the agent
works on. It stops following when the agent closes, opens or launches a
project by choice, and follows again when it attaches back. While it
follows a window that is gone, tools answer `desktop_closed`, as does
`desktop {op: attach}` naming no window. `project_save {destination}` saves
a copy; a window writes it in its own time (`project_status` reports it as
`last_save`). `history {op: undo}` keeps
the MCP rule: it takes back only the agent's own entries recorded since it
last attached, so after leaving and coming back the earlier entries need
`allow_person: true` or the person's own Undo. The window counts its own
changes and, from tool results that report a change, the changes made in
other projects; the turn's note names both. The instructions tell the agent
that it starts on the window's project, to say so when it leaves that
project, and to come back to it unless asked otherwise.

**Folders are a live file and the folders the person adds.** The window
does not pass `--root`. It passes `--roots-file`, a JSON file
`{"roots": [DIR, …]}` beside its agent session file (owner-only on Unix,
replaced whole), which the server reads again whenever it changes. A thread
of the window's own writes it; the window never waits on the disk while
drawing, except for at most one second when a request is sent while no
conversation is going on and the folders changed since the file was last
written, so the server about to start reads the current folders. With the
folders unchanged it never waits. While the agent talks, the window
rewrites the file, without a wait, when the project's revision, its path
or the person's folders change. It names the project file's folder first,
then the folder of every asset, the folders the person added in Settings ›
Agent › Folders, then the person's Videos or Movies, Desktop,
Documents, Downloads, Music and Pictures folders. The server filters them with
`usable_roots`: it keeps that order, canonicalizes, keeps directories, drops
roots that are too wide (home, a drive or a system folder, by the server's
own rule) and drops a root inside another. With a roots file and no
`--root`, the working directory is not a root. Anything else is refused
with `outside_roots`, whose message says the person can add a folder in
Settings › Agent › Folders. That row is the in-app equivalent of `--root`:
"Add folder…" opens a native folder dialog, a folder too wide is refused
with a notice, and each added folder can be removed; a removed folder is
refused again in the same conversation. Decided on the owner's behalf
(2026-10-14): these roots, rather than the project folder alone, so the
agent can import media from the person's usual folders.

**Each provider runs with Neptune Video's server and nothing else.**

- Claude Code (verified 2.1.300) runs with `--tools ""`,
  `--strict-mcp-config`, a private MCP config (owner-only on Unix) naming
  one server,
  `--allowedTools "mcp__neptune-video__*"`, `--permission-mode dontAsk`,
  `--setting-sources ""`, `--disable-slash-commands` and
  `--no-session-persistence`. It does not run with `--safe-mode`: in 2.1.300
  that drops every MCP server, Neptune Video's included (probed). The planner keeps
  `--safe-mode`. A turn fails unless Claude's init line reports the server
  `connected`.
- Codex (verified codex-cli 0.162.1) runs as `codex app-server` with the
  planner's disabled features except `code_mode_host`, which Codex 0.162
  uses to call MCP tools; with it off, Codex answered that its tool host was
  disabled (probed). The shell tool, web search, project docs and notify
  stay off, the person's own MCP servers are disabled for the thread, the
  approval policy is never and the sandbox is read-only. An approval
  request, a tool of another server or a failed server start ends the turn
  as an error. Other Codex versions are unverified.

Neptune Video's system instructions, its own text with no person data, are visible
in the provider's process arguments; the person's words go on stdin, never
in arguments.

**Stop kills the provider.** Stop and Cancel end the provider process at
once and end the conversation; there is no graceful interrupt, and the next
request starts fresh. Edits
already made stay and undo takes them back. An export the agent already
queued keeps running in the window's render queue, where the person can
cancel it. The conversation also ends on "New conversation", when the
person opens or creates another project in the window, on a failure,
after 12 hours, and after 600 seconds of a request without a word from the
provider. Decided on the
owner's behalf
(2026-10-14): Stop takes effect at once, at the cost of the conversation.

**A conversation is its provider and model.** One worker thread per window
owns the provider process. A request continues the conversation while the
provider, executable, model, effort and the tool server's command and
arguments are the same. The folders are not among them: they live in the
roots file, so importing media from a new folder or adding a folder does
not end the conversation. Otherwise the next request starts a new
conversation and the panel says so: "A new conversation started, because
the agent's provider, model or effort changed. It remembers nothing of the
last one."

**More reaches the provider than in review.** The planner sends metadata
and no pixels. The live agent can call `frame_render`, `contact_sheet`,
transcription and `media_inspect`, so the project's editing data, rendered
frames, contact sheets, transcripts, level and loudness measurements, and
file names and paths inside the roots reach Anthropic or OpenAI under the
person's own sign-in and terms
([what reaches the provider](../native-ai.md#what-reaches-the-provider)).
Neptune Video does not upload source media files; the provider receives what tools
return. The agent cannot read files outside the roots, run shell commands,
browse the web or use other MCP servers.

Three alternatives were set aside:

- keeping proposals only, which cannot look at frames, transcribe or
  export, so the in-app agent would have less than an external MCP agent;
- a second, in-process tool layer for the in-app agent, which would
  duplicate `neptune-video-mcp` and drift from it;
- letting the agent edit through Claude's or Codex's built-in shell and
  file tools, which pass no command boundary and respect no roots.

## Consequences

- `editor-agent` gains `Conversation`, `ToolServer` and `TurnEvent`, and
  per-provider stream readers. Conversation output lines may be up to
  256 MiB; a conversation's stream has no total limit, because it is
  long-lived. At most 4096 unread events are kept; the oldest steps are
  dropped first, never a turn's end.
- `neptune-video-mcp` gains `--window FILE`, `--roots-file FILE` (read again
  whenever it changes, at most 256 KiB; a missing, too large or unreadable
  file names no folder) and `--tag TAG` (1 to 64 ASCII letters, digits and
  dashes; request IDs become `TAG-N`, default `mcp`). `project_status`
  lists the current roots.
- `AiSettings` gains `folders`, the folders the person added (empty by
  default).
- A new MCP tool reaches the Agent tab with no change to the window; the
  panel names its steps by the tool's name until `ai::step_words` has
  words for it.
- The agent cannot listen to sound; it judges audio by levels and loudness
  measurements.
- The first decision was verified on one macOS host with debug builds
  only. The 2026-10-15 revision is covered by tests (`tests/mcp.rs` with
  `NEPTUNE_VIDEO_TEST_DESKTOP=1`, and a unit test of the folders file in
  `src/ai.rs`), and its native runs were made on 2026-10-15 with release
  builds on one macOS host
  ([run notes](../../.scratch/in-app-agent/evidence/native-runs-2026-10-15.md),
  [In-app agent](../verification.md#in-app-agent)). Linux and Windows
  windows are unverified. On Windows the code refuses a Claude `.cmd`
  or `.bat` launcher for a conversation as it does for the planner, through
  the same executable resolution; that was not run there.
  Provider sign-in, model access and usage limits are the person's.
- Captures of the running window from the first decision, before the
  revision; the restart note shown there has the earlier wording:
  [steps as they happen](../../.scratch/in-app-agent/captures/01-claude-steps-live.png),
  [a follow-up in the same conversation](../../.scratch/in-app-agent/captures/02-claude-follow-up-same-conversation.png),
  [stopped mid-request](../../.scratch/in-app-agent/captures/03-stopped-mid-request.png),
  [the person undoes the agent's edit](../../.scratch/in-app-agent/captures/04-person-undoes-agent-edit.png),
  [Proposed for review](../../.scratch/in-app-agent/captures/05-proposed-for-review-mode.png),
  [Claude working](../../.scratch/in-app-agent/captures/06-final-claude-working.png)
  and [done](../../.scratch/in-app-agent/captures/07-final-claude-done.png),
  [Codex in a narrow window](../../.scratch/in-app-agent/captures/08-final-codex-narrow.png)
  and [its follow-up](../../.scratch/in-app-agent/captures/09-final-codex-follow-up.png),
  [the working row narrow](../../.scratch/in-app-agent/captures/10-final-narrow-working-row.png),
  [the turn's note](../../.scratch/in-app-agent/captures/11-final-codex-done-note.png),
  [the agent menu](../../.scratch/in-app-agent/captures/12-final-agent-menu.png)
  and [the restart note](../../.scratch/in-app-agent/captures/13-final-restart-note.png).

## Revised 2026-10-15

First decided (2026-10-14): the server was pinned to its window.
`project_create`, `project_open`, `project_close` and `desktop {op: open}`
were refused with `unavailable` before any path was looked at, and the
window passed its folders as `--root` arguments, so the roots were fixed
for a conversation and importing from a new folder started a new one.

Why it changed: the in-app agent must have the same capability as an
outside MCP agent, and a pinned server did not. It could not make, open or
close a project, and it could use no folder beyond the project's, its
assets' and the usual ones. The window's project is where the agent starts,
not a limit. Its folders are a live file the server reads again and the
folders the person adds in Settings › Agent › Folders, the equivalent of
`--root`, so they change without ending the conversation. The command
boundary, the shared history, the provider's isolation and Stop are
unchanged.
