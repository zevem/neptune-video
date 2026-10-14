# Commands, realignment and rebasing

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 01

## Scope

`SetTextStyle`, `SetTextAnimation`, `SetWords`, `SetGraphic`, `SetTrackLanguage`, `CopyCaptions`; `SetText` keeps style and realigns words; split, trim, rate and linked retime rebase words and animations.

## Comments

Done in fa1c1bc: the six new commands, realignment kept one for one, split/trim/rate rebasing of words and animations; `crates/editor-core/tests/titles.rs`.
