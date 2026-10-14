# SRT/VTT and cue chunking

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 01

## Scope

Pure parse and format of SRT and WebVTT (with VTT inline word timestamps), refusing malformed input whole; `captions::cues` groups timed words into cues by characters per line, lines, duration and pauses.

## Comments

Done in 0d7c979: `editor-core::captions` (SRT/VTT parse and format, VTT word tags, cue chunking presets); `crates/editor-core/tests/captions.rs`.
