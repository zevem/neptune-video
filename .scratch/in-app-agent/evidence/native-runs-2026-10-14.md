# In-app agent native runs, 2026-10-14

Host: macOS 26.5.2 (Darwin 25.5.0), Apple Silicon, 8 CPUs, 8 GB, rustc
1.97.1. Debug builds of `neptune-video` and `neptune-video-mcp` (`cargo build --locked
--bins`), Whisper `tiny`. Each run used a task-owned folder with
`footage/` (interview.mp4, street.mp4, bars.mp4, music.m4a). Requests were
typed into the Agent tab of the running window, in Ask mode, Made as it
works unless stated. Claude Code 2.1.300 with `claude-opus-5-5`; Codex CLI
0.162.1 with `gpt-6.1-sol`.

Where a request is quoted with "…", or described rather than quoted, its
full wording was not kept.

## Builds

| Build | Hashes (SHA-256, debug) | Runs and captures |
| --- | --- | --- |
| Before A | not recorded | runs 1 to 7, captures 01 to 05 |
| A | `neptune-video` 94e5cee8…, `neptune-video-mcp` 03c65742… | run 8 and the first two requests of run 9, captures 06 to 09 |
| B | `neptune-video` cfb88ea4…, `neptune-video-mcp` 827ebb47… | the third request of run 9, captures 10 and 11; shorter working row and the "changes" wording |
| C | `neptune-video` 06af350d…, `neptune-video-mcp` f0d5557b… | run 11, captures 12 and 13; adds the restart note |
| D (final) | `neptune-video` 3ceedd91bc57ebb4ef33f315beea9e4d84cfb8eda9e4784d63c071457a489f87, `neptune-video-mcp` a8dea38a9f59f96cd6183c4ab833501eb2dd05020c163bb48b3e8df409658124, `neptune-video-agent` f1a4901ed32cce7acd2f6afc44ce0e62d970dbf1f070d6f268e4effd97ee0147 | run 12, captures 14 and 15; the panel's introduction names every folder the agent may use, and the Windows launcher message no longer says "planning" |

## Claude Code, earlier builds

1. Effort medium, wide window (1280x800 pt). The full promo brief, of the
   kind [the real-client runs](../../native-mcp/evidence/real-clients-2026-10-14.md)
   used, typed in the Agent tab; its exact wording was not kept. About 2
   minutes and 11 edits. It imported, built interview → street (2 s) → bars with a dissolve,
   ripple-trimmed the interview, transcribed and made 5
   captions, titled "Neptune Video Promo" and then enlarged it with an outline,
   added music in two pieces with a fade, wrote `promo_preview.mp4`
   (960x540) and `promo.mp4` (1280x720, 30/1, 9.472 s, with audio) and
   saved. Steps appeared as they ran
   ([`01-claude-steps-live.png`](../captures/01-claude-steps-live.png)).
2. The same conversation: "Change the title text to Neptune Video Promo 2026 and
   make it yellow." (1 edit), then "Actually, undo the colour you just
   picked but keep the new words." It remembered the previous turn, undid
   its own step and applied the words again (2 changes)
   ([`02-claude-follow-up-same-conversation.png`](../captures/02-claude-follow-up-same-conversation.png)).
3. A request for a contact sheet, a fade out and an export to
   `promo-v2.mp4`. Stop during "Waiting for a job". The note read
   "Stopped. The 1 edit already made stays; undo takes them back.", wording
   fixed since. The export finished in the window's render queue. No
   `claude` or `neptune-video-mcp` process was left
   ([`03-stopped-mid-request.png`](../captures/03-stopped-mid-request.png)).

## Codex, earlier builds

4. The first attempt answered "I couldn't access Neptune Video's tools". The
   cause was `code_mode_host` being disabled; with it off, a probe
   answered "I couldn't call neptune_video_window.project_status because the tool
   host is disabled". Fixed by keeping it on. The rerun added a marker
   "Codex was here" at 3 s and made the street clip black and white, and
   checked a frame: 2 edits.
