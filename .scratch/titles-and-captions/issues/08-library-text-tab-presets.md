# Library Text tab and presets

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 07

## Scope

Built-in title and caption styles, graphic templates (lower thirds, title cards, callouts), vector stickers and emoji; saving a title's style or a graphic as the person's preset, kept in a versioned file beside the settings; font import into the library.

## Comments

- The library has a Text tab between Media and Effects. Its groups are Titles, Captions, Lower thirds, Title cards, Callouts, Stickers and Emoji (48), plus Yours. Each tile is drawn by the title lane at the tile's size and plays its entrance while the pointer rests on it. A caption tile moves its highlight through the words. The tab asks for no repaints while nothing is pointed at.
- A double click or Return adds a tile at the playhead. Dragging it onto a text track adds it where it is let go, with a ghost of its length. Either way it is one command: on the given track at the first free time, otherwise on the first plain text track with room, otherwise on a new "Text N" track. Caption tracks are used only when the tile is dropped on one. Sizes are made for a 1080-pixel short side and scaled to the canvas. Graphics get fresh element IDs. A clip lasts three seconds, or longer when its animations need it.
- Dropping a title's look on a title gives it that look as one step and keeps its words and word times. Animations are fitted to the clip. On a caption track, every caption of the track takes the look. The context menu also offers "Style selected titles". Locked tracks are left alone, and a look with nothing to apply to says why.
- Presets: the inspector's Text section has "Keep this look" and the Graphic section has "Keep as template". Both are named in a popup. Yours lists the kept looks first, which can be renamed and removed, along with brand colours that can be added and removed. Every colour picker in the inspector offers the brand colours above the picker.
- Presets are written to `presets.json` beside the settings, versioned, atomically and on a worker, and again on exit. Word times are never kept. A newer or damaged file is not read and is never written over: it is moved aside to `presets-unread-<time>.json` before the first save, and the person is told.
- "Import fonts" in the tab's header opens a picker for .ttf/.otf/.ttc/.otc files. They go through the media import as font assets.
- When the tab names would touch, they shrink to 11 pt and then to icons with hover names.
- Tests: 13 in `src/ui/text_tab_tests.rs`, covering:
  - every look adds and undoes as one step
  - placement, captions skipped and quantizing
  - canvas scaling and kept positions
  - restyle keeps words and fits animations
  - caption-track restyle
  - refusals
  - listing and search
  - double click and Return
  - timeline drop and restyle drop
  - idle repaint
  - presets and brand colours
  - a 236 px library

  Also 2 inspector tests (Keep with brand colour, Keep as template) and 3 preset file tests. Desktop: 635 passed. Clippy `-D warnings` and the architecture check pass.
- Native captures of the debug build under Xvfb + lavapipe:
  - the Text tab with lane-drawn tiles at 1440×900, scrolled to the lower thirds, title cards and the emoji grid
  - a tile carried over a text track with its ghost, and the clip it added
  - the Keep popup, and Yours after keeping
  - the brand colour picker, and a brand colour in the inspector picker applied to a shadow
  - a look dropped on a title, with its highlight and the result
  - at 760×560, the Text tab with presets read back after a restart, and the icon tabs at a 220 px library

  Key presses could not be delivered to the window on this display (even End and Space did nothing), so typing a preset name and Return were not captured natively. They are covered by the bench tests. Sound reports no device on this host.
