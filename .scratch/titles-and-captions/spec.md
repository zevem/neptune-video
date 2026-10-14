# Titles, captions and motion graphics

Status: ready-for-agent
Progress: resolved
Type: task

Styled titles, text animation, automatic and edited captions with word
timing, karaoke highlighting, subtitle files, caption languages and
translation, lower thirds, title cards, callouts, stickers and emoji,
graphic templates with editable parameters, and reusable style presets:
what short-form video (TikTok, Reels, Shorts), advertisements and YouTube
edits need from text, without weakening the editor's contracts (validated
reversible commands, pure core, bounded stale-safe workers, quiet idle,
honest claims).

This document is the binding design. Where a ticket and this file disagree,
this file wins; change it deliberately and note the change under Comments.

## Decisions

1. **One text renderer, on what is already linked.** Titles and graphics
   are drawn to pictures by a CPU renderer in `editor-media` built on
   `vello_cpu`, `skrifa` and `harfrust`, the crates egui 0.36 already links
   for its own text. Shaping, colour emoji, strokes, Gaussian blur and drop
   shadows therefore add no new native dependency and almost no binary
   size. egui's text path (the interface font, no outline or shadow, a
   480-point cap) is set aside; a GPU glyph renderer is set aside as a
   second implementation the export path would not share.
2. **Pictures are made off the frame.** A bounded text lane makes a title's
   picture from the layer the plan resolved, at the size the screen shows
   it; the viewer draws the newest picture it holds as one quad. Results
   name their clip and the exact layer they were drawn from and are
   dropped when stale, like decoded frames. Text stays outside the
   compositor in this epic: effects, masks and blend on titles stay refused.
3. **Style is durable data in font-relative units.** Outline width, shadow
   offset and blur, background padding and radius are fractions of the font
   size (em), so a style scales with its title. A style names a font family,
   weight and slant; it never names a file. The family is looked up in the
   project's font assets, then the bundled faces, then the system's.
4. **Animation is a preset over units, evaluated in core.** An enter or exit
   animation is `{kind, unit, duration, delay}`. `resolve` turns the
   frame-snapped clip time into one progress per animation and the karaoke
   word state; a pure core function spreads a progress over the units, so a
   paused viewer has nothing to redraw and the plan stays the test oracle.
5. **A caption is a title on a caption track.** A text track with a
   language is a caption track and each of its titles is one cue; cues
   carry word times relative to their clip, as keyframes do, so moving or
   rippling a cue touches none. Every existing timeline gesture (move, trim,
   split, delete, ripple, lock) therefore works on captions unchanged.
6. **Words are times, not text.** A title's `words` are `{at, end}` pairs
   for the words of its text, counted by one core rule. Editing the text
   realigns the times to the new words by a deterministic diff, so fixing a
   typo keeps the karaoke timing.
7. **Graphics are one clip of named elements.** A lower third, title card,
   callout or vector sticker is a graphic clip: a design box and up to 32
   elements (text, rectangle, ellipse, line or arrow), each named, framed
   and animated on its own. Its editable parameters are its elements' texts
   and the colours and font family its elements share, read from the data;
   no separate parameter table is stored. A template is a graphic kept
   outside the project; adding it copies it in.
8. **Presets stay outside the project.** Built-in styles, caption styles,
   templates, stickers and emoji live in the desktop; brand presets (saved
   styles, colours and templates) live in the person's preset file beside
   the settings. Giving one copies its values into clips, so a project never
   depends on a preset file.
9. **Speech is local and opt-in.** Automatic captions use whisper.cpp
   through `whisper-rs`, built into `editor-media` from source (C++ and
   CMake at build time, nothing to install at run time). The model is
   downloaded on first use only when the person asks, from a fixed list
   with sizes and SHA-256 sums, verified, and kept in the user data
   directory; a model file can also be chosen by hand for offline machines.
   The download is a `curl` process (present on current Linux, macOS and
   Windows 10+) so no TLS stack is linked. WhisperX (Python, PyTorch,
   wav2vec2 alignment) was set aside as gigabytes of runtime; cloud speech
   APIs as accounts and uploads; Vosk as a prebuilt C library with weaker
   accuracy.
10. **Captions come from the timeline's sound.** The audio mixer renders the
    chosen range offline at 16 kHz mono, so cuts, gain, mute and constant
    speed changes are what the transcript hears, and cue times are timeline
    times. Word times come from whisper.cpp's DTW alignment heads.
