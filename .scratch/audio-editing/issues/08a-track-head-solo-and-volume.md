# Track head solo and volume

Status: ready-for-human
Progress: resolved
Type: task
Blocked by: 05a, 06

## Scope

Track-level mix controls in the head.

- `src/ui/timeline.rs`, `track_heads` (`:838`): one more `head_toggle` cell,
  solo, on every track that shows mute (all but text), sending
  `SetTrackState { solo: Some(..) }`. A head whose track is silenced by
  another's solo draws its mute cell dimmed. The head's context menu
  (`:986-998`) gains "Volume" with a percent field (0 to 800) sending
  `SetTrackState { gain: Some(..) }` on commit, and "Reset volume". A head
  whose volume is not 100 % shows the figure under the name when the lane is
  tall enough.
- `src/agent.rs`: `solo track NAME`, `unsolo track NAME`,
  `set track volume NAME 50%`.

Files: `src/ui/timeline.rs`, `src/ui/mod.rs`, `src/agent.rs`,
`docs/usage.md`, `docs/design.md` (track controls line).

## Result

Implemented as scoped, with one departure in the layout of a video head and
apart from the native capture of the open menu, which needs a hand on the
pointer. Solo and track volume are reached from the head, from its menu and
from the command panel, and each ends in `SetTrackState` through
`Action::Edit`, so each is validated by the project, is one history entry
and undoes. The serialized `set_track_state` is ticket 05c's and is
untouched; no shortcut and no row of the project menu is added, the ticket
names none; save and load are the fields of 05a and 05b.

- `src/ui/tracks.rs` (new): what the routes share, and nothing that draws.
  `carries_sound` (every track but text), `percent`, `figure`, `MOST` (800),
  `solo`, `volume` (held to 0..=800 percent, none for a number that is none)
  and `silenced_by_solo`.
- `crates/editor-core/src/sound.rs`: `soloed_out(project, track)`, the solo
  part of `sounds` alone, public so that the head asks the rule the mix
  uses and does not repeat it. Not in the ticket's file list.
- `src/icons.rs`: `Icon::Headphones`, the solo cell's glyph. Not in the
  ticket's file list.
- `src/ui/timeline.rs`: `head_rect`, `head_toggles`, `head_lines`,
  `head_cell` and `head_name_room` are the head's arithmetic; the solo
  cell; the dimmed mute cell; the figure under the name; `volume_menu`.
- `src/agent.rs`: `solo track NAME`, `unsolo track NAME`,
  `set track volume NAME 50%` and an `EXAMPLES` row (24 rows).
- `docs/usage.md`, `docs/design.md`. `src/ui/mod.rs` only names the module.

Settled while building, beyond what the scope says:

