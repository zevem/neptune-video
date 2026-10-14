# Tabler icon migration

The user selected Tabler on 2026-10-14 and requested a complete migration of
Aster's UI icons. This supersedes the visual preference in the earlier
[research](research.md).

Use one pinned Tabler set for every editor control glyph, including window
controls, dropdown chevrons, layout presets, keyframe toggles, solo and the
caption timing separator. Preserve control actions, stable targets, labels,
tooltips, keyboard routes, hit areas and state colors. Timeline marks, masks,
curve handles, content previews, progress animations and launcher branding
remain the geometry of their content or native behavior.

The [asset manifest](../../assets/icons/tabler/mapping.json) maps all 67
previous icon variants and 11 previously separate UI glyphs to Tabler 3.49.0.
It records upstream sources, styles, code points and hashes; the
[asset README](../../assets/icons/tabler/README.md) explains regeneration and
preserves MIT attribution. The 78 variants use 76 distinct glyphs, with
outline as the default, filled player controls and a filled on-key diamond.

Embed only dedicated, generated outline/filled font subsets. Keep icon font
families out of text fallbacks. Center the full Tabler design cell to preserve
its optical spacing. Use state colors at painting time, share cached layouts
across states and quantize glyph raster sizes to physical pixels while scaling
fractional animation bounds smoothly. No network, file access or SVG decoding
belongs in the paint path. Cargo builds use checked-in assets and the existing
pinned toolchain/lockfile.

Verification covers every mapped glyph in its intended font, drawing at small
and large bounds with display scales 1 and 2, existing native UI interaction
benches, affected-package denied-warning Clippy and the final native build.
Capture the running build with fresh owned fixtures, private settings/storage,
Xvfb and D-Bus at 1440x900 and 760x560. Inspect enabled/disabled/selected,
outline/filled, menu/dropdown, layout, graph/solo, library and window states.
Retain exact process cleanup, build identity, host/backend and unverified cases
in the [native capture record](../../artifacts/native/tabler-icons-20261014/).