11. **Translation reuses reviewed proposals.** Whisper can translate speech
    to English offline while transcribing. Any other target goes through the
    native AI provider the person already signed in to: one `copy_captions`
    command carrying only the translated texts, reviewed and applied as one
    entry. No translation model is bundled.
12. **Schema version 3.** Read accepts 1 to 3; write is always 3, so an
    older build refuses a file with titles it cannot keep.
13. **Strict now, relax later.** Validation rejects what the renderer cannot
    draw.
14. **The native planner sees a narrow title surface.** The provider schema
    shares the Claude command line with the planner text, inside Windows'
    32,767 characters. Proposals add plain titles, edit their text, animate
    them, set caption languages and translate (`copy_captions`); title
    styles, graphics and word times stay with the inspector, presets and
    speech. Context shows a timed title's words as a count and a span. The
    external agent protocol accepts every command.

## Domain model (`crates/editor-core/src/title.rs`)

New types derive `Debug, Clone, PartialEq, Serialize, Deserialize,
JsonSchema`; enums are unit `snake_case` or internally tagged with `type`; no
`flatten` or untagged enums. New fields on existing types are
`#[serde(default, skip_serializing_if = …)]`.

```rust
// model.rs
pub enum MediaKind { …, Font }        // never a clip: duration 1 µs, size 0
pub struct Track { …, pub language: Option<String> }   // text tracks only
pub enum ClipContent {
    Media { asset_id },
    Text { text, font_size, color,
           style: TextStyle,                 // default: plain
           enter: Option<TextAnimation>,
           exit: Option<TextAnimation>,
           words: Vec<WordTime> },           // empty, or one per word
    Graphic { size: [f32; 2], elements: Vec<Element> },
}

// title.rs
pub struct TextStyle {
    pub font: FontFace,                  // { family, weight 100..=900, italic }
    pub align: TextAlign,                // left | center | right
    pub line_height: f32,                // 0.5..=4 × size, default 1.2
    pub tracking: f32,                   // em, -0.5..=2, default 0
    pub wrap: f32,                       // canvas px, 0 = none, 0..=16384
    pub case: TextCase,                  // as_typed | upper | lower
    pub outline: Option<TextOutline>,    // { color, width em 0.005..=0.5 }
    pub shadow: Option<TextShadow>,      // { color, offset [em; 2] ±2, blur em 0..=1 }
    pub background: Option<TextBackground>, // { color, padding [em; 2] 0..=4, radius em 0..=4, shape: block | lines }
    pub highlight: Option<Highlight>,    // karaoke, used only with words
}
pub struct Highlight { pub mode: HighlightMode /* word | box | fill */,
    pub color: [u8; 4], pub scale: f32 /* 1..=2 */, pub upcoming: f32 /* 0..=1 */ }
pub struct TextAnimation { pub kind: TextAnimationKind, pub unit: TextUnit,
    pub duration: Time, pub delay: Time }
pub enum TextAnimationKind { Fade, Pop, Bounce, Typewriter,
    SlideUp, SlideDown, SlideLeft, SlideRight, Wipe }
pub enum TextUnit { Whole, Line, Word, Letter }
pub struct WordTime { pub at: Time, pub end: Time }    // clip-relative
pub struct Element { pub id: Uuid, pub name: String, pub kind: ElementKind,
    pub frame: [f32; 4], pub rotation: f32, pub opacity: f32,
    pub enter: Option<TextAnimation>, pub exit: Option<TextAnimation> }
#[serde(tag = "type")]
pub enum ElementKind {
    Text { text, font_size, color, style: TextStyle },
    Rect { color, radius, stroke: Option<ShapeStroke> },
    Ellipse { color, stroke: Option<ShapeStroke> },
    Line { color, width, arrow: bool },
}
```

### Rules

- **Defaults.** A plain style is Geist 600 upright, centred, line height 1.2,
  no tracking, no wrap, as typed, nothing else: exactly how a title was drawn
  before this epic, so v1 and v2 titles look as they did, apart from the
  renderer.
- **Words.** `title::word_ranges(text)` is the one count: runs split at
  whitespace, and each Han, Hiragana or Katakana character is a word of its
  own. `words` is empty or has exactly that many entries, at most 4096; each
  `0 <= at <= end <= duration`, `at` non-decreasing.