- **A video head has its four cells in two columns, not in a row.** Side by
  side, four cells leave the name 20 points of a 156-point head, and
  "Video 1" is wider than that (asserted with the head's own font). Hide
  stands under lock and solo under mute, each a whole 22-point cell, and the
  name has 66 points. An audio head has lock, mute and solo in one row and
  43 points of name, what a video head had before. A four-cell head too low
  for two rows would fall back to one row; no track kind is that.
- The solo cell is a pair of headphones, in the accent when the track is
  soloed. It is named "Solo track" and "Unsolo track".
- The dimmed mute cell is at 45% of the off colour and is named "Mute track
  (silenced by a solo)", so the state is read out and not only seen. A
  muted track shows its mute and is not dimmed: mute wins. It still mutes
  when clicked.
- The menu's field holds the volume the track has and reads it to a
  hundredth of a percent, without trailing zeros; the head's figure reads
  the same. A volume set from the field or a phrase is kept to that
  hundredth, so what is entered is what is then read. A typed volume is
  taken on Return (or when the focus leaves the field) and closes the menu;
  Escape gives it up. A drag is one entry when let go, and none when Escape
  ends it with the button down; an Up or Down arrow is one entry to the next
  whole percent; after a drag or an arrow the menu stays. A field left as it
  was is no edit; one given a number that would keep a different volume is,
  so a track at 0.4% is silenced by typing 0. "Reset volume" is disabled at
  100%. (Changed after review; see Comments.)
- The figure is written when the volume is not exactly 1 and the head is at
  least 40 points tall; every head that carries sound is.
- The phrases refuse a text track ("A text track has no sound to solo.",
  "A text track has no sound to set the volume of.") and a volume outside
  the bound ("A track's volume is from 0% to 800%."), where the menu's
  field holds to the bound. `set track volume Audio 1` without a percentage
  is told what to say and is not read as a track "Audio" at 1%.
- Solo is durable and in the history: owner decision 4 is unanswered and
  the spec's default is what is built.

## Evidence

On macOS 26.5.2 (arm64), Rust 1.97.1, Homebrew FFmpeg and ffprobe 8.1.2
(`scripts/check-media-tools.py` passed), 2026-10-13. No test was skipped.

- `cargo test --locked --bin aster`: 658 passed. `cargo test --locked --bin
  aster -- heads:: tracks:: agent::tests`: 34 passed, three runs.
  `cargo test -p editor-core --locked`: passed, with one new test.
  `cargo clippy --locked --all-targets -- -D warnings` on the desktop crate
  and on `editor-core`, and `cargo fmt --all -- --check`, are clean;
  `scripts/check-architecture.py` passes.
- `src/ui/heads_tests.rs`, nine tests, eight of them through one real egui
  pass of the timeline a frame with the cells, the menu rows and the field
  found by their accessible names: the arithmetic of each kind's head (cells
  22 points, inside the head, not overlapping, 23 apart, the default name
  and "800%" within the room, the two columns of a video head and why);
  an audio head has three cells, a video head four and a text head lock and
  hide, each where the arithmetic puts it; each of the four toggles is one
  entry, changes that field only and undoes by itself; a locked track is
  soloed from its head; over a sequence of solos and mutes each head is
  dimmed exactly where `sound::sounds` is false for an unmuted track, and a
  soloed text track dims nothing; a typed 999 is no entry until Return and
  is then one `SetTrackState` at 8, a typed value equal to what the field
  reads and one given up with Escape are none, arrows are one each, Reset
  volume is one; a drag is no entry while held and one when let go; a text
  head's menu has no volume; the figure is written under the name, clear of
  the cells, only where the volume is not 100%.
- `src/ui/tracks.rs`, four tests: the bound, NaN and infinity, one entry and
  undo for volume and for solo, the figure, and `silenced_by_solo` against
  `sound::sounds` over all 32 settings of solo and mute.
- `crates/editor-core/tests/sound.rs`: `soloed_out` over the same 32.
- `src/agent.rs`: `a_track_is_soloed_and_given_a_volume_by_name`;
  `every_listed_example_is_understood` covers the new row.
- Native captures of the release build (`target/release/aster`, sha256
  `2db5451402cb84c1…`, built from the final source), each run with its own
  settings, session and project under
  `artifacts/audio-editing/08a-track-heads/<run>/`, on owned fixtures made
  by FFmpeg: `rest-normal`, `mixed-normal`, `audio-normal` (asked for at
  1440x900, which came up at 1280x678 points on this display) and
  `rest-narrow`, `mixed-narrow`, `audio-narrow` (760x620). Looked at:
  `rest-normal`, `mixed-normal`, `mixed-narrow` and `audio-narrow`. They
  show an audio head with three cells and its name whole, a video head with
  four in two columns, a soloed head's headphones in the accent, the dimmer
  mute of the heads a solo silences beside a muted one, and "50%", "150%"
  and "800%" under the names. Every process exited by itself; none was left.

Not done, and why this is not resolved: nothing was clicked or typed in the
running build, so **no capture shows the volume menu**, a hovered or focused
cell or a tooltip. The field's alignment in the menu, the dimmed cell's
contrast in the light theme, Linux and Windows are unseen. Nothing was
listened to: that a solo or a track volume is heard is ticket 06's readback,
not a listening test.

For a person, at 1440x900 and 760x620: right-click an audio head and a video
head and capture the menu; type a volume and enter it.

