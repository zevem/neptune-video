# ADR 0007, glossary, usage, design, performance and verification docs

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 01, 02, 03, 04, 05a, 05b, 05c, 06, 07, 08a, 08b, 09, 10, 11, 12, 13, 14a, 14b, 15, 16a, 16b, 17, 18, 19, 20, 21a, 21b, 21c

## Scope

Write the documents against the code as it stands, and bring each ticket's
Result, Evidence and Known limits to what was built. A ticket left open by
an owner decision (17 to 21c) is described as it is, not waited for; say so.

- `docs/adr/0007-sound-is-levels-in-the-mixer-and-filters-in-the-decoder.md`:
  the two places sound is shaped, audio kinds in the one effect table,
  fades as edge fields, the one definition of what sounds in core, derived
  peaks outside the project, analysis and recording outside the command
  boundary, filters probed by running them, and what was set aside (an
  in-process DSP graph, a live sidechain, a parallel effects field, baked
  audio files).
- `docs/adr/0006-advanced-editing-is-generic-data-resolved-once.md`: line
  21 says the provider schema has a 24 KiB cap. Since ticket 10 it is
  `MAX_SCHEMA_BYTES`, 23 KiB, with `MAX_SYSTEM_BYTES`, 6 KiB, beside it
  (`crates/editor-agent/src/planner.rs`), built as owner decision 1's
  default. Add a dated note to the ADR giving the caps the code has, or
  the owner's answer to decision 1 if there is one by then; the ADR's
  reasoning stands.
- `GLOSSARY.md`: "Effect" is defined as a picture operation (`:137-141`);
  add "Audio effect" for the filter kinds (equalizer, compressor, limiter,
  noise reduction, pitch, centre reduction) and "Sound effect" for an
  imported sound clip such as a whoosh or a hit, each saying it is not the
  other. Add Fade, Solo, Track volume, Waveform, Detached audio, Owned
  sound, Take, Beat grid, Stem. Keep one term per idea.
- `docs/usage.md`: waveforms and the non-features list (`:365-366`), the
  sound paragraph (`:355-359`), the sound clip's section (`:103`), every
  new action, phrase and shortcut.
- `docs/design.md`: the sound clip's section (`:49`), track controls
  (`:24`), fade handles, the record state.
- `docs/architecture.md`: the sound paragraphs (`:334-347`, `:126`): the
  chain, the level stage, the limiter, the sound lane, recording.
- `docs/performance.md`: budgets actually chosen (peak cache, held peaks,
  beat grids, sound lane queue, record ring) and anything measured, with
  its conditions.
- `docs/verification.md`: an "Audio editing" section in the shape of
  "Advanced editing" (`:291-375`): where the evidence comes from, a matrix
  row per feature (Headless tests, Readback, Native capture, Measured,
  Known limits and unverified), and the standing unverified list.
- `docs/agent-protocol.md`, `docs/native-ai.md`: checked against tickets
  05c, 10, 11 and 12 (audio effects are heard; an enabled `pitch` keeps
  pitch).
- `docs/installation.md`: the filters the editor runs, that each is probed,
  and the optional stem separator if ticket 21a named one.
- This epic's spec: dated Comments for every rule the tickets changed; the
  header set to what is true.

## Result

Written against the code as it stands on 2026-10-14. Tickets left open by an
owner decision are described as they are, not waited for: every open owner
decision was delegated and taken on the owner's behalf (spec Comments,
2026-10-14); 21b and 21c are recorded as not built and stay open research.

- `docs/adr/0007-sound-is-levels-in-the-mixer-and-filters-in-the-decoder.md`
  (new): the two places sound is shaped, audio kinds in the one effect
  table, fades as edge fields, solo durable, the one definition in
  `editor_core::sound`, derived peaks outside the project, analysis and
  recording outside the command boundary, the media folder, filters and the
  separator probed by running them, and what was set aside (an in-process
  DSP graph, a live sidechain, a parallel effects field or struct per effect,
  baked audio files, parsing `ffmpeg -filters`).
- `docs/adr/0006-…`: a dated note giving `MAX_SCHEMA_BYTES` 23 KiB and
  `MAX_SYSTEM_BYTES` 6 KiB in place of the 24 KiB line, and schema version 3;
  the reasoning is left as it was.
- `GLOSSARY.md`: Solo and Track volume under the timeline; a Sound section
  with Audio effect and Sound effect (each saying it is not the other),
  Fade, Waveform, Peak file, Detached audio, Owned sound, Take, Media
  folder, Beat grid, Stem, Ducking and Sound lane; "Effect" now names audio
  effect as what it is not.
- `docs/usage.md`: the Audio section's cards, sound taking audio effects
  and not picture effects, solo saved with the project, every sound phrase
  in "By command", a pointer to the verification section, the non-features
  sentence (no rendered mix; the three sound files the editor writes), and
  what the AI planner cannot start. The rest of the sound guide was written by
  the implementing tickets and was checked, not rewritten.
- `docs/design.md`: the library's kind filter, background sound work and
  host-tool-dependent controls, and the audio states visual acceptance
  must cover.
- `docs/architecture.md`: the ownership table's core, store and media rows;
  an Audio editing section with a concern-to-owner table and how sound
  results enter as commands; the media folder under persistence.
