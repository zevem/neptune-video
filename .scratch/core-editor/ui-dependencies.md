# UI stack, dependencies and requested backend interface

Root owns the workspace manifest and core crates; this file is the UI side of
the coordination. Status lines at the bottom are updated as the UI lands.

## Stack (taken from Neptune, verified in its Cargo.toml and src/)

Neptune is `eframe`/`egui` 0.36.2 on `wgpu` 30: no webview, immediate mode,
natively drawn vector icons on a 24-point grid (`src/icons.rs`), theme tokens
in one `Palette` (`src/theme.rs`), Geist for interface text, workers for all
I/O and a window that sleeps when nothing changed. The editor reuses that
stack unchanged:

- Toolchain: Rust 1.97.1, edition 2024, resolver 3 (same as Neptune).
- `eframe` 0.36.2 App trait is `logic(ctx, frame)` + `ui(ui, frame)`, not the
  older `update`.
- Present mode: `AutoNoVsync` on Linux (Neptune measured multi-second FIFO
  acquire stalls on NVIDIA hybrid graphics), `AutoVsync` elsewhere.
- Native OS window decorations for now. Neptune's undecorated transparent
  window needs per-platform resize/drag code that is only verified on X11.

## Desktop package dependencies (root: please add to the desktop `[dependencies]`)

```toml
eframe = { version = "0.36.2", default-features = false, features = ["default_fonts", "wgpu", "wayland", "x11", "accesskit"] }
rfd = "0.17.2"            # native open/save/folder dialogs
image = { version = "0.25", default-features = false, features = ["png", "jpeg", "gif", "webp", "bmp"] }
anyhow = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
directories = "6"         # settings file location
```