Done when:

- Head tests: the solo cell fits a 156-point head beside lock and mute (the
  layout arithmetic asserted); each toggle is one entry; a text track has no
  solo cell; the dimmed state follows the same solo rule as
  `editor_core::sounds`.
- Menu test: a committed volume is one `SetTrackState` held to 0..=8.
- Agent phrase tests.
- Native captures, normal and narrow: an audio head with three cells, a
  video head with four, the volume menu.

## Known limits

- A volume a file or a serialized agent command holds finer than a hundredth
  of a percent reads rounded in the menu's field (typing what it reads sets
  it to that). The head's figure writes such a volume to six decimals where
  two would say "0%" or "100%"; that figure may be clipped by the head, and
  a volume under a millionth of a percent still reads "0%".
- No pan, no meter, no track automation, no track effects.
- Track volume lives in a menu, not a slider: the head is 156 points wide.
- Percent, not dB.

## Comments

2026-10-13: Written with the spec. Nothing implemented or measured. Whether
solo should be session state is owner decision 4 in the spec.

2026-10-13 (review): Split from the former ticket 08; the volume line and
the keyframe lane change are ticket 08b.

2026-10-13 (implemented): Built and tested headless on macOS 26.5.2 (arm64,
Rust 1.97.1), with six native captures of the heads; the capture of the open
menu is still to be taken, and nothing was listened to. A video head's four
cells stand in two columns, which the ticket did not ask for: details are
under Result and Evidence.

2026-10-13 (after review): Two findings held and two rules changed, tested
headless on the same host. The field and the figure read what is kept, to a
hundredth of a percent, where they read whole percent: a track at 0.4% said
"0%" and could not be set to 0 from its field, and one at 100.4% said
"100%". `tracks::volume` now keeps a volume to that hundredth, for the field
and the phrase alike. Escape during a drag of the field gives the drag up;
it was entered as an edit. Three head tests and two in `tracks.rs` are new;
the head test that held a typed 12 over 12.4 to be no entry was wrong and is
replaced. No native capture or pointer run of the menu was made.

2026-10-14 (native acceptance): Resolved. The captures this ticket waited for are taken and look right. Release build `target/release/aster` (sha256 fadbe5aa8244d800b2d9b16bf86bf121a6e9157af7e8067402761d40318e5763, `cargo build --release --locked`) on macOS 26.5.2 (25F84), Apple M1, Rust 1.97.1, Homebrew FFmpeg 8.1.2, built-in Retina display at scale 2. Fresh task-owned projects (made by `aster-agent` over ffmpeg-generated media: a 12 s testsrc2 interview with a speech-like tone, 30 s of 120 BPM clicks, a 0.8 s whoosh) and fresh `--settings` per run. Driven by real pointer and key events into the running window; the normal window (asked 1440x900) came up at 1280x709 points, the narrow one at 760x620 (and 760x560 for ticket 13). Captures are in `.scratch/audio-editing/captures/`. Every capture was looked at against `docs/design.md`. Dark theme only; Linux, Windows and the light theme were not run; nothing was listened to.

`normal-01`/`narrow-01` audio heads with three cells and the video head with four; `normal-13`/`narrow-13` Audio 1 soloed and the others dimmed; `normal-14`/`narrow-14` an audio head's menu with Volume 100 % and Reset volume disabled; `normal-43`/`narrow-40` a video head's menu; `normal-15`/`narrow-15` the field clicked, menu kept; `normal-16`/`narrow-16` 70 typed; `normal-17`/`narrow-17` the head reading 70% under its name.

One defect was found in the running build and fixed: the head menu closed on any click inside it (egui's `CloseOnClick`), so the volume field could not be clicked to type into. The head menu now uses `egui::Popup::context_menu(..).close_behavior(PopupCloseBehavior::CloseOnClickOutside)`; every row already closes it itself. Test `a_click_on_the_volume_field_keeps_the_menu_and_takes_what_is_typed` (`src/ui/heads_tests.rs`) failed before and passes after. `normal-15..17` and `narrow-15..17` show it natively.
