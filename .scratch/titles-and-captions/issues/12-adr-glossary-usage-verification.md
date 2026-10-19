# ADR 0007, glossary, usage and verification

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 01, 02, 03, 04, 05, 06, 07, 08, 09, 10, 11

## Scope

Record the decision, the vocabulary, the user guide and what was measured on which host.

## Comments

- Written:
  - [ADR 0007](../../../docs/adr/0007-titles-and-captions-are-clips-drawn-off-the-frame.md): the decision, the speech options set aside, and its consequences.
  - [Glossary](../../../GLOSSARY.md): a Titles and captions section with twenty terms, and fonts added to Asset.
  - [Architecture](../../../docs/architecture.md): the ownership rows and a Titles, captions and speech section.
  - [Performance](../../../docs/performance.md#titles-captions-and-speech-budgets): every bound with its constant, the model sizes, and the measurements with their conditions.
  - [Usage](../../../docs/usage.md): titles, animation, graphics, presets, captions, captions from speech and translation.
  - [Installation](../../../docs/installation.md): CMake, a C/C++ compiler and libclang per platform, and a Speech recognition section.
  - [Agent protocol](../../../docs/agent-protocol.md): the title, caption and graphic commands.
  - Also [native AI](../../../docs/native-ai.md), [design](../../../docs/design.md) and the README.
  - [Verification](../../../docs/verification.md#titles-captions-and-motion-graphics): where the evidence comes from, a twelve-row feature matrix, and what was not tried for any row.
- Reference review: each claim was checked against the code. Corrections made before commit:
  - the inspector path;
  - when the system font scan runs;
  - what the preset tests do;
  - what a kept look holds (no word times);
  - "one track per language";
  - model sizes shown as the panel shows them;
  - a line element's frame;
  - the split rule for plain titles;
  - the speech panel's labels and their order;
  - titles refuse effects, masks and blend modes but take rate changes;
  - the ADR said the model was built in; the recognizer is, and the model is downloaded.
- Defect found while driving the release build: generating captions again made a second track also called "English captions", so the track chooser listed two identical names.
  - Fixed: every new caption track takes a free name ("English captions 2") whether made by hand, by import, from speech or by translation.
  - Changing a language still renames a numbered track named for its old language.
  - Test: `a_second_caption_track_in_a_language_is_numbered`.
  - Native, rebuilt release binary, fresh copies of the project that already had "English captions": captions from speech made "English captions 2", and the chooser listed both names, at 1440x900 and at 760x700 (library shown in place of the Agent panel).
  - Still open: the timeline's track heads cut both names to "English ca…".
- Removed `write_samples` from the text tests. It was an `#[ignore]`d development aid that wrote PNGs and asserted nothing.
- Final runs on the 4 vCPU Xeon container (`dev` profile):
  - editor-core 165, editor-store 78, editor-agent 86 (with the provider fixture), desktop 672 and `neptune-video-agent` 12, all passed;
  - editor-media 247 of 248. The failure is `decode::tests::a_paced_walk_backwards_at_preview_size_is_measured`, which fails the same way on the base commit and was left as it is;
  - fmt, clippy with `-D warnings` on the changed packages, and the architecture check (and its self-test) are clean.
- Release build: `cargo build --release --locked --bin neptune-video` from clean took 7 min 44 s, and 3 min 9 s again after the name fix (34,148,904 bytes). The binary is 34,147,232 bytes, with about 1.7 MB of whisper.cpp and ggml symbols. Driven under Xvfb: the Karaoke box look drawn, and captions from speech (Tiny, language detected) appeared between 4.2 and 4.9 s after the click.
- Not verified: CI (this branch has not run it), macOS, Windows, a hardware GPU, a physical display, playback rates, real recordings and real provider translations. See the verification section.
