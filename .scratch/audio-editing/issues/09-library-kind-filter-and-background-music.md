# Library kind filter and background music

Status: ready-for-human
Progress: resolved
Type: task
Blocked by: 05a

## Scope

Find sound quickly and lay a music bed in one action. No schema change, no
new command, no bundled content.

- `src/ui/library.rs`, the Media tab (`:100`): a kind filter (All, Video,
  Sound, Images) above the grid, session state, combined with the folder
  view and the sort.
- Asset context menu (`:1007`) on an asset of kind `Audio`: "Add as
  background music". One `Command::Batch` labelled "Add background music":
  `AddTrack` of an audio track named "Music" (always a new track, placed
  last), then `AddClip` of the asset end to end from zero until the
  project's duration is covered, the last clip trimmed to the end. Every
  clip has volume 0.5; the last has `fade_out` of 2 s or its own length if
  shorter. Ids are made when the action is made.
- Refused in words when the project is empty, when the bed would need more
  than 256 clips, or when the asset is shorter than a tenth of a second.
- `src/agent.rs`: `add background music NAME`.

Files: `src/ui/library.rs`, `src/ui/mod.rs`
(`Action::AddBackgroundMusic(AssetId)`), `src/app.rs`, `src/agent.rs`,
`docs/usage.md`.

## Result

Implemented; one piece of evidence is missing, the native capture of the
menu item, which needs a pointer and is a person's to take.

- `rules::background_music(project, asset) -> Result<Command, String>`
  builds the batch and `rules::bed_refusal` says why it would not; both are
  pure and in `src/rules.rs`, which `src/app.rs` (`App::background_music`,
  beside `detach_audio`), the library menu and the command line all ask.
- Routes: the context menu of a library asset of kind `Audio` (the row is
  shown disabled, with the reason as its hover text, when the bed is
  refused), the phrases `add background music NAME` and, for the sound
  selected in the library, `add background music`, and the batch as
  serialized commands. No shortcut, no project-menu row, no timeline gesture
  and no inspector control: the ticket lists none.
- `Action::AddBackgroundMusic(AssetId)` carries the asset; the track and clip
  ids are made when the action is carried out, in the same frame, as ticket
  02 reads the same sentence. The selection is left where it was.
- The filter is `KindFilter` in `src/ui/library.rs`, held in
  `LibraryState::kind`: a 22-point row of four under the search, drawn by
  the function that draws the library's tabs (`segments`, the old `tabs`
  made generic). It is kept across a project replace, as the tab is, and in
  no file. What the tab lists is the pure `listed`, and the words of an
  empty list are `nothing_listed`.
- A fourth refusal the scope does not list: media that is not a sound file
  ("Background music is made of a sound file."), which the phrase can name.

## Evidence

On macOS 26.5.2 (arm64), Rust 1.97.1, Homebrew FFmpeg and ffprobe 8.1.2
(`scripts/check-media-tools.py` passed), 2026-10-13. No test was skipped.

- `cargo test --locked --bin aster`: 689 passed (after review). `cargo test --locked --bin
  aster -- rules:: agent::tests library::`: 51 passed (after review).
  `cargo clippy --locked --bin aster --bin aster-agent --all-targets -- -D
  warnings` and `cargo fmt --all -- --check` are clean;
  `scripts/check-architecture.py` passes. Only the desktop crate changed, so
  no other crate was tested.
- `src/rules.rs`, four tests. `a_bed_repeats_the_sound_to_the_projects_end_as_one_step`:
  a 10 s sound under a 25 s project is clips of 10, 10 and 5 s on a last
  track named "Music", each at 0.5 with `source_in` 0, the last alone with a
  2 s fade; the result validates, is one history entry labelled "Add
  background music", undo gives the project back whole and redo the same
  bed; a second bed is a second track. `a_bed_ends_with_the_project_and_fades_out_there`:
  a longer sound is one trimmed clip; a last clip of 1 s fades over 1 s; a
  sound that divides the project is not cut and still fades; the length is
  the last clip's end on any track. `a_bed_that_cannot_be_made_is_refused_in_words`:
  an empty project, 257 clips (the bed of 256 is applied, validates, is one
  history entry, ends with the project and is undone whole), 0.099 s (0.1 s
  is taken), a video, and media that is gone. `a_bed_that_fails_changes_nothing_and_one_that_holds_is_saved_and_read`:
  a batch whose sound left the library since is refused whole with no
  history entry; the batch read back from JSON is the same edit; a store
  save and load keeps the bed.
