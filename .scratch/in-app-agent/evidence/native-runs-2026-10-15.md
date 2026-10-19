# In-app agent native runs, 2026-10-15: release build, the same reach as an outside agent

Host: macOS 26.5.2 (Darwin 25.5.0), Apple Silicon, 8 CPUs, 8 GB, rustc
1.97.1. Release builds of `neptune-video` and `neptune-video-mcp`
(`cargo build --release --locked --bins`), started from `target/release`,
with the FFmpeg fetched into `.tools/media`. One task-owned folder:
`work/edit.nvp` ("Release run": interview then bars, made beforehand by
an `neptune-video-mcp` client), `work/footage/` (interview.mp4, street.mp4,
bars.mp4, music.m4a) and, outside it, `extra/b-roll.mp4`; settings,
sessions and the Whisper models in its own `settings/`. Requests were
typed into the Agent tab in Ask mode, "Made as it works". Claude Code
2.1.300 with `claude-opus-5-5`, effort Medium; Codex CLI 0.162.1 with
`gpt-6.1-sol`, effort Medium.

## Builds

| Build | SHA-256 | Runs |
| --- | --- | --- |
| E | `neptune-video` `6cbc3c0119608bfff4ae60cf2055f5fef31e45c8f6c45a29950631df6b890665`, `neptune-video-mcp` `387e6d3d58443374ce9563fc70404776309c860d1505d88a8911aa3fd4f4ba0c`, `neptune-video-agent` `6f7c7fb7a0af2620accb4ec21c7207281504ad11687cf203d5cb47f58dba614a` | 1 to 6, captures 16 to 25 |
| F | `neptune-video` `3c184d512495b0e317cbdfe259c5fd3449706b434c8b74d86aae07592ec24964`; `neptune-video-mcp` and `neptune-video-agent` unchanged from E | 7 and 8, captures 26 to 28; each window tags its agent's requests with its own instance (defect 12) |
| G (final) | `neptune-video` `4ac9f25dd9c484a7b9c5ab9536d1af68773330aee0cdb94c4d98aadabf78d5e0`, `neptune-video-mcp` `350d1ab77af9f7491869f9b372bbc0cbda20f9e1d20b8baa0c3dea1278d508b6`, `neptune-video-agent` unchanged | 9, capture 29; the folders file is written and waited for only when the folders changed before a conversation starts; the window-project refusal names Settings › Agent › Folders |

## Runs

1. Build E, Claude, wide window (1280x800 pt): "Make a separate new project
   called teaser.nvp next to this one, without changing mine. In it,
   import footage/street.mp4 and footage/bars.mp4, put street then bars,
   add the title Teaser for the first 2 seconds, and export it to
   teaser.mp4 next to it. Then come back to my project and add a marker
   named Teaser made at 0 seconds." The agent made `teaser.nvp` with
   local authority, exported `teaser.mp4` (ffprobe: 1280x720, 30/1 fps,
   7.019 s, video and audio), came back to the window and added the marker.
   The note read "Claude · claude-opus-5-5 · 1 change, which undo or redo
   takes back · 3 changes in other projects"
   ([`16-release-claude-makes-another-project.png`](../captures/16-release-claude-makes-another-project.png)).
   The roots file beside the session was mode 0600 and named the project
   folder, `footage/` and the six usual folders.
2. Build E, Claude, the same conversation: "Import …/extra/b-roll.mp4 into
   my project and put it at the end of the timeline. If you cannot reach
   it, just tell me and stop." Refused `outside_roots`; the agent named
   Settings › Agent and changed nothing ("no changes")
   ([`17-release-folder-outside-refused.png`](../captures/17-release-folder-outside-refused.png)).
   In Settings › Agent, "Add folder…" opened the native folder dialog. The
   first attempt chose a folder of the person's by mistake (the Go to
   Folder field did not take the pasted path); it reached the roots file
   at once, and "Remove" took it out of the settings and the roots file
   again. The second attempt added `extra`
   ([`18-release-settings-folder-added.png`](../captures/18-release-settings-folder-added.png)),
   and the roots file named it within the frame. "I added that folder in
   Settings. Try again please." The agent, in the same conversation and
   the same `claude` process (PID unchanged across runs 1 and 2), imported
   `b-roll.mp4` and put it at 7.49 s: "2 changes"; no restart note
   ([`19-release-same-conversation-uses-folder.png`](../captures/19-release-same-conversation-uses-folder.png)).
   The person saved and quit. No `claude` or `neptune-video-mcp` process, session
   file or roots file was left; `edit.nvp` held 3 video clips and the
   marker "Teaser made".
