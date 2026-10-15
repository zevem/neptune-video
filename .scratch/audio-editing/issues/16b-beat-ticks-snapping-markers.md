# Beat ticks, snapping, markers and phrases

Status: ready-for-human
Progress: resolved
Type: task
Blocked by: 04, 16a

## Scope

Show a music clip's beats on its tile and let edits snap to them. The grid
is derived and never stored.

- `src/peaks.rs`: beat grids for at most 8 assets, held in memory, least
  recently used dropped; "show beats" is session state per asset. A grid is
  asked for when its asset's peak file exists and taken only when the
  request is outstanding and the project and source match, as peaks are.
- `src/ui/timeline.rs`: "Show beats" and "Hide beats" in the clip menu of
  an audio-lane clip. Ticks in `paint_clip` at each beat inside the tile,
  mapped from source time through the clip (skipped where ticks would be
  closer than 4 points). `snapped()` (`:459`) adds the beats of visible
  clips that show them as candidates, beside the playhead, excluding the
  clips being moved. Beats on a freeze, reverse or ramp segment
  (`audible_at` false) are neither drawn nor snapped to.
- "Markers from beats": one `Batch` of `AddMarker` for the beats inside the
  clip, every beat or every second or fourth so that at most 256 are
  written, labelled "Beat".
- `src/agent.rs`: `show beats`, `markers from beats`.

Files: `src/peaks.rs`, `src/ui/timeline.rs`, `src/ui/mod.rs`, `src/app.rs`,
`src/agent.rs`, `docs/usage.md`.

## Result

Implemented as scoped, apart from the native captures, which need a hand on
the running build (see Evidence). The grid is derived: nothing about beats
enters the project, its history or a file, except the markers a person asks
for, which are ordinary `AddMarker` commands in one `Batch`.