- **Animation.** `duration` is one canvas frame to `min(10 s, clip)`, and
  `delay + duration <= clip duration`; enter starts at `delay` from the clip
  start, exit ends `delay` before its end. Enter and exit may not overlap.
  Each unit animates for `L = D` when there is one unit and
  `L = 2D / (n + 1)` otherwise, starting at `i (D - L) / (n - 1)`; a
  typewriter unit appears at its start. An exit plays its kind backwards in
  time, last unit first, except that a slide leaves in the direction it is
  named for. Kinds: fade (opacity), pop (scale from 0 with overshoot),
  bounce (drop from above with two bounces), typewriter (appear), slides (one
  em of travel and fade), wipe (revealed left to right).
- **Highlight.** `word` colours the active word, `box` draws a rounded box of
  `color` behind it, `fill` colours every spoken word and sweeps the active
  one left to right. The active word is scaled by `scale`, easing in over
  80 ms; words not reached yet are drawn at `upcoming` opacity (0 reveals
  word by word). Between words the last spoken one stays active.
- **Graphics.** `size` each side 1..=16384; 1..=32 elements; frame sides
  finite, width and height positive except for a line, whose frame runs from
  its start to its end; opacity 0..=1; an element's text obeys a title's
  bounds (no words). Element IDs are unique within the clip.
- **Track language.** 1..=35 ASCII letters, digits and hyphens, starting with
  a letter (a BCP 47 tag, not checked against the registry); only on text
  tracks.
- **Font assets.** `MediaKind::Font` is imported from `.ttf`, `.otf`, `.ttc`
  or `.otc` and named for its family; it is on no track and referenced by no
  ID. Removing it leaves titles drawn in the fallback face, said in the
  inspector.
- Effects, masks, non-normal blend and retime stay refused on titles and
  graphics.

## Commands

| Command | Rule |
| --- | --- |
| `SetText { clip_id, text, font_size, color }` | Keeps style and animations; realigns `words` to the new text (diff of old and new words; matched words keep their times, new ones are spread between neighbours) |
| `SetTextStyle { clip_id, style }` | Titles only |
| `SetTextAnimation { clip_id, edge, animation }` | Titles only; `edge` in/out; `None` removes |
| `SetWords { clip_id, words }` | Titles only; empty clears |
| `SetGraphic { clip_id, size, elements }` | Graphics only |
| `SetTrackLanguage { track_id, language }` | Text tracks only; `None` makes it a plain text track |
| `CopyCaptions { from_track_id, track_id, name, language, texts }` | New text track directly above `from`, one title per source title in order with fresh IDs, its text replaced; `texts` has one entry per title; word times are spread over the new words by length |

Existing arms change: `SplitClip` on a title with words gives the words
before the cut and their text to the left and the rest to the right (a plain
title keeps its text on both sides), the enter animation to the left and the
exit to the right, each held inside its half. `TrimClip` keeps every word and
holds times and animations inside the clip. `SetPlaybackRate` and a linked
`SetRetime` scale word times and animations with the duration.

## Evaluation (`render.rs`)

`LayerSource::Text` carries the text, size, colour and style, plus
`enter`/`exit: Option<Motion { kind, unit, progress }>` and
`words: Option<WordState { active, progress, spoken }>`; `LayerSource::Graphic`
carries the design size and each element with its own motions. Progress is
computed at the frame-snapped clip time (`Layer::time`), so playback steps
animation once a frame and a paused viewer has nothing to redraw.
`title::unit_progress(motion, index, count)` is the one spread.

## Rendering, workers and UI