- `src/ui/library.rs`, four tests, two through real egui passes of the Media
  tab with pointer events. `the_kind_filter_lists_media_by_its_kind_in_the_folder_in_view`:
  each filter by `MediaKind` at the top and inside a folder, folders kept,
  a search across folders within the kind, the sort over what is left, every
  kind under exactly one filter and a LUT under none.
  `nothing_of_a_kind_says_so_and_where`: the words. `a_click_on_a_kind_hides_the_rest_and_an_empty_result_says_so`:
  the four cells on one row inside a 320-point panel; a click on each hides
  and shows the tiles; Images over a library without one draws "No images"
  and the row stays; no action is asked. `a_sounds_menu_offers_background_music_and_a_pictures_does_not`:
  a secondary click on a sound opens the menu with the row, choosing it asks
  `AddBackgroundMusic` of that asset, on an empty project the row is drawn,
  the pointer resting on it draws the reason, and a click on it asks
  nothing, and a video's menu has no such row.
- `src/agent.rs`: `a_sound_is_laid_under_the_project_by_name` (the listed
  example, any case, the selected sound when unnamed, each refusal, one
  step). `of_media_with_one_name_the_bed_is_of_the_sound`: a video, a sound
  too short and a usable sound sharing a name but for case; the usable sound
  is taken, and without it the refusal is the short sound's.
  `every_listed_example_is_understood` passes over this example, as
  it does over `follow`: it needs a sound in the library.
- Not tested: the `Action::AddBackgroundMusic` arm of `App::perform` with
  its draft commit and toast, and the kind kept across a project replace.
  `App` is not built in a unit test.
- `target/debug/aster-agent --project … --create` took the bed as an agent
  writes it, one batch of `add_track` and three `add_clip`, over generated
  fixtures (25 s video with sound, 10 s and 0.6 s WAV, a PNG): applied as
  "Add background music", undone to one track fewer, redone to the same bed
  (`artifacts/audio-editing/09-background-music/headless-agent.jsonl`).
- Native captures of the release build (`target/release/aster`, sha256
  `c4ad00dcd1bbffd2…`, built from the final source), each run with its own
  settings, session and copy of that project under
  `artifacts/audio-editing/09-background-music/<run>/capture.png`:
  `bed-normal` (asked for at 1440x900, which came up at 1280x677 points on
  this display), `bed-narrow` and `filter-narrow` (760x620). All three were
  looked at. `bed-normal` and `filter-narrow` show the row All, Video,
  Sound, Images under the search with All chosen, whole and unclipped;
  `bed-normal` shows the Music track last with three `song.wav` tiles end
  to end to 0:25, the volume line at half and the fade drawn on the last.
  At 760x620 the window shows one side panel, so `bed-narrow` has the
  inspector and no library, and in both narrow captures the Music track is
  the last row and is cut by the window's bottom edge, its tiles' top half
  in view. Every process exited by itself; none was left.

Not done, and why this is not resolved: nothing was clicked or typed in the
running build, so **no capture shows the menu item**, a filter other than
All chosen, or the empty-result words. The bed in the captures was made by
the headless batch, not by the menu. Nothing was listened to: that the bed
is heard at half volume and fades is ticket 06's readback of `level_at`,
not a listening test. Linux and Windows were not run.

For a person, at 1440x900 and 760x620: right-click a sound in the library
and capture the menu (row enabled), the same on an empty project (row
disabled, with its reason), choose the row and confirm the bed; choose
Sound and Images in the filter and capture each.

Done when:

- Builder tests: a 10 s asset under a 25 s project gives clips of 10, 10 and
  5 s, the last with `source_in` 0, duration 5 s and a 2 s fade; an asset
  longer than the project gives one trimmed clip; the batch validates and is
  one history entry; each refusal.
- Library test: the filter hides and shows by `MediaKind`, and an empty
  result says so.
