# Translation and agent surface

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 09

## Scope

Whisper translate-to-English option; Translate captions through the native AI provider as one reviewed `copy_captions` proposal; provider schema under the cap; planner text; command-panel phrases.

## Comments

- Whisper's translate-to-English switch shipped with ticket 10 in the speech panel. Captions it makes are tagged English (`caption_language`). This host has only Flite, which speaks English, so the switch was not run on speech in another language.
- Translation through the AI provider:
  - The caption track menu has "Translate into…" (marked AI). It turns into the language list, which also takes a typed BCP 47 tag.
  - A language asks the native provider for one proposal. The request names the track, its language, the target language and the track name to use, and it selects every caption of the track, so the provider sees each one whole.
  - The proposal is one `copy_captions` command. It is shown as a plan card and applied as one undoable step. The core makes the new track directly above the source, with fresh IDs and each cue's times and word timings, and hides the source track.
  - The request is refused, with a message and nothing sent, when the track has no language, is already in that language, has no captions or also holds graphics. The menu item is disabled for an empty track or one that holds graphics.
  - The request bubble shows "Translate “English captions” into Spanish", not the prompt with IDs.
- Planner: the system text now explains titles, `set_text`, caption tracks, `copy_captions` and removing an animation.
  - Sizes measured on the command line the fake provider received:
    - proposal schema: 21,452 bytes;
    - system text: 5,411 bytes;
    - schema with Windows quote escapes plus system text: 30,499 of the test's 31,744-character bound, under CreateProcess's 32,767.
  - Only about 1.2 KB is left for future planner text or commands.
- Command panel phrases:
  - "animate in pop", "animate out fade for 1s", "animate in none" (one step over the selected titles; a new animation's span is held to what the clip leaves after its other edge);
  - "generate captions" / "captions from speech" (opens the Captions tab and runs the last speech settings over the whole timeline);
  - "translate captions to Spanish" / "translate captions into pt-BR" (uses the caption track of the selected clip, or the only caption track shown).
- Language tags: a typed language must now also have the shape of a BCP 47 tag ("Klingonese" is refused, "tlh" and "x-klingon" are kept).
- Native verification: debug build under Xvfb on this Linux container. Task-owned settings point the Claude provider at a fake executable that logs its arguments and stdin and returns a fixed Spanish `copy_captions` proposal (each text prefixed "[es] ").
  - 1440×900:
    1. "generate captions" in the command panel switched to the Captions tab and added 6 English captions with the Tiny model. The panel says "Done" while the job is still running.
    2. "animate in pop" on a selected caption showed Pop, By word, 0.50 s in the inspector.
    3. Track menu → "Translate into…" → Spanish. The Agent tab opened with the bubble "Translate “English captions” into Spanish". The fake received the 21,452-byte schema, the 5,411-byte system text, `--effort high` and all 6 selected captions with their texts.
    4. The plan card read "Spanish captions from the English ones · 1 Translate captions · Clips 7→13 · Tracks 3→4".
    5. Apply put "Spanish captions" above the hidden English track, and the viewer drew "[es] Three things no body". One Undo removed the Spanish track and showed the English one again.
  - 760×700:
    1. The track menu, "Translate into…" and the full language list fit.
    2. German was clicked directly. The request asked for "German captions", language "de", and the library gave way to the Agent tab with the plan card's Discard and Apply in view. Discard left the project as it was.
  - Faults found and fixed:
    1. A menu that changes into the language list kept its old height of about two rows. The list now asks for its own height.
    2. The request bubble showed the prompt with raw IDs.
- Tests: `cargo test --bin neptune-video` 671 passed. `cargo test -p editor-agent` with `NEPTUNE_VIDEO_PROVIDER_TEST_FIXTURE` set: 72 and 14 passed. `cargo clippy --workspace --tests -- -D warnings` and `cargo fmt --check` are clean.
  - New desktop tests: `a_translation_asks_for_one_copy_with_a_text_for_each_caption`, `the_track_menu_translates_into_a_language_chosen_or_typed`, `titles_are_animated_in_and_out_by_name_as_one_step_each`, `captions_are_translated_from_the_selected_track_or_the_one_shown` and `captions_from_speech_are_asked_for_in_the_captions_tab`.
  - The two track-menu tests click Spanish and German directly; they fail without the list height fix.
- Not verified:
  - a real provider's translation quality or its keeping of line breaks and caption counts (a proposal with the wrong count is refused by the core);
  - a long caption track near the context limits;
  - the Codex provider for this request;
  - macOS and Windows.
