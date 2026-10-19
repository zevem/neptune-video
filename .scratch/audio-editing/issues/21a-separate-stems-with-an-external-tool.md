# Separate stems with an external tool

Status: ready-for-human
Progress: claimed
Type: task
Blocked by: 03, 17

## Scope

Vocal and instrument separation without a model, a runtime or a download in
the editor: an external separator executable the person installs, treated
as FFmpeg is treated. This is the chosen route for feature 10.

Step 1, before anything is built (a person, since it reads licences and
installs a tool): pick the tool and write under Comments its exact command
line, its output layout (file names, format, sample rate), how it obtains
its own model, and the licence of the tool and of its pretrained weights.
The candidate is the Demucs command-line tool. `facebookresearch/demucs` is
archived and lists MIT for the code; the licence of the pretrained weights
and the flags were not confirmed. If no tool passes, this ticket stops here
and feature 10 has only `CenterCut`.

Then:

- Finding the tool: `NEPTUNE_VIDEO_SEPARATOR`, else the tool's name on `PATH`, as
  `tool_command` does for `NEPTUNE_VIDEO_FFMPEG` (`process.rs:270-277`). Probed
  once per session on a worker by a run that exits at once (its version or
  help flag, from step 1). "Separate stems" is disabled with the words
  "Needs a stem separator; see the documentation" when it is absent.
- `crates/editor-media/src/sound.rs`: job `Separate { asset, to:
  PathBuf }` on the sound lane, target (`Separate`, asset). The tool writes
  into a fresh directory `<to>.part/` that the job creates. On success each
  stem is moved to `<to>/<asset name> (<stem>).wav`, never over an existing
  file, and probed with the crate's own ffprobe step (the function import
  uses, called directly, so the answer carries this request's identity).
  On failure or cancellation the `.part` directory and what the job put in
  it are removed; nothing else is ever deleted. Progress comes from the
  tool's output if step 1 finds a parseable form, else it is indeterminate
  with a read deadline suited to a long silent run.
- Answer: `SoundEvent::Separated { request, project, asset_id, source,
  result: Result<Vec<Asset>, String> }`.
- Desktop: `Action::SeparateStems(ClipId)` with the clip id captured at the
  click; `<to>` is the folder ticket 17 settles. On the answer, with the
  clip still there and its asset unchanged, one `Command::Batch` "Separate
  stems": `AddAssets`; for each stem an `AddTrack` named after it and an
  `AddClip` with the source clip's `start`, `source_in`, `duration`,
  `playback_rate`, `retime`, volume and volume keys; `LinkClips` over the
  source clip and the stems; and the source clip's volume set to 0 with its
  volume keys removed, so the stems are heard once and not on top of the
  original. Undo restores all of it; the files stay.
- Whole source file, not the clip's span: the stems line up with the source
  and survive a later trim.
- Routes: clip context menu on a clip that `sounds`, `separate stems` in
  the command panel.

Files: `crates/editor-media/src/sound.rs`, `process.rs` (the tool lookup),
`src/app.rs`, `src/ui/mod.rs`, `src/ui/timeline.rs`, `src/agent.rs`,
`docs/usage.md`, `docs/installation.md` (the optional tool).

## Result

Implemented and tested headless in `editor-media`, through a shell-script
stand-in separator and the real lane, and in the desktop crate through real
egui passes; once through the real Demucs 4.1.0 on one host (see Evidence).
The native captures are not taken and nothing was listened to. Owner
decision 10 was decided on the owner's behalf: Demucs (Comments, step 1).

