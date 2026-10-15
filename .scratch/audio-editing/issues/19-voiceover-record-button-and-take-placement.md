# Voiceover: record button, take clock and placement

Status: ready-for-human
Progress: claimed
Type: task
Blocked by: 18

## Scope

Arm, speak, stop: a clip appears where the words were said. A take is as
long as the person speaks, whatever is on the timeline.

- `src/ui/mod.rs`: `Action::Record` and `Action::StopRecord`.
  `src/ui/viewer.rs`, the transport (about `:2095-2215`): a record button
  that shows its state, and the elapsed time and a small level bar while
  recording. Bare `R` toggles it (no `Key::R` shortcut exists in `src/`
  today; confirm in `shortcuts()` before taking it).
- Start (`src/app.rs`, `src/record.rs`): refuse in words when ticket 17's
  storage rule says so (for the proposed rule: an untitled project), when
  there is no input device, or when a take is running. Otherwise capture
  the project instance and the playhead, start `RecordService` at the path
  from ticket 17, then start playback from the playhead.
- The record state has its own clock. Elapsed time is
  `RecordService::elapsed()`, with a repaint asked for ten times a second
  while a take runs and none after. The transport stops itself at the
  project's end (`playback.rs:47-86`) and the audio worker goes quiet there
  (`audio.rs:681-689`); neither ends the take. The playhead then holds at
  the end, and in an empty project it never moves. Looping is not applied
  while recording.
- Anchor. The take is tied to the playback clock, not to the button press:
  the audio worker waits for the device and starts its decoders before any
  sample is heard (`audio.rs:621-647`), and the microphone is already
  running. On the first tick after start where `AudioService::position()`
  is `Some(p)`, keep the pair (`RecordService::frames()`, `p`). If playback
  did not start (empty project, playhead at the end) or no position has
  come within one second (no device, nothing audible: the wall clock is
  the clock), keep (`frames()`, the transport's position) on that tick.
- Stop (`R`, Space, Escape, the button, or a failure; not the end of the
  project): stop playback and ask `RecordService` to stop. When
  `RecordEvent::Finished(Take)` arrives, build the asset from the take
  itself, with no import and no ffprobe: `Asset::new(path,
  MediaKind::Audio, frames / rate)`, `has_audio` true, `file_size` from
  `Take::bytes`, the path as the service canonicalized it. Then one
  `Command::Batch` "Record voiceover":
  1. `AddAssets` with that asset;
  2. `AddTrack` of an audio track named "Voiceover" unless an unlocked
     audio track is free over the clip's span (looked for now, not at
     start);
  3. `AddClip` at the anchor's position, with `source_in` at the anchor's
     frames over the rate and the rest of the take as its duration. The
     pre-roll stays in the file and can be trimmed back in.
  `MediaService::import` is not used: its answers carry only the shared
  generation, and the desktop commits each as its own "Import media" entry
  (`app.rs:2354-2389`), which would add the take before this batch and make
  its `AddAssets` fail on the duplicate.
- If the editor instance changed since the start (another project was
  opened), the file is kept, nothing is added, and a notice names the path.
- A take whose placed part is shorter than 0.1 s is kept on disk and not
  placed. Dropped frames are said in the notice.
- Undo removes the clip and the asset from the project; the file stays.

Files: `src/record.rs`, `src/app.rs`, `src/ui/mod.rs`, `src/ui/viewer.rs`,
`docs/usage.md`, `docs/design.md`, `docs/verification.md` (what needs
native acceptance).

## Result

Implemented and tested headless in the desktop crate, with a stand-in for
`RecordService`, a stand-in clock and real egui passes; the native captures
and a run with a real microphone are not done (see Evidence), so the ticket
waits for a person. No microphone was opened and nothing was listened to.