- `src/peaks.rs`: `HeldBeats`, held as `HeldPeaks::beats` so that every
  route that already reaches the held peaks (the timeline, `ask`,
  `answered`, `begin_frame`, `clear`, `media_replaced`) reaches the beats
  without a new parameter. Its seam: `show(asset, shown)`, `shown`,
  `want(asset)` (a tile drawn), `ask_with(project, kept, ask)` (called by
  `HeldPeaks::ask` with "the peaks are held" as `kept` and
  `SoundService::beats` as `ask`), `take(…)` (the answer, through
  `HeldPeaks::answered`), `grid`, `state` (`BeatState::{Hidden, Finding,
  Held, Unclear, Failed}`, for the menu's words), `markers(project, clip)`,
  `forget_asset` and `give_up_wanted`. At most `MAX_HELD_GRIDS` (8); the
  least lately shown is dropped when a ninth comes, and none is asked for
  while eight are on screen or on their way, so nine tiles in view do not
  ask the lane without end.
- `beats_through(clip, grid, from, to)`: a grid's beats on the timeline
  through a clip, over the straight forward runs `sound::audible_run`
  gives (a clip without a time map is one run at its rate), by binary
  search in the grid. `beat_markers(project, clip, grid)`: the `Batch`
  labelled "Markers from beats" of `AddMarker`s labelled "Beat".
- `src/ui/timeline.rs`: `beats_drawn` wants the grids of the sound tiles
  drawn whose media's beats are shown and gives the beats in view, clip by
  clip, once a frame (`TimelineState::beats`), for a clip on an audio lane
  that `sounds`. `beat_ticks` leaves out a beat closer than `BEAT_GAP` (4)
  points to its neighbour. `paint_beats` draws them as one mesh, a point
  wide and `BEAT_TICK` (7) points tall at the foot of the tile's sound, in
  the fade curves' ink; a clip being moved draws its ticks where it would
  land. `snapped()` takes the beats as candidates beside edges, markers and
  the playhead: every route that snaps (move, trim, transition handle,
  fade handle, the ruler's playhead, a library item dropped) takes them,
  and a move leaves out the beats of every clip moving with it. The clip
  menu of an audio-lane clip has "Show beats" or "Hide beats", and
  "Markers from beats" while they are shown, disabled until they are held.
- `src/ui/mod.rs`: `Action::ShowBeats { assets, shown }` and
  `Action::MarkersFromBeats(ClipId)`, the clip captured when it is made.
- `src/app.rs`: both actions; a lane that cannot start gives up the beats
  wanted (said once with the waveforms); a `Beats` answer goes to
  `HeldPeaks::answered`, which says "No clear beat was found in …" or "The
  beats of … were not found: …".
- `src/agent.rs`: `show beats`, `hide beats`, `markers from beats` (and
  `mark beats`), and an example line.
- `docs/usage.md` (a Beats section), `docs/design.md` (the tick),
  `docs/performance.md` (the held-grid bound).

Settled while building, beyond what the scope says:

- *Heard is the whole of `audible_at`.* Ticks, snap candidates and markers
  are only where the clip is heard: a muted, soloed-out or 0% clip shows
  none, as well as none in a freeze, a reverse or a ramp. The spec has beat
  ticks ask the same function as the mixer; the waveform under them is
  still drawn.
- *Trims, fades and transitions land on the clip's own beats.* Only a move
  leaves out the beats of the clips it moves, which move with them; a trim
  keeps every beat where it is, so trimming music to its own beat works.
- *"Too close" drops both.* A tick is drawn when it is at least 4 points from
  each neighbour, so the ticks do not change as the view scrolls; at a
  zoom where the beats are under 4 points apart none is drawn.
- *More than 1,024 beats* go on halving: every eighth, sixteenth and so on,
  so "at most 256" holds for any clip. A beat that already has a marker at
  that exact time is passed over; when all have, nothing is written and it
  is said.
- *"The peak file exists" is read as "the peaks are held".* The lane finds
  beats in the cache's file; peaks read but not kept (the cache refused
  them) answer "not read" in words.
- *Shown is kept across a relink, not across a project replace;* shown
  again, beats that were unclear or not found are asked for anew.

## Evidence

On macOS 26.5.2 (arm64, Apple M1), Rust 1.97.1, Homebrew FFmpeg and
ffprobe 8.1.2 on `PATH` (`scripts/check-media-tools.py` passed),
2026-10-14, `dev` profile. No native window was opened and nothing was
listened to.

- `cargo test -p aster-editor --locked`: 779 and 12 passed. The beat tests
  (`peaks::`, `timeline::beating`, `agent::tests::beats`): 31 passed, three
  runs in a row. `cargo clippy -p aster-editor --all-targets --locked -- -D
  warnings`, `cargo fmt --all -- --check` and
  `scripts/check-architecture.py` pass. No other crate changed.
- Snap, through real egui passes (`src/ui/beats_tests.rs`):
  `a_trimmed_edge_within_reach_of_a_beat_lands_on_it_and_shows_the_guide`
  (the guide on the beat while held, one step, the end at 5.75 s; the
  ruler's playhead lands on 3.25 s);
  `a_moved_clip_lands_on_another_clips_beat_and_never_on_its_own`;
  `with_its_beats_hidden_nothing_lands_on_them` (a frame instead).
- Ticks: `a_tile_at_twice_the_speed_ticks_at_half_the_spacing_and_a_trimmed_one_only_inside`
  (and none under 4 points apart);
  `a_tile_whose_beats_are_shown_draws_a_tick_at_each_while_it_is_heard`
  (12 ticks in one mesh, none muted, none at 7 points a second);
  `beats_fall_through_the_clip_where_it_plays_forward` (a window, a trim,
  rate 2, none in a freeze, a reverse or a ramp).
- Markers: `a_thousand_beats_are_marked_at_most_256_times_in_one_step`
  (256, 257, 512, 1,000 and 2,000 beats give 256, 129, 256, 250 and 250
  markers; one history entry, Undo takes them all, asked again nothing is
  written); `beat_markers_are_saved_and_read_back_as_written`
  (`editor_store` round trip in a scratch directory);
  `markers_from_beats_are_refused_in_words_where_no_beat_is_shown`; the
  menu writes them as one step in
  `the_menu_of_an_audio_lane_clip_shows_and_hides_beats_and_marks_them`.
- Held grids: `a_ninth_grid_drops_the_one_least_lately_shown_and_waits_while_eight_are_on_screen`;
  `beats_shown_are_asked_for_once_their_peaks_are_kept_and_taken_only_as_asked`
  (another request, project or source is dropped);
  `beats_that_are_unclear_or_not_found_are_said_once_and_asked_again_when_shown_again`;
  `a_project_replaced_shows_no_beats_and_replaced_media_is_asked_again`.
- Through the real lane: `the_beats_shown_are_found_by_the_lane_in_the_peaks_it_keeps`
  (a WAV the test writes: the beats are asked after its peaks are kept,
  answered once, "No clear beat was found in tone.wav.").
- Phrases: `beats_are_shown_and_marked_for_the_selected_clips_on_audio_tracks`.

Not done, and why this is not resolved: **no native capture was taken.**
Beats are shown only from the clip menu or the command panel and a snap
guide only under a held drag, so neither is reachable by a `--screenshot`
launch without a pointer, and nothing was clicked in a running build.
Unseen with them: whether a 7-point tick reads over a loud waveform, the
light theme, real music of any kind (only grids written by the tests were
drawn), the cost of a frame with many shown tiles (not measured), Linux
and Windows.

For a person, at 1440x900 and 760x620, with music on an audio track:
choose Show beats in its clip menu and capture the ticks at two zooms; hold
a trim near a beat and capture the guide on it.

Done when:

- Snap tests: a clip edge within reach of a beat lands on it; a moved
  clip's own beats are not candidates; with beats hidden nothing changes.
- Tick tests: a clip at rate 2 shows beats at half their source spacing; a
  trimmed clip shows only the beats inside it.
- Markers: 1,000 beats in the clip write at most 256 markers in one
  entry.
- Held-grid test: a ninth asset drops the least recently used grid.
- Native captures, normal and narrow: ticks at two zooms, a snap guide on a
  beat.

## Known limits

- No manual nudge of the grid.
- Nothing cuts picture to the beat; snapping is the whole of
  "synchronization".
- The grid is recomputed each session and is not visible to an agent or the
  headless CLI; markers are.
- The detector's limits are ticket 16a's.
- A clip that is not heard (muted, soloed out, at 0%) shows no ticks and
  gives no snap or markers until it is heard again.
- Only beats in view are snapped to; ticks under 4 points apart are not
  drawn or snapped to, so a zoomed-out timeline has none.

## Comments

2026-10-13: Written with the spec. Nothing implemented or measured.

2026-10-13 (review): Split from the former ticket 16.

2026-10-14 (implemented): Built and tested headless on macOS 26.5.2; the
captures are a person's to take. What was settled is under Result and in
the spec's Comments.

2026-10-14 (review fixes): A grid held back because eight were still
counted on screen now brings about the frame that asks for it:
`HeldBeats::room_is_due` is true when one was passed over for room while a
held grid was on screen in the last frame and not in this one, and
`HeldPeaks::room_is_due` (which the window repaints on) includes it. With
all eight still on screen it stays false, so a waiting grid does not keep
the window drawing. Before, a ninth asset scrolled into view in the frame
another left stayed "being found" until unrelated input drew a frame.
Checked in
`a_ninth_grid_drops_the_one_least_lately_shown_and_waits_while_eight_are_on_screen`.
The implemented entries here and in the spec were re-dated to 2026-10-14,
the day the checks ran.

2026-10-14 (native acceptance): Resolved. The captures this ticket waited for are taken and look right. Release build `target/release/aster` (sha256 fadbe5aa8244d800b2d9b16bf86bf121a6e9157af7e8067402761d40318e5763, `cargo build --release --locked`) on macOS 26.5.2 (25F84), Apple M1, Rust 1.97.1, Homebrew FFmpeg 8.1.2, built-in Retina display at scale 2. Fresh task-owned projects (made by `aster-agent` over ffmpeg-generated media: a 12 s testsrc2 interview with a speech-like tone, 30 s of 120 BPM clicks, a 0.8 s whoosh) and fresh `--settings` per run. Driven by real pointer and key events into the running window; the normal window (asked 1440x900) came up at 1280x709 points, the narrow one at 760x620 (and 760x560 for ticket 13). Captures are in `artifacts/audio-editing/captures/`. Every capture was looked at against `docs/design.md`. Dark theme only; Linux, Windows and the light theme were not run; nothing was listened to.

`normal-10`/`narrow-10` beat ticks after Show beats; `normal-11`/`narrow-11` at a second zoom; `normal-31` the menu with Hide beats and Markers from beats; `normal-12`/`narrow-12` a move held with the guide on a beat; `normal-44`/`narrow-41` a trim held near a beat with the guide on it. Only the synthetic click track was drawn.
