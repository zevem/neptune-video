# Native AI editing

The Agent tab's Ask mode works with the person's installed Claude Code or
Codex CLI in one of two ways. The **Edits** row in Preferences › Agent
[As it works | For review] and the model menu in the composer (Edits: Made as
it works / Proposed for review) choose between them. The setting is
`AiSettings.review`, false by default.

- **Made as it works**, the default. A conversation with the provider whose
  only tools are Neptune Video's MCP server, `neptune-video-mcp`: the same program and the
  same 39 tools that outside MCP clients use, with nothing refused. The
  server starts in this window, so an edit to the window's project enters
  its authenticated agent session and goes through the same command
  boundary, validation and undo history as the person's edits. Each such
  edit appears in the window at once as one undo step. Like any MCP agent,
  it may also create, open, save and close other projects.
  See [Made as it works](#made-as-it-works) and
  [ADR 0011](adr/0011-the-agent-tab-edits-through-neptune-video-mcp-started-in-its-window.md).
- **Proposed for review**. Neptune Video's Rust backend invokes the CLI with every
  tool and MCP server disabled to propose typed edits to the captured
  project. The person reviews the proposal before applying it through the
  same reducer and history as manual editing
  ([ADR 0004](adr/0004-native-ai-produces-reviewed-edit-proposals.md)).
  Caption translation always uses this path, whatever the setting.

Command mode, the local command language, is unchanged.

The two modes disclose different things to the provider. The planner behind
**Proposed for review** receives project metadata only, no pixels. The live
agent can render frames, contact sheets and transcripts and send them to the
provider; [what reaches the provider](#what-reaches-the-provider) lists it.

For proposals, actual provider completion and Linux desktop interaction,
including unchanged proposals, are verified below at named checkpoints.
Source/build, desktop editing and provider acceptance retain their recorded
identities; native target/provider and broader production gates remain. The
live agent was built and run on one macOS host, first in debug builds; its
second round, which lets it leave its window and read its folders live, was
run there on 2026-10-15 with release builds
([run notes](../.scratch/in-app-agent/evidence/native-runs-2026-10-15.md),
[verification](verification.md#in-app-agent), [evidence](#evidence)).
Linux and Windows windows are not verified for it.

## Made as it works

Decided on the owner's behalf (2026-10-14): the in-app agent is an MCP agent.
Ask defaults to it, and the earlier proposal path stays as the **Proposed for
review** setting. The decision is recorded in
[ADR 0011](adr/0011-the-agent-tab-edits-through-neptune-video-mcp-started-in-its-window.md);
the feature folder is [.scratch/in-app-agent](../.scratch/in-app-agent/).

### Launching the agent

The window runs the program the `NEPTUNE_VIDEO_MCP` environment variable names, else
the `neptune-video-mcp` beside the running `neptune-video` executable, found through its
canonical path. If `NEPTUNE_VIDEO_MCP` names something that is not a file, or no
`neptune-video-mcp` is beside `neptune-video`, the request fails with a message in the
panel. Its arguments are:

```text
--window <this window's session file> --tag neptune-video-tab-<instance>
--roots-file <session file with extension .roots>
--desktop-settings <settings file> --models <models dir>
```

`--window` starts the server in the window whose session descriptor is that
file: it attaches at once, or at its first call if the window is not
answering yet, and goes back to that window when `desktop {op: attach}`
names no window. It does not lock the server to the window. `--tag` names
its request IDs `neptune-video-tab-<instance>-N`, where `<instance>` is the window's
agent session instance ID in 32 hex digits, so the window can tell its own
agent's requests from an outside client's and from another window's agent.
With one shared tag, a window that an agent opened with `desktop {op: open}`
took that agent's edits for its own agent's and hid their "Agent: …" rows
(found in the 2026-10-15 native runs). `--roots-file` names the folders the
agent may use; see [Folders](#folders). The [MCP guide](mcp.md) describes
all three options.

The provider is launched once per conversation, and the person's words reach
it on stdin, never in its arguments.

**Claude Code** (verified with 2.1.300):

```text
claude --print --verbose --input-format stream-json --output-format stream-json
  --tools "" --strict-mcp-config --mcp-config <neptune-video-tools.json>
  --allowedTools "mcp__neptune-video__*" --permission-mode dontAsk
  --setting-sources "" --disable-slash-commands --no-session-persistence
  --max-turns 400 --system-prompt <instructions> [--model …] [--effort …]
```

`neptune-video-tools.json` is a private file, created with mode 0600 on Unix in the
conversation's private working directory, that names one stdio server,
`neptune-video`.
The server's path and arguments are therefore not on Claude's command line. Built-in
tools are off. The stream's first `init` line must report the `neptune-video` server
as `connected`, or the request fails. A conversation does not use
`--safe-mode`: in 2.1.300 it drops every MCP server, Neptune Video's included
(probed). The planner still uses it.

**Codex** (verified with codex-cli 0.162.1): `codex app-server` with the
planner's disabled features except `code_mode_host`, web search disabled,
`project_doc_max_bytes=0`, empty project-doc fallback names and `notify=[]`.
`thread/start` sets every MCP server the person's configuration names to
`enabled: false`, by name only, and adds one server, `neptune_video_window`, with
`startup_timeout_sec` 30, `tool_timeout_sec` 300 and
`default_tools_approval_mode` `approve`. The thread uses approval policy
`never`, a read-only sandbox without network access, the shell tool off and
Neptune Video's instructions as both base and developer instructions. Codex 0.162.1
calls MCP tools through its code-mode host: with `code_mode_host` off it
answered "I couldn't call neptune_video_window.project_status because the tool host
is disabled". Any request from the server to the client (an approval), a
call to another server's tool or a failed server start ends the request as an
error. Follow-ups are new turns on the same thread.

The instructions are Neptune Video's own text and carry no person data. They say that
the neptune-video tools are the agent's only tools; that every edit appears at once
and is one undo step shared with the person; to look before changing
(`project_inspect`, `media_inspect`, `frame_render`, `contact_sheet`); to pass
`expected_revision`; to wait and retry on `person_editing` or `desktop_busy`;
to check work by looking at frames; that long work runs as jobs; that it
starts on the project open in the window and, like any agent with these
tools, may also create, open, save and close projects; that a project no
window shows is edited by the server itself and saved after every edit;
that `desktop {op: open}` shows a project in a new window and
`desktop {op: attach}` naming no window brings it back to the person's
window; to say so when it leaves the person's project, and to come back
unless asked otherwise; that `project_save {destination}` saves a copy, which
a window writes on its own time, shown by `project_status.last_save`; that
only folders the person allowed can be used, and the person can add one in
Settings › Agent; to undo only its own edits unless asked; to ask instead of
guessing; and to answer briefly in the person's language.

Each request is prefixed with the window's state at the moment of asking:
"[In the window, the playhead is at X s (frame N) and the selected clips are
… / no clips are selected.]" The playhead and selection are not targets the
agent is bound to; it reads the project through its tools.

### Conversation runtime

[conversation.rs](../crates/editor-agent/src/conversation.rs) owns the
provider process on one worker thread per window. It never edits: the
provider edits through the server, and the runtime reports what it said and
did.

| Interface | Contract |
| --- | --- |
| `ToolServer` | The MCP server's program and arguments, passed to the provider as text. |
| `Conversation::ask` | Sends one request of 1–65,536 bytes without NUL; a second request while one is active returns `Busy`. Starts a new conversation when none is running or its party changed. |
| `Conversation::stop` | Kills the provider process and ends the conversation. |
| `Conversation::end` | Ends the conversation; the next request starts a new one. |
| `Conversation::poll` | Returns `TurnEvent`s without waiting: `Started {fresh}` first, then `Said`, `Called {tool, op}` and `Returned {ok, detail}`, and exactly one of `Finished {model}`, `Failed` or `Cancelled` last. |
| `INSTRUCTIONS` | Neptune Video's system text for the provider, above. |

A conversation continues while the provider, executable, model, effort and
the server's command and arguments stay the same. A change to any of them
makes the next request start a new conversation, and the panel says "A new
conversation started, because the agent's provider, model or effort changed.
It remembers nothing of the last one." The folders the agent may use are not
in the arguments, so a change to them does not start a new conversation.

| Resource | Current bound |
| --- | --- |
| Request | 1–65,536 bytes, no NUL |
| Active requests per window | 1 |
| Provider process lifetime | 12 hours |
| Silence from the provider during a request | 600 s, then the request fails |
| Unread events | 4096; the oldest steps are dropped first, never a request's end |
| Claude turns per request | 400 (`--max-turns`) |
| Process line / total stdout / stderr | 256 MiB / not bounded in total / not bounded in total; a conversation is long-lived and one line holds one tool result |
| Codex tool server start / tool call | 30 s / 300 s |

### The panel

A request shows its steps as the agent works. A step has words for its tool
(for example "Looked at frames", "Trimmed clips", "Undid an edit") and is
running (a spinner), done (✓ with the edit's history label), refused (with
the reason) or stopped. The agent's replies are shown with a small Markdown
subset: **bold**, `code`, `#` headings and `-` or `*` bullets. While it works,
the row reads "Working… Ns · N changes" ("Working… Ns" before the first
change) with **Cancel**. While a conversation
continues, the field's hint is "Reply, or ask for more".

After each request a note names the provider, the model and the changes:
"Claude · claude-opus-5-5 · 3 changes; undo and redo take them back"
("1 change, which undo or redo takes back"; "no changes"). The panel counts
changes rather than edits because the agent's own undo is one too. It counts
this window's changes by their `neptune-video-tab-<instance>-N` requests, and the
tool results that report a change; any more than this window saw are changes
in other projects, which this window's undo does not reach. The note then ends
" · 1 change in another project" or " · N changes in other projects", and
when none were made here its first part reads "Claude · claude-opus-5-5 ·
no changes here". The window does not add the "Agent: …" rows it shows for
outside MCP clients for its own agent's requests; those rows stay for
outside clients.

The panel's introduction, under "Ask the agent", says what the agent may
use: "It sees the frames, transcripts and sound levels it asks for, and the
files in this project's folders, in your Movies, Music, Pictures, Desktop,
Documents and Downloads folders and in folders you add in Preferences. It can
also make, open and export other projects. Follow-up requests continue the
conversation."

The model menu, **Model: …** at the composer's foot, has **Answered by**,
**Effort**, **Edits** (Made as it works / Proposed for review) and **Model…**,
which opens Preferences › Agent. **New conversation** is in the session row's
More menu (⋯), next to **Copy the connect command**, while a conversation goes
on.

### What reaches the provider

The planner's metadata-only rule in
[planning context and disclosure](#planning-context-and-disclosure) applies
to **Proposed for review** only. The live agent's provider receives the
person's words, prefixed with the window's playhead and selection note, and
whatever the tools the agent calls return: the project's editing data,
rendered frames and contact sheets as PNG or JPEG images, transcripts, level
and loudness measurements, and file names and paths inside the folders it
may use. This includes other projects it opens or creates. It goes to
Anthropic or OpenAI under the person's own CLI sign-in and terms. Neptune Video does
not upload source media files; the provider receives tool results only.

The agent cannot read files outside the folders it may use, run shell
commands, browse the web or use other MCP servers. Neptune Video's instructions are
visible in the provider's process arguments to other users of the computer
(`ps`); they are Neptune Video's own text. The person's words, the server's path and
the folders are not in the arguments.

### Folders

The window wants the server to use these folders, in order: the project
file's folder, the parent folder of every asset, the folders the person added
in Preferences › Agent › Folders, and the person's Videos or Movies, Desktop,
Documents, Downloads, Music and Pictures folders. It writes them to a roots
file beside its agent session file, `{"roots": [DIR, …]}`, with mode 0600 on
Unix, written whole (a part file renamed over it) by a thread of its own.
While the agent talks, the window checks each frame whether the project's
revision, its path or the person's folders changed, and only then works the
list out again and has the file rewritten. It never waits on the disk while
drawing, except when a request is sent while no conversation is going on,
so a server is about to start and read the file, and the folders changed
since the file was last written: then it waits up to one second for the
thread to finish. When the folders are unchanged it never waits, and while
a conversation goes on the file follows changes without a wait. The file is
removed when the window closes.

The server reads the file again whenever it changes, so a folder added or
taken out applies to the next tool call in the same conversation. It filters
the list as it filters `--root`: canonical paths, existing directories only,
no folder too wide (home, drive or system folders) and no folder inside
another. With a roots file and no `--root`, the working directory is not a
folder it may use. A missing, unreadable or over-256-KiB file names no folder.
`project_status.roots` lists the current folders. A path outside them is
refused `outside_roots`, whose message ends "the person can add one with
--root, or for Neptune Video's own agent in Settings › Agent › Folders".

Importing media from a new folder adds that folder to the file, and the
conversation continues.

**Preferences › Agent › Folders.** Below **Edits**, each added folder has a row
with its name (the full path on hover) and **Remove**. The **Folders** row's
**Add folder…** opens a native folder dialog titled "A folder the agent may
use". A folder that is too wide is refused with a notice: "… is too wide for
the agent; choose a folder inside it". The note under the Agent settings
names the folders the agent may use. The setting is `AiSettings.folders`,
empty by default. It works like `--root` for an outside MCP agent.

### Other projects and refusals

The server starts in its window but is not locked to it. `project_create`,
`project_open`, `project_close` and `desktop {op: open}` work as for any MCP
client: a project no window shows is edited by the server itself and saved
after every edit, `desktop {op: open}` shows a project in a new window, and
`desktop {op: attach}` with an `instance_id` attaches to another window.
`desktop {op: attach}` naming no window goes back to the window it started
in.

The server follows its window until the agent closes, opens or launches a
project by choice. While it follows and has no project, it attaches to that
window again at its next call; attaching back to the window makes it follow
again. If that window is gone, calls fail with
`desktop_closed` (or `indeterminate` when it stopped answering after an edit
was sent), and so does `desktop {op: attach}` naming no window. The agent can
still create and open projects of its own.

The agent's `history {op: undo}` follows the MCP rule: it takes back only
the agent's own entries recorded since it last attached. After it leaves the
person's window and comes back, its earlier entries there need
`allow_person: true` or the person's own undo.

Refusals shared with outside clients apply to the in-app agent too:
`person_editing` while the person is mid-gesture and `desktop_busy` while a
dialog is open. Outside MCP clients and the in-app agent can edit the same
window at once. The window serializes their requests one at a time, and typed
tools with `expected_revision` are guarded by revision.

### Stopping and ending

**Stop** or **Cancel** kills the provider process at once and ends the
conversation; there is no graceful interrupt. Changes already made stay, and
the note says "Stopped. The N changes already made stay; undo and redo take
them back." ("Stopped. The change already made stays; undo or redo takes it
back." for one; "Stopped. Nothing was changed." for none). Changes the agent
made in other projects are added as " · 1 change in another project" or
" · N changes in other projects"; when it changed nothing here but something
elsewhere, the first part reads "Stopped. Nothing was changed here." An
export the agent already queued in a window keeps running in that window's
render queue, where the person can cancel it.

A conversation also ends on **New conversation** ("New conversation. The
agent remembers nothing of the last one."), when the person opens or creates
another project in the window ("Another project is open, so the agent's
conversation ended."), on a failure, after 12 hours and after 600 s of a
request without a word from the provider. The agent's own work on other
projects does not end it.

### Evidence

[Verification](verification.md#in-app-agent) records the tests, the eleven
native runs with both providers and the captures in
[.scratch/in-app-agent/captures](../.scratch/in-app-agent/captures/), with the
builds that made them and the defects fixed during the runs. In short: on one
macOS 26.5.2 arm64 host with debug builds, `cargo test --locked --lib` passed
1124 tests, the MCP integration tests passed 11, three runs in a row,
including a server editing its own window, and Claude Code 2.1.300 and
Codex 0.162.1 conversations edited, looked at frames, followed up and undid
their own steps; the person's Cmd+Z took back a Codex edit. Claude also
exported, saved and was stopped mid-request. The
[run notes](../.scratch/in-app-agent/evidence/native-runs-2026-10-14.md) keep
each request. Those runs used the first design, in which the server refused
to open, create or close projects and the folders were fixed for a
conversation.

The second round (2026-10-15) removed both restrictions. Tests cover it on
the same host: the MCP integration tests, now 12, include a server started
for a closed window that still makes, edits, closes and opens projects of its
own; a server that edits its window, makes and edits a project of its own
while the window's file stays untouched, and comes back to its window with
`desktop {op: attach}`; and a roots file whose folders are read again when it
changes. A unit test in [ai.rs](../src/ai.rs) covers the roots file being
written whole, for its owner only, replaced and removed. Native runs of this
round were made on 2026-10-15 with release builds on one macOS host, with
both providers; [verification](verification.md#in-app-agent) records them
and the [run notes](../.scratch/in-app-agent/evidence/native-runs-2026-10-15.md)
keep each request. Linux and Windows windows are not verified.

### Limitations

- **Stop** is not a graceful interrupt. It kills the provider and ends the
  conversation; the next request starts a new one.
- The agent's undo follows the MCP rule: it takes back only its own entries
  recorded since it last attached. After it leaves the person's window and
  comes back, its earlier edits there need `allow_person: true` or the
  person's own undo.
- The panel's change count and the window's undo cover this window only.
  Changes in other projects are counted in the note but are not taken back
  by this window's undo.
- The agent may use only the folders in [Folders](#folders); media elsewhere
  is refused until the person imports it or adds its folder in Preferences ›
  Agent › Folders.
- The second round, in which the agent may leave its window and the folders
  change during a conversation, was run natively on 2026-10-15 with release
  builds on one macOS host only
  ([run notes](../.scratch/in-app-agent/evidence/native-runs-2026-10-15.md),
  [verification](verification.md#in-app-agent)); Linux and Windows windows
  are not verified.
- Neptune Video's instructions are visible in the provider's process arguments.
  They are Neptune Video's own text; the person's words go on stdin.
- Outside MCP clients and the in-app agent can edit the same window at once.
  The window takes their requests one at a time; only typed tools given
  `expected_revision` refuse a project the other changed first.
- The agent cannot listen to sound. It judges audio by levels and loudness
  measurements only.
- Codex needs its `code_mode_host` feature for MCP tools in 0.162.1. Other
  Codex versions are unverified, as are Claude Code versions other than
  2.1.300.
- No window ran on Linux or Windows. From the macOS host, `editor-agent`
  type-checks and passes Clippy for both targets, and a Linux container ran
  the root package's Clippy and tests without a display
  ([verification](verification.md#in-app-agent)). On Windows the planner refuses a Claude `.cmd` or `.bat` launcher
  because its schema is in the arguments. A conversation puts no schema in
  the arguments, but it resolves the executable the same way, so the code
  refuses such a launcher for it too, with the planner's message; this was
  not run on Windows.
- Release builds ran on one macOS host only, for the second round
  (2026-10-15); the first round ran in debug builds there.
- Provider sign-in, model access and usage limits are the person's own.
- These checks establish that edits land, validate and undo; they do not
  establish that a result matches the person's creative intent.

## Runtime contract

This contract and the sections after it describe the planner behind
**Proposed for review** and caption translation, and the evidence recorded
for it.

The public interface lives in
[editor-agent](../crates/editor-agent/src/lib.rs),
[planner.rs](../crates/editor-agent/src/planner.rs) and
[ai.rs](../crates/editor-agent/src/ai.rs):

| Interface | Contract |
| --- | --- |
| `AiConfig` | Provider, optional executable/model/effort and 10–600 second timeout; no credentials. Default is Claude, CLI-selected model, high effort and 180 seconds. |
| `AiContext::capture` | Retains an immutable project snapshot, context ID, revision, selected clip IDs and playhead without provider or filesystem work. |
| `AiRuntime::request` | Accepts one request, returning its ID; a second active request returns `Busy`. Provider work and metadata serialization run on its worker. |
| `AiRuntime::poll` | Returns `Started`, validated `Proposal`, `Failed` or `Cancelled` events without waiting for provider work. |
| `AiRuntime::cancel` | Signals the owned request; cancellation wins a concurrent validation result. Dropping the runtime signals cancellation without waiting on the GUI thread. |
| `AiProposal` | Summary, typed commands, actual resolved provider/model where available, immutable before/after project states and captured guards. |
| `AiProposal::apply` | Rechecks context, project, revision and captured root, then commits the exact validated state. A changed batch adds one undo entry; a net no-op preserves revision, dirty state, root and redo. An empty proposal has no apply action. |
| `command_schema` / `proposal_schema` | Full external command schema and the narrower closed native-plan schema respectively. |

Selection and playhead are captured targets. Changing them later does not
retarget the request; editing, reopening, recovery or replacing the project
invalidates the proposal through its guards. Before delivery, one command batch
is reduced and validated against the full captured project. Invalid
references, source ranges, transforms or locked-track edits therefore leave the
live document unchanged. The core's `PreparedEdit` with private roots preserves the exact
validated roots, including IDs allocated by split/link/group edits. Apply
installs that root without reducing the commands again, preventing different
IDs from those shown during review. Public proposal display fields do not
confer authority to replace that validated state.
These checks establish editing correctness, not whether the suggested result
matches the person's creative intent; proposal review remains required.

The native-plan schema excludes `AddAssets` and `RestoreProject`, including
inside nested batches. Source import remains a native workflow, and checkpoint
restoration remains an explicit editing operation. The full authenticated
[external protocol](agent-protocol.md) retains those commands for clients that
already have the required metadata and authority. Every look, motion and speed
command, including `SetTracker` and `RemoveTracker`, is available to native
plans; the planner instructions carry their units and the parameter keys of each
effect kind. They state that `apply_preset` gives a look in place of the one a
clip has. A plan cannot start region tracking or a stabilization analysis:
both run in the desktop's analysis lane on the person's request, and the
planner is told to write tracker keys only from positions the person states.
The seven audio effect kinds are named with their keys and one sentence of
units (Hz, dB, octaves, ratio, semitones), and the planner is told that they
go on clips on audio tracks only, at `mix` 1 and without keyframes, and that
an enabled `pitch`, 0 included, makes the clip's speed keep its pitch. The
preview applies them through the clip's decoder chain. Fades and track
volume have one sentence of units; solo is in the schema only. A plan cannot
start a loudness measurement, a duck, a beat search, a recording, a saved
sound or a stem separation: the person starts those in the desktop, and a
plan can write only their kind of result (a volume, volume keyframes,
markers).
The planner sees no pixels, so it cannot choose a key colour, a region or a
grade from the picture. No real provider request has been made with the
look, motion and speed instructions; their evidence is the schema-size and
planner tests in [planner.rs](../crates/editor-agent/src/planner.rs).

## Planning context and disclosure

This section applies to **Proposed for review** and caption translation only.
The live agent sends more; see [what reaches the provider](#what-reaches-the-provider).

The worker sends the person's request, selected IDs, playhead and serialized
project metadata. That metadata includes names, local source paths, text clips,
canvas settings, timing, transforms, track state and available source metadata.
Neptune Video sends no source video, image or audio bytes, decoded frames or waveform
samples to this planner. Requests requiring scene understanding, transcription,
beat detection or semantic media selection are beyond this implementation; ask
for edits expressible from metadata and explicit targets.

[context.rs](../crates/editor-agent/src/context.rs) serializes a complete
project when it fits within 1 MiB. Otherwise it constructs a focused view:

- Selected clips and their transitive linked/grouped relatives remain exact
  and complete, even far from the playhead. Their referenced asset IDs and
  source duration/rate/dimensions/audio metadata remain available.
- Nearby clips are included as complete relationship components when they
  fit. Track state/order, bounded folder inventory and nearby markers are
  included with explicit total and omission counts.
- `context_scope` declares `mode: "focused"`, `complete: false`, completeness
  of selected relatives and referenced assets, omissions and guidance. Oversized
  optional names/paths are null only with an accompanying `omitted_fields`
  declaration; clip data and selected relationships are never silently cut.
- An animation track or tracker with more than 32 keys is sent as `keys: null`
  with `key_count` and `omitted_fields: ["keys"]`, in complete and focused
  documents alike. A focused view lists only the trackers its included clips
  follow, with the assets those trackers and the clips' effects reference.
- A title with word times is sent with its text, style and animations, but
  its words as `words: null`, a `word_count`, the `spoken` span and
  `omitted_fields: ["words"]`. A proposal edits the text, and the editor keeps
  the times of the words it keeps.
- If the required selection and its relationship/source context cannot fit,
  planning fails with a request for a smaller independent selection. An omitted
  object still exists; its absence cannot justify deletion or imply an empty
  track/library.

The immutable project remains complete for validation even when the provider
receives a focused view. Five-thousand-clip regression fixtures cover exact
source limits, distant relationship closure, explicit omissions, complete
small-project shape and rejection of an oversized required component in
[context tests](../crates/editor-agent/src/context.rs).

### Titles and caption translation

The planner can add plain titles, change their text, add or remove their
animations, set a caption track's language and translate a caption track
with `copy_captions`. Title styles, graphics and word times are left out of
its schema: they are designed in the inspector and presets or come from
speech, and the schema shares the provider's command line with the planner
text.

**Translate into…** in the Captions tab's track menu, and
`translate captions to <language>` in the command panel, send a proposal
request built by the editor, whatever the **Edits** setting. It selects every caption of the track, so each
is sent whole, and asks for one `copy_captions` command with a text per
caption in timeline order. The answer is reviewed and applied like any other
proposal; the core refuses one with the wrong number of texts. The request
bubble shows "Translate “English captions” into Spanish" rather than the
prompt.

With the planner text, the provider's command line is 21,452 bytes of schema
and 5,411 of text; with Windows' quote escapes that is 30,499 of the
31,744 characters the tests allow, under `CreateProcess`'s 32,767. About
1.2 KB is left for later commands and instructions.

This flow was checked natively against a fake Claude executable that logged
its arguments and stdin and answered with a fixed proposal: the request
carried the schema, the planner text and all six captions with their texts,
and the plan applied and undid as one step. No real Claude or Codex model was
asked to translate, so translation quality, kept line breaks and caption
counts from a real provider are unverified.

## Provider boundary and process ownership

This section describes the planner's launch. The live agent's launch is in
[launching the agent](#launching-the-agent); it shares the executable
resolution, credential handling and process ownership below, with the
conversation limits in its own table.

Install and sign in with the selected provider CLI separately. Neptune Video resolves
its executable on the planning worker before entering a private temporary
working directory; an explicit configured path is relative to the application's
launch directory. `AiConfig` stores no credential values, and Neptune Video does not
open or copy credential files. The provider CLI retains responsibility for its
own sign-in and account configuration. Requests use that provider's configured
model unless a model override is supplied; recorded resolved models are
observations, not promises of availability on another account.

The [Claude adapter](../crates/editor-agent/src/provider/mod.rs) uses
`--print --safe-mode --output-format json --json-schema`, with an empty tool set, strict
empty MCP configuration, empty settings sources, disabled slash commands and
no session persistence. Safe mode disables instruction/skill/plugin/hook
discovery while retaining CLI authentication. It requires a successful result with
`structured_output`. It deliberately preserves CLI subscription sign-in rather
than using `--bare`, which disables OAuth/keychain authentication according to
the [official Claude CLI documentation](https://code.claude.com/docs/en/headless).

The [Codex adapter](../crates/editor-agent/src/provider/codex.rs) implements
initialize/initialized, account readiness, effective configuration, ephemeral
thread creation and schema-constrained turn completion. It disables inherited
MCP names individually, native tools, search, plugins, hooks and delegation;
also sets `project_doc_max_bytes=0`, empty instruction fallback names and
`notify=[]` at process and thread levels. Supplied instruction sources must be
empty. Before starting, it points `model_instructions_file` to an owned file
containing only Neptune Video's planner instructions inside the private working
directory, then supplies Neptune Video's explicit
`baseInstructions` and `developerInstructions` for the thread. This keeps an
inherited custom model-instruction file out of startup and the model prompt.
It preserves configured provider/authentication settings; verifies the returned
read-only/never-approval policy; and accepts only the matching successfully
completed turn. Unexpected tools or approval requests are denied and fail the
request. Commentary, partial, asynchronous, failed and interrupted output cannot
become a proposal. Typed failures map to fixed messages without raw private
provider payloads. Native tagged command unions use `anyOf` and singleton
`enum` tags, matching the
[structured-output subset](https://developers.openai.com/api/docs/guides/structured-outputs?api-mode=responses);
the full external Command schema retains its ordinary JSON Schema shape.
The wire contract was checked against the installed Codex
0.162.1 [primary schemas](../artifacts/production-readiness/codex-0.162.1-schema)
and [official app-server documentation](https://learn.chatgpt.com/docs/app-server).

[Process ownership](../crates/editor-agent/src/provider/process.rs) uses a
watchdog and bounded stdout/stderr readers. Unix children enter an owned process
group; nonblocking pipes keep cancellation from waiting on an escaped
descendant's held pipe. Windows children start suspended, enter a configured
kill-on-close Job Object and resume only after assignment through
[the stable Windows adapter](../crates/editor-agent/src/provider/windows.rs).
Only owned process groups/jobs are terminated. No tools-disabled CLI setting is
a general sandbox for an arbitrary untrusted replacement executable.

Planner bounds:

| Resource | Current bound |
| --- | --- |
| User request / serialized planning context | 32 KiB / 1 MiB |
| Active requests / unread runtime events | 1 / at most 8 |
| Proposal / summary | 2 MiB / 1–2048 bytes |
| Edits | 128 top-level, 256 total including nested batches; depth at most 8 |
| Native-plan schema / planner instructions | Under 23 KiB encoded / under 6 KiB; the two share the Claude command line, 29 KiB between them, and stay under 31 KiB with Windows' quote escaping |
| Keys sent per animation track or tracker | 32; longer lists are sent as a count |
| Effects / masks / animation tracks per clip | 16 / 8 / 64 |
| Keys per animation track / retime map / tracker | 256 / 2–64 / 1–2048 |
| Curve points / trackers per project | 2–16 / 64 |
| Transition length | One canvas frame to the shorter of the clip and 10 s |
| Fade in plus fade out / track volume | At most the clip's length, on clips with sound / 0–8 linear |
| Process JSON frame / total stdout / stderr | 2 MiB / 32 MiB / 256 KiB |
| Inherited Codex MCP names | At most 64, each explicitly disabled |

Oversized, missing-executable, sign-in, protocol, model-access, deadline and
cancelled requests surface an error or cancellation event. They do not produce
a fallback edit or an invented successful response. Raw provider error/account
payloads and stderr are not passed through to the UI.

## Evidence and remaining acceptance

This section records the planner and the editor around it. The live agent's
evidence is under [Made as it works](#evidence).

The immutable [round-fifteen source/build checkpoint](../artifacts/production-readiness/final/round15-build-record.json)
(`dfbb53e9` source, `362ba180` Linux release) passed 521 Linux tests, including 81 agent
tests, and full-workspace/all-target denied-warning Clippy for Linux, macOS
ARM64, Windows MSVC and Windows GNU, with
[independent backend identity/log verification](../artifacts/production-readiness/final/round15-backend-root-verification.json)
and [exact release dependency graph](../artifacts/production-readiness/final/round15-release-libraries.json).
R15's [thirteen native editing/input checks](../artifacts/native/round15-editing-smoke/root-verification.json)
pass, including exact reversals, command-panel rename, durable save and stale-plan
refusal, with five viewed captures and eleven sampled process identities gone.
No inference was requested; wake/cache/multi-owner acceptance remains unrun.
Its [three startup captures](../artifacts/native/round15-window-captures/root-visual-verification.json)
verify normal/compact presentation only, with unavailable audio; they made no
provider requests. Earlier cache/overlap/preview/narrow workflows remain
R14 `f74d6560` / `8b446cca`, recorded in the
[accepted native build record](../artifacts/production-readiness/final/round14-native-editing-build-record.json)
and [native editing/preview/narrow/cache/overlap checks](../artifacts/native/cache-culling-acceptance-round1/report.json), with
[independent identity/cleanup verification](../artifacts/production-readiness/final/round14-native-editing-root-verification.json).
Finite paused five-layer native composition and
[public PNG delivery](../artifacts/production-readiness/final/round13-public-preview-root-verification.json)
remain R13 evidence; these records do not establish provider, audio or performance acceptance.
Audio was unavailable; no model requests, GTK dialogs or performance were
retested. Native interface performance remains `b29ccb69` / `14435a4a` at the
333-test round-nine checkpoint. Latest actual
native real-provider AI/GTK evidence remains source `28259254`, release
`2c596cb0`, as scoped below. Separate round-twelve
[virtual silent-stall recovery](../artifacts/production-readiness/final/round12-virtual-stall-root-verification.json)
and [healthy physical silence-clock verification](../artifacts/production-readiness/final/round12-physical-clock-root-verification.json)
establish clock release/Playback fallback and healthy zero-PCM clock behavior
respectively. Neither establishes a native fatal error callback, acoustic quality
or A/V synchronization; they were not exercised by the UI run.
The separate [optimized R14 worker diagnostic](performance.md#optimized-distinct-video-worker-diagnostic)
now records distinct-video batch latency; it is not AI, native presentation or
follow-up dispatch/consumer acceptance.
The [liveness correction](../artifacts/production-readiness/preview-request-liveness-round1/phase-b/root-green-verification.json)
passes 21 focused/198 desktop tests and is now included with 112 media tests
in R15's 521-test checkpoint. The [completed review](../artifacts/production-readiness/preview-request-liveness-round1/review-round1.md)
finds no P1/P2 behavior defect. The later [canonical bridge proof](../artifacts/production-readiness/frontend-retention-bridge-round1/root-verification.json)
and [independent review](../artifacts/production-readiness/frontend-retention-bridge-round1/review-round1.md)
observe the nonempty retention call and restart reset (7/10 and 14/20 starts,
60 full-pixel checks). This is a four-case artifact probe at frozen R15, not a
standing suite guard; retire/clear/Closed, source/cut/reorder and playing remain
unproved. The separate
[81-case R15 worker comparison](performance.md#optimized-r15-legacyexplicitprotected-comparison)
passed with root verification; it establishes no native or provider result.
Complete native R15 input, wake/cache/multi-owner, platform-pack integration and new provider results
remain unverified. The six prepared bookkeeping tests were declined without
integration or execution; the 198 desktop/521 workspace counts are unchanged.

On 2026-10-12, an earlier backend checkpoint's public `AiRuntime` using default
PATH-resolved CLIs completed a generated title-split request with both real providers:

| Provider | Recorded resolved model | One-run elapsed time | Evidence |
| --- | --- | --- | --- |
| Claude | `claude-opus-5-5` | 3.3215 s | [Claude split](../artifacts/production-readiness/final/claude-split-explicit-semantics.json) |
| Codex | `gpt-6.1-sol` | 6.9987 s | [Codex split](../artifacts/production-readiness/final/codex-split-explicit-semantics.json) |

Each record verifies completed provider output, the requested split, preserved
settings, read-only review, committed state identical to review including new
IDs, one undo entry and exact undo/redo. Earlier
[Claude](../artifacts/production-readiness/ai/claude-final.json) and
[Codex](../artifacts/production-readiness/ai/codex-final.json) records verify
rename/move/portrait canvas edits on a prior backend checkpoint. These are
single backend-runtime
observations on generated metadata; native interaction has its own records below.
They do not establish comparative model performance or visual media understanding.
Deterministic protocol,
context, proposal and process regressions are in the
[round-fifteen 81-test agent/workspace record](../artifacts/production-readiness/final/round15-tests.log).

Acceptance caught and corrected two integration problems: Codex rejected the
original `oneOf` schema on some requests, and its loader rejected a zero-byte
instruction file. The
[owned no-model startup comparison](../artifacts/production-readiness/codex-owned-prompt-startup-fq9txewt/classification.json)
confirms the latter; the override now contains only Neptune Video's known nonempty
instructions. Two subsequent model responses described a split but proposed
an unchanged transform; explicit operation semantics then produced the correct
split above. These observations reinforce review of command intent as well as
typed validity. In the later
[native GTK/Ask round](../artifacts/native/ai-round8/report.json), on db30 source
and the preserved `9f721c8b` release, both providers applied a correct text split
with exact undo/redo. Claude completed two proposals and one cancellation from
three requests; Codex completed three proposals, including one unchanged
transform with a split summary that was discarded before a correct retry.
That failure occurred despite the explicit split wording. Claude's observed
default there was `claude-haiku-5-5`; the command line controls defaults.

The native-verified `2c596cb0` release reports a nonempty net-no-op as "No changes",
preserving existing history and clean state without an undo promise. Four new
desktop regressions passed in the 309-test source record. The
[round-nine native checkpoint](../artifacts/native/ai-round9/report.json) verifies the
actual Ask/apply path with a labeled owned no-op fixture: clean state, revision,
full project and redo stay unchanged; the card gives quiet feedback without an
Apply toast. Real Claude and Codex each produced the correct split on their first
request, with one exact undo/redo and no failed/bad plans in this round. This
does not erase the earlier bad responses. Narrow/normal captures, a stale
fixture, import dialogs and protected unbound GTK recovery also passed.

The round-fifteen full workspace, including desktop/media and test targets,
passed Rust 1.97.1 cross-target compilation and denied-warning Clippy for
[macOS ARM64](../artifacts/production-readiness/final/round15-macos-arm64-clippy.log),
[Windows MSVC](../artifacts/production-readiness/final/round15-windows-msvc-clippy.log) and
[Windows GNU](../artifacts/production-readiness/final/round15-windows-gnu-clippy.log).
These checks neither linked nor ran a native application.
[Portable process-fault fixtures](../.scratch/native-ai/issues/01-portable-process-fault-fixtures.md)
are integrated; [12 focused adapter cases](../artifacts/production-readiness/portable-fixture/adapter-tests.log)
ran on Linux, and the round-fifteen workspace passed 521 tests. Native macOS/Windows
execution is deferred by the user while tests are prepared here. The
[native acceptance pack](native-platform-tests.md) now has 64 cases, all
`not_run`, and [106 offline guards passed on the pinned pack](../artifacts/native/platform-regressions-integration-round1/root-pinned-attempt-01/root-verification.json).
Fixture bytes and ten Linux native startup openings are verified, without
catalog passes. Source/fixture pins are set; [final independent review](../artifacts/native/platform-regressions-integration-round1/review-round2/REVIEW.md)
found no P1/P2 and closed all four preparation findings.
The R14 51-case/57-guard baseline and separate eleven-case draft remain
historical. These checks establish no native target execution.
[Verification](verification.md#platform-matrix)
keeps that work separate from the historical baseline and target execution.
Actual provider/job behavior, packages and
signed-in review/failure/cancel/apply on those platforms remain acceptance work.
Linux native review/stale/cancel/exact apply passed at db30, followed by
`2c596cb0` no-op and both-provider split/undo/redo acceptance. See
[verification](verification.md) for broader editor/platform gates and
[the Native AI contract](../.scratch/native-ai/spec.md) for the active feature.
