# Store schema v3

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 05a

## Scope

Raise the document version once, for the fields of ticket 05a and the audio
kinds of ticket 10. No migration code: versions 1 and 2 read through serde
defaults.

- `crates/editor-store/src/lib.rs`: `SCHEMA_VERSION = 3`. Read accepts 1, 2
  and 3; write is always 3; 4 is refused and protected as a future file.
- `crates/editor-store/tests/schema.rs`: a v1 and a v2 fixture load and the
  next save writes 3; "version three is refused" becomes version four, at
  every entry (open, checkpoint, recovery); file, checkpoint and recovery
  round trips with fades, track gain and solo.
- No recovery archive change: nothing new is project-level.

Files: `crates/editor-store/src/lib.rs`, `tests/schema.rs`, the fixtures
beside it, `docs/architecture.md` (the version line, if it names one).

## Result

Implemented. `SCHEMA_VERSION` is 3 in `crates/editor-store/src/lib.rs`;
versions 1 and 2 are read as strict subsets and never written, and version 4
is refused through the comparison that refused 3. No migration code and no
change to the recovery archive: `recovery.rs` and `document.rs` are untouched,
since a track and a clip are written whole wherever they are written.
`list_checkpoints` now reads through the same version gate as every other
entry (`read_versioned`) before it parses the body, where it used to parse
first and compare after.

- Fixtures. `crates/editor-store/tests/fixtures/` holds a project, a
  checkpoint and a recovery archive of one session with every advanced
  feature, written by this tree's store while it still wrote version 2 (the
  step before the constant was raised) and not edited since. The version 1
  fixtures stay inline JSON in `schema.rs`.
- Routes. The store has three entries and each is covered: the document
  (`load_project`, `load_project_with_session`, `save_project`,
  `save_project_with_session`), checkpoints (`checkpoint`,
  `list_checkpoints`) and recovery (`load_recovery`,
  `load_recovery_with_document`, `save_recovery`). Shortcuts, menus,
  gestures, the inspector and agent commands reach the store only through
  these; nothing in `src/` or `editor-agent` names a version.
- `docs/architecture.md`: the version line.

## Evidence

`crates/editor-store/tests/schema.rs`, 19 tests, on macOS 26.5.2 (arm64,
Rust 1.97.1):

- Version 1 project, checkpoint and recovery load and the next save writes 3.
- Version 2 project, checkpoint and recovery fixtures load and the next save
  writes 3. The saved project is the fixture's but for the version and the
  resolved media path, so nothing is stripped and no new key appears; the
  archive's two undone steps redo to the saved document before and after it
  is rewritten.
- Version 4 is refused and the file left as it was at open, session open,
  one-shot save, recovery load (both loaders), recovery save, checkpoint
  list, and a checkpoint opened or saved over as a document. The version 4
  document and checkpoint hold an effect kind and a project key this build
  does not know, and the archive holds no version 3 shape at all; the same
  document marked version 3 is refused as damaged, so the refusal comes from
  the version and not from a body that happens to parse. Version 0 stays
  unsupported at open, save, recovery load and checkpoint list.
- Fades, track volume and solo round-trip through a project file (set values
  written, defaults writing no key), a checkpoint and a recovery archive with
  history (undone steps, shared tracks, redo and undo to nothing).
- A file or archive whose fades or track volume are out of bounds is refused
  with the core's reason and preserved; a project with such fades is written
  nowhere.

The whole crate's 85 tests pass (`cargo test -p editor-store --locked`).

## Known limits

- Version 3 is not complete until ticket 10 adds the audio effect kinds; a
  file written between the two tickets is read by the finished build, and no
  build is released between them.
- A build without this epic refuses a version 3 file through the existing
  protected path; it cannot open it read-only.
- No build that knows only version 2 was run against a version 3 file. The
  refusal is shown for version 4 on this build through the same comparison.
- The version 2 fixtures were written by this tree before the constant was
  raised, with ticket 05a's fields already in the model at their defaults;
  no build from before the epic wrote them. They hold no key version 2 did
  not have, which the tests check by comparing the next save to them.
- `docs/verification.md` (the advanced-editing row that says version 3 is
  refused) and ADR 0006 still name version 2; they are ticket 22's.
- One checkpoint that cannot be read (future, version 0 or damaged) fails
  the whole checkpoint list with that file's reason; the others are not
  listed beside it. That behaviour predates this ticket and is unchanged.
- A checkpoint with no `schema_version`, or version 0, is now refused by the
  list as `Invalid` like every other entry refuses it. Before the review fix
  a version 0 checkpoint whose body parsed was listed, and then refused when
  opened.
- Verified on one macOS host only.

## Comments

2026-10-13 (review): Split from the former ticket 05, following
advanced-editing ticket 04.

2026-10-13: Implemented as above. The spec gained no entry: no rule was
settled or changed.

2026-10-13 (review fix): `list_checkpoints` parsed the whole document before
it compared the version, so a version 4 checkpoint holding anything this
build cannot parse (a new effect kind, the reason spec rule 10 gives for the
bump) was reported as a damaged file and not as a future one. The earlier
test wrote a version 1 body under the number 4 and passed for this entry
without proving it. The list now goes through `read_versioned`, and the
future fixtures hold a body this build cannot parse at every entry. Rule
changed: a version 0 or unversioned checkpoint is refused by the list
(`Invalid`), where a version 0 checkpoint whose body parsed used to be
listed. No file is modified either way. The spec gained no entry.

2026-10-14 (ticket 22): Version 3 is complete: ticket 10 added the audio
kinds under it. ADR 0006 now carries a dated note giving version 3, and the
advanced-editing row of `docs/verification.md` says the writer emits 3 and
refuses 4. The rule changed in this ticket's review (the checkpoint list's
version gate) is now in the spec's Comments.