- Native captures, normal and narrow: the filter, the menu item, the bed on
  the timeline.

## Known limits

- The bed is as long as the project when the action runs; it does not follow
  later edits.
- Repeats are hard cuts at the file's ends; no loop-point detection.
- A bed spends a voice wherever it sounds. With picture sound, voiceover and
  effects, sixteen simultaneous clips can be reached and the whole mix then
  fails with a message.
- No sound or music is shipped, downloaded or generated.
- A bed is of a sound file only. A video's sound is detached first, and the
  detached clip is still of a video asset, so it makes no bed.
- A last clip is as short as what is left of the project, down to a
  microsecond; it is not held to a frame, and the clips' starts are
  multiples of the sound's length, not frames.
- The filter is by the asset's kind: Sound does not list a video that has
  sound. A LUT is listed under All only.
- Under a filter a folder is listed whether or not it holds media of that
  kind, and its item count is of all its media.
- Of several usable sounds with one name the phrase takes the first the
  library holds, which is by id and not by anything a person sees; the menu
  names the one meant.

## Comments

2026-10-13: Written with the spec. Nothing implemented or measured.

2026-10-13 (implemented): Built and tested on macOS as recorded under
Evidence. Read as ticket 02 reads it, "ids are made when the action is made"
is: the action carries the asset's id and the track and clips are made when
it is carried out. Settled here and noted in the spec: a GIF is under
Images and a LUT under All alone; the folders stay listed under a filter; a
bed is refused of media that is not a sound file; the phrase without a name
takes the sound selected in the library. `docs/design.md` is not changed:
the ticket does not list it and ticket 22 owns it. Left `claimed` for the
menu capture.

2026-10-13 (after review): Four findings, three fixed in code and tests.
The phrase now takes, of several media with the name, the one a bed can be
made of, then a sound file, then the first (it took the first of any kind
and could refuse a sound that was there). The 256-clip bed is now applied
and undone in its test, and the disabled row's hover reason is drawn in the
menu test; removing either the hover text or the new choice fails a test.
The fourth stands as it was: the menu, the disabled row and a chosen filter
are not captured natively, so the status stays `ready-for-human` /
`claimed`. After the review `cargo test --locked --bin aster` is 689 passed
on the same host, with clippy (`-D warnings`), `cargo fmt --check` and
`scripts/check-architecture.py` clean; the release build and its captures
are from before these changes and were not remade.

2026-10-14 (native acceptance): Resolved. The captures this ticket waited for are taken and look right. Release build `target/release/aster` (sha256 fadbe5aa8244d800b2d9b16bf86bf121a6e9157af7e8067402761d40318e5763, `cargo build --release --locked`) on macOS 26.5.2 (25F84), Apple M1, Rust 1.97.1, Homebrew FFmpeg 8.1.2, built-in Retina display at scale 2. Fresh task-owned projects (made by `aster-agent` over ffmpeg-generated media: a 12 s testsrc2 interview with a speech-like tone, 30 s of 120 BPM clicks, a 0.8 s whoosh) and fresh `--settings` per run. Driven by real pointer and key events into the running window; the normal window (asked 1440x900) came up at 1280x709 points, the narrow one at 760x620 (and 760x560 for ticket 13). Captures are in `.scratch/audio-editing/captures/`. Every capture was looked at against `docs/design.md`. Dark theme only; Linux, Windows and the light theme were not run; nothing was listened to.

`normal-01`/`narrow-33` the kind filter on All; `normal-36`/`narrow-36` Sound chosen; `normal-37`/`narrow-37` Images chosen with "No images / Nothing of that kind is in this project."; `normal-34`/`narrow-34` a sound's menu with Add as background music enabled; `normal-35`/`narrow-35` the bed it made on a Music track, fading out at the end; `normal-39`/`narrow-43` the row disabled on an emptied timeline with its reason. Seen and left: a right-click on the centre play button of a sound tile opens no menu; elsewhere on the tile it does.

2026-10-14 (fix): A right-click on a sound tile's centre play offer now
opens the tile's own menu; a click still previews. See the spec's Comments
of the same date; captures `fix-normal-04`, `fix-normal-05`,
`fix-narrow-04`, `fix-narrow-05` of the rebuilt release.
