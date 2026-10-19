# Follow-ups found during implementation

Status: needs-triage
Progress: open
Type: task

## Scope

Gaps the implementers reported, none blocking:

- `src/export/frame.rs` reads `.cube` files itself; `editor-media` could
  expose one reader for both.
- The stabilization sampling chain exists in `src/steady.rs` and
  `compose::assemble`; one could call the other.
- The export decode size ignores zoom transitions and distort effects, which
  can need more pixels than the keyframed scale and crop.
- `Capabilities` does not list the `mov_text` and `webvtt` subtitle encoders,
  so `plan` cannot confirm them.
- `Resolution::Short` above 8192 on the long side is scaled down rather than
  refused; on extreme-aspect canvases the short side is lifted to 16 and the
  aspect is not kept. `MAX_OUTPUT_PIXELS` can never bind under 8192-a-side
  limits.
- The export reader and preview decoder read assets without a frame rate on a
  30/1 grid, while `source_frame` falls back to the project rate (already so
  in the preview).
- Part files and working folders left by a crash are swept only once
  unchanged for an hour, and only in a folder a later job writes to; the
  sweep could remove the files of another Neptune Video instance's job paused for
  more than an hour.
- `check_queue_export` and the window's replace check read file metadata
  on the window's thread; `enqueue` itself no longer does.
- A sound-only job paused during its sound render keeps rendering the sound
  and stops before FFmpeg starts.
- The proxy budget is not shared between two sessions using one cache.
- The frame-rate list in `export_sheet.rs` copies `ui/inspector.rs`'s.
  (`QUEUE_LIMIT` is now `MAX_JOBS`.)

## Comments

2026-10-14: Gaps left by the review fixes:

- No `PR_SET_PDEATHSIG` or Windows job object ties the FFmpeg processes to
  the editor's life when it is killed. `Process::spawn_with` in
  `crates/editor-media/src/process.rs` has no `pre_exec` hook to set one.
- No free-space check before the sound is rendered or the encode starts.
- Windows: hard links to a source are not detected (std has no stable
  volume and file index), and the `.neptune-video-render-<id>` folder is not given
  the hidden attribute. Explorer's `/select` quoting was not run on Windows.
- `+faststart` is still used for MOV.
- AMF, QSV, VA-API, VideoToolbox and Media Foundation quality mappings and
  sizes are unmeasured; AMF and VA-API borrow NVENC's offsets. AV1 and VP9
  quality mappings and bits per pixel are unmeasured on every encoder.
- The size estimate was fitted on synthetic testsrc2 pictures; real camera
  footage was not measured. Higher and Maximum spread from 0.42 to 4 times
  the estimate.
- No round-trip test for the `export_preset` setting in `src/settings.rs`.
- The preset dropdown's rows are about 37 pt tall and the list is capped
  near 400 pt, so it scrolls; a queue row has about 10 pt of extra space
  between its name and its folder line.
- One frame between the viewer opening a proxy and the lane hearing it is in
  use, in which eviction can remove it; it recovers through
  `ProxyEvent::Removed` and the unreadable-proxy retry. Closing it needs a
  change in `src/ui/viewer.rs`.
- A retry refused because another unfinished job writes the file gives the
  agent the generic "cannot be retried" message and the person no feedback.
- `Proxies::clear` and `forget_proxies` do not clear the unreadable-proxy
  map directly; a later remake does.
- The toolbar and queue write "34 %" and the library "42%".

2026-10-14: From the native verification (ticket 12), Linux release build:

- Fixed (see below): at 760x560, while the pointer is over the export
  sheet's left column, its widened scroll bar covers the last letter of
  each summary value.
- Fixed (see below): finished jobs stay above the running one in the render
  queue, so with five or more finished jobs the active row is below the
  fold at 1440x900.
- Fixed (see below): at 760x560, "Exported …" notices cover the top of the
  render queue panel.
- Fixed (see below): "Stop and quit" does not stop the render until the
  following save question is answered; in one run the job finished then and
  the complete file was kept.
- Fixed (see below): the sheet's folder button shows the folder's base
  name, the queue rows the full path.
- Not driven natively: captures of a paused and a failed row; Add to queue,
  Start all and the pause and resume buttons in the window; retrying a
  failed job; the Fill fit (all driven since, see below); 96 kHz and mono
  sound; libaom AV1 (still open).

2026-10-14: The five interface defects above are fixed and were checked in
a release build (`neptune-video` `07b644ca…f9ad1b`) at 1440x900 and 760x560,
`artifacts/export-phase7/polish-20261014-102125/results.md`:

- Scroll bar: the export sheet's scroll areas keep a gutter for the
  widened bar (`reserve_scroll_bar`); values end at x=282, the bar at about
  286 to 292.
- Queue order: the job under way first, then waiting and held jobs in
  render order, then finished jobs newest first; a starting job is scrolled
  into sight once (`Order::listed` in `src/ui/render_queue.rs`).
- Notices: under an open sheet they sit at the bottom of the window; while
  the queue is open, finished and failed jobs post none
  (`wiring::finished_notice`).
- Stop and quit cancels every job at once, waiting ones first
  (`wiring::stop_all`), then asks about unsaved changes; staying leaves
  them cancelled, with Retry. Export FFmpeg ended 6 and 11 ms after the
  click, part file and working folder 165 and 170 ms.
- The sheet's folder button uses the queue's `short_folder`
  (`~/…/Trip/Final`), the full path on hover.
- Driven in the window too: Add to queue, the Held row and Start all;
  Pause and Resume; a failed job (folder removed) retried after the folder
  was made again (attempts 2); Fill with the TikTok preset on a 16:9 canvas
  (1080x1920, 42.2 dB against a centre crop, no black bars). Paused and
  failed rows are captured.

Still open from this run:

- `stop_and_quit_cancels_every_job_at_once_and_starts_none` passes without
  the waiting-first order; that order has no failing regression test.
- The panel learns finish order only while open; jobs that finished while
  it was closed are taken to have finished in render order.
- One 1080p NVENC job of 1200 frames, with the export sheet open at
  1440x900, reported "Done in 1:06" against 0:13 to 0:21 in every other
  run. Not reproduced; its log shows nothing.
- After a clean Don't save exit a 0-byte `recovery/<id>.nvp.lock` stayed
  beside `settings.json` (recovery's lane, not export's).
- 96 kHz and mono sound, libaom AV1; macOS, Windows and real displays.