3. Build E, Codex, narrow window (820x640 pt): "Fade the b-roll clip out
   over half a second, export the whole timeline to codex.mp4 next to the
   project, wait for the export to finish, then save the project." 1
   change; `codex.mp4` 1280x720, 30/1 fps, 11.520 s, video and audio;
   the title bar read "Saved" and `edit.nvp` held the fade as the
   clip's `transition_out`
   ([`20-release-codex-narrow-export-saved.png`](../captures/20-release-codex-narrow-export-saved.png)).
4. Build E, Codex, narrow: a request for ten markers, a transcription and
   captions. Codex had made the markers and five captions at 22 s
   ("Working… 22s · 11 ch…",
   [`21-release-codex-working-before-stop.png`](../captures/21-release-codex-working-before-stop.png))
   when Stop was pressed: "Stopped. The 11 changes already made stay; undo
   and redo take them back."
   ([`22-release-codex-stopped-changes-stay.png`](../captures/22-release-codex-stopped-changes-stay.png)).
5. Build E, Codex, narrow: "Make a contact sheet of the whole timeline,
   then export it again to codex-2.mp4 …", Stop after 7 s: "Stopped.
   Nothing was changed."; no `codex-2.mp4`
   ([`23-release-codex-stopped-nothing-changed.png`](../captures/23-release-codex-stopped-nothing-changed.png)).
   After each Stop no Codex process of the task and no `neptune-video-mcp` was
   left; the person's two Codex app-server daemons were left alone.
6. Build E, Codex, narrow: "Open teaser.nvp (next to this project) in a
   new Neptune Video window, change its Teaser title text to Teaser 2 there and
   save it, then come back to my window and tell me how many clips my
   timeline has." `desktop {op: open}` started a second release window;
   Codex edited and saved `teaser.nvp` there, came back and answered 8
   clips (3 video and 5 captions). Note: "Codex · gpt-6.1-sol · no changes
   here · 1 change in another project"
   ([`24-release-codex-back-from-another-window.png`](../captures/24-release-codex-back-from-another-window.png)).
   The second window showed "Teaser 2" and "Saved", but its panel listed
   only "Agent: Save", not the text edit
   ([`25-release-second-window-missing-row-defect.png`](../captures/25-release-second-window-missing-row-defect.png)):
   defect 12.
7. Build F, Claude, wide: the same request with "Teaser 2 to Teaser 3".
   The second window now lists "Agent: Edit text" and "Agent: Save"
   ([`26-release-second-window-shows-agent-rows.png`](../captures/26-release-second-window-shows-agent-rows.png));
   the home window's note read "no changes here · 1 change in another
   project"
   ([`27-release-claude-back-in-its-window.png`](../captures/27-release-claude-back-in-its-window.png)).
8. Build F, narrow: Settings › Agent with the added folder "extra" and
   "Add folder…" fits at 820x640
   ([`28-release-settings-folders-narrow.png`](../captures/28-release-settings-folders-narrow.png)).
   After both windows were quit, no task process, session file or roots
   file remained.

9. Build G, Claude, wide, a new window and conversation: "Which folders may
   you use right now? Check project_status, tell me whether a folder named
   extra is among them, and change nothing." It listed the project's
   folders, `extra` and the six usual folders, and changed nothing
   ([`29-release-g-new-conversation-reads-folders.png`](../captures/29-release-g-new-conversation-reads-folders.png)).
   No task process, session file or roots file was left after quitting.

## Defect found and fixed

12. Every window started its agent's server with the same `--tag
    neptune-video-tab`, and a window took any request whose id began `neptune-video-tab-`
    for its own agent's, whose steps it shows in its own panel. When one
    window's agent edited a project shown in another window, that window
    hid the edit's row. Each window now tags its agent with its own
    instance (`neptune-video-tab-<instance>`) and claims only those requests; a
    unit test checks that another window's tag, the bare `neptune-video-tab-`
    prefix and an outside client's `mcp-` are not its own.

## Not verified here

- Linux and Windows windows. `editor-agent` type-checks and passes Clippy
  for Windows and Linux from this host; a Linux arm64 container ran the
  root package's Clippy, the MCP tests (12 passed with the pinned FFmpeg
  and software Vulkan, window tests skipped without a display) and the
  unit tests ([verification](../../../docs/verification.md#in-app-agent)).
- Claude in a narrow window doing a long request; Claude ran wide, Codex
  narrow.
- An Intel Mac.
