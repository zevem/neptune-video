# Transition handles and effects/transitions browser

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 08, 10

## Scope

Transitions on the timeline and a library of effects, transitions and looks.

## Result

A transition is a marking on its clip's edge with a drag handle
(`src/ui/timeline.rs`, `src/ui/transitions.rs`); a selected marking shows
kind, duration, easing, the kind's value, Remove and Save as default in the
inspector (`src/ui/inspector/transition.rs`). `Action::AddTransitions` carries
the clip and edge captured when asked (Cmd+T, the clip menu, the "+" badge, a
dropped tile). The library has Media, Effects, Transitions and Looks tabs
(`src/ui/browser.rs`); a tile is given by double-click, Return or drag.

## Evidence

`src/ui/transitions_tests.rs`. Native at 1440x900: badge, handle drag 0.5 s to
1.5 s, kind change, remove, library double-click and drag; Ctrl+T at 760x560
(wave 3). The library opening at 760 was re-captured in wave 4.

The [verification matrix](../../../docs/verification.md#advanced-editing)
states the host, adapters and what is unverified on every platform.

## Known limits

- The spec's UI bullet lists three library tabs; there are four (Looks).
- No canvas preview while a handle or the Duration field is held.
- "Save as default" was not verified across a restart.
- Dragging a tile has no keyboard equivalent beyond selecting the clip and
  pressing Return on the tile; a transition's length by keyboard goes through
  the Duration field.
- A transition on a text clip is always a fade and shows no status for it.

## Comments

2026-10-13: Written by ticket 17 from the code as it stands and the
implementing tasks' reports. Verified on one Linux host only; macOS, Windows,
a hardware GPU in a windowed session, HiDPI and playback rates are unverified.