- `src/record.rs` (new). `Recorder` (`stop`, `poll`, `frames`, `elapsed`),
  implemented for `RecordService`; `Recording<R = RecordService>` holds one
  take through `Path` (the path is being made), `Opening`, `Running` and
  `Stopping`. `ask(instance, document)` refuses "A take is already being
  recorded" and, with no document, "Save the project to record" before
  anything is opened, and hands back where the path is made;
  `path_answered(instance, answer, start)` starts the service only for the
  take still waiting in that instance (a replaced project, or a take given
  up meanwhile, opens nothing; a path that could not be made is "Cannot
  record: …"). `frame(project, instance, playback, heard, now)` reads the
  service: `Started` plays the timeline from its playhead at 1× when
  anything lies ahead (not in an empty project or at the end; not restarted
  when it already plays at 1×); `Level` keeps the newest; `Failed` is said,
  except `Cancelled` after a stop; `Finished` places. The anchor is taken on
  the first frame after `Started`: `(frames(), heard)` when playing and the
  device answers, `(frames(), playhead)` when nothing plays or after
  `DEVICE_WAIT` (1 s) from `Started`. `ask(project, document, playback)`
  pauses a playing transport, so the playhead holds until `Started`.
  `stop(playback)` pauses the transport in every phase,
  anchors a take whose anchor has not come at the playhead, and asks the
  service to stop; a take whose path is still being made is given up.
  `project_replaced()` stops the take; `close()` stops and drops it (the
  service's `Drop` joins its thread). `repaint_after()` is
  `Some(REPAINT)` (100 ms) only while running. `view()` gives `RecordView
  { Idle, Starting, Recording { elapsed, level }, Stopping }`.
  `placement(project, take, anchor)` is the `Batch` "Record voiceover":
  `AddAssets` of `Asset::new(path, Audio, frames / rate)` with `has_audio`
  and `file_size = bytes`, `AddTrack` of "Voiceover" unless an unlocked
  audio track is free over the span (looked for when the take lands, with
  `rules::free_start`), and `AddClip` at the anchor's position, `source_in`
  the anchor's frames over the rate and the rest as its duration. A placed
  part under 0.1 s is "The take was too short to place. It is kept in …";
  a take of a replaced project is "The take is kept in …: the project it
  was recorded for was closed, so nothing was added"; frames the recorder
  lost and `Take::ended` are added to either notice and to "Recorded N s of
  voiceover".
- `src/app.rs`: `Job::TakePath { project, result }`, run as `aster-take`
  (`editor_store::new_take(document, now)`), whose answer starts the
  service with a repaint wake. `Action::Record` commits drafts, asks, closes
  the source monitor and spawns the job; `Action::StopRecord` stops. While
  a take is under way Space (`play_key`) asks for `StopRecord`, and
  `TogglePlay` (the play button, K, L, a double-click, the panel's "play"),
  Seek, Step, the jumps, speed and preview are refused ("Stop recording to
  play or pause.", "… to move the playhead.", "… to change the speed.",
  "… to preview media."); L during a take asks once, never twice; edits and
  undo are allowed. `logic` reads `AudioService::position` once for the
  transport and the take, does not loop while a take is under way, steps the
  take after the transport, and asks for the take's repaint. A landed
  command goes through `edit`; a refusal is said with the path. A replaced
  project stops the take; `on_exit` closes it (kept, not placed). R
  (`record_key`, read before the source monitor's keys) asks for a take or
  ends it, and is a letter while a field has the keyboard; Escape ends a
  take after a held drag and before a duck or the selection.
- `src/ui/mod.rs`: `Action::Record`, `Action::StopRecord`.
- `src/ui/viewer.rs`: `ViewerInput::record`; the timeline's deck has a
  record control just before speed (`TransportText::record`,
  `TransportLayout::record`; the source monitor has none). At rest a red
  disc ("Record voiceover (R)"), dimmed while starting (a press gives the
  take up), grey and disabled while closing. A running take draws one
  control over record and speed: a red stop square where the disc was, the
  whole seconds recorded and a 30-point level bar (-60 to 0 dBFS, red
  within 1 dB of full scale); speed is not drawn. `src/icons.rs`:
  `Icon::Record`.
- Docs: `docs/usage.md` (Voiceover, and R in the shortcut table),
  `docs/design.md` (the record control), `docs/verification.md` (what is
  tested and what needs native acceptance), `docs/architecture.md` (the
  desktop's part; "nothing in the desktop calls it yet" replaced) and
  `docs/performance.md` (no file work in the frame loop, the repaint rule).

Routes: the transport's control and R start and end a take; Space, Escape
and the control end it. The play button, K, L, a double-click on the
picture and the panel's "play"/"pause" do not end it; they are refused in
words. No project-menu row, inspector control, timeline
gesture, agent command or panel phrase starts one, as the ticket and the
spec's decision 8 ask; the step that places a take is ordinary commands
(`AddAssets`, `AddTrack`, `AddClip` in a `Batch`) that an agent reads,
undoes and could write. No store format change.

## Evidence

On macOS 26.5.2 (arm64, Apple M1), Rust 1.97.1, 2026-10-14, `dev` profile.
No native window was opened, no microphone or output device was used and
nothing was listened to.

- `cargo test -p aster-editor --locked`: 814 and 12 passed;
  `record::` 19 passed. `cargo clippy -p aster-editor --all-targets
  --locked -- -D warnings`, `cargo fmt -p aster-editor -- --check` and
  `python3 scripts/check-architecture.py` pass. No other crate was
  changed, so none other was built or tested.
- `src/record.rs`, through a stand-in service and clock driving the frame
  as `App::logic` does (transport tick, then the take):
  `an_empty_project_takes_a_five_second_take_at_zero_whole` (on "Audio 1",
  nothing plays, one "Record voiceover" step, the asset's length and size
  from the take); `a_take_outlasts_the_project_and_the_playhead_holds_at_its_end`
  (2 s project, 10 s take: the transport stops at 2 s while the take reads
  3 s, the clip is 10 s, the playhead stays at 2 s);
  `the_device_clock_answering_after_300_ms_is_where_the_clip_starts`
  (playhead 1 s: the wall clock moves the playhead meanwhile, the device's
  first position 1 s is the clip's start, `source_in` 0.3 s, the media
  keeps the pre-roll); `without_a_device_position_in_a_second_the_transport_is_the_clock`;
  `an_import_in_flight_keeps_its_own_step_beside_the_take` ("Import media"
  committed between stop and `Finished`, then "Record voiceover", each
  undone alone); `a_track_filled_or_removed_during_the_take_gives_a_new_track`
  and `a_locked_audio_track_is_passed_over`;
  `a_project_replaced_during_the_take_adds_nothing_and_says_where_it_is`
  (the service is stopped, the file stays, the notice names it);
  `an_untitled_project_is_refused_before_any_device_is_opened`;
  `a_path_answered_for_a_replaced_project_opens_no_device` (also an old
  project's answer after the new one asked, and a path that could not be
  made); `a_second_take_is_refused_while_one_is_under_way`;
  `undo_takes_the_take_out_of_the_project_and_leaves_its_file` (the file is
  in `Film media/` by `new_take`; redo); `a_placed_take_is_saved_and_read_back_as_placed`
  (`editor_store` save and load); `a_short_take_is_kept_and_not_placed_and_lost_frames_are_said`;
  `a_take_that_ends_by_itself_stops_the_transport_and_says_why`;
  `no_microphone_is_said_and_nothing_plays`;
  `a_take_given_up_while_starting_says_nothing`;
  `no_repaint_is_asked_for_once_the_take_is_placed`;
  `the_control_reads_whole_seconds_and_the_level_in_decibels`.
- `src/app.rs` `r_asks_for_a_take_or_ends_it_and_is_a_letter_in_a_field`
  (real egui passes: R gives `Record`, or `StopRecord` while busy; nothing
  while a focused text field has the keyboard, which the test checks took
  it; nothing with Command held).
- `src/ui/transport_tests.rs`, real egui passes at 168, 498 and 900 points:
  `only_the_timeline_records_and_a_press_on_record_asks_for_a_take` (no
  control in the source monitor; between play and speed);
  `a_running_take_shows_its_time_in_speeds_place_and_a_press_stops_it`
  (the time inside the pane, no speed, the stop and the time both ask
  `StopRecord`, a closing take asks nothing). The earlier transport tests
  pass unchanged at those widths. `src/ui/viewer.rs`: the layout test of
  every width from 60 to 1,400 points runs with and without the record
  control; `the_timeline_keeps_record_just_before_speed_in_its_narrowest_pane`.
- Not run: `App` is not built in a test, so the wiring in `app.rs` (the job,
  the wake, `logic`'s order, the refusals while recording, `on_exit`) was
  read, not exercised. No native capture (idle, recording, past the end,
  "No microphone", the placed clip with its waveform, normal and narrow)
  was taken, and no host was tried with a real microphone.

Done when:

- `src/record.rs` tests with a fake service and a fake clock:
  - an empty project: a 5 s take is placed at 0 with its whole length;
  - a 2 s project and a 10 s take from 0: the clip is 10 s long and the
    playhead stopped at 2 s;
  - the device clock first answers 300 ms after the service started: the
    clip starts at that position with `source_in` of 300 ms;
  - no device position within a second: the wall-clock pair is used;
  - a user import in flight when the take stops: history gains exactly one
    "Record voiceover" entry for the take, and the import's own entry is
    separate and intact;
  - a track filled or deleted during the take gets a new track instead; a
    project switched during the take adds nothing and reports the path; an
    untitled project is refused before any device is opened; undo leaves
    the file.
- A shortcut test that `R` reaches `Action::Record` and is not taken while
  a text field has the keyboard.
- An idle test: no repaint is asked for once the take has been placed.
- Native captures of the final build, normal and narrow: idle, recording,
  recording past the end of the project, "No microphone", and the placed
  clip with its waveform.
- A native run on each platform with a real microphone is acceptance, not
  part of this ticket; say which hosts were tried.

## Known limits

- What remains of the misplacement is the round trip of the devices (output
  latency before the sound is heard, input latency before the voice is
  captured) plus up to one UI frame in taking the anchor. It is not
  measured or compensated, and there is no offset setting. The playback
  clock excludes device latency (`audio.rs:314-317`).
- Past the project's end there is nothing to speak against; the take
  continues in silence with the playhead held.
- The timeline's own sound plays while recording; without headphones it is
  recorded too. No echo cancellation, no monitoring.
- No punch-in, no takes, no countdown, no device choice.
- An agent cannot start or stop a recording.
- The asset's duration is what the writer counted, not what ffprobe would
  report; the two are expected to agree for a plain WAV and are compared
  only in ticket 18's fixture test.

## Comments

2026-10-13: Written with the spec. Nothing implemented or measured.

2026-10-13 (review): Three changes after the code was read. (1) "The end of
the project" is no longer a stop: `Project::duration()` is the largest clip
end (`model.rs:505-512`), 0 for an empty timeline, so the old rule ended a
take at once in a new project and cut narration off at the last clip. (2)
The stop path no longer goes through `MediaService::import`
(`service.rs:304-323` returns the shared generation; `editor.rs:718-726`
refuses a duplicate asset). (3) The take is anchored to the device clock:
the start-up wait of the worker was not measured on a device, but a decoder
alone took 33 to 35 ms to its first 1,024 frames in the reviewer's run, and
the callback's acknowledgement comes before that.

2026-10-14: Ticket 17 is resolved, decided on the owner's behalf with the
proposed default, so the start rule above stands. An untitled project (no
document, recovered unsaved work included) is refused with "Save the
project to record" before any device is opened. A Save button may sit
beside the words; it only saves (no `Pending` through `after_save`, which
re-asks on edits made during the save) and the take starts only when Record
is pressed again. The test "an untitled project is refused before any
device is opened" stands. The take's path is
`editor_store::new_take(document.path(), now)`. It is filesystem work and
must not run in the frame loop: Start spawns a short job for it (as
`find_orphan` uses `self.spawn`, `src/app.rs`) whose answer carries the
project instance; that answer starts `RecordService`, an answer for a
replaced project starts nothing, and an error is said in words and nothing
records. Add a test that a path answered for a replaced project opens no
device.

2026-10-14 (ticket 18 built): `RecordService::start` returns at once and
opens nothing on the caller; "No microphone" arrives through `poll` as
`RecordEvent::Failed(MediaError::NoMicrophone)`, and a new
`RecordEvent::Started { rate, channels }` says the take is running.
`frames()` and `rate()` are 0 until the file is made, just before
`Started`; `frames()` counts from when the device began to play, so it may
be past 0 when `Started` is read (corrected after ticket 18's review). The
anchor pairs `frames()` with the position on one tick and never takes
`Started` as frame 0. So Start shows a starting state, and
playback starts on `Started`, not with `start`; a `Failed` before it starts
no playback. The anchor is the first tick after `Started` where
`AudioService::position()` is `Some`, and the one-second fallback counts
from `Started`. `Take::ended` says why a take ended by itself (device
error, failed write, the 2-hour or WAV limit), for the notice.

2026-10-13 (implemented): Built and tested headless on macOS 26.5.2 (arm64);
the native captures and a real microphone remain, so Status is
ready-for-human. Rules the implementation settled, recorded in the spec's
Comments too: a take plays the timeline at 1× (speed is set on `Started`
and not changeable until the take ends); while a take is under way the
playhead cannot be moved and the source monitor not opened (refused in
words), Space and Escape end it, edits and undo are allowed; a replaced
project stops the take (its file is then named, nothing added); a take
stopped before its anchor came is anchored at the playhead, so one given
up while starting places nothing (a file already made is kept); closing
the window closes the take and places nothing. The optional Save button beside "Save the project
to record" was not built: the notice says it, and Save is Cmd+S and the
project menu.

2026-10-14 (review fixes): Three findings of the adversarial review held
and are fixed. (1) Record pressed while the timeline played left it playing,
at its own speed, until `Started`; a take ended before then left it playing
on. `Recording::ask` now pauses the transport (the desktop syncs its sound),
so the playhead really holds from the press until `Started` rolls it at 1×,
and `stop` pauses it in every phase. Tested by "record pressed while playing
holds the playhead until the take runs" and "a take stopped in any phase
stops the transport under it". (2) L during a take with the transport
stopped asked for 1× speed and then `TogglePlay`, which ended the take; K,
the play button, a double-click on the picture and the panel's "play" and
"pause" ended it too, though only R, Space, Escape and the control were
named. Now Space alone asks for `StopRecord` (`play_key` in `app.rs`);
`TogglePlay` during a take is refused with "Stop recording to play or
pause.", and L asks once. Tested by "during a take Space ends it and K and L
ask once and never end it"; the refusal in `App::perform` is read, not run.
(3) Lost sound was said in whole seconds and at least "0:01", so 50 ms read
as a second. It is now said to the tenth of a second ("0.1 s", "1.5 s",
"1:30" past a minute), and under a tenth as "Less than a tenth of a second";
the tests assert the exact words.

2026-10-14 (native acceptance): Still `ready-for-human`. Release build `target/release/aster` (sha256 fadbe5aa8244d800b2d9b16bf86bf121a6e9157af7e8067402761d40318e5763, `cargo build --release --locked`) on macOS 26.5.2 (25F84), Apple M1, Rust 1.97.1, Homebrew FFmpeg 8.1.2, built-in Retina display at scale 2. Fresh task-owned projects (made by `aster-agent` over ffmpeg-generated media: a 12 s testsrc2 interview with a speech-like tone, 30 s of 120 BPM clicks, a 0.8 s whoosh) and fresh `--settings` per run. Driven by real pointer and key events into the running window; the normal window (asked 1440x900) came up at 1280x709 points, the narrow one at 760x620 (and 760x560 for ticket 13). Captures are in `artifacts/audio-editing/captures/`. Every capture was looked at against `docs/design.md`. Dark theme only; Linux, Windows and the light theme were not run; nothing was listened to.

Taken: `normal-01`/`narrow-01` the record control at rest; `normal-40`/`narrow-39` its hover "Record voiceover (R)". Still missing: recording, past the end, and a placed take, normal and narrow. Record was not pressed: it opens the host's microphone and may raise the system permission prompt on a person's desktop.
