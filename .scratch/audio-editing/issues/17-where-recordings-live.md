# Where recordings live

Status: ready-for-agent
Progress: resolved
Type: grilling

## Question

A voiceover take is the first media the editor itself creates. Today a
project only references paths: nothing copies media, no project media folder
exists, the only app-owned directories are settings and an evictable
stabilization cache, and an untitled project has no folder at all
(`crates/editor-store/src/lib.rs:212-222`, `:287`). Where is a take written,
who may delete it, and what happens in an untitled project?

The spec's decision 9 fixes two things whatever the answer: a take is never
in a cache that evicts, and the editor never deletes one, including when the
clip is undone or removed.

Options:

1. **Beside the document (proposed).** `<document name> media/` next to the
   saved project, takes named `Voiceover <date> <time>.wav`. Recording in an
   untitled project is refused with "Save the project to record". The person
   can see and back up their takes beside the project. They do not yet move
   with it: load resolves a relative asset path against the document's
   folder (`crates/editor-store/src/lib.rs:212-222`), but nothing writes
   one. Import canonicalizes to an absolute path
   (`crates/editor-media/src/import.rs:116`) and save serializes
   `asset.path` as held (`lib.rs:193-205`), so a take is stored by absolute
   path and goes missing when the folder is moved or copied to another
   machine, unless the first item below is taken.
2. **App data directory.** `ProjectDirs::data_dir()/recordings/<project
   id>/`. Works untitled. The takes are hidden from the person, orphaned
   when a project file is deleted, and left behind when it is moved to
   another machine.
3. **Ask each time.** A save dialog per take. No policy, and a dialog
   between "stop" and seeing the clip.
4. **1 with 2 for untitled projects,** moving takes on first save. The move
   is a second way to lose a take.

Also to settle:

- Does save write a path under the document's own folder as relative? That
  is what would make option 1's takes (and ticket 21a's stems) move with
  the project. It is a store change with its own tests: file, checkpoint
  and recovery round trips (checkpoints live in `<doc>.checkpoints/`, a
  different folder from the document, so the base must be the document's),
  "Save as" to another folder, and a path that only looks relative. Or is
  option 1 accepted without move-with-project for now?
- "Save as" to another folder: leave the takes where they are (absolute
  paths keep working) or offer to copy them?
- A bound on one take (proposed: 2 hours) and a free-space check before
  starting (proposed: refuse under 256 MiB free).
- Is the folder name fixed, or a setting?
- Does the same folder later hold "Save audio as…" output (ticket 20) and
  stems (ticket 21a), or only takes?

## Answer

Decided on the owner's behalf on 2026-10-14: the owner delegated every open
decision of the epic. Option 1, without move-with-project.

- **Where.** `<document stem> media/` in the document's own folder:
  `film.aster` keeps its takes in `film media/`. The name is fixed, not a
  setting. `editor_store::media_folder(document)` names the folder and
  creates nothing; `editor_store::new_take(document, now)` makes the folder
  on the first take and returns a path for the take that does not exist yet,
  `Voiceover 2026-10-14 15.30.12 UTC.wav`, with ` 2`, ` 3`… when that name
  is taken (by a file, a folder or a link, dangling included, since
  `create_new` refuses all three). A name that cannot be looked at is an
  error, not a free name. Ticket 18's writer opens it with `create_new`,
  which refuses one made in between. The stamp is UTC and says so: the
  workspace has no time zone database and no crate is added. Dots, not
  colons, which Windows refuses in a name.
- **Where it runs.** `new_take` makes a folder and looks at names, possibly
  on a slow or sleeping volume, so it runs off the frame loop: the desktop
  calls it in a short job of its own (as `find_orphan` uses `self.spawn`,
  `src/app.rs`) whose answer carries the project instance and starts
  `RecordService`; an answer for a replaced project starts nothing.
- **Who deletes.** Nobody in the editor. The folder and everything in it are
  the person's: undo, removing the clip or the asset, a take too short to
  place, a take whose project was replaced, closing the project and Save as
  delete, move and empty nothing (decision 9).
- **Untitled project.** No document, which includes recovered unsaved work:
  Record is refused with "Save the project to record", option 1's proposed
  default. A Save button may sit beside the words; it only saves, and the
  take is not started by it: the person presses Record again. No input
  device is opened before the document has a path. Saving and then
  recording by itself is left to the owner as an open alternative (spec,
  owner decision 5): it would start the microphone without a fresh press,
  and `after_save` (`src/app.rs`) re-asks whatever waited on a save when
  edits came in while the file was written, so it could not carry Record
  unchanged.
- **Paths stay absolute.** No store change and no format change: save
  writes `asset.path` as held, and a take's path is the canonical one its
  writer reports. A take therefore does not move with the project: moved or
  copied to another machine without the same folder, the asset is missing,
  and the editor has no relink route today. Writing paths under the
  document's folder as relative is left as a follow-up with the tests this
  ticket listed (file, checkpoint and recovery round trips, Save as to
  another folder, a path that only looks relative).
- **Save as to another folder.** Takes stay where they were recorded and
  their absolute paths keep working; nothing is copied or offered. Later
  takes go beside the new document. Duplicate is the same.
- **Bounds.** A take ends itself at 2 hours, or sooner at the WAV size
  limit, and is placed as if stopped. With ticket 18's 44-byte header the
  RIFF size is 36 + data and both it and the data size are `u32`, so the
  limit is data bytes ≤ `u32::MAX − 36` = 4,294,967,259, rounded down to
  whole frames: 4,294,967,256 for mono or stereo float (about 3 h 6 min of
  stereo 48 kHz, 1 h 33 min of stereo 96 kHz). Exactly 4 GiB would wrap
  both fields. No free-space check before
  starting: the standard library has no free-space query, and the platform
  crates the lockfile holds (`libc`, `windows-sys`) are not `editor-media`
  dependencies; adding one would change a manifest, which the spec rules
  out. Instead a failed write, a full disk included, stops the take, keeps
  what was written and says so (ticket 18). The 256 MiB threshold is
  therefore not built.
- **What the folder holds.** Takes, and ticket 21a's stems (its `<to>` is
  this folder). "Save audio as…" (ticket 20, built) still writes where the
  person picks.

