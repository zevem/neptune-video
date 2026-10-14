# Viewer draws title pictures

Status: ready-for-agent
Progress: resolved
Type: task
Blocked by: 05

## Scope

Title and graphic layers are drawn from the lane's pictures as quads through their transform; handles from the layout box; drafts ask for their own picture; egui text drawing of titles is removed.

## Comments

- `compose::runs` parts media runs at titles and graphics alike (`Run::Title`); neither is composited.
- `src/titles.rs` holds one texture per title on screen. A layer asks the lane again only when its resolved source, its quantized scale (eighth octaves of the scale it covers on screen) or the project's font files change, and it shows its last picture until a newer serial comes. Answers from before a holding began, or for clips no longer drawn, are dropped; a layer undrawn for two application frames gives its texture up, and a replaced project clears them all.
- The picture is one quad through the transform (position, scale, flips, rotation, opacity × fade). The handles come from the picture's text or design box, so an outline, shadow or a karaoke word's growth does not move them. Nothing is drawn or held before the first picture arrives.
- Drafts hold the whole generated content (`ClipDraft::content`), and `ClipOverride::content` makes `resolve` draw it, so typing, and later style and animation edits, ask for their own picture. A media clip ignores a content draft. On commit the draft splits into SetText, SetTextStyle, SetTextAnimation and SetGraphic as each part differs.
- Tests: four viewer tests (title quad and text-box handles at the on-screen scale, a typed draft asks for and then lets go of its own picture, a scaled graphic's picture and design-box handles, turn/mirror geometry), the graphic case in compose runs, and a core test for content drafts. Desktop: 609 passed. Core: all passed.
- Native captures of the debug build under Xvfb + lavapipe, at 1440×900 and 760×560, show a lower third, a Box-highlighted karaoke caption with colour emoji and a rotated colour-emoji sticker, all drawn from lane pictures. The Pop-entrance hook title is correctly invisible at 0:00. The container needed `libxkbcommon-x11-0` and Mesa's Vulkan drivers installed to open the window. Sound reports no device on this host.
