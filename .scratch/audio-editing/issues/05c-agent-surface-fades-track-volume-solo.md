# Agent surface for fades, track volume and solo

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 01, 05a

## Scope

Name the new command and fields to an agent, within the two caps measured
in ticket 01.

- `crates/editor-agent/src/lib.rs`: `set_fades` in the capability list.
- `context.rs`: project `gain` and `solo` on tracks and `fade_in` and
  `fade_out` on clips in the exhaustive destructures (`LeanTrack`,
  `LeanClip`, `SummarizedClip`, the focused structs), skipped at their
  defaults.
- `planner.rs`: one sentence in `SYSTEM`: fades are lengths in microseconds
  at a clip's edges, for clips with sound; track `gain` is linear 0 to 8.
  The planned-command test is extended with `set_fades` and a
  `set_track_state` carrying `gain` and `solo`. Both sizes are re-measured
  and printed.
- `docs/agent-protocol.md` (command row and rules), `docs/native-ai.md`
  (bounds row).

Files: `crates/editor-agent/src/lib.rs`, `context.rs`, `planner.rs`, the two
docs.

## Result

Implemented. The agent is told of fades, track volume and solo; nothing new
is heard or drawn by this ticket.

- `context.rs`: `LeanTrack` and `FocusedTrack` send `gain` and `solo`, and
  `SummarizedClip` sends `fade_in` and `fade_out`, each skipped at its
  default as the document skips it. The two destructures bind the four
  fields in place of ignoring them. A clip without a long animation track
  is still sent as the document's own `Clip`, which already had the fields.
- `planner.rs`: `SYSTEM` gains "Fades are lengths in microseconds at a
  clip's edges, for clips with sound; track gain is linear 0..8." after the
  sentence on audio-track clips. `look_and_motion_commands_are_planned_and_
  committed_like_any_edit` plans a `set_fades` and a `set_track_state` with
  `gain` and `solo` in its batch, on an asset that now has sound, and
  refuses fades longer than the clip and a track volume of 8.5.
- `lib.rs`: unchanged here; `set_fades` was listed with ticket 05a.
- `docs/agent-protocol.md`: a "Sound commands" section with the `set_fades`
  and `set_track_state` rows and their rules. `docs/native-ai.md`: one
  bounds row.
- Routes. The serialized command is still the only route to these fields
  (05a's Result); the shortcuts, menus, gestures and inspector rows are
  tickets 07 and 08a, and no panel phrase is added here. What this ticket
  owns is that route's two ends: the JSON an external agent sends and the
  context a native plan reads.

## Evidence

On macOS 26.5.2 (arm64), Rust 1.97.1, 2026-10-13, `dev` profile. No provider
request was made.

- `cargo test -p editor-agent --locked --lib -- --nocapture context::
  planner::`: 32 passed. Printed: `planner_instructions: 4880 bytes` (102
  more than 4,778; 239 left under 5 KiB) and `proposal_schema: 21783 bytes`
  (unchanged from 05a; 2,793 left under 24 KiB).
- New in `context.rs`: `fades_track_volume_and_solo_are_sent_when_set_and_
  absent_at_their_defaults` (a complete document: an exact clip, a
  summarized clip with fades, a summarized clip without, a track with both
  fields and tracks with neither) and `focused_view_sends_fades_track_
  volume_and_solo_and_omits_their_defaults` (the five-thousand-clip view).
- `cargo test -p editor-agent --locked --test protocol`: 15 passed.
  `capabilities_name_every_command_in_the_schema` now also asks for
  `set_fades` by name. New: `serialized_fades_track_volume_and_solo_are_
  validated_inspected_and_undone` sends the commands as JSON text: four
  refusals (fades past the clip, a negative fade, volume 8.5, a batch whose
  second command fails) answer `invalid_edit` and leave the same snapshot
  and revision; the two accepted commands show in an inspection, with the
  defaults of another track absent; two undos restore the project.
- `python3 scripts/test-workspace.py --package editor-agent` (`ci`
  profile): 71 of 72 unit tests pass. The one failure is
  `provider::tests::an_unrepresentable_native_path_is_refused_before_
  creating_an_instruction_file`, which fails on this host's filesystem
  before the code under test runs, as tickets 01 and 05a record. The script
  stops there, so the protocol tests were run by the command above.
- `cargo clippy -p editor-agent --all-targets --locked -- -D warnings`,
  `cargo fmt --all -- --check` and `python3 scripts/check-architecture.py`:
  clean. The desktop crate was not built again: no type it uses changed.

Done when `capabilities_name_every_command_in_the_schema` passes,
a context test shows the four fields present when set and absent at their
defaults, the planned-command test accepts the two commands, and the schema
and instruction sizes are printed under 24 KiB and 5 KiB.

## Known limits

- No real provider request is made; what a model does with the sentence is
  unverified.
- The sentence on audio-track clips is replaced in ticket 10, not here.
  Until then `SYSTEM` says such clips "take only gain keyframes and retime"
  one sentence before it says clips with sound take fades.
- `SYSTEM` does not name solo or say that the two fades must fit in the
  clip; the schema carries `solo` and validation refuses the rest.
- 239 bytes are left in `SYSTEM`. Ticket 10's text needs 375 (forced lines
  163, units sentence 180, the replaced sentence 32), so owner decision 1
  still has to be answered before it.
- Linux and Windows were not run.

## Comments

2026-10-13 (review): Split from the former ticket 05, following
advanced-editing ticket 05.

2026-10-13 (05a review): `set_fades` is already in the capability list in
`crates/editor-agent/src/lib.rs`; it was added with ticket 05a so that
`capabilities_name_every_command_in_the_schema` is not red between the two
tickets. The rest of the scope is unchanged.

2026-10-13 (implemented): Built and tested on macOS as recorded under
Evidence. Read where the ticket left room:

- *The focused view* skips the four fields at their defaults too, so a
  focused track has `muted` always and `gain` and `solo` only when set.
- *The docs* gained a "Sound commands" section in `agent-protocol.md`
  in place of rows in the look, motion and speed table, and `set_track_state`
  has a row there for the first time.

2026-10-13 (adversarial review): One finding, a test gap; no rule changed.
`planner_text_names_every_parameter_table_and_stays_short` now also asserts
that `SYSTEM` contains the fades and track-gain sentence word for word, and
the planned-command test accepts a track volume of exactly 0 and exactly 8,
the two ends the sentence states, beside the existing refusal of 8.5.
Rerun on the same macOS host, `dev` profile: the lib `context::` and
`planner::` filter 32 passed with the same two sizes (4880 and 21783
bytes), `--test protocol` 15 passed, clippy with `-D warnings` on
`editor-agent`, `cargo check --locked` on the root crate and
`cargo fmt --all -- --check` clean. Ticket 10 replaces the neighbouring
sentence and must keep this assertion in step if it rewords this one.
