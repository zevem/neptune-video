# Save audio as a file

Status: ready-for-human
Progress: claimed
Type: task
Blocked by: 03

## Scope

Waits on owner decision 9 in the spec: is a written file wanted, or is
"Detach audio" (ticket 02) the whole of extraction?

If wanted: "Save audio as…" on a library asset with sound, or on a clip,
writes that source's sound to a WAV at a path the person picks. It is the
editor's first FFmpeg job that writes a file, so it sets the pattern.

- `crates/editor-media/src/sound.rs`: job `Extract { asset, from, span,
  to: PathBuf }`:

  ```
  ffmpeg -hide_banner -nostdin -loglevel error -progress pipe:1 -y
    [-ss <from> -t <span>] -i <path> -map 0:a:0 -vn -sn -dn
    -c:a pcm_s16le -f wav <to>.part
  ```

  Progress and the read deadline come from the `-progress` lines. On
  success the `.part` file is renamed over the target; on failure or
  cancellation it is removed. Nothing else is ever deleted. The job runs on
  the sound lane, so it waits behind a waveform and a waveform behind it.
  Its replacement target is (`Extract`, asset, from, span): it does not
  cancel, and is not cancelled by, the same asset's `Peaks`, `Beats` or
  `Loudness` jobs.
- Desktop: an `rfd` save dialog (the default name is the asset's with
  `.wav`), progress with Cancel in the notice area, and on success an offer
  to import the file ("Add to project") through the ordinary import.
- The source's own sound is written: no clip volume, fades, effects or
  speed.

Files: `crates/editor-media/src/sound.rs`, `src/app.rs`, `src/ui/mod.rs`,
`src/ui/library.rs`, `src/ui/timeline.rs`, `docs/usage.md`.

## Result

Implemented and tested headless, in `editor-media` through a stand-in lane
and through the real lane and FFmpeg, and in the desktop crate through real
egui passes and once through the real lane; the native captures are not
taken (see Evidence). Nothing was listened to. Owner decision 9 is taken as
yes by default.

