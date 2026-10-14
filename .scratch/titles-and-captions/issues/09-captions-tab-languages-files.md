# Captions tab, languages and subtitle files

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 04, 07

## Scope

Captions tab: caption tracks and their language, cue list with times and text, add/split/merge/delete cues, style the whole track, import SRT/VTT to a new caption track, export a caption track to SRT or VTT.

## Comments

- The library has a Captions tab between Text and Effects. It shows one caption track at a time, which is a text track with a language. The header row has the track chooser ("New caption track…" at its foot) and an eye that hides or shows the track. Under it are the language chooser and "Style", then the cue list, "Add caption" and a count ("N captions", or "found of all" while searching). The tab follows the track of a selected caption. With no caption track it says what captions are and offers "New caption track" and "Import SRT or VTT…".
- Cues are edited in place:
  - Clicking a caption's words opens them for typing. Shift+Return adds a line, Return or leaving the field keeps the text, and Escape gives it up. Empty text is refused ("delete it instead").
  - Clicking a start or end time opens it for typing as `0:01.20`, `2.5`, `12f` or `500ms`. A time is quantized to the frame and stops at the neighbouring captions. A refusal says why: before or after its other edge, covering a neighbour, shorter than a frame, or already there.
  - The row menu has "Start at playhead", "End at playhead", "Split at playhead", "Merge with next" (words and word times kept when both have them) and "Delete caption". These are disabled on a locked track.
  - "Add caption" adds two seconds at the playhead, shortened to end where the next caption starts, with the look of the caption before it. Each gesture is one command and one undo step.
- Languages: the menu lists 40 languages and takes any well-formed BCP 47 tag typed or named ("pt_br" becomes pt-BR, "Spanish" becomes es). A tag the list lacks, such as pt-BR, is checked above the list. A track named for its language ("English captions") is renamed with it; a renamed track keeps its name.
- Import ("Import SRT or VTT" in the header, several files at once):
  - Files are read on a worker and capped at 8 MiB. They are decoded as UTF-8 or UTF-16 by their BOM, then UTF-8, then Windows-1252, and parsed whole or not at all.
  - Each file becomes one new caption track in one step. Its language comes from the VTT `Language:` header or a tag in the file name (`talk.pt-BR.srt`), otherwise the track in view, otherwise English. The notice says when it guessed.
  - Cue times are kept as the file gives them, not quantized to frames.
  - A replaced project discards a read that finishes late.
- Export (the header's export menu, SubRip or WebVTT): the track in view is formatted on the main thread and named `Project.lang.ext`. It is written on a worker to a temporary file and renamed into place. VTT carries the `Language:` header and karaoke word times.
- The list draws only the rows in view, and follows the playhead except while a caption is being typed. The bench test with 2000 captions draws 5 to 40 rows.
- Fixed in the inspector: its scroll helper read the focused control's response on every focus change. egui treats that read as a test for a click elsewhere, so a control that another panel focused in the frame a click ended lost the keyboard straight away. It is now read only when the keyboard, not the pointer, moved the focus. The new inspector test fails without the fix.
- Tests:
  - `src/ui/captions_tab_tests.rs`, 17 tests: 9 on the commands, files and times, and 8 benches of the tab that run real egui passes (empty state, adding and typing, typed times, row menu, track and language menus, style menu, header import and export, and 2000 rows).
  - 1 inspector test for the focus fix, and 1 for language tags.
  - Desktop: 654 passed. `cargo clippy --workspace --tests -D warnings`, `cargo fmt --check` and the architecture check pass.
- Native captures of the debug build under Xvfb + lavapipe on this Linux container, with a generated project holding English (VTT, karaoke word times) and Spanish (SRT) caption tracks:
  - At 1440×900:
    - the tab with the current-cue bar and the karaoke caption in the viewer
    - a caption opened by a click, with " today" typed and Return pressed: the list, viewer, inspector and timeline show the new words, and undo is offered
    - the track, language, style, row and export menus
    - a start time open for typing
    - the empty state and its new-track menu
    - "pt-br" typed into it, making "Portuguese captions" with pt-BR checked in its language menu
  - At 760×560: the tab at a 300 px library, the icon tabs at 220 px, and a caption added and opened for typing.

  The window was given the X keyboard focus first, so typed keys reached it. The system file pickers for import and export were not driven natively: those flows are covered by the bench (the actions asked for) and the file tests (reading, writing, encodings). Sound reports no device on this host.
