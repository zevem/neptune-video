# Real MCP clients, 2026-10-14

Host: macOS 26.5.2 (Darwin 25.5.0), Apple Silicon, 8 CPUs, 8 GB. Debug
build of `aster-mcp` (`cargo build --locked --bin aster-mcp`), FFmpeg 9.0.2
from `.tools/media/macos-aarch64`, Whisper `tiny` installed by the server.
Each client had a fresh folder as the only root, with `footage/`
(interview.mp4: a test pattern with macOS `say` speech; street.mp4;
bars.mp4; music.m4a), and the same brief: create a 1280x720 30 fps
project, import and inspect, build interview → street (first 2 s) → bars
with a dissolve, transcribe and caption the speech, add a title with an
entrance animation, music at −12 dB, look at frames, fix, preview, export.

## Claude Code 2.1.300

`claude -p BRIEF --mcp-config mcp.json --strict-mcp-config --allowedTools
"mcp__aster__*"` with the built-in file and shell tools disallowed.

- 41 tool calls, 42 turns, one refusal: `text_edit` with a shadow offset
  in pixels ("Invalid text style"). It read `edit_schema {command:
  set_text_style}` and retried with fractions. The refusal now gives the
  ranges.
- It looked at three contact sheets and two `frame_render` sets. It then
  enlarged and moved the title, added a fade-out, and retimed the last
  caption to the cut.
- `promo-preview.mp4` (960x540) and `promo.mp4` (1280x720, 30 fps, 9.47 s,
  284 frames, with sound) were written. The project was at revision 14 and
  saved.
- Opened afterwards in the debug `aster`, it showed the five captions, the
  title, the three clips with the dissolve, and the music in two pieces.
  The window offered to restore a recovery copy: the client had stopped
  the server with a signal, which this build then did not handle. Fixed
  the same day; the final build's captures `1-…` and `5-…` have no prompt.

## Codex CLI 0.162.1

`codex exec --json -c mcp_servers.aster.command=… -c
mcp_servers.aster.args=[…] -c mcp_servers.aster.tool_timeout_sec=120 -s
read-only BRIEF`.

- First run: Codex refused every non-read-only tool ("MCP tool call
  requires approval, but approval policy is never"); read-only tools
  worked. `default_tools_approval_mode = "approve"` is now in the setup
  guide.
- Second run with it: 39 tool calls, none refused. It used contact
  sheets, `frame_render` and `edit_schema`, and passed `expected_revision`
  on its edits. Its exports `promo-preview.mp4` and `promo.mp4`
  (1280x720, 30 fps, with sound) completed, and the project was saved at
  revision 11.

## Claude Code 2.1.300 on the final build

Same brief, a fresh folder, and the build whose hashes
[verification](../../../docs/verification.md#native-mcp) records.

- 38 tool calls, 39 turns, $1.00, no refusals. It read `edit_schema`
  twice before restyling the title.
- Its own fixes after looking: it trimmed the interview to a whole frame
  (its sound is longer than its picture), pulled the last caption back to
  the cut, and made the title bolder with an outline and shadow so it
  reads on the bars.
- `promo_preview.mp4` (960x540) and `promo.mp4` (1280x720, 30/1 fps,
  9.472 s, video and audio by ffprobe) were written. The project was
  saved at revision 12.
- Things it said it could not do: listen to the mix, so the music level
  was not checked by ear. Its music was shorter than the video, so it was
  repeated with a fade.
- The server stopped with the client and left no recovery copy. The
  project opened in `aster` as saved
  (`../captures/5-person-opens-claude-code-project.png`).