Recommended profiles (Neptune's; egui is slow to use unoptimised):

```toml
[profile.dev]
opt-level = 1
[profile.dev.package."*"]
opt-level = 2
[profile.release]
lto = "thin"
codegen-units = 1
strip = "debuginfo"
```

The UI needs nothing else. It does not want an ffmpeg, audio or GPU dependency
of its own: decoding and audio output belong behind the core's interface.

## Layout the UI will own

```
src/main.rs            window options, launch arguments
src/lib.rs             module tree (testable)
src/app.rs             frame loop, action dispatch, shortcuts, playback clock
src/theme.rs icons.rs  tokens and native icons
src/ui/*.rs            toolbar, library, preview, timeline, inspector, agent, menus, settings
src/runtime/*.rs       UI-side helpers only: texture cache, settings file, worker glue
assets/fonts/          Geist (OFL), copied with its licence
```

## Interface the UI would like from the core

Whatever `backend-contract.md` says wins; this is what the UI needs so that the
contract can be checked against it. Names are suggestions.

1. **One owned document, mutated only by commands.** `Editor::apply(Command) ->
   Result<_, Error>` with `undo()`, `redo()`, `can_undo()`, `can_redo()` and a
   `revision() -> u64` that changes on every mutation (the UI caches layout by
   it). Read access as plain borrowed structs: project settings (width,
   height, frame rate), media items, tracks, clips, markers.
2. **Stable ids** (`MediaId`, `TrackId`, `ClipId`, `MarkerId`) that are `Copy +
   Eq + Hash` and survive undo/redo and save/load.
3. **Time as an exact type** (ticks or rational), with conversion to seconds and
   to frames at the project rate. The UI never stores f64 seconds in the model.
4. **Commands the timeline needs**: add/remove track; mute/lock/hide track;
   insert clip from media at (track, time); move clip (track, start); trim
   start/end; split at time; delete (with optional ripple); duplicate; set
   transform (position, scale, rotation, opacity), volume and speed; add/move/
   remove marker; rename. A way to group one drag into a single undo step
   (`begin`/`commit` or a "replace last" flag).
5. **Import off the UI thread**: `import(paths)` returning immediately, with
   progress/results polled or delivered on a channel, carrying probed metadata
   (kind, duration, dimensions, frame rate, audio channels) or a per-file
   error.
6. **Frames for real textures**: `frame(media, source_time, max_size)` giving
   RGBA8 (non-blocking: returns cached or schedules and reports later), plus
   thumbnails and audio waveform peaks. The UI uploads them as `wgpu` textures
   through egui and keeps a bounded cache.
7. **Composition as data**: `layers_at(time)` → ordered list of
   `(clip, media, source_time, transform, opacity)`, so the preview draws the
   same stack the exporter renders.
8. **Project file**: `new`, `open(path)`, `save(path)`, a dirty flag.
9. **Agent**: `submit(prompt) -> id`, events (`started`, `message`, proposed or
   applied commands, `failed`) polled each frame; applied edits go through the
   same command path so they undo like any other.
10. **A wake callback** (`Arc<dyn Fn() + Send + Sync>`) the core calls when a
    worker has news, so the window can sleep instead of polling.

## Status

- 2026-10-12: stack chosen, waiting for `backend-contract.md`.
- 2026-10-12: read `backend-contract.md`; the UI is being written against it.
  The manifest already has what the UI needs (no `anyhow`; the UI uses its own
  error strings). The UI declares its modules from `src/main.rs` and adds **no
  `src/lib.rs`**, so the root manifest's targets are unaffected.

## Assumptions the UI makes (root: correct me here if any is wrong)

1. **Stacking order.** `Project::tracks` is bottom-to-top for pictures: a later
   visual track draws over an earlier one (so the default T1 text sits over
   V1). The timeline shows visual tracks top-down in reverse `Vec` order and
   audio tracks under them in `Vec` order.
2. **Scale 1 means "fit".** A media clip with `Transform::default()` is
   aspect-fitted (contained) in the canvas; `scale` multiplies that size,
   `position` is canvas pixels from the centre (+y down), `rotation` is
   clockwise degrees about the clip's centre, `crop` trims the source before
   fitting. Export must draw the same.
3. **Text clips** are centred on `position`, `font_size` is canvas pixels at
   scale 1.
4. `EditError`, `StoreError` and `MediaError` implement `Display`.
5. `FrameReady.size` echoes the *requested* size, so the UI can route a frame
   to the lane (library thumbnail or viewer layer) that asked for it.
6. `Playback::tick` returns `true` while the position advanced.

## Gaps the UI cannot fill itself (requests)

- **Agent protocol.** The contract names `src/bin/aster-agent.rs` but no
  interface. The panel currently runs a local, real interpreter (plain
  commands and raw JSON `Command`s, dispatched through `Editor::dispatch`).
  To drive an external agent the UI needs: how to start it, what it reads and
  writes (line-delimited JSON on stdio?), and the event shapes.
- **Export/render.** No API, so the UI has no Export control yet.
- **Audio waveform peaks** per asset (`Vec<[i8;2]>` or similar at a fixed
  bucket rate). Audio clips are drawn without a waveform until then.
- **Two clips of one asset on screen at once**: frames are latest-win per
  asset+size, so the viewer asks for layer `n` of the same asset at height
  `h + 2n` to get a lane of its own. A lane key in `request_frame` would be
  cleaner.


## UI status — 2026-10-12 (Claude Opus 5.5 high)

Desktop UI is implemented and wired: `src/main.rs`, `src/app.rs`, `src/{theme,icons,settings,timefmt,textures,rules,agent}.rs`, `src/ui/{mod,widgets,toolbar,library,viewer,timeline,inspector,agent,sheets}.rs`. No backend crate, root manifest, `src/bin/aster-agent.rs` or project doc was changed. No new dependency is needed beyond what the root manifest already has.

Verified here (Linux, Wayland, wgpu): `cargo check`, `cargo clippy --bin aster -- -D warnings` clean, `cargo test --bin aster` 29 passed, and native runs with real PNG/JPEG/GIF imports captured via `aster --screenshot PATH [--screenshot-delay S] [--size WxH] [--settings PATH]`. A live `aster-agent --connect` session added tracks/clips/markers, undid/redid, and a stale revision was refused; the window followed.

### ui-review.md findings

1. Done. `Playback::tick_with_audio(now, duration, audio.position())`; `sync` only on transport/document changes.
2. Done. A project without a file keeps its recovery copy at `<settings dir>/recovery/<editor instance id>.aster.recovery`; the newest one is offered on a launch without a project (sheet: Restore / Discard). A restored copy keeps writing to the same place until it is saved to a file.
3. Done. If the document is still dirty after the acknowledgement, whatever waited on the save asks again instead of proceeding.
4. Done. Completions are matched on `context_id` + `project_id` (+ the save target path).
5. Done. The sidecar is removed only when the document is clean after `mark_saved`; if a recovery write is in flight it is removed when that lands; if dirty a fresh copy is requested at once.
6. Done. Load/recovery results carry the revision at their start; a dirty document that moved on is asked about, not replaced. Same for the open dialog.
8. Done. Unused icons, `Action::{AgentClear,Problem}` and the `Hit::Lane` payload are gone; `Settings::forget` is used when a recent project fails to open.

### Notes for root

- `Editor::default()` is dirty by construction (`saved: None`). The UI makes new projects with `Editor::new(Project::default())` so an untouched window is not "Edited" and is not asked about on quit. If root wants the default to be clean, the UI's `blank()` helper can go.
- Two windows share one recovery folder: a second window launched without a project may offer the first window's live recovery copy. A per-file lock or owner pid in the recovery file would close that; it needs store support.
- Still open from "Gaps" above: waveform peaks (audio clips draw a level line, not a waveform), a lane key for repeated assets in `request_frame` (the UI varies the requested height by 2 px per occurrence), export (out of scope).
- Not built in the UI yet: named-checkpoint save/restore and project duplicate menus (store API exists), library sort order, asset relink. Input-driven behaviour (drag, trim, marquee, shortcuts, file dialogs, save) compiles and is unit-tested at the rules level but was not exercised by synthetic input here.

## UI round 2 — 2026-10-12 (Claude Opus 5.5 high) — compile window released

Built, all reachable from the interface:

- **Clipboard.** Copy/Cut publish the clips to the system clipboard as text (`{"aster_clips":1,…}`, `src/clipboard.rs`); a paste event is taken only when its text is that. Text fields and other clipboard text are left alone; the timeline menu still pastes the window's own last copy. Clips also paste into another Aster window when their media is in that project.
- **Checkpoints.** Project menu → Checkpoints…: name + Save, list with age and Restore. One worker at a time for write/list/read. Restore is `Command::RestoreProject` after an off-thread `load_project`: one undo step, same file, project id and context. Command line: `checkpoint NAME`, `restore NAME`, `checkpoints`.
- **Duplicate.** Project menu → Duplicate…: save dialog, `duplicate_project` (fresh id, name from the file), written on a worker; the window carries on in the copy when the document did not move meanwhile. The source file is never written; a duplicate onto its own file is refused. Command line: `duplicate project /abs/path.aster`.
- **Library sort.** Sort button: Name, Duration, Kind, File size × Ascending/Descending; kept in settings (`library_sort`).
- **Folders.** Add menu → New folder (named inline, made in the folder in view); asset menu → Move to; drag an asset onto a folder, or onto "All media" in the breadcrumb.
- **Recovery leases.** The lease is acquired on a worker before the first recovery write and passed with `request_with_lease`; scans and the open-project offer skip active copies; Restore and Discard acquire first; a window removes or rewrites only a copy it owns.
- Fixed on the way: media named on the command line beside a project was cancelled when the project finished loading; menus took the popup's full width.

Verified (Linux, X11 on the task-owned Xvfb :92, `ci` profile): `cargo clippy --bin aster -- -D warnings` clean, `cargo test --bin aster` 37 passed, `scripts/native-smoke.py` 9/9 (`artifacts/native/smoke3/report.json`), and the round-2 behaviour driven by real XTest input with captures in `artifacts/native/ui-round2/` (`drive.py` is the helper used; the steps were run one at a time, not as one script).

Notes for root:

- I ran a workspace-wide `cargo fmt` once by mistake. `crates/editor-store/src/worker.rs` was not rustfmt-clean before and is now (whitespace only, e.g. `context_id:x` → `context_id: x`); no other backend file differs. Later formatting used `rustfmt src/main.rs` only.
- No `RemoveFolder`/`RenameFolder` command exists, so a folder can be made and filled but not renamed or deleted except by undo.
- `.lock` sidecars are never unlinked (your rule), so each unsaved session that wrote a recovery copy leaves an empty `<id>.aster.lock` in the settings `recovery/` folder.
- Checkpoints cannot be deleted from the interface (no store function).
- Not driven natively: the rfd file dialogs (Open, Save as, Duplicate…, Import) — the test display has no portal, and the session bus was pointed away so no dialog could reach the real desktop. Duplicate was exercised through the command line, which shares everything after the dialog.
- Still open: waveform peaks, asset relink, export.

## UI round 3 — 2026-10-12 (Claude Opus 5.5 high) — compile window released

Frontend only (`src/**`); no backend crate, manifest, script or doc was edited.

**Source monitor (asset previews).** A library asset plays from its file in the viewer's place, on a `Project::asset_preview` document that is thrown away. Nothing is dispatched: project, revision, history and dirty state are untouched.

- Ways in: asset context menu → Preview; the play badge on a tile or row under the pointer; Enter on the selected asset; the inspector's Preview button; command line `preview NAME` / `close preview`. With the monitor open, choosing another asset in the library shows that one.
- Ways out: Esc, the close button, or anything asked of the timeline (its transport, a ruler seek, selecting a clip, `play`/`go to`). The timeline is paused on opening and is where it was on closing. "Add to timeline" stays in the monitor's header; library drag and double-click are unchanged.
- Transport: own `Playback`, same keys as the timeline (Space, ←/→, Shift+←/→, Home/End, J/K/L, Cmd+L), a scrub bar, speed and loop. Positions stay inside the source; the frame shown never passes the last one. `AudioService::position()` is the clock when the source has sound, the wall clock otherwise. Actions are `Action::Source(Transport)`, separate from the timeline's. While the monitor is open the keys that edit at the timeline playhead (S, M, T, Delete) are not heard.
- Audio shows its icon, name, length and size with the same transport; no waveform is drawn. A still shows the picture and its facts, with no transport.
- The shared transport row now drops to steps + play, then play only, when the viewer is narrow (it overlapped the timecode before, in the timeline viewer too).

**Texture/decode invalidation.** `App::media_changed` compares the library with the one last seen (pointer-equal `Arc` → nothing to do). Per asset it compares path, kind, duration, size, frame rate, has_audio and file size (`textures::stale`). A changed or removed asset loses its poster and layers. Only a *changed* one calls `MediaService::cancel()`; what was asked for and not delivered is asked again, and import batches still waiting are handed back to the worker (`App::imports` keeps their paths). Names, folders, clips, transforms and trims cost nothing. A frame for media no longer in the library is dropped. The source monitor reopens on the asset's new description, or closes when it is gone.

**Autosave.** `Autosave { at, kept, pending }`: `kept` moves only on the worker's successful answer; a failed write, a refused request and a failed lease all leave the revision due again after the interval. One write in flight at a time. The lease is still passed with `request_with_lease`.

Verified (Linux, X11 on the task-owned Xvfb :92, `ci` profile, Rust 1.97.1):

- `cargo clippy --profile ci --bin aster -- -D warnings` clean; `cargo test --profile ci --bin aster` 47 passed (10 new: source transport bounds, clock end/loop, still, last-frame clamp, lane separation, stale-asset rules ×2, autosave retry ×2, `preview` command).
- `scripts/native-smoke.py` 9/9 on the final binary: `artifacts/native/ui-round3/smoke/report.json`.
- Native XTest runs, captures in `artifacts/native/ui-round3/` (`drive3.py` is the helper; steps were run one at a time):
  - 02–06, 08–09, 23: menu → Preview, play (00:00:03:08 after about 3 s), frame steps, End, scrub click, on H.264+AAC files. 10–11: another asset chosen in the library, playing 1080p. 12–16: GIF from the hover badge, playing and paused. 17–19: WAV. 21: PNG.
  - Project JSON and revision from the live agent session were identical before and after all of the above (`state-before.json` is the later run's reference); "Add to timeline" from the monitor was one history entry and one undo restored equality (24). 20, 26: Esc and a ruler seek give the viewer back to the timeline at its own position.
  - 27–31: `restore_project` through the agent session with the same asset id pointed at another file: library poster, timeline thumbnail and viewer showed the new file (28); undo showed the old one (29); redo and undo again with the monitor open on that asset followed (30, 31).
  - Autosave: project folder made read-only after an edit; the recovery copy stayed unchanged for two intervals; 9.9 s after the folder was writable again the copy held the edit, with no new edit (revision 8 before and after). The failure notice itself is not in a capture (32 missed it).
  - 35, 36, 38: 780×560 window, monitor and timeline viewer, final build.
- Captures 00–06, 08–21 are from the build before two small fixes found while driving (a click on the scrub bar that began and ended in one frame did nothing; Space on a still raised a notice). 23 onward and the smoke run are the final build.
- Test media was generated for the task at about −84 dB so the real output device ran without being heard; audible output was not judged.

Open, for root:

- **GIF playback lags in the `ci` build.** Paused GIF frames are exact (frame 22 at 2.2 s, capture 15). While playing, the picture was about 1.3 s behind its playhead (frame 7 at 2.07 s, capture 14). Video did not lag. Not measured in a release build; the decode is the backend's.
- A GIF has no `frame_rate`, so its monitor steps on a 30 fps grid: three presses per 10 fps frame.
- `cargo clippy --tests` fails in `src/bin/aster-agent.rs` (two `collapsible_if`, lines 63 and 103). Not mine to edit.
- With Shift held, XTest → moved the timeline one frame, not one second (seen once while driving; not investigated).
- **Wayland file drop is a release gate, not supported today.** winit 0.30.13's Wayland backend never emits `DroppedFile`/`HoveredFile` (only `platform_impl/linux/x11` does), so `raw.dropped_files` stays empty on a Wayland session; the import button and command-line paths work there. Neptune fills this with its own `wl_data_device` listener on the window's connection (`src/platform/file_drag.rs`, the `linux::wayland` module plus `uri_list_paths`). Porting it needs two direct dependencies in the desktop manifest, both already in `Cargo.lock` transitively: `wayland-client = "0.31"` and `wayland-backend = { version = "0.3", features = ["client_system"] }` (Linux only), and the `eframe::Frame` display/window handles in `App::logic`. I did not add them (root's manifest) and did not port the helper untested: this task's display is X11. X11 drop goes through winit and was not driven here either (no XDND source on the test display).
- Still open from before: waveform peaks, asset relink, export, folder rename/delete, checkpoint delete.


## UI round 4 — 2026-10-12 (Claude Opus 5.5 high) — compile window released

Frontend only: `src/app.rs`, `src/main.rs`, new `src/file_drop.rs`. No backend crate, manifest, lockfile, script or doc was edited. No Cargo process of mine is running.

**Shutdown race (item 1).** `on_exit` no longer calls `save_recovery` beside the worker. `part_with_recovery` queues the last copy with `SaveWorker::request_with_lease` *before* `flush(2 s)`; the lease is the one held, one a worker was already asked for (taken from the job channel, waited for at most 1 s), or a new one. A direct write happens only when the worker refused the request *and* the flush succeeded (idle worker, nothing to race). A copy is removed (quit with "don't save", or a clean document whose copy this window owns) only after a successful flush; when the flush times out the copy is left and is offered at the next launch (documented on the function). Copies of documents given up earlier in the session whose write only landed during that flush are removed then too, under a freshly acquired lease, never the current base.

**Shift+Arrow (item 2).** Cause: `Driver.chord` sends Shift↓ key↓ key↑ Shift↑ in one burst, all inside one frame; the shortcuts read `i.modifiers`, the state the frame *ends* with, so the key looked unmodified. `struck(events, key, Held)` now reads each key with the modifiers of its own `Event::Key`. Timeline and source-monitor paths both use it (arrows, J/K/L, Space, S/M/T/N, Delete, Shift+Z, Shift+Delete). The `consume_shortcut` chords already matched per event.

**Wayland file drop (item 3).** `FileDrop` (attach in `logic`, `poll` in `ui`, `stop` first thing in `on_exit`, and again from `Drop`). X11/macOS/Windows still come from winit; the Wayland listener is added only for a Wayland window, root viewport, own surface (decorations are other surfaces); paths from both are merged, each once.
- One thread, `aster-file-drop`. The foreign `Backend` and the surface `ObjectId` are made on the window's thread; the thread owns the `Connection`, its queue and every proxy and drops them before it ends. No round trip: globals arrive as registry events (first seat ≤ v5, manager ≤ v3; seat removal handled). Loop: `dispatch_pending` → `flush` → `prepare_read` → `poll(display fd, stop eventfd, done eventfd)` → `read`. It waits nowhere else; the read intent is dropped before anything else is done. `stop` raises the eventfd and joins; on the way out offers are destroyed, the device and seat released, the connection flushed.
- The list is read on `aster-file-drop-read`, one at a time: non-blocking pipe, `poll` with the stop signal, 1 MiB budget (over it the whole drop is refused), 5 s deadline. The protocol thread `finish`es the offer only when the list was read, the compositor reported an agreed action and the offer is v3+ (an unagreed `finish` is a protocol error that would close the window's connection), then destroys it.
- Bounds: 8 unclaimed offers, 4096 files per drop enforced while parsing (set-based dedup; a 1 MiB list of shortest entries is refused in well under a second — unit test), 4096 paths waiting for a frame, path ≤ 4095 bytes.
- URIs: `file:` only (any case); host empty, `localhost` or this computer's name (`/proc/sys/kernel/hostname`); `file:/abs` accepted; percent escapes decode to raw bytes (non-UTF-8 names survive); refused: other schemes, other hosts, userinfo/port, relative paths, malformed escapes, NUL raw or escaped, raw control bytes, unescaped `?`/`#`. Notices: none usable / some not usable / too many / not handed over in time / unreadable.
- Root's four review points are in: count enforced during parsing, `?`/`#` refused, `Drop` joins, listener stopped before other shutdown I/O.

**Import hand-back (item 4).** `restart_media` goes through `hand_back(waiting, import)`. Two tests: order kept and one refusal not stopping the batches behind it (pure); and against the real `MediaService` — import a PNG, `cancel()`, hand back, exactly one current-generation `Imported` with that asset and none after.

Verified (Linux, Rust 1.97.1, `ci` profile):
- `cargo clippy --locked --profile ci --bin aster --tests -- -D warnings` clean; `cargo test --locked --profile ci --bin aster` 62 passed (15 new: recovery at exit ×3 using the worker's own result backpressure to hold a write back deterministically; key modifiers; hand-back ×2; URI list ×3 + merge; reader budget/deadline/stop/end ×4; inbox bound).
- `scripts/native-smoke.py` 13/13 on the final binary: `artifacts/native/ui-round4/smoke/report.json`.
- **Wayland, native, task-owned.** Weston 14 from `.tools/wayland-test` with its X11 backend as a window on Xvfb :92 (`weston.sh`, pixman renderer, socket in `artifacts/native/wayland/runtime`); Aster as a real Wayland client on lavapipe; a GTK3 drag source inside Weston (`source.py`); the drag driven by XTest pointer input on :92 (`pointer.py`, `wl.py`). Nothing touched the person's desktop.
  - `drop/`: 6 URIs dragged (3 local incl. a space, `é`, a literal `%`; `https://…`; `file://elsewhere/…`; one repeated). 01 hover tint + "Drop to import media"; 02 after the drop: the 3 files in the project (agent inspect), notice "Some dropped items are not files on this computer." `aster.log` is the `WAYLAND_DEBUG` trace: `accept`, `set_actions(1,1)`, `action(1)`, `receive`, `finish`, `destroy`.
  - `shut/`: source that accepts and never writes; window closed with the reader waiting: process gone 0.09 s after the click; trace ends `wl_data_offer.destroy` (no `finish`), `wl_data_device.release`, `wl_seat.release`, then the toolkit's own teardown. No panic.
  - `late/`: same source, left alone: two file-drop threads at 2 s, one at 6 s; notice "The dropped files were not handed over in time." (01); nothing imported.
- **Shift+Arrow, X11 on :92**, same `Driver.chord` as before: `keys/01` 00:00:01:00 after one Shift+→; `keys/timecodes.png` 00:00:00:00 → (3× Shift+→, 1× →) 00:00:03:01 → (Shift+←) 00:00:02:01.
- **GIF on the rebuilt app** (root's media fix): `gif/01-playing.png`, source monitor playing `loop.gif`, playhead 00:00:02:00 with the picture's own stamp 00:00:02.000 / frame 20; header reads 10 fps. `ci` build, X11.

Not verified / open:
- The exit path was not driven natively (Xvfb has no window manager to close a dirty window, and Weston's close on a dirty document raises the sheet); it is covered by the three unit tests only.
- Wayland drop was run under Weston only, not GNOME/KDE, and not from a sandboxed source (`application/vnd.portal.filetransfer` is not handled: such a drop is not accepted).
- Shift+Arrow was not driven inside Weston; X11 only. Multi-seat: the first seat only.
- `App::drop` joining assumes the window outlives `App` when `on_exit` did not run; eframe's ordering there was not checked. `on_exit` is the path that was run.
- Still open from before: waveform peaks, asset relink, export, folder rename/delete, checkpoint delete.

## UI round 5 — Claude Opus 5.5 high; root validation

`src/file_drop.rs` now merges borrowed toolkit/display paths through a set,
preserves first-seen order and copies at most 4096 unique accepted paths.
Overflow refuses the whole drop with the existing notice. The limit applies
on all platforms; duplicate reports from both sources do not consume extra
capacity. Four new tests cover stable order, exact capacity, atomic overflow
and a large repeated report.

Root's final workspace validation passes formatting, all-target Clippy with
warnings denied and 169 tests (66 frontend, 103 backend), with actual task-local
FFmpeg/ffprobe enabled. Independent read-only review traced eframe/calloop
teardown and found no normal Wayland lifetime or protocol blocker. Native
release evidence and remaining platform gates live in `docs/verification.md`.

## Export and render queue — 2026-10-14

The "Export/render. No API" gap and the "still open: … export" lines above are
answered by [export and rendering](../export-and-rendering/spec.md): the
export dialog, render queue panel, preview quality and proxies are specified
there, and the export draws the same plan the preview draws, through the
compositor. Window integration is its ticket 09.