- `crates/editor-media/src/sound/separate.rs` (new). `Separator`, made only
  by `Separator::probe(program)`: `<program> --help` must end well within
  60 s and name `--two-stems`; a missing program is "No stem separator was
  found: … Install Demucs so that demucs is on PATH, or set NEPTUNE_VIDEO_SEPARATOR
  …" (`separator_absent` tells it from a broken tool), a tool that fails is
  its last line. `look_for_separator(wake)` starts the session's probe once
  on `neptune-video-separator-probe`; `stem_separator()` reads the answer, `None`
  until it comes, and never waits. `SeparateJob { project, asset, to,
  separator }`; `STEMS` is `vocals` → Vocals, `no_vocals` → Instruments.
  The job makes `to`, then `<to>/<media> (stems).part` with `create_dir`
  (one there is refused and left), runs `<tool> --two-stems vocals -n
  htdemucs -o <part> --filename {stem}.{ext} <media>`, drains its output on
  a thread of its own, reads its progress from the last `NN%|` of its error
  pipe every 0.5 s, and holds it to 15 minutes plus 4× the media's length.
  Once it has ended well and both files are found it claims its last step,
  moves each stem to the first free `<media> (<Stem>).wav`, ` 2`, ` 3`… (a
  hard link, which refuses a name taken in between; a rename where links
  are refused) and probes it with the import's own `probe`. The part folder
  goes with whatever is in it however the job ends; a failure after a stem
  was placed names what was kept.
- `crates/editor-media/src/process.rs`: `tool_program(tool)`, `demucs` named
  by `NEPTUNE_VIDEO_SEPARATOR`. `import.rs`: `probe` is `pub(crate)`.
- `crates/editor-media/src/sound.rs`: `Work::Separate`, target
  `Separate(asset)`; `SoundService::separate(job) -> Extracting`;
  `SoundEvent::Separated { request, project, asset_id, source, result:
  Result<Vec<Asset>, String> }`. `cancel_all` spares it as it spares a saved
  file, and `cancel` answers `Written` for stems whose answer waits.
- `src/stems.rs` (new). `LABEL`, `NEEDS_TOOL`, `LOOKING`, `UNTITLED`,
  `Tool::{Looking, Missing, Found}`, `refusal`, `Asked`, `folder(document)`
  (`editor_store::media_folder`), `place(project, clip, stems)` and
  `Separating` (cards, progress, `answered`, `cancelled`). `place` is one
  `Batch` "Separate stems": `AddAssets`; per stem `AddTrack` (named Vocals
  or Instruments) and `AddClip` of the source's start, `source_in`,
  duration, speed, time map, volume, volume keys, fades, audio effects and
  group, named `<clip> (<Stem>)`; `LinkClips` over the source and the
  stems; `SetGain` 0 and, when it had them, `SetKeyframes` with no volume
  keys. A stem shorter than the span the clip plays cuts a clip of one
  speed short and refuses a time map in words.