Built and tested in `editor-store` (`src/media.rs`,
`tests/media_folder.rs`): the folder's name and that naming it creates
nothing; a path with no file name refused; the first take makes the folder;
a name held by a file or a folder gets the next number and the earlier take
is untouched; a name held by a dangling link gets the next number (Unix); a
name that cannot be looked at is an I/O error (Unix); a file in the
folder's place refused and kept; a folder that cannot be made is an I/O
error and names no take (Unix); a clock before the
epoch refused; a take's absolute path written as held and loaded unchanged
through save, Save as to another folder (which gains no media folder) and a
checkpoint; UTC stamps over leap days and century years. Nothing in the
desktop calls these yet; tickets 18, 19 and 21a do.

## Comments

2026-10-13: Written with the spec. Tickets 18 and 19 wait on this. All
three proposals recommended option 1; one recommended option 4.

2026-10-13 (review): Option 1 claimed takes would move with the project
because relative paths resolve on load. Nothing in `editor-store` writes a
relative path (no relativizing code in `lib.rs` or `document.rs`), so the
claim was false of the code. The wording is corrected and the store change
is now a question of its own. Ticket 21a also waits on this ticket.

2026-10-14: Answered and resolved, decided on the owner's behalf (see
Answer). `media_folder` and `new_take` added to `editor-store`; tests in
`crates/editor-store/tests/media_folder.rs` pass on macOS 26.5.2 (arm64,
Rust 1.97.1). Tickets 18, 19 and 21a are unblocked by this ticket. Ticket
22 documents the folder.

2026-10-14 (review): Four rules changed. (1) The untitled-project rule had
become "save, then record by itself", which is not the proposed default; it
is back to the refusal, with a Save button that only saves, and the
alternative is an open owner question. (2) `new_take` used `Path::exists`,
which follows links: a dangling link counted as free and the writer's
`create_new` then refused it. A name is now free only when
`symlink_metadata` says not found; any other error is reported. (3) The size
limit was "4 GiB of data", which wraps the header's `u32` sizes; it is now
`u32::MAX − 36` rounded down to whole frames. (4) `new_take` is filesystem
work and is called off the frame loop.
