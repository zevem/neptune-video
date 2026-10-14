# Title model, validation and schema v3

Status: ready-for-agent
Progress: resolved
Type: task

## Scope

`title.rs` types and defaults; `ClipContent::Text` gains style, enter, exit and words; `ClipContent::Graphic`; `Track.language`; `MediaKind::Font`; every bound in `Project::validate`; `title::word_ranges`; store writes schema 3 and reads 1 to 3.

## Comments

Done in fa1c1bc: `editor-core::title` (style, animation, words, graphics, track language), `MediaKind::Font`, schema 3 reading 1 and 2; tests in `crates/editor-core/tests/titles.rs` and the store's round trips.
