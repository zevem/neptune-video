# Verify the complete core flow on native desktops

Status: ready-for-agent
Progress: open
Type: task

## Task

Run the complete [feature acceptance](../../../docs/verification.md#implemented-feature-matrix)
on owned fixtures/projects after the final build. Record Linux X11 and Wayland
separately; macOS and Windows require their own actual hosts. Verify every
[spec criterion](../spec.md), keyboard/pointer/drop routes, decode/audio behavior,
reverse actions and failure states. Capture normal and narrow windows.

## Completion criteria

Each criterion has a reproducible successful observation or an explicit defect
with a focused follow-up. Evidence states host, source revision, profile,
GPU/backend, window scale, fixtures and artifact paths. Update the platform
matrix without implying that CI compilation established native behavior.

## Comments

2026-10-12: Created as a production gate. No platform acceptance result is implied.

2026-10-12: Linux X11 passed all 13 final optimized real-input smoke checks and
native exit/relaunch retaining complete recovery history. Earlier captures
cover clipboard, checkpoints, duplication, library organization, recovery
ownership and nonmutating video/GIF/WAV/PNG source input. Weston has actual
URI-drop/timeout/close acceptance and a fresh optimized URI-drop result.
macOS/Windows, GNOME/KDE, full Wayland input, portal chooser and actual
file-manager-drop acceptance remain open; see
[verification](../../../docs/verification.md#native-linux-evidence).

2026-10-12: The unchanged smoke harness passed [all thirteen R15 native editing/input controls](../../../artifacts/native/round15-editing-smoke/root-verification.json)
on `dfbb53e9` / `362ba180`, including exact single-commit gesture reversals,
clipboard, locked tracks, frame/marker/split, rename/save, stale-plan refusal
and transport/settings. Root viewed five 1440×900 captures and checked eleven
sampled process identities gone; fixture, media, script, source and binary were
unchanged. This is a Linux X11 subset with unavailable audio and no DRI3, not
complete input/platform, hardware-GPU, cache/multi-owner/wake cadence or fps
acceptance. R14 cache/overlap and broader narrow evidence remains historical;
the task stays open.