- `editor-media::text`: font index (project font assets, Geist from the
  desktop, system fonts discovered once on the lane's thread), shaping with
  per-character fallback, layout (wrap, align, line height, tracking, case),
  and a picture: background, shadow, outline, fill, colour glyphs. A picture
  is premultiplied RGBA with its origin and layout box in canvas pixels at
  scale 1.
- Text lane: one thread, the newest request per clip, a 64 MiB picture
  budget, pictures at most 4096 pixels a side; scale is quantized to eighth
  octaves so a zoom does not redraw every frame.
- Viewer: a title layer draws its newest picture as one quad through its
  transform; handles use the picture's layout box. Until one arrives the
  title is not drawn. Drafts are drawn by asking for the draft's picture.
- Inspector: a title has Video, Text, Animate and Motion; a graphic has
  Video, Graphic, Animate and Motion; a cue adds its track's language.
  Library: a Text tab (title styles, caption styles, templates, stickers,
  emoji, the person's presets) and a Captions tab (generate, import, export,
  translate, the cue list of the selected caption track).
- Speech lane: one job at a time (download or transcription), progress and
  a named cancel; results carry project and request and are held for review
  ("Captions are ready") when the project changed meanwhile.

## Acceptance

| Area | Proof |
| --- | --- |
| Model and commands | Core tests: bounds, NaN, defaults, v2 JSON decode, realignment, split/trim/rate rebasing, reversal, atomic failure |
| Store | v2 fixtures load; writer emits 3; a v2-pinned reader refuses v3 |
| Evaluation | Plan tests: progress per kind and edge, unit spread, karaoke state, frame snapping |
| Subtitles | Round trips of SRT and VTT, malformed input refused whole, VTT word tags |
| Renderer | Picture tests with the bundled font: ink bounds, outline grows ink, shadow offset, background box, wrap, emoji fallback where a system face exists |
| Speech | Model catalog checksums; transcription of an owned fixture with the real model when present, reported `SKIP` otherwise |
| Agent | Provider schema under 24 KiB and the escaped command line under 31 KiB; planner text covers the new commands |
| UI | One history entry per gesture; captures at normal and narrow widths |

## Not claimed

Video export or burned-in captions in a file (no export path exists);
effects, masks or blend on titles; real-time measurements of text animation;
speaker labels; word timing better than Whisper's DTW gives; transcription
of retimed stretches (silent in the mix); image elements in graphics;
right-to-left layout review; vertical text; macOS/Windows acceptance.

## Tickets

| Id | Title | Depends |
| --- | --- | --- |
| 01 | Title model, validation and schema v3 | – |
| 02 | Commands, realignment and rebasing | 01 |
| 03 | Resolve: motions and word state | 02 |
| 04 | SRT/VTT and cue chunking | 01 |
| 05 | Text renderer and text lane | 03 |
| 06 | Viewer draws title pictures | 05 |
| 07 | Inspector Text, Graphic and Animate sections | 06 |
| 08 | Library Text tab and presets | 07 |
| 09 | Captions tab, languages and subtitle files | 04, 07 |
| 10 | Automatic captions (speech lane) | 09 |
| 11 | Translation and agent surface | 09 |
| 12 | ADR 0007, glossary, usage and verification | all |

## Comments

2026-10-13: Spec written from the code as it stands (advanced editing
resolved) and a survey of speech options: whisper.cpp 1.7 via `whisper-rs`
0.16 (DTW word timestamps, VAD, language detection, translate to English),
WhisperX, Vosk, Parakeet through onnxruntime, and FFmpeg 8's `whisper`
filter (not enabled in common builds, segment times only). Nothing
implemented yet.

2026-10-14: All twelve tickets resolved. Where what shipped differs from the
text above:

- Speech uses whisper.cpp 1.8.3 (through `whisper-rs` 0.16 and
  `whisper-rs-sys` 0.15), not 1.7. The speech panel offers Tiny, Base, Small,
  Medium and Large v3 Turbo with the Silero detector, and Base is the default.
  Models are fetched with the system's `curl` against fixed sizes and SHA-256
  sums, or chosen as files.
- Translation into English is Whisper's switch in the speech panel. Any other
  language goes through the native provider as one reviewed `copy_captions`
  proposal (ticket 11).
- A new caption track takes a free name ("English captions 2" when "English
  captions" is taken), whether made by hand, by import, from speech or by a
  translation. A numbered name still counts as named for its language. Two
  tracks with one name were found while driving the release build.
- Acceptance, Store row: a reader pinned to version 2 was not run against a
  version 3 file. The future-schema refusal is tested with version 4 through
  the same path.
- Acceptance, UI row: most captures at 1440 and at 760 are of the debug
  build. The release build was driven at both sizes for captions from speech
  and the caption track chooser.

ADR 0007 records the decision. The glossary, architecture, performance,
installation, usage, agent protocol, native AI and verification documents
describe the result. [Verification](../../docs/verification.md#titles-captions-and-motion-graphics)
states what was and was not tried.

2026-10-14: An export path now exists:
[export and rendering](../export-and-rendering/spec.md). Captions can be burned
into an exported picture, written beside it as SRT or VTT, or carried inside
MP4, MOV and WebM files; titles are drawn by the export through the
compositor at the output scale. The "no export path exists" statements above
are superseded by
[ADR 0008](../../docs/adr/0008-export-renders-originals-through-the-preview-compositor.md).
