# Neptune Video Tabler icon subset

These SVGs are unmodified official Tabler Icons 3.49.0 source assets, pinned
to commit `a4ce1404bc6d24d3c365afe7b258d6bf6f48d62d`. They retain `currentColor`, the
24×24 viewBox and upstream comments. Outline assets use Tabler's rounded
2-point strokes; playback assets use the official filled style.

`mapping.json` records the complete Neptune Video `Icon` mapping, each source URL,
upstream icon version, font Unicode code point and SHA-256 of the exact
source bytes. The upstream MIT notice is preserved in `LICENSE.txt`.

Sources:

- [Tabler Icons repository](https://github.com/tabler/tabler-icons/tree/a4ce1404bc6d24d3c365afe7b258d6bf6f48d62d)
- [Tabler icon design and customization](https://tabler.io/icons)
- [Pinned upstream license](https://github.com/tabler/tabler-icons/blob/a4ce1404bc6d24d3c365afe7b258d6bf6f48d62d/LICENSE)

Go-to-start/end use the filled player-skip glyphs, whose vertical bar marks
the boundary. Previous/next frame use a single chevron. The graph editor
uses `ease-in-out-control-points`, its value-axis fit uses `viewport-tall`,
and transitions use `transition-right`. A keyframe at the playhead uses
the official filled `keyframe` variant. `Music` retains an audio-wave
metaphor with `wave-sine`, and track solo uses `letter-s`. Canvas layout
controls use `aspect-ratio` and the corresponding `align-box` corner or
side glyphs. These are all upstream glyphs, with no bespoke paths.

The native UI embeds `outline.ttf` and `filled.ttf`, containing only the 76
glyphs used by Neptune Video's 78 `Icon` variants. They are generated from the official
`@tabler/icons-webfont` package at the same pinned version. `font-source.json`
records the verified archive and font hashes, retained glyph counts and font
metrics; `glyphs.rs` supplies the exhaustive Rust mapping.

The font subsets retain the upstream contour coordinates and 24×24 spacing.
The generator recalculates glyph bounds and corrects left side bearings to
match them: Tabler's source fonts contain zero bounding-box minima, which would
otherwise move icons left when a subsetter recalculates the true bounds.
Every retained glyph keeps its 1000-unit advance. The 900/-100 ascent/descent
maps a font size of 24 points onto Tabler's full 24-point viewBox. Rendering
centers that square, retaining upstream spacing and optical offsets.

Regenerate with the pinned optional maintenance dependency:

```sh
uv run --with fonttools==4.60.1 python assets/icons/tabler/generate.py
uv run --with fonttools==4.60.1 python assets/icons/tabler/generate.py --check
```

The tool verifies the pinned package archive and checks every manifest code
point, SVG hash and unchanged glyph contour. For offline regeneration, add
`--archive /path/to/icons-webfont-3.49.0.tgz`; the same archive SHA-256 check
still applies. The checked-in fonts require no npm, Python, SVG parsing or
network access during Rust builds or application startup.
