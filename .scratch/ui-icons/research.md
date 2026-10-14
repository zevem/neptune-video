# UI icon library research for Aster

Researched 2026-10-14. Research and static asset comparison only; no icon
replacement, dependency change, compilation or native acceptance run.

## Recommendation

**Choose Phosphor as the leading candidate: Regular outline icons with Fill
transport controls.** In the [comparison](comparison.png), its smooth curves,
restrained detail and rounded player glyphs best fit the requested Apple-like
feel. This is an aesthetic judgment, not a claim that Phosphor reproduces
Apple's symbols. Light looks too delicate in the comparison's 14–16 px rows.
Regular still needs review inside Aster at its actual sizes and display scales.

Lucide is a strong alternative if preserving the existing outline vocabulary
and adjustable stroke widths is the priority. Phosphor wins this comparison
because it also provides designed solid transport glyphs and a current egui
integration; Lucide explicitly does not officially support fills.
[Phosphor assets](https://github.com/phosphor-icons/core/blob/main/README.md),
[Lucide fills](https://lucide.dev/guide/react/advanced/filled-icons).

## Existing implementation

[src/icons.rs](../../src/icons.rs) defines a native vector set on a 24-point
coordinate grid. Its painter uses **fixed 1.5 egui-point strokes**, round end
discs, rounded rectangles and solid transport shapes. Geometry scales with
the drawing rectangle, but the stroke does not. [docs/design.md](../../docs/design.md)
asks for rounded, optically aligned icons with accessible names and tooltips;
[Cargo.toml](../../Cargo.toml) uses eframe 0.36.2 and currently has no SVG loader
or icon dependency.

Do not equate an SVG's nominal stroke with the current rendered weight. At
16-point display size, Phosphor Regular is approximately 1 point, Lucide's
default is 1.33 points and Iconoir's default is 1 point; Aster currently stays
at 1.5 points. Font em size can add different margins again. The migration
must choose visual weight at 14–16 points rather than copy a nominal number.

## Candidate facts and tradeoffs

| Library | Verified design system | Fit for Aster |
| --- | --- | --- |
| **Phosphor** | Six variants: Thin, Light, Regular, Bold, Fill, Duotone. SVG viewBox 256×256; inspected raw paths have round caps/joins and Regular stroke 16, Light 12, Bold 24. | Best visual candidate in the sample; official solid transport, captions, Bézier curve, film and audio icons. Separate weights offer controlled alternatives. |
| **Lucide** | 24×24 canvas, 2 px strokes, round caps/joins; 1–2 px corner radii depending on element size. Stroke width is adjustable. Fills are not officially supported. | Good familiar outline system; nearest flexible replacement for handwritten outline geometry. Filled playback would require a deliberate custom treatment. |
| **Iconoir** | Official defaults 24×24, 1.5 px stroke; inspected SVGs have round caps/joins. Regular and solid directories; MIT. | Good fallback for a fine outline look with solid transport. In this sample the gear is more angular and the eye uses a different silhouette. |
| **Tabler** | 24×24 outline system with 2 px strokes, adjustable size/color/stroke and round caps/joins; outline and filled assets; MIT. | Broad, practical alternative with stronger outline weight. Sample is polished, but Phosphor better matches the preferred aesthetic. |

Primary design sources:
[Phosphor raw Regular folder](https://github.com/phosphor-icons/core/blob/2b75f3ad12b420c9504ef05df8d2564a28f8500e/raw/regular/folder.svg),
[Light folder](https://github.com/phosphor-icons/core/blob/2b75f3ad12b420c9504ef05df8d2564a28f8500e/raw/light/folder-light.svg),
[Bold folder](https://github.com/phosphor-icons/core/blob/2b75f3ad12b420c9504ef05df8d2564a28f8500e/raw/bold/folder-bold.svg),
[Phosphor overview](https://github.com/phosphor-icons/homepage/blob/master/README.md),
[Lucide specification](https://lucide.dev/contribute/icons/specification),
[Lucide stroke customization](https://lucide.dev/guide/react/basics/stroke-width),
[Iconoir](https://iconoir.com/),
[Iconoir SVG](https://github.com/iconoir-icons/iconoir/blob/195a8b2b7ecb3c1cdebf7d4e3c1290633795fa67/icons/regular/settings.svg),
[Tabler repository](https://github.com/tabler/tabler-icons),
[Tabler gallery](https://tabler.io/icons).

### Editor coverage spot-check

The following assets exist in the inspected official repositories. Mapping an
available icon to an editing action still requires semantic review; this is
not an exhaustive audit of Aster's entire `Icon` enum.

| Aster role | Phosphor | Lucide |
| --- | --- | --- |
| Playback | `play`, `pause`, `skip-back`, `skip-forward`, also Fill variants | `play`, `pause`, `skip-back`, `skip-forward`, outlines |
| Split/delete | `scissors`, `trash` | `scissors`, `trash` |
| Project/media | `folder`, `folder-plus`, `floppy-disk`, `film-strip`, `image` | `folder`, `folder-plus`, `save`, `film`, `image` |
| Text/captions | `text-t`, `subtitles`, `closed-captioning` | `type`, `captions`, `closed-caption` |
| Sound/speech | `music-note`, `waveform`, `microphone`, `speaker-high`, `speaker-slash` | `music`, `audio-waveform`, `mic`, `volume-2`, `volume-x` |
| Track states | `eye`, `eye-slash`, `lock`, `lock-open`, `magnet` | `eye`, `eye-off`, `lock`, `lock-open`, `magnet` |
| Animation/color | `diamond`, `bezier-curve`, `eyedropper` | `diamond`, `spline`, `chart-spline`, `pipette` |
| History | `arrow-u-up-left`, `arrow-u-up-right`, `arrow-counter-clockwise` | `undo`, `redo` |

Sources: [Phosphor inventory at inspected revision](https://github.com/phosphor-icons/core/tree/2b75f3ad12b420c9504ef05df8d2564a28f8500e/assets/regular),
[Lucide inventory at inspected revision](https://github.com/lucide-icons/lucide/tree/dab294f313897bacaa968b725b3259fa37c17bd1/icons).
Aster's specific `Transition`, `FitValues` and eased `Curve` metaphors should
be reviewed individually; a small consistent custom exception may remain.

## Licensing and SF Symbols

Phosphor SVG assets are MIT; use, modification, vendoring and commercial
redistribution are permitted with the copyright and permission notices.
Lucide is ISC, while listed Feather-derived icons retain their MIT notice;
preserve both notices when using that subset. Iconoir and Tabler are MIT.
[Phosphor license](https://github.com/phosphor-icons/core/blob/main/LICENSE),
[Lucide license and Feather notice](https://lucide.dev/license),
[Iconoir license](https://github.com/iconoir-icons/iconoir/blob/195a8b2b7ecb3c1cdebf7d4e3c1290633795fa67/LICENSE),
[Tabler license](https://github.com/tabler/tabler-icons/blob/main/LICENSE).

**SF Symbols is unsuitable as the shared Linux/macOS/Windows library.** Apple
provides nine weights and three scales, but its Xcode agreement section 2.10
licenses system-provided symbols for applications on Apple-branded products
running the applicable system. This rules out bundling them as Aster's shared
cross-platform set. They remain a useful aesthetic reference.
[Apple SF Symbols](https://developer.apple.com/sf-symbols/),
[Apple Xcode agreement, section 2.10, pages 6–7](https://www.apple.com/legal/sla/docs/xcode.pdf).

## Native integration proposals — not tested

1. **First evaluate `egui-phosphor` 0.14.0 with a subset of required icons.**
   Its published release is not yanked, its manifest depends on egui 0.36,
   and its README documents startup font registration, separate variants and
   compile-time font subsetting. That nominally matches Aster's egui family;
   compatibility and appearance have not been compiled or run here. Its Rust
   wrapper is MIT OR Apache-2.0, and the bundled icons are MIT. The actual
   bundled regular TTF and the official web regular TTF both identify MIT in
   their name-table license field; the official web repository also carries
   MIT. [Wrapper](https://github.com/amPerl/egui-phosphor),
   [manifest](https://github.com/amPerl/egui-phosphor/blob/5a4656014c05268d8563a9ea4e768055d6470b3e/Cargo.toml),
   [published release](https://crates.io/crates/egui-phosphor/0.14.0),
   [bundled font](https://github.com/amPerl/egui-phosphor/blob/5a4656014c05268d8563a9ea4e768055d6470b3e/res/Phosphor.ttf),
   [official web license](https://github.com/phosphor-icons/web/blob/master/LICENSE).
2. **Alternative: vendor a pinned SVG subset behind the existing `Icon` seam.**
   egui_extras has an `svg` loader using resvg, and egui can display embedded
   bytes. `Image::tint` multiplies pixel color, so normalize monochrome assets
   to white before applying Aster's state tint. `Image::paint_at` requests the
   physical-pixel target size, which matters for crisp scaling. This pipeline
   rasterizes SVGs into cached images/textures; it is not native path rendering.
   The first SVG decode is synchronous in the inspected loader, so any adoption
   should initialize/prewarm a bounded set outside interactive editing and use
   fixed sizes and stable cache identities. [Loader features](https://docs.rs/egui_extras/latest/egui_extras/),
   [egui 0.36.2 image source](https://github.com/emilk/egui/blob/0.36.2/crates/egui/src/widgets/image.rs),
   [egui_extras 0.36.2 SVG loader](https://github.com/emilk/egui/blob/0.36.2/crates/egui_extras/src/loaders/svg_loader.rs).

Keep action names, accessible labels, tooltips, hit areas and state colors
owned by Aster's widgets. Replacing glyphs should not expose font codepoints
as accessible names or change action targets. Check the final native build at
14/16/24-point bounds, normal and narrow windows, 1× and higher display scales,
and enabled/disabled/selected states. No native rendering, performance or
platform acceptance is established by this research.

## Comparison artifacts

[comparison.png](comparison.png) uses actual upstream SVGs at 24 px and
14/16 px, with two device pixels per logical pixel. It is a static design
reference, not a capture of Aster or a font-wrapper acceptance result.
[comparison-sources.json](comparison-sources.json) pins every sample asset and
revision; [sample-licenses.md](sample-licenses.md) preserves their notices.