5. The person pressed Cmd+Z in the window. The agent's last edit, the
   black-and-white effect, was undone and Redo became available
   ([`04-person-undoes-agent-edit.png`](../captures/04-person-undoes-agent-edit.png)).
6. Narrow window (820x640 pt): a marker "Narrow" at 1 s and a frame check.
   Steps and the note fit.

## Proposed for review, earlier build

7. Claude: "Rename the marker at 1 second to Opening." A reviewed proposal
   (Remove marker, Add marker) with Discard and Apply; nothing was applied
   ([`05-proposed-for-review-mode.png`](../captures/05-proposed-for-review-mode.png)).

## Final builds

8. Build A, Claude, wide window, a fresh folder: "Import everything in the
   footage folder. Put interview then bars … half-second dissolve …, title
   Final build for 2 s, music.m4a at -12 dB, look at a frame, export
   final.mp4 next to the project and save." 7 edits. `final.mp4` is
   1280x720, 30/1 fps, 7.509 s, with video and audio (ffprobe); the
   project was saved. The working row still truncated "3 edi…"
   ([`06-final-claude-working.png`](../captures/06-final-claude-working.png)),
   fixed after this capture; done
   ([`07-final-claude-done.png`](../captures/07-final-claude-done.png)).
9. Codex, narrow window (820x640 pt), the project of run 8.
   - Build A: "Make the Final build title bigger and move it near the top,
     check a frame at 1 s" (2 edits)
     ([`08-final-codex-narrow.png`](../captures/08-final-codex-narrow.png)).
   - Build A: "Undo only the move you just made, keep the bigger size." It
     undid only the move, on the same thread
     ([`09-final-codex-follow-up.png`](../captures/09-final-codex-follow-up.png)).
   - Build B: "Add a marker named Cut at 4 s, fade bars in over 0.5 s, look
     at frames at 4 and 5 s". The working row "Working… 15s" fits
     ([`10-final-narrow-working-row.png`](../captures/10-final-narrow-working-row.png));
     the note read "Codex · gpt-6.1-sol · 2 changes; undo and redo take
     them back"
     ([`11-final-codex-done-note.png`](../captures/11-final-codex-done-note.png)).
10. After the windows were closed, no `claude`, `neptune-video-mcp` or `ffmpeg`
    process of the task remained. Two Codex app-server daemons started on
    Sep 27 and Oct 10 belong to the person and were left alone.
11. Build C, Claude, effort Low: "How many clips are on the timeline? Do
    not change anything." A correct answer and the note "no changes". The
    agent menu shows Answered by, Effort, Edits (Made as it works,
    Proposed for review), New conversation and Model…
    ([`12-final-agent-menu.png`](../captures/12-final-agent-menu.png)).
    Effort changed to Medium, then "What did I ask you a moment ago?" The
    panel showed the restart note and the agent said it could not tell
    ([`13-final-restart-note.png`](../captures/13-final-restart-note.png)).
    No task process was left afterwards.
12. Build D, a new window with no conversation: the panel's introduction
    names the Movies, Music, Pictures, Desktop, Documents and Downloads
    folders, as `agent_roots` allows
    ([`14-final-intro-wide.png`](../captures/14-final-intro-wide.png)). At
    820x640 the second paragraph is below the fold of the panel's scrolling
    log ([`15-final-intro-narrow.png`](../captures/15-final-intro-narrow.png)).
    No request was sent.

## Not verified

- Linux, Windows and release builds; only this macOS host.
- Codex versions other than 0.162.1, Claude Code other than 2.1.300.
- A Claude `.cmd` shim on Windows for a conversation.
- Sound by ear: the agent cannot listen; it judges audio by levels and
  loudness measurements only.
- The hashes of the builds before A are not recorded here; captures 01
  to 05 come from them.