- `crates/editor-media/src/sound/extract.rs` (new). `ExtractJob { project,
  asset, from, span, to, keep }` (`ExtractJob::whole`; `keep` is the
  project's media paths), `part_path(to)` (the
  target's name with `.part`, beside it). Refused before any process:
  "Media has no sound to save", "Media has no length", "The span to save is
  empty", "The span to save starts after the media ends", "Choose a file to
  save the sound to" and "The sound is not saved over the media it is read
  from" (the same path), "… is media of the project: its sound is not saved
  over it" (a `keep` path). On the worker, before FFmpeg: the source or a
  `keep` file under another name (the same file on disk: device and inode
  on Unix, the resolved path elsewhere), a folder ("… is a folder"), and a
  `.part`
  that is already there ("… is in the way: … left as it is"); the `.part`
  is made with `create_new`, is the only file ever removed, and is removed
  only after the process has been waited for. The command is the ticket's
  with `-nostats -threads 1` added; a whole file (from 0, span to its
  length or past) has no `-ss`/`-t`. Progress is `out_time_us` over the
  span, or what of it the file has. A process that reads no sound is an
  error ("ffmpeg: no sound was read from that span"). On success the job
  claims its last step under the lane's lock, then `sync_all`s the part
  through a handle opened for writing (Windows flushes no read-only one)
  and renames it over the target, and answers with the length written.
- `crates/editor-media/src/sound.rs`: `Work::Extract`, target (`Extract`,
  asset, from, span) beside the others; `SoundService::extract(job)`
  answers `Extracting { request, replaced }`, `replaced` naming the files
  of the same span it actually gave up (not one that had claimed its
  rename);
  `SoundEvent::Extracted { request, project, asset_id, source, to, result:
  Result<Time, String> }`. `cancel(request)` now answers a `Cancel`:
  `Written` when its `Ok` answer waits, `Finishing` once it has claimed its
  rename (its answer comes, written or not), else `GivenUp` (a waiting
  failure is dropped with it). `cancel_all` (project replace) spares written files: a waiting or
  running extract, its progress and its answer stay. `MAX_LINE` and
  `out_time` of `loudness.rs` are shared.
- `src/extract.rs` (new). `LABEL` "Save audio as…", `OFFER_SECONDS` 12,
  `Source::{Asset, Clip}`, `Asked::of(project, instance, source)`: a media
  file's whole sound, or a clip's span from the lowest source time it
  reaches to the highest (`Clip::source_span`), named for the clip;
  refusals "That media has no sound to save.", "That clip has no sound to
  save." (a title, a silent media), "That media is no longer in the
  library.", "The clip no longer exists.". `file_name()` is the media's
  with `.wav`. `target(picked, exists)` runs on the dialog's thread: no
  extension gets `.wav` and a name so made that is taken is refused,
  another extension is refused ("Audio is saved as WAV: …"). `guard`
  refuses a path of the project's media as the library has it, with its
  name; `Asked::job(project, to)` hands the lane every media path for the
  aliases. `Saving` holds the files being
  written (progress, Cancel) and the written ones offered; an answer for
  the instance that asked is offered for 12 s, one for a replaced project
  is a notice with the whole path and no offer; a failure says FFmpeg's
  first line; a newer file of the same span says it took the older's
  place only for the requests the lane gave up, and an older one it was
  too late for keeps its card and answers under its own name.
  `cancelled(request, Cancel)` says "Saving … was stopped; … was not
  written." and drops the card, or "… is being put in place; what came of
  it follows." or "… is written already.", keeping it.
- `src/app.rs`: `Action::SaveAudio(Source)` shows the `rfd` save dialog
  ("Save audio", a "WAV audio" filter, the default name) on the dialog
  thread; `Job::AudioPicked` sends the job (`save_audio`); `take_sound`
  hands progress and `Extracted` answers to `saved_audio` before peaks;
  `CancelSaveAudio` hands the lane's `Cancel` to `saved_audio.cancelled`; `AddSavedAudio` is `import(vec![to])`, the ordinary import and
  its one step; `DismissSavedAudio`; project replace drops the offers and
  leaves the writing; `logic` lets offers go and wakes for the next.
- `src/ui/sheets.rs`: `saves` draws a card a file over the viewer where
  the notices are, in their width: words, a progress bar named as a
  progress indicator with its value, the percentage and Cancel; once
  written, Add to project and Dismiss. Notices start under the cards.
- Routes. Library media menu (`library.rs`): "Save audio as…" before Copy
  path, of a file with sound (an audio file or a video with sound),
  disabled with the reason on hover when it has none to save. Clip menu
  (`timeline.rs`): at the end of the sound rows of any clip whose media is
  heard (`fades::has_sound`), the picture of a detached pair included, for
  the clip under the menu. Project menu, Clip (`toolbar.rs`, not in the
  ticket's list): for the one selected clip, as Detach audio is, else
  "Select one clip to save its audio.".
- No serialized agent command and no command-panel phrase: writing a file
  is work a person starts (spec principle 8). Nothing is edited until Add to
  project.
- `src/peaks.rs` ignores `Extracted`.
- Docs: `usage.md` (after Detach audio); `performance.md` ("Save audio as
  a file", bounds and two fixture measurements).

## Evidence

On macOS 26.5.2 (arm64, Apple M1), Rust 1.97.1, Homebrew FFmpeg and
ffprobe 8.1.2 on `PATH` (`scripts/check-media-tools.py` passed),
2026-10-14, `dev` profile. No native window was opened and nothing was
listened to.

- `cargo test --locked -p editor-media`: 296 passed; `sound::` 49, three
  runs in a row. `cargo test --locked -p neptune-video`: 790 and 12 passed.
  After the review fixes: 297 passed (`sound::` 50, three runs), and 791
  and 12.
  `cargo clippy --locked -p editor-media --all-targets -- -D warnings`,
  the same for `neptune-video`, `cargo fmt --all -- --check`, `cargo check
  --locked` and `scripts/check-architecture.py` pass.
- Through FFmpeg (`crates/editor-media/src/sound/tests.rs`):
  `a_videos_tone_is_written_as_a_wav_of_its_length_and_its_tone` (3 s MP4,
  `mpeg4` picture and an AAC 1 kHz tone at half scale: ffprobe reads an
  audio file of 3.008 s, within a frame; the tone 1000.0 Hz at -9.02 dBFS
  RMS; one FFmpeg started); `a_clips_span_is_written_and_no_more` (1 s
  from 1.5 s of a stereo WAV whose tone starts at 2 s: 1 s long, the tone
  24,001 frames in, 24,000 expected; a span past the end gives what the
  file has); `a_file_given_up_or_replaced_leaves_neither_itself_nor_its_part`
  (Unix only: a never-ending FIFO source, cancelled and replaced
  mid-write, leaves neither target nor `.part`);
  `a_file_that_is_there_is_replaced_only_by_a_whole_one` (FFmpeg failing
  on noise and on a missing file leaves the existing target byte for byte;
  a `.part` in the way is refused and left; `sub/../` aliasing the source
  and a folder are refused; another media file of the project is refused
  by `sub/../`, a linked folder and, on a folder that ignores case, its
  name in capitals, and is left byte for byte (this fails with the lane's
  `keep` check removed); a success replaces the target).
- Through a stand-in lane: `a_written_file_and_the_same_medias_other_jobs_leave_each_other_alone`
  (Peaks, Beats, Loudness and Extract of one media do not cancel each
  other; a second Extract of the same span does),
  `a_file_being_written_outlives_a_project_replace_and_is_given_up_by_name`,
  `a_file_that_has_claimed_its_last_step_is_not_given_up` (`Finishing`,
  then `Written`; a newer file names nothing as replaced; a claimed file
  that fails is `Finishing`, then `GivenUp` with its failure dropped),
  `a_file_that_cannot_be_written_is_refused_before_any_process`;
  `a_part_is_put_in_place_whole_and_otherwise_removed` (the part flushed
  and renamed on a real file; meant for a Windows run, where a read-only
  flush fails, and not run there).
- Desktop (`src/extract.rs`):
  `a_media_file_gives_its_whole_sound_and_a_clip_the_span_it_plays` (speed
  and trim), `a_clip_run_backwards_gives_the_span_from_its_lowest_source`,
  `what_has_no_sound_is_refused_in_words`,
  `the_picked_file_is_a_wav_and_no_media_of_the_project`,
  `a_file_the_lane_was_too_late_to_give_up_keeps_its_card_and_its_name`,
  `a_written_file_is_offered_to_the_project_that_asked_and_to_no_other`
  (the stale answer after a project switch is said with its path and
  offers nothing), `a_file_not_written_is_said_and_a_replaced_or_stopped_one_too`,
  and through the real lane
  `a_span_written_by_the_lane_is_offered_and_after_a_replace_only_said`
  (the dialog's default name, the source itself, is refused by the lane;
  `cancel_all` and a replaced project still write the file and say where;
  no `.part` is left).
- Routes, through real egui passes: `src/ui/library.rs`
  `media_with_sound_offers_to_save_it_and_a_silent_picture_does_not`;
  `src/ui/fades_tests.rs` `the_clip_menu_saves_the_span_of_sound_the_clip_plays`
  (video and audio lanes, nothing edited, none on a title, the detached
  picture still offered); `src/ui/toolbar.rs`
  `the_menu_saves_the_audio_of_the_one_selected_clip`; `src/ui/sheets.rs`
  `a_file_being_saved_can_be_stopped_and_a_saved_one_added_or_dismissed`
  (at 900 and 300 points: progress value 0.42, Cancel, Add to project,
  Dismiss inside the viewer, notices under the cards).

Not verified: the native captures of the dialog route and the progress card
(the system save dialog and a running write need the running build and none
was taken); the app's own frame loop (dialog thread, `take_sound`, the
offer's expiry) is tested only in its parts; long or compressed real
recordings and their time; Linux and Windows, where the FIFO cancellation
test does not run, the writable flush handle was never exercised and the
resolved-path identity on Windows is untried.

The criteria this was held to:

- Media fixture tests: a video with a tone gives a WAV ffprobe reads, of
  the same duration within a frame and the same tone; a clip's span gives
  that span; cancellation leaves neither the target nor the `.part` file;
  an existing target is replaced only on success; a failing FFmpeg leaves
  an existing target as it was.
- A stale answer after a project switch still reports the written file and
  offers nothing.
- Native capture of the dialog route and the progress.

## Known limits

- PCM WAV only: no stream copy, no compressed output, first stream only.
- This is a copy of the source's sound, not a render of the mix. There is
  still no export.
- Long sources hold the sound lane; waveforms wait.
- The file is the stream's own samples from its first one. A stream that
  starts after container time 0 is written without the gap, unlike the
  peak file.
- No RF64: a WAV past 4 GiB (about 6.2 hours of 48 kHz stereo) has a header
  that cannot say its length.
- A `.part` is left behind if the editor exits while it writes: closing the
  window does not wait for the lane. A later save to the same name is then
  refused until it is removed by hand.
- A clip's span is from its lowest source time to its highest, so a clip
  that freezes or runs back writes the stretch once, forwards.
- Offered for 12 s and only to the project that asked; afterwards the file
  is imported as any other.
- The project's media is what it was when Save audio was sent: media
  imported while a file waits is not in its `keep`. A link at the target
  that points at media is refused too, though the rename would replace the
  link and not the media.

## Comments

2026-10-13: Written with the spec. Nothing implemented or measured. "Export
mix as WAV" was proposed alongside and left out of the epic.

2026-10-13 (review): Gained its dependency on ticket 03, whose lane it runs
on, and the lane's replacement target.

2026-10-14 (implemented): Built as the Result says. Owner decision 9 was
taken as yes by default; the triage it waited on is answered so. Settled
beyond the ticket: `cancel_all` on project replace spares a file being
written, since the file is the person's and asked of no project (the spec's
lane says `cancel_all` gives up everything); `cancel` answers whether it gave
up, and a file that has claimed its rename or whose answer waits is not;
the `.part` is made with `create_new`, so one already there is refused, not
written over; `-nostats -threads 1` were added and a whole file has no
`-ss`/`-t`; empty output is an error. The project menu's Clip section got
the row too, as Detach audio has it.

2026-10-14 (review fixes): Four review findings held and were fixed.
The part is flushed through a handle opened for writing, since
`FlushFileBuffers` refuses a read-only one on Windows. `cancel` answers
`Cancel { GivenUp, Finishing, Written }` in place of a bool: only an `Ok`
answer counts as written, and a claimed file whose rename may still fail
is `Finishing`, so the desktop no longer says "That file is written
already." before a failure notice. `extract` answers `Extracting`, naming
what it gave up, and the desktop drops cards and says "in place of" for
those only. The lane is handed the project's media (`keep`) and refuses
any of them as the same file on disk, not only by the exact path. While
checking this, macOS's `canonicalize` was found to fold case already; the
identity is the file's device and inode on Unix all the same.

2026-10-14 (native acceptance): Still `ready-for-human`. Release build `target/release/neptune-video` (sha256 fadbe5aa8244d800b2d9b16bf86bf121a6e9157af7e8067402761d40318e5763, `cargo build --release --locked`) on macOS 26.5.2 (25F84), Apple M1, Rust 1.97.1, Homebrew FFmpeg 8.1.2, built-in Retina display at scale 2. Fresh task-owned projects (made by `neptune-video-agent` over ffmpeg-generated media: a 12 s testsrc2 interview with a speech-like tone, 30 s of 120 BPM clicks, a 0.8 s whoosh) and fresh `--settings` per run. Driven by real pointer and key events into the running window; the normal window (asked 1440x900) came up at 1280x709 points, the narrow one at 760x620 (and 760x560 for ticket 13). Captures are in `artifacts/audio-editing/captures/`. Every capture was looked at against `docs/design.md`. Dark theme only; Linux, Windows and the light theme were not run; nothing was listened to.

Taken: `normal-08`/`narrow-08` Save audio as… in the clip menu; `normal-22`/`narrow-22` in the project menu; `narrow-34` in the library menu. Still missing: the save dialog and the progress card. Choosing the row opened the system save panel on the person's own desktop, listing their folders; it was cancelled at once, nothing was written, and its captures were deleted. This host has no private display, so the dialog route was not driven further.
