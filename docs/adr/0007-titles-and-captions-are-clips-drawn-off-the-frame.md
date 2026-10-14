---
status: accepted
---

# Titles and captions are clips drawn off the frame, with local speech

Styled titles, text animation, captions with word times, karaoke, subtitle
files, caption languages, graphics such as lower thirds and callouts, and
reusable presets are added as data on the clips and tracks the editor already
has. They are drawn by one CPU renderer on a bounded worker. Captions are made
from the timeline's own sound by a speech recognizer built into the binary,
with a model downloaded when first asked for. The existing contracts stay as
they were: every edit is a validated reversible
command, the core knows no GUI, font file or codec, and workers are bounded and
stale-safe.

**One renderer on what is already linked.** Titles and graphics are drawn to
premultiplied pictures by `editor-media::text`, built on `vello_cpu`, `skrifa`
and `harfrust`, the crates egui 0.36 already links for its own text. Shaping,
colour emoji, outlines, blurred shadows and backgrounds therefore add no native
dependency. egui's text path was set aside because it has one font, no outline
or shadow and a 480-point cap. A GPU glyph renderer was set aside as a second
implementation that an export path would not share.

**Pictures are made off the frame.** The `aster-titles` lane makes a title's
picture from the layer `resolve` produced, at the scale the screen shows it.
The viewer draws the newest picture it holds as one quad through the layer's
transform. Answers name their clip and exact layer and are dropped when stale,
as decoded frames are. Titles stay outside the compositor, so effects, masks
and blend modes on titles remain refused.

**Style and animation are durable data, evaluated in the core.** A style names
a font family, weight and slant, never a file. Its lengths are fractions of the
font size, so a style scales with its title. An enter or exit animation is a
preset (`fade`, `pop`, `bounce`, `typewriter`, four slides, `wipe`) over units
(whole, line, word or letter) with a duration and a delay. `resolve` turns the
frame-snapped clip time into one progress per animation and the karaoke word
state, so a paused viewer has nothing to redraw and the render plan stays the
test oracle. Building text animation from keyframes was set aside: per-letter
motion would need a track per letter and could not follow an edit to the text.

**A caption is a title on a caption track.** A text track with a BCP 47
language is a caption track, and each title on it is one cue. Word times are
`{at, end}` pairs relative to their clip, as keyframes are, so moving, rippling
and pasting touch none of them. Every timeline gesture (move, trim, split,
delete, ripple, lock, hide) works on captions unchanged, and split, trim and
rate changes rebase the words in the reducer. Editing the text realigns the
times to the new words by a deterministic diff, so fixing a typo keeps the
karaoke timing. A separate subtitle stream type was set aside: it would have
needed its own copy of every timeline gesture.

**Graphics are one clip of named elements.** A lower third, title card,
callout or vector sticker is a graphic clip: a design box and up to 32 text,
rectangle, ellipse or line elements, each with its own frame and animations.
Its editable parameters are read from the data: each text element's words, and
the colours and font its elements share. A template is a graphic kept outside
the project; adding it copies it in. A parameter table stored beside the
elements was set aside because it could disagree with them. Lottie or
After Effects templates were set aside as a second animation runtime.

**Presets stay outside the project.** Built-in looks, templates, stickers and
emoji live in the desktop. The person's kept looks, templates and brand colours
live in `presets.json` beside the settings, written atomically on a worker.
Giving one copies its values into clips, so a project never depends on a preset
file and a damaged preset file is moved aside rather than written over.

**Speech is local and opt-in.** Captions from speech use whisper.cpp through
`whisper-rs`, compiled into `editor-media` from source. Nothing is installed at
run time and no Python, PyTorch or GPU runtime is involved. A model is
downloaded only when the person asks: it comes from a fixed list with sizes and
SHA-256 sums, through the system's `curl` so that no TLS stack is linked, and
is kept in the user data directory. A model file can also be chosen by hand on
an offline machine. Word times come from whisper.cpp's DTW alignment of tokens,
and the Silero detector keeps music and silence away from the model. The sound
is the timeline's own mix, rendered offline at 16 kHz, so cuts, gain and mutes
are what is transcribed and cue times are timeline times.

The alternatives set aside were:

- WhisperX, whose better word times come from a wav2vec2 alignment model run in
  Python and PyTorch, gigabytes of runtime a native editor cannot ship;
- cloud speech services, which need accounts and uploads;
- Vosk, a prebuilt C library with weaker accuracy;
- NVIDIA Parakeet through ONNX Runtime, a second large native runtime;
- FFmpeg 8's `whisper` filter, which common FFmpeg builds do not enable and
  which gives segment times rather than word times;
- bundling a model, which would add 33 MB to 575 MB to every download.

**Translation reuses reviewed proposals.** Whisper can translate speech into
English while transcribing. Any other language goes through the native AI
provider the person already signed in to, as one `copy_captions` command
carrying only the translated texts. It is reviewed like any other proposal and
applied as one history entry. It makes a new caption track above the source,
with the source's times, and hides the source. No translation model is bundled.
A local translation model (NLLB or Marian) was set aside as hundreds of
megabytes and another runtime.

**Schema version 3.** The file reads versions 1 to 3 and always writes 3. An
older build therefore refuses a file with titles it cannot keep through its
future-schema protection, instead of saving it back without them.

**Consequences.**

- Building now needs CMake, a C and C++ compiler and libclang.
- whisper.cpp is built for AVX2, FMA, F16C and BMI2 on x86-64 rather than for
  the build machine, so one build serves processors from 2013 onwards. Older
  x86-64 processors cannot run the recognizer and may not start the binary.
  64-bit ARM uses NEON.
- whisper.cpp runs on the CPU only. Its GPU back-ends are not enabled.
- Titles cannot carry effects, masks or blend modes until they enter the
  compositor.
- There is no export path, so captions are not burned into a file. SRT and VTT
  export is the way out of the editor.

The binding rules, the commands and their dated changes are in
[the titles and captions design](../../.scratch/titles-and-captions/spec.md).
[Verification](../verification.md#titles-captions-and-motion-graphics) lists
what was measured and on which host.