- `src/app.rs`: `Action::SeparateStems(ClipId)` refuses without a separator
  ("Needs a stem separator; see the documentation. <why>") or before the
  probe answers, for an untitled project ("Save the project to separate
  stems") and for a clip `refusal` names; `Action::CancelSeparate(u64)`;
  `take_sound` hands progress and `Separated` to `separating` and edits the
  step, or says why not and names the files; the probe starts on the first
  frame and a separator that is there and cannot be run is said once.
- `src/ui/timeline.rs`: "Separate stems" in the clip menu after Save audio
  as…, of a clip whose media has sound, disabled with the reason on hover.
  `TimelineInput::separator`. `src/ui/sheets.rs`: a card per separation,
  with the tool's figure or "…" and Cancel. `src/agent.rs`: `separate
  stems`, `separate`, `split stems`, and the example.
- No serialized agent command starts it (spec principle 8); the step is
  ordinary commands.
- Docs: `usage.md` (Separate stems), `installation.md` (Stem separator),
  `performance.md` (Stem separation).

## Evidence

On macOS 26.5.2 (arm64, Apple M1), Rust 1.97.1, Homebrew FFmpeg and ffprobe
8.1.2 (`scripts/check-media-tools.py` passed), 2026-10-14, `dev` profile.
No native window was opened and nothing was listened to.

- `cargo test --locked -p editor-media`: 327 passed (14 in
  `sound::separate`, of them 9 through the stand-in on Unix);
  `cargo test --locked -p neptune-video`: 827 and 12 passed. `cargo clippy
  --locked -p editor-media -p neptune-video --all-targets -- -D warnings`,
  `cargo fmt --all -- --check`, `cargo check --workspace --all-targets
  --locked` and `scripts/check-architecture.py` pass.
- Media, with a stand-in (`separate_tests.rs`, Unix: a `sh` script that
  answers `--help` and copies the source to `vocals.wav` and
  `no_vocals.wav`): `stems_are_put_in_the_media_folder_probed_and_said_in_order`
  (the folder made, the stems moved, probed at 0.5 s, the tool's 50% said,
  the part folder gone, 2,000 lines of output not waited on);
  `a_separation_given_up_or_replaced_leaves_neither_stems_nor_its_part_folder`;
  `a_stem_that_is_there_is_never_replaced` (a file and a dangling link
  keep their names, the stem is ` 3`); `a_tool_that_fails_says_its_last_words_and_leaves_nothing`;
  `a_tool_that_ends_well_without_its_stems_is_an_error`;
  `a_part_folder_in_the_way_is_refused_and_left_as_it_is`;
  `a_separation_outlives_a_project_replace`;
  `a_separator_answers_its_probe_by_naming_two_stems` (a traceback is its
  last line); `a_missing_separator_found_after_its_probe_is_said_when_run`;
  and on every host `a_missing_separator_is_found_at_probe_time_and_said_in_words`
  (no `Separator`, so no job), the progress and last-line readers, the
  names, and the refusals before any process.
- Desktop (`src/stems.rs`): `stems_are_placed_under_the_clip_as_one_step_and_undone_whole`
  (one entry, span, speed, volume, keys, fades, effects and link shared,
  the source at 0% without keys, validated; undo gives back tracks and
  assets as they were), `a_picture_with_sound_gives_its_sound_and_not_its_look`,
  `a_stem_shorter_than_its_source_cuts_a_clip_of_one_speed_and_refuses_a_time_map`,
  `a_stale_answer_adds_nothing_and_names_the_files` (another project, a
  deleted clip, changed media, a failure),
  `a_separation_stopped_is_forgotten_and_one_finishing_keeps_its_card`,
  `a_clip_that_is_heard_is_asked_and_the_rest_refused_in_words`.
- Routes, through real egui passes: `src/ui/fades_tests.rs`
  `the_clip_menu_separates_a_clip_that_is_heard_once_a_separator_is_found`
  (disabled with "Needs a stem separator; see the documentation" and with
  "Looking for a stem separator…" on hover; the clip captured on video and
  audio lanes; refused for an owned picture and a locked track; nothing
  edited); `src/ui/sheets.rs`
  `a_separation_shows_how_far_it_is_once_said_and_can_be_stopped` (at 900
  and 300 points: a bar without a value and "…", 37 %, Cancel);
  `src/agent.rs` `stems_are_asked_of_the_one_selected_clip_that_is_heard`.
- The real tool: Demucs 4.1.0 (torch 2.14.1, Python 3.12, through `uvx
  --with numpy` from a task-owned cache, model `htdemucs` `955717e8` fetched
  from Hugging Face by the tool), set as `NEPTUNE_VIDEO_SEPARATOR_TEST` for
  `the_persons_separator_splits_twenty_seconds_into_two_stems`: a 20 s
  stereo 44.1 kHz WAV of two tones FFmpeg made; probe 1.24 to 1.87 s;
  separated through the lane in 15.72 s (model fetched), 5.65 s and 6.04 s;
  two 20.000 s stems of 3,528,044 bytes; progress 25, 50, 75 (100) %. The
  same command line alone on 180 s of tones: 26.7 s wall. Without the
  variable the test says SKIP.

Not verified: the native captures (the disabled item, a card, placed
stems), which need the running build and a pointer on the clip menu, and the
app's own frame loop (`take_sound`, the probe's notice) beyond its parts;
speech or music through the real tool, its quality and its memory; Linux and
Windows (the stand-in tests are Unix only; the second handle to the output
pipe is untried on Windows).

The criteria this was held to:

- Step 1 is written under Comments with the tool's version and the licence
  texts read.
- Media tests with a fake separator (a script that writes two short WAVs):
  the stems are moved and probed; cancellation leaves neither stems nor the
  `.part` directory; an existing stem file is not replaced; a tool that
  exits non-zero reports its last output; a missing tool is found at probe
  time and no job starts.
- App tests with a fake lane: the batch is one entry, the stems share the
  source's span and link, the source is silent, and undo restores its
  volume and keys; a stale answer (clip deleted, another project) adds
  nothing and names the files.
- A native run with the real tool on at least one host, with the host, the
  tool's version, the source's length and the run time written down.
- Native captures, normal and narrow: the disabled item, progress, the
  placed stems.

## Known limits

- Nothing is separated unless the person has installed the tool; the editor
  neither ships nor fetches it or its model. Whatever network request the
  tool makes to get its model is the tool's own.
- Separation is slow and holds the sound lane: waveforms, loudness and
  beats wait behind it. Run time is unmeasured.
- Stems are as large as their source and are the person's files in the
  project's media folder; the editor never removes them.
- Quality is the tool's. Nothing was run or listened to at design time.
- One more voice for every stem where the clip plays (the 16-voice bound).
- An agent cannot start it.
- The tool reads the media's first sound stream itself: a stream that
  starts after container time 0 gives stems early by that gap.
- The part folder is left behind if the editor exits while the tool runs;
  a later separation of that media is then refused until it is removed.

## Comments

2026-10-13 (review): New. The former ticket 21 put separation, neural
denoise and speech detection in one research question with five candidates,
none chosen. This route was in that ticket's own table and needs no crate,
no bundled model and no network request by the editor, so only storage
(ticket 17) and step 1 stand before it. `ffmpeg -hide_banner -filters` on
8.1.2 lists no stem or separation filter. No separator was run and no
licence text was read; the statements about Demucs are the reviewer's web
check and are unverified here. If ticket 20 is built first, its temp-then-
rename job is the pattern; this ticket does not wait for it.

2026-10-14: Ticket 17 is resolved. `<to>` is
`editor_store::media_folder(document)`, `<document stem> media/` beside the
saved document, made on first use and never emptied by the editor; an
untitled project is refused with words asking to save it, as for a take
(review, 2026-10-14: not saved and then run by itself). Stem paths are
stored absolute, like every asset path. Naming or making the folder is
filesystem work, done off the frame loop.

2026-10-14 (step 1, decided on the owner's behalf): Owner decision 10 is
taken, the owner having delegated it. The tool is the Demucs command line,
installed by the person; nothing is bundled or fetched by Neptune Video.

- *Tool and version.* `adefossez/demucs`, which its README calls the
  officially maintained Demucs, fork of `facebookresearch/demucs` (archived
  on 1 January 2025, read-only). PyPI `demucs` 4.1.0, 11 July 2026, run here
  (4.0.1 of September 2023 before it). Python 3.10 or later. 4.1.0 does not
  install NumPy and fails at import without it; with it, it ran.
- *Command line.* `demucs --two-stems vocals -n htdemucs -o <out>
  --filename {stem}.{ext} <media>`; `--help` (argparse, exit 0) lists
  `--two-stems STEM` and `--filename`, and has no `--version`, so the probe
  is `--help`.
- *Output.* `<out>/htdemucs/vocals.wav` and `no_vocals.wav` (`--other-method
  add`, the default: the sum of the other stems), 16-bit stereo PCM at
  44.1 kHz, as long as the source (20.000 s of 20 s). Progress is tqdm on
  stderr, ` 97%|███…| 175.5/181.35 [00:23<00:00, …]`, rewritten with carriage returns;
  stdout has two or three lines.
- *Model.* `htdemucs`, the default, fetched by the tool from Hugging Face
  (`adefossez/HTDemucs`, `955717e8.safetensors`, 87 MB of cache) on its
  first run.
- *Licences read.* The code: the repository's `LICENSE` is the standard MIT
  text, "Copyright (c) Meta Platforms, Inc. and affiliates", and names no
  models; PyPI says MIT. The weights: no licence text was found. The author
  wrote "Weights are provided for research purpose!" (2 August 2023,
  `facebookresearch/demucs` issue 508); the model card on Hugging Face reads
  no licence in its text, and a discussion there (unanswered) says its
  metadata is tagged `license: mit`. Taken as unsettled.
- *Why it passes.* The editor bundles, downloads and recommends no weights
  for any use: the person installs the tool, and whatever it fetches is its
  own doing, which `docs/installation.md` says with the weights' status.
  The code is MIT and maintained, with a 2026 release, and its command line
  has the two-stem mode the design needs. No alternative was compared
  further.

2026-10-14 (implemented): Built as the Result says. Settled beyond the
ticket, recorded in the spec: the part folder is `<media> (stems).part`
inside the media folder, not `<to>.part` beside it, so a leftover blocks one
media and the stems move by a link within one folder; a taken name gets the
next number rather than refusing; the stems carry the source's fades, audio
effects and group too, and not its picture effects; a short stem cuts a
clip of one speed and refuses a time map; `cancel_all` spares a separation;
`SoundService::separate` answers `Extracting`. A native run took the place
of a person's step 1 run: Demucs was run from a task-owned cache, which was
removed afterwards.


2026-10-14 (review fixes): Three findings held and were fixed. *Track
volume and solo:* the stems' tracks were made at 100% and not soloed, so a
clip heard only through its track's solo went silent, and one on a track at
another volume changed level; each stem's track now takes the volume and
solo of the source clip's track (`src/stems.rs`,
`stems_are_heard_as_loud_as_a_soloed_clip_at_half_volume`). *Heard when
placed:* the answer checked only that the clip and its media were
unchanged, so a picture whose sound was detached while the tool ran was
turned down while its audio clip played over the stems; `place` now
refuses, with the files named, a clip whose sound is on a linked audio clip
("its sound is now on the linked audio clip") or that is no longer heard
("it is no longer heard: its track is muted", and the other reasons of
`normalize::unheard`), as `separable` did when asked. *Docs:*
`docs/installation.md` gave `<folder>.part` for the tool's output folder,
now `"<document> media/<media> (stems).part"`, and stated a Hugging Face
MIT tag as fact; it now says the model card's text names no licence and an
unanswered discussion there reports the tag, unchecked.

2026-10-14 (native acceptance): Still `ready-for-human`. Release build `target/release/neptune-video` (sha256 fadbe5aa8244d800b2d9b16bf86bf121a6e9157af7e8067402761d40318e5763, `cargo build --release --locked`) on macOS 26.5.2 (25F84), Apple M1, Rust 1.97.1, Homebrew FFmpeg 8.1.2, built-in Retina display at scale 2. Fresh task-owned projects (made by `neptune-video-agent` over ffmpeg-generated media: a 12 s testsrc2 interview with a speech-like tone, 30 s of 120 BPM clicks, a 0.8 s whoosh) and fresh `--settings` per run. Driven by real pointer and key events into the running window; the normal window (asked 1440x900) came up at 1280x709 points, the narrow one at 760x620 (and 760x560 for ticket 13). Captures are in `artifacts/audio-editing/captures/`. Every capture was looked at against `docs/design.md`. Dark theme only; Linux, Windows and the light theme were not run; nothing was listened to.

Taken: `normal-08`/`narrow-08` Separate stems disabled; `normal-09`/`narrow-09` its reason "Needs a stem separator; see the documentation". Still missing: the progress card, a finished separation and the placed stems, normal and narrow. No stem separator is installed on this host.