- `docs/performance.md`: the section's conditions, a Mixer level stage and
  master limiter subsection and a Pitch subsection with their figures. The
  peak cache, held peaks, beat grids, lane queues and record ring were
  already written by their tickets and were checked.
- `docs/verification.md`: an Audio editing section in the shape of Advanced
  editing (where the evidence comes from, a twenty-row matrix, the standing
  unverified list), replacing ticket 19's Voiceover recording section, which
  is now a row; an Audio editing row in the implemented feature matrix and
  in the remaining production gates; the advanced-editing row that said v2
  is written and v3 refused now says v3 and v4.
- `docs/agent-protocol.md`: sound work is not a command, and the commands
  each result is made of. `docs/native-ai.md`: fades and solo in the text,
  and what a plan cannot start. Both already said that audio effects are
  heard and that an enabled `pitch` keeps pitch (tickets 10 to 12).
- `docs/installation.md`: the filters the editor runs, the probe, where the
  peak cache lives, and that no microphone permission is set up for a
  packaged build. The stem separator section was ticket 21a's.
- The spec: the header set to what is true; each owner decision marked as
  decided on the owner's behalf; Comments for the owner decisions and for
  the rule ticket 05b changed without an entry.
- Tickets: 01 resolved (its last item was owner decision 1); 21b and 21c
  record the decision; 02, 03, 05a, 05b, 10, 11, 14a and 16a gained a dated
  comment naming the statements later tickets made out of date.

## Evidence

On macOS 26.5.2 (arm64, Apple M1), Rust 1.97.1, 2026-10-14. No Rust source
changed. Run for this ticket:

- `cargo test -p editor-agent --locked --lib -- --nocapture planner::tests`:
  21 passed, printing `planner_instructions: 5377 bytes` and
  `proposal_schema: 21859 bytes`, the figures the documents give.
- `python3 scripts/check-architecture.py`: passed.
- A link check over `docs/`, `GLOSSARY.md` and this epic's files: every
  relative link and heading anchor added here resolves. The links that do
  not resolve are older ones to Linux artifacts (`artifacts/production-readiness/`,
  `artifacts/native/`, `artifacts/advanced-editing/`, `artifacts/packaging/`)
  that are not on this host; none was added or changed.
- Reference review: every constant, function, type, file, thread name and
  interface phrase the new text names was looked up in the code (among them
  `MAX_SYSTEM_BYTES`, `MAX_SCHEMA_BYTES`, `MASTER_CEILING`,
  `LIMITER_RELEASE_SECONDS`, `Mixer.limit`, `Sounding`, `soloed_out`,
  `PITCH_KEPT`, `missing_audio_filters`, `media_folder`, `new_take`,
  `HeldBeats`, `aster-audio`, `aster-take`, "Save the project to record",
  "Replace this clip's volume keyframes?", "Not available in this FFmpeg
  build", "Reduce centre", "Stop ducking", "Key volume", and the commands
  each batch builder in `src/rules.rs`, `src/record.rs` and `src/stems.rs`
  emits).
- Every "Measured" cell of the verification matrix is a figure with its
  conditions in `docs/performance.md`, or "No"; every row names what is
  unverified.

## Known limits

- Documentation only: it establishes no platform acceptance, no audible
  quality and no playback rate.
- Test counts and figures other than the two sizes above are quoted from the
  implementing tickets' runs on the same host, not re-run here. No
  whole-workspace run on one source exists for the epic.
- The matrix says what the tickets recorded; the native captures that need a
  pointer are named as missing, and the tickets that wait on them stay
  claimed. Ticket 03 stays claimed for a person's reading, not a capture.
- Audio figures are from the `dev` test profile (opt-level 1, dependencies at
  2), which is partly optimized and not a release build, or from the
  unoptimized `ci` profile where a row says so.
- Only macOS evidence exists for this epic; the documents say so where the
  rest of the record is Linux evidence.

## Comments

2026-10-13: Written with the spec. Nothing implemented or measured.

2026-10-13 (review): Dependencies follow the split tickets. The glossary
entry was going to give "Sound effect" the filter meaning; it now keeps the
owner's meaning and the filters are "Audio effect".

2026-10-13 (ticket 10 review): Ticket 10 left ADR 0006's 24 KiB sentence for
this ticket, and this ticket did not carry it; it is now in Scope.

2026-10-14: Written from the code and the implementing tickets, on macOS
26.5.2 (arm64). Owner decisions taken on the owner's behalf are in the spec's
Comments of 2026-10-14. No rule was changed by this ticket; the rule ticket
05b changed in review is recorded in the spec (2026-10-13 entry).

2026-10-14 (review fixes): Four findings, all held; documents only, no rule
changed. (1) `performance.md` and `verification.md` called the `dev` test
profile unoptimized; `Cargo.toml` gives it opt-level 1 and its dependencies
2, and only `ci` is unoptimized. Both now say so. (2) The spec header and the
verification list of claimed tickets left out 03, which waits on a person's
reading rather than a capture; both now name it, so the header matches every
ticket's Progress field. (3) ADR 0007 said the only files the editor writes
are a copy, takes and stems, though it writes peak files; it now says sound
files, as `usage.md` does. (4) The glossary's Media folder said `<document
name> media`; `media_folder` uses the file stem, and the entry now says
`<document stem> media` as the other documents do.
