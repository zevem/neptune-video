# In-app agent acceptance on Linux and Windows

Status: ready-for-human
Progress: open
Type: task
Blocked by: 01, 02, 03

## Scope

Build and run the in-app agent on Linux (X11 and Wayland) and Windows, and
a release build on macOS. Repeat the macOS runs in the
[native runs](../evidence/native-runs-2026-10-14.md) with both providers:
a multi-step request, a follow-up in the same conversation, the agent's
own undo, the person's Undo, Stop during a queued export, Proposed for
review, and a narrow window. Capture each state.

Check on Windows:

- whether a Claude `.cmd` or `.bat` shim can carry a conversation. The
  planner refuses one because its schema is in the arguments. A conversation
  puts no schema there, but it resolves the executable the same way, so the
  code refuses a shim for it too, with the planner's message; `INSTRUCTIONS`
  is a multi-line system-prompt argument;
- `neptune-video-tools.json`, which is created owner-only (0600) on Unix only;
- that `neptune-video-mcp.exe` is found beside `neptune-video.exe`, and the roots from
  `directories::UserDirs`;
- the roots file (2026-10-15, [issue 06](06-same-reach-as-an-outside-agent.md)),
  which is owner-only on Unix only and which the server tells apart by
  modified time and length alone there, with no inode: that a folder added
  in Settings › Agent › Folders is usable in the same conversation;
- that Stop leaves no provider or server process behind.

## Result

Not run. Linux, Windows and release builds are unverified for this work.

## Comments

2026-10-14: Opened. This work was built and run only on one macOS host
(macOS 26.5.2, arm64) with debug builds. It needs a person with Linux and
Windows hosts signed in to Claude Code and Codex.
