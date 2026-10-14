# Core editor guide

Aster's current core is a native editing workspace. It keeps source files
unchanged and records durable edits through the same validated history used by
external agents. The [verification record](verification.md) identifies tested
flows and remaining platform/media gates.

## Projects and recovery

The project menu creates, opens, saves and duplicates `.aster` documents.
Duplicate writes a new file with a fresh project identity and keeps the original
file unchanged; a duplicate cannot replace its own source path. Preferences
persist the window's library/inspector layout, appearance, interface size,
recent projects, snapping, autosave and library sort mode.

Interface size (Settings, or Command/Ctrl+Plus / Minus / 0) scales the whole
window from 50% to 300% in 10% steps. A size larger than leaves the editor its
720 × 520 point layout is held to the largest the window has room for: about
150% at 1280 × 800 and 200% at 1920 × 1080. The chosen size returns as the
window grows.

Autosave retains recoverable editing state. The ordinary `.aster` file stores
the current project; its `.aster.recovery` sidecar preserves undo/redo chains.
Opening the ordinary file starts fresh history; accepting recovery restores the
session's history. Unsaved projects have an isolated recovery destination under
the settings directory. Save and recovery errors remain visible.

Checkpoints are named project snapshots beside the project file. Save the
project first, then open Checkpoints from its menu. Restoring a checkpoint is
one reversible edit in the current project: Undo/Redo remains available, and
the project path and live context stay intact. Checkpoints from another project
identity are rejected.

An OS lock on `.aster.lock` owns the recovery sidecar. A second editor does not
offer or overwrite another active editor's recovery; after an actual crash,
the lock is released and recovery can be offered. The lock file can remain
after ownership ends. This lease does not provide conflict resolution for two
explicit saves to the same ordinary project file. Ordinary saves separately
retain the file as opened or last saved by this editing session, compare its
byte count and SHA-256 under a write lock, and refuse a changed destination.
The conflict sheet offers Save As or a deliberate discard and reopen; it never
silently adopts another writer's file. A second window keeps its own private
recovery copy while the first owns the named sidecar.

A named recovery copy retains that original save baseline. If the ordinary
file changed after a crash, restoring shows Changed on disk before saving and
preserves undo/redo. A legacy, private or moved copy without a usable binding
restores dirty with no file. Its first Save As requires a new destination and
refuses an existing file, even if the OS chooser offered Replace. After a
successful save to that new file, normal protected saves resume there.

## Media and timeline

Import source files or a directory from the library controls, drop sources into
the window, or pass paths at launch. Images/GIFs decode natively; video/audio
require the [media executables](installation.md#media-executables). Directory
import retains virtual folder organization and caps one batch at 10,000 entries.
Native drops are bounded to 4,096 paths. The Wayland listener supports ordinary
local file URI lists on the root window's first seat; sandboxed portal-only
transfer formats are not yet accepted. Weston native drop/timeout/close has been
exercised; actual GTK Open/Save As/Duplicate dialogs have also been driven on an
isolated Linux display. GNOME/KDE file-manager, import chooser and sandboxed
portal-transfer acceptance remain separate platform gates.
Sources remain file references, rather than media embedded in the project.
Unavailable sources keep their assets/clips and report preview errors; an asset
relinking interface or portable media bundle is not part of this first build.

The library offers thumbnails, grid/list views, search and name/duration/kind/
file-size sorting in either direction. Create virtual folders from its add
menu; move an asset through its context menu or drop it onto a folder. These
folders organize the project, independently of source directories on disk.

Double-click an asset to add it to the timeline, or drag it to an appropriate
track/time. Video, audio and text tracks have no fixed model count limit.
Timeline rows and library entries are virtualized. Zoom, snapping and markers
help place edits; performance at very large project sizes still needs measured
acceptance.

Select clips to trim their handles, drag/move, split, copy/paste, duplicate or
delete. Ripple deletion/trimming shifts following eligible content. Linking
preserves editing relationships and grouping moves a selection together. Track
locks block clip edits; hide suppresses picture and mute suppresses sound.
Clipboard copies work between native Aster windows and carry source references;
unrelated text on the OS clipboard does not silently create clips.

The inspector edits crop, position, scale, rotation, opacity, horizontal/vertical
reflection, gain, clip rate and a title's text, style and animation. Project settings select 16:9, 9:16,
1:1 or custom dimensions, frame rate and background. Numeric/drag interactions
commit reversible edits rather than adding a history entry for every pointer
sample.

A clip can also be selected from the keyboard: C takes the clip under the
playhead (pressed again, the next one down the stack), Alt+Left/Right the clip
before or after it on its track, and Alt+Up/Down the clip on the track above or
below. The selected clip is scrolled into view with a focus ring and the
playhead moves onto a clip it was off.

## Looks, animation and motion

Everything in this section is drawn by a GPU compositor, in the viewer and in
an [export](#exporting). If the window has no usable graphics device the
viewer keeps position, scale and opacity only and shows "Effects not shown";
an export opens a graphics device of its own, a software one if need be. The
[verification record](verification.md#advanced-editing) says which of these
were driven natively and on what host; none has a measured playback rate.

The inspector of a selected video, image or GIF clip has five sections:
**Video**, **Look**, **Color**, **Speed** and **Motion**. A title has Video,
Text, Animate and Motion, and a graphic has Video, Graphic, Animate and Motion
(see [titles, captions and graphics](#titles-captions-and-graphics)); a sound
clip has none of them and keeps its volume, speed and timing. A
locked track disables the rows and the project refuses the edit. Every change
is one undo step: a drag, a scrub or a typed value commits once, and Escape
gives up a drag or a value being typed and leaves the selection.

### Keyframes and the graph editor

Rows that can be animated carry a diamond. Put the playhead on the clip and
press the diamond (or Enter with it focused) to add a keyframe at the playhead;
press it on a keyframe to remove that one. Shift+Enter opens the diamond's
menu: Change easing, Remove keyframe, Remove animation. Arrows beside the
diamond jump to the previous and next keyframe. Once a row is keyed, changing
its value at another time writes a keyframe there; a paired row such as
Position keys X and Y together. Dragging a keyed clip in the viewer writes
keyframes at the playhead in the drag's one undo step. A diamond pressed while
the playhead is off the clip says so and keys nothing.

Position, scale, rotation, opacity, crop and volume can be keyed on the clip,
and so can each numeric parameter of an effect or a mask. Easing choices are
Linear, Hold, Ease in, Ease out, Ease in and out and Overshoot; easing is not
offered on a track's last keyframe.

With one clip that has animation selected, U (or the chevron on its tile, or
"Keyframes of the selected clip" in the timeline's tool strip) opens its
keyframe lane: one row a parameter under the clip's track. G or "Graph editor"
puts curves in the tracks' place for the one selected clip. In either:

- Drag a keyframe in time; it moves on the frame grid, snaps to the playhead
  when snapping is on, stays inside the clip and replaces a keyframe it lands
  on. In the graph a drag also changes the value, and Shift keeps it to one
  axis.
- Select a keyframe in the graph to show the handles of the curves beside it.
  A linear stretch becomes a curve when a handle is moved; a hold has none.
- With keyframes selected, Left/Right move them one frame (ten with Shift),
  Up/Down change the value in the graph, Delete removes them, and Command/
  Ctrl+C, X and V copy, cut and paste at the playhead into the same
  parameters. Each press is its own undo step.
- The context menu offers Select all keyframes, Copy, Paste at playhead,
  Delete and, in the graph, Fit values.
- Escape gives up a held drag, then the selected keyframes, then the clip.

The speed of a clip is not drawn in the graph; the Speed section edits it.

### Effects and looks

**Look → Add effect** lists the kinds by family: Stylize (glow, grain,
vignette, shadow, glitch), Blur and sharpen (blur, sharpen, motion blur),
Distort (shake, wave) and Key (chroma key). The library's **Effects** tab lists
the same kinds and the colour ones: double-click or press Return to add one to
the selected clips, or drag it onto a clip. Each effect in the stack has its
rows, a Mix slider, a show/hide switch, Move up/Move down and Remove. Effects
apply top to bottom; a clip holds sixteen. Motion blur streaks the picture
along an angle and does not accumulate frames; Distort is a wave.

**Look → Looks** gives the clip one of Cinematic, Teal & orange, Vintage,
Faded, Noir, Warm or Cool at the Strength shown. A clip holds one look: giving
another replaces it, Strength remakes it in one step and Remove look takes it
away. The look is made of ordinary grade, curve, grain and vignette effects
that can then be adjusted like any others. The library's **Looks** tab does the
same by double-click or drag.

Effects, masks and blend modes are for pictures on video tracks; titles and
sound take none.

### Colour grading, curves and LUTs

The **Color** section edits one grade for the clip: exposure, contrast,
saturation, temperature, tint, lift, gamma and gain, with Reset grade. Under
it:

- **Hue bands** shift one range of hues (Reds, Yellows, Greens, Cyans, Blues,
  Magentas) in hue, saturation and lightness. Add hue band and Remove hue band
  add and remove them by name.
- **Curves** has Master, Red, Green and Blue. Click the curve to add a point,
  drag a point to move it, double-click to remove it; a focused point takes
  the arrow keys and Delete. A curve holds 2 to 16 points. Reset curve returns
  it to a straight line.
- **LUT** grades with an imported `.cube` table (2 to 65 points an edge, 32 MiB
  at most). Import a LUT file adds it to the library as a LUT asset, which
  cannot be placed on the timeline; the Intensity row is the effect's mix. A
  missing table changes nothing and is shown as Missing.

A grade or a curve the section made is taken out again when it is back where
it changes nothing, unless keyframes drive it.

### Background removal

**Color → Remove background** holds one method at a time:

- **Chroma key** removes a key colour. Pick key colour arms an eyedropper:
  click the picture in the viewer to take the colour under the pointer from
  the clip's own frame, before its effects. Tolerance, Softness and Spill tune
  the edge. Escape disarms the eyedropper.
- **Clean plate** compares each frame with a still of the scene without its
  subject (Import a clean plate, or Choose one from the library) and keeps what
  differs. The camera must not have moved between the plate and the clip.

There is no automatic subject cut-out; the section says so. **Background →
Add background** puts a picture or a video from the library on the nearest free
unlocked video track under the clip, or on a new track made directly under it,
as one undo step. Where no track has a picture under the clip the project's
canvas colour shows, and it can be set from the same group.

### Masks, blend modes and picture-in-picture

**Video → Masks → Add mask** adds a rectangle or an ellipse with centre, size,
rotation, feather and roundness, an Invert switch and an Add, Subtract or
Intersect choice. The first mask adds to nothing or takes from the whole
picture; later ones combine in order. A clip holds eight masks. The selected
mask is outlined on the picture; it is edited from its rows, not dragged on
the canvas.

**Video → Compositing → Blend** chooses Normal, Multiply, Screen, Overlay,
Soft light, Add, Darken, Lighten or Difference. A clip blends with the pictures
under it. A blended clip above a title blends only with the pictures between
the two.

Picture-in-picture is a clip on a higher track made smaller. **Video →
Layout** has Fill the canvas, Left half, Right half and the four insets (top
left, top right, bottom left, bottom right); a preset sets position, scale and
crop and is refused while any of them is keyed. A shadow effect, a mask and a
blend mode finish the inset.

### Transitions

Add a transition with Command/Ctrl+T (each selected clip's edge nearest the
playhead), the clip menu's Add transition at start or at end, the "+" badge
that appears at a cut or free edge under the pointer, or the library's
**Transitions** tab (double-click, Return or drag onto a clip's edge). The
first one is a half-second dissolve unless a default was saved for its kind.
A cut between two clips belongs to the incoming clip; asked for at the end of
a clip another follows, the transition goes to that next clip.

The kinds are Fade, Dissolve, Slide (left, right, up, down), Zoom in and out,
Wipe (left, right, up, down), Glitch and Blur. A transition lasts from one
frame to the shorter of its clip and ten seconds. It is drawn as a marking on
the clip's edge: drag the marking's inner end to change how long it lasts, or
click the marking to edit kind, duration, easing and the kind's own value in
the inspector, where Remove and Save as default also live. Delete removes a
selected marking's transition and leaves the clip. Titles always fade and
sound takes no transition. The incoming clip is not decoded ahead of the
transition, so its first frames can arrive late.

### Speed, reverse, freeze and ramps

The **Speed** section has a speed field, 0.25× to 4× presets, the resulting
duration and a Reverse switch. A speed change shortens or lengthens the clip
only into the room its track has; later clips are not moved.

- **Freeze frame**: F, the clip menu's Freeze frame, or Freeze at playhead
  holds the frame under the playhead for the Hold for length (2 seconds unless
  changed). F acts on the selected clip under the playhead, else the top clip
  there.
- **Ramp** presets (Ramp up, Ramp down, Montage, Hero, Bullet, Flash in,
  Flash out) replace the clip's speed curve.
- **Speed curve** shows speed over the clip. Drag a point to change the speed
  there (0 to 16×), choose Constant, Linear or Smooth for a stretch, and Add
  point or Remove point. A curve the project would refuse is not applied; the
  last valid one stays and a line says why. The viewer and the timeline follow
  when the drag is released.

A retimed clip shows a badge on its tile (for example 2×, Reversed, Freeze,
Ramp). Its keyframes move with the picture: a freeze shifts later ones by its
length and a reverse mirrors them. Sound plays only on straight forward
stretches and is silent where the clip is held, reversed or ramped; a speed
change alters pitch. Stepping or playing backwards through video reads frames
in groups and is not smooth at every size.

### Motion tracking and follow

**Motion → Track a region** puts a box on the picture of a video clip. Draw a
new box or move the one there in the viewer, or type its centre and size, then
press Track. The
region is followed forwards through the source from the frame the viewer shows
at the playhead to the furthest frame the rest of the clip shows, in the
background, with a progress bar and Cancel tracking. The result is a tracker
named "Tracker n" that can be renamed and removed; its path is drawn over the
picture while the Motion section is open. Escape gives up a box that has not
been tracked. Where a clip plays backwards Track is disabled and the section
says why: frames are only read forwards, so put the playhead where the clip
plays forwards. On a stabilized clip the box is drawn on the steadied picture
and is the place of the frame shown under it.

A finished path is added as one edit. If it arrives while there is something
to redo, or while a plan from the model is under review, it is not added by
itself: the section shows "Tracker n is ready" with Add tracker and Discard,
and it is added without asking once the redo is used or the plan is settled.

Tracking finds where one patch of the picture moved. It does not detect
objects, follow rotation or scale, or find a region again once it is lost: the
path ends where the match fails. A notice says once that the region was lost
and after how long; afterwards the tracker's card shows only its length and
point count, and a follower holds the last point from there. Only video is
tracked.

**Motion → Follow** on another clip picks a tracker to take its position from.
The clip keeps its place at the playhead; Offset X/Y move it relative to the
tracked point, Centre on point puts its centre on the point, and Stop following
ends it. The point is put where the tracked clip draws that place of its
picture, through that clip's position, scale, rotation, crop and flips, its
keyframes, and the reading of a stabilized picture, so the follower sits on the
object in a letterboxed, moved or picture-in-picture clip too. Follow has no
effect while the tracked clip is not on screen. Over a stabilized clip the
path overlay draws every point through the current frame's correction, so it
shows the shake around the object rather than a still line.

Removing media from the library also removes the trackers of that media when
no clip uses it any more, as one undo step, and a notice names them. The
command `remove tracker NAME` removes one with nothing selected.

### Stabilization

**Motion → Stabilize** switches steadying on for a video clip and offers
Analyse. The analysis measures the camera's motion through the whole video in
the background, with progress and Cancel analysis, and is kept for later
sessions in a cache outside the project. Until it is ready the clip is shown
as the camera took it. Strength sets how much the path is smoothed and Crop
zoom how far the picture is cropped in; Fit crop sets the least crop that
hides the borders of this clip. With less crop than that, some frames are
steadied less so that no border shows.

This is a two-dimensional deshake with a crop. It does not correct rolling
shutter or parallax, and it has been tried on generated fixtures only, not on
camera footage. If the media file changes the analysis is shown as stale and
Analyse again measures it anew.

### By command

The command panel (Command/Ctrl+K) understands, for the selected clips:
`add effect glow`, `remove effect glow`, `apply look cinematic 60%`,
`set blend screen`, `add transition wipe left at start`, `remove transition`,
`reverse`, `freeze frame for 2s`, `add keyframe opacity 50%`,
`chroma key green`, `follow Tracker 1`, `stop following` and
`remove tracker Tracker 1`. Starting a tracking or a stabilization analysis
has no phrase.

## Titles, captions and graphics

Titles, captions and graphics are drawn on this computer by a background
worker and shown in the viewer as each picture is ready, so a title can appear
a moment after it is added or changed. An [export](#exporting) draws them at
the output size and can burn captions into the picture, write them beside the
file or carry them inside it. The [verification record](verification.md#titles-captions-and-motion-graphics)
says what was driven natively and on what host.

### Titles and text styles

Press T, use the timeline's text button, type `add text Hello for 3s` in the
command panel, or add a tile from the library's **Text** tab. A title is a clip
on a text track and edits like any other clip.

The inspector's **Text** section holds the words, in a field that keeps several
lines; leaving the field makes the change one undo step. Under it:

- **Font**: family, weight, italic, size and colour. The list joins the faces
  installed on this computer, the project's font assets and the bundled Geist.
  A family that is not on this computer is named, and Geist stands in for it.
- **Layout**: alignment, line height, spacing between letters, a width to wrap
  at, and case (as typed, UPPERCASE or lowercase).
- **Outline**, **Shadow** and **Background** are switched on and off as cards.
  A background is one box behind the text or a box behind each line, with its
  own padding and corners.

Lengths such as an outline's width are shown in pixels at the title's size and
grow with it. Emoji are drawn in colour when the system has a colour emoji
font. Import font files (`.ttf`, `.otf`, `.ttc`, `.otc`) with **Import fonts**
in the Text tab's header; each becomes a font asset named for its family and
travels with the project as a file reference, like media.

Effects, masks and blend modes cannot be put on titles.

### Text animation

The **Animate** section has **In** and **Out** cards. Each takes a style
(Fade, Pop, Bounce, Typewriter, Slide up, Slide down, Slide left, Slide right or
Wipe), an order (All at once, By line, By word or By letter, each part starting
a little after the one before), a duration and a delay; for Out the delay is
"Ends before" the clip's end. Times are whole frames and fit the room the clip
leaves; a clip too short for one says so. The command panel takes
`animate in pop`, `animate out fade for 1s` and `animate in none` for the
selected titles, as one step.

**Karaoke** marks the word being said: in a colour (Word), with a box behind it
(Box), or with a colour sweeping through the words (Fill). **Word size** grows
the word being said and **Words to come** sets how visible the words not yet
said are, down to revealing them one by one. It needs word times, which
captions from speech have.

### Lower thirds, title cards, callouts, stickers and emoji

The **Text** tab's groups are Titles, Captions, Lower thirds, Title cards,
Callouts, Stickers and Emoji, then **Yours**. A tile plays its entrance while
the pointer rests on it. Double-click a tile or press Return to add it at the
playhead, or drag it to a text track. It goes on the track you choose, or the
first plain text track with room, or a new one, and lasts three seconds or as
long as its animations need. Sizes are made for the canvas.

Lower thirds, title cards, callouts and drawn stickers are **graphics**: one
clip of named parts. The inspector's **Graphic** section has a field for each
text part, the colours the parts share (changing one changes every part using
it) and one font for all of its text. Its Animate section gives every part the
same entrance and exit with a **Between parts** stagger.

Drop a title or caption tile on a title to give it that look as one step; its
words and word times stay. On a caption track every caption takes the look.
**Style selected titles** in a tile's context menu does the same for the
selection.

### Your presets and brand colours

**Keep this look** in the Text section and **Keep as template** in the Graphic
section save the selected title or graphic under a name. Kept looks and
templates appear under **Yours**, where they can be renamed and removed,
together with **Brand colours**, which every colour picker in the inspector
offers above the picker. They are saved in `presets.json` beside the settings,
not in the project: adding one copies it into the project, so a project opens
the same on a computer without your presets. A preset file this build cannot
read is kept aside as `presets-unread-<time>.json` rather than written over.

### Captions

A caption track is a text track with a language, and each title on it is one
caption. The library's **Captions** tab shows one caption track at a time:

- The header row chooses the track (with **New caption track…** at its foot)
  and hides or shows it. Under it are the track's language and **Style**,
  which gives every caption of the track a caption look.
- Click a caption's words to type; Shift+Return adds a line, Return or leaving
  keeps it and Escape gives it up. Click its start or end time to type one as
  `0:01.20`, `2.5`, `12f` or `500ms`; a time snaps to the frame and stops at the
  captions next to it.
- A caption's menu has Start at playhead, End at playhead, Split at playhead,
  Merge with next and Delete caption. **Add caption** adds two seconds at the
  playhead in the look of the caption before it.

Every change is one undo step. Captions are clips, so they can also be moved,
trimmed, split and rippled on the timeline; word times follow.

The language menu lists 40 languages and takes a typed name or BCP 47 tag
(`Spanish`, `pt-BR`, `pt_br`). A new caption track is named for its language
("English captions"), numbered when that name is taken ("English captions 2").
Changing the language renames a track that is still named for its last one.

**Import SRT or VTT** reads one or more files of up to 8 MiB each, as UTF-8,
UTF-16 or Windows-1252, and makes one caption track for each file in one
step. The
language comes from a VTT `Language:` header or a tag in the file name
(`talk.pt-BR.srt`), and the notice says when it was guessed. A file that does
not parse adds nothing. **Export** writes the track in view as SubRip or
WebVTT; VTT keeps the language and word times.

### Captions from speech

The microphone in the Captions tab's header (or **Generate from speech** when
there are no captions, or `generate captions` in the command panel) opens the
speech panel. Captions are made on this computer from the timeline's own
sound, so cuts, volume and mutes are what is heard; nothing is uploaded.

- **Model**: Tiny (33 MB), Base (60 MB, the default), Small (191 MB), Medium
  (540 MB) or Large v3 Turbo (575 MB), each with the speech detector. Larger
  models are more accurate and slower. **Model file…** takes a Whisper model
  you already have, for a computer without a network.
- **Language spoken**: detected from the speech, or chosen.
- **Captions from**: the whole timeline or the selected clips.
- **Caption length**: A few words (for Shorts and Reels), Full lines (for
  subtitles) or One word (for fast cuts).
- **Translate to English**: Whisper writes English captions from speech in
  another language.

The first use of a model downloads it once (**Download and generate**), with
`curl` from the whisper.cpp model repository, and keeps it only if its
SHA-256 sum matches. Models are kept in `models/` beside the settings file when
Aster is started with `--settings`, otherwise in the platform's local data
directory (on Linux, `~/.local/share/aster/models`). **Stop making captions**
stops a download at once and recognition at the end of the model's current
pass, which took about 3 seconds with Tiny and takes longer with larger
models.

The captions arrive as a new caption track in one undo step, with word times,
in the Clean caption look; pick a karaoke look from **Style** to light words
as they are said. If the project was edited while they were being made, or
adding them would drop a redo or an open AI plan, they wait in the panel
("Captions are ready") until you add or discard them.

### Translating captions

**Translate into…** in the caption track menu, or `translate captions to
Spanish` in the command panel, asks the AI provider chosen in the Agent panel
(Claude or Codex, signed in through its own CLI) for a copy of the track in
another language. The captions' text, times and the rest of the project's
editing data are sent; no sound is. The answer is shown as a plan to review:
**Apply** adds the translated track above the original, with the same times,
and hides the original, as one undo step. Translation quality depends on the
provider and was not checked by this project.

## Playback and source preview

Timeline preview decodes source media and composes visual layers. Scrub the
playhead, step project frames or change preview speed. Frame stepping uses the
rational project rate. An audio-device clock is used when available; actual
device output, A/V drift and native display latency remain verification gates.

The current source monitor opens an asset through its Preview control, context
menu or Enter after selecting it. Its transport scrubs, steps and plays that
source independently of the timeline, and Escape returns to the paused timeline.
It does not insert a clip or change the document. Round-three Linux XTest
captures exercised video/GIF/audio/still source controls without changing the
document, and rebuilt GIF preview matched its frame/time stamp. The final
optimized Linux build passed native editing and shutdown/recovery checks;
see the [acceptance record](verification.md#native-linux-evidence) for exact scope.

The software audio preview mixes up to 16 simultaneous audible clips; exceeding
that budget reports an error. For a linked video/audio pair using the same
asset, the explicit Audio-track clip owns the sound, including when it is muted,
so embedded video sound does not double it. Links between different assets mix
normally. Rate changes currently change pitch.

The frame cache retains up to 64 MiB/256 frames and five software video decoders;
GIF sequential working state has a separate 64 MiB/four-state budget.
Preview is RGBA software decoding, with up to 4096 pixels per requested dimension
and 64 MiB per frame. Large videos can be previewed from smaller copies; see
[preview quality and proxies](#preview-quality-and-proxies). Hardware/zero-copy
decoding, HDR, exact variable-frame-rate PTS handling and waveforms are not
current features.

## Exporting

Export makes a video or sound file from the timeline. Every frame is drawn
again from the original media, at the size you choose, by the same compositor
the viewer uses; the sound is mixed the way playback mixes it. Exporting does
not change the project and is not an undo step. It needs FFmpeg (see
[media executables](installation.md#media-executables)), and what it can write
depends on that FFmpeg. What was verified, on one Linux host only, is in the
[verification record](verification.md#export-and-rendering); the
measurements are in [performance](performance.md#export-and-proxy-budgets).

### The export dialog

Open it with **Export** at the right end of the toolbar, **Export…** in the
Project menu, Command/Ctrl+E, or `export` in the command panel. Over the
render queue, Command/Ctrl+E opens it too. In a narrow window its two columns
become one.

The left column shows what will be made:

- A picture of the output frame at the playhead, or at the range's first or
  last frame when the playhead is outside the range, with the export's black
  bars or the cut of the fit. It is drawn from the original media, at most
  640 pixels on its long side, once the playhead and settings have stayed the
  same for 150 ms. Until it is drawn, or if it cannot be (the reason is on
  hover), a plain placeholder in the output's shape stands in; a sound-only
  export shows "Sound only".
- **File name**, with the extension shown beside it; typing the extension does
  not double it, and `/ \ : * ? " < > |` are refused. **Folder** opens a folder
  chooser titled "Export to folder"; its button shows the folder as the
  render queue does (`~/…/Trip/Exports`, or "Home folder"), with the whole
  path on hover. The first export starts in the project's folder, or Videos, or home;
  after that the last folder used is remembered.
- **Range**: **Whole timeline** (from the start to the end of the last clip),
  **Selected clips** (from the first to the end of the last selected clip;
  with nothing selected it is disabled with "Close Export and select clips on
  the timeline first") or **Custom range**, typed as times such as `0:12` or
  `00:00:12:15`, with **Start at playhead** and **End at playhead**. A range
  is widened to whole frames of the output.
- **Summary**: size and aspect, frame rate, sound, duration and frame count,
  an estimated file size marked "≈", and the encoder that will be used (for
  example "NVIDIA NVENC · h264_nvenc" or "Software · libx264"; for sound only,
  the sound encoder such as "aac"). Warnings follow, and in red the reason the
  settings cannot be exported, if there is one.

The estimate for a bitrate is close. For a quality level it is a typical size
for the encoder that will run: on the synthetic test pictures measured, Lower
and Recommended came to 0.89 to 1.29 times the estimate. Higher and Maximum
depend on the footage: 0.42 to 0.9 times the estimate on clean pictures,
and three to four times it with libx264 on grainy ones. Camera footage has not
been measured.

The right column holds the settings. **Preset** comes first: a list grouped
by where the file is going, with your own presets last, a one-line note
under it (what the preset makes, "A preset of your own", or "Changed from the
presets; Save preset… keeps them"), and a row of as many of Recommended,
YouTube 1080p, TikTok, Instagram feed (4:5) and Audio (WAV) as fit, to pick
with one click:

| Group | Presets |
| --- | --- |
| General | Recommended (MP4 H.264), High quality, Small file (H.265), Web (VP9) |
| YouTube | YouTube 1080p, YouTube 4K (H.265), YouTube Shorts (1080 × 1920) |
| TikTok, Instagram, Facebook, X, Vimeo | TikTok and Instagram Reels (1080 × 1920), Instagram feed (4:5, 1080 × 1350), Facebook, X and Vimeo (1080p) |
| Master | ProRes 422 HQ master (MOV, PCM 24-bit sound) |
| Audio | Audio (AAC), Audio (WAV), Audio (MP3), Audio (FLAC) |
| Saved | The presets you saved |

Every built-in preset keeps the project's frame rate, the Auto encoder,
Letterbox and captions burned in. A preset this computer cannot encode is
disabled, with the reason on hover. Changing any setting shows
**Custom settings**.

- **Video**, switched off to export sound only:
  - **Format**: MP4, MOV or WebM; with Video off, M4A, MP3, WAV or FLAC. Only
    formats this FFmpeg can write are listed.
  - **Resolution**: 720p, 1080p, 1440p and 4K (named by the short side, at the
    project's shape), Project, or Custom width and height, from 16 to 8192
    pixels a side. Sides are rounded to even numbers.
  - **Fit**, shown when the output's shape differs from the canvas: Letterbox
    shows the whole canvas with black bars; Fill fills the output and cuts what
    overflows equally from both sides.
  - **Frame rate**: the project's, or one of the standard rates. Titles,
    grain and other animation step at the output rate.
  - **Codec**: H.264, H.265 (HEVC) or AV1 in MP4; H.264, H.265 or Apple ProRes
    in MOV; VP9 or AV1 in WebM. A codec no encoder here can make is disabled
    with the reason its test failed.
  - **Quality**: Lower, Recommended, Higher or Maximum, which hold a quality
    and let the bitrate vary, or a **Bitrate** in kbit/s (100 to 800,000). ProRes
    has a **Profile** (LT, 422, HQ, 4444) instead.
  - **Encoder and speed**, closed at first: **Encoder** is Auto (it names what
    it picks), Software, or a hardware encoder found here; **Speed** is
    Faster, Balanced or Smaller, trading encoding time against file size.
- **Audio**, always on for a sound-only file: codec (AAC, Opus in WebM, MP3,
  FLAC, or PCM 16 or 24-bit in MOV and WAV), bitrate, sample rate (44.1, 48 or
  96 kHz; Opus is 48 kHz only and MP3 at most 48) and Mono or Stereo.
- **Captions**, shown when the project has a caption track:
  - **Burn into the picture** draws the caption tracks in the video. Turned
    off, caption tracks are left out of the picture; plain titles are always
    drawn.
  - **Subtitle file**: None, SRT or VTT, written beside the export as
    `<name>.<language>.srt` (or `.vtt`).
  - **Subtitle stream in the file**, for MP4, MOV and WebM, which viewers can
    turn on and off.
  - **Track** chooses the caption track to write.

At the foot, **Save preset…** keeps the settings under a name you give, without
the range or the caption track; your presets are kept beside the editor's
settings, not in the project, and one that matches the settings can be
deleted. Saving is available once the encoders have been checked. **Add to
queue** holds the export in the render queue until you start it there ("…
is held in the render queue; start it there"). **Export** queues it to render
as soon as the jobs ahead of it are done and opens the render queue. Both are
disabled, with the reason, while the encoders are being checked, after a
failed check (with **Try again**), while a field is invalid, when the
settings cannot be exported, and when the queue holds 64 jobs.

If a file of that name is already in the folder, you are asked "Replace
“<name>”?" before anything is queued; with a subtitle file whose name is
taken too, "Replace “<name>” and “<subtitle name>”?". The files there stay as
they are until the export has finished. **Cancel** brings the dialog back as
it was.

**Cancel**, Escape or a click beside the dialog closes it and keeps what you
set: opening it again for the same project brings it back. Queuing an export,
or opening another project, starts the next one afresh from the settings,
preset and folder you used last; the range and caption track are not
remembered.

The export renders the project as it was when you queued it: later edits do
not change it. A file of the project's own media, or the project file and its
recovery, lock and checkpoint files, cannot be the destination, nor can a
file another unfinished export in the queue writes ("Another export in the
queue already writes <name>; choose another name"). Names that differ only in
case count as the same file, on every system. A clash found only by following
links, or a folder removed after queueing, fails the job when it starts,
with the same reason.

### The render queue

Open it with the queue button beside **Export**, **Render queue** in the
Project menu, Command/Ctrl+Shift+E, or `render queue` in the command panel.
Over the export dialog, Command/Ctrl+Shift+E opens it and keeps the dialog's
settings. The queue button shows a progress ring, and the number of jobs when
there are several, while a job renders or waits its turn; its tooltip gives
the state, for example "rendering, 34 % · 2 waiting · 1 held". The panel's
heading counts the jobs the same way ("1 rendering · 2 waiting · 1 held").

Each job shows its name, state, its folder (as `~/…/Trip/Exports`, or "Home
folder", with the whole path on hover), a progress bar once it has started,
a line such as "Frame 120 of 360 · 42.0 fps · 1:02 left", the encoder and any
message.

Jobs render one at a time, in the background; you can keep editing. The
list shows the job under way first (rendering, or paused after it started),
then the waiting and held jobs in the order they will render, then the
finished jobs, the most recent first. A job that starts is scrolled into
sight. Jobs that finished while the panel was closed are listed as if they
finished in the order they rendered.

| State | Actions |
| --- | --- |
| Queued | Render sooner / later (move up or down), Cancel |
| Held ("Held until you start it") | Start, Render sooner / later, Cancel |
| Preparing, Rendering | Pause, Cancel |
| Paused | Resume, Cancel |
| Finishing | Cancel |
| Done | Show in its folder, Remove |
| Failed, Cancelled | Render again (Retry), Remove |

A held job was queued with **Add to queue**, or paused before its turn. It is
passed over while the jobs after it render, and **Start all** starts every
held job, in the order listed. Pause on a job that has started stops between
frames and Resume continues the same file; that paused job keeps the jobs
behind it waiting. Retry renders the same snapshot of the project again, not
the project as it is now; it does nothing while another unfinished job writes
the same file. **Clear finished** takes done, failed and cancelled jobs off
the list, and **New export…** opens the dialog. A notice says when a job is
done ("Exported <name>") or has failed ("<name> was not exported: <reason>"),
unless the render queue is open, where the job's row says so; failed jobs stay
in the list with their reason. While a dialog or panel is open, notices
appear at the bottom of the window, below it, instead of over the viewer.

Closing the window while a job is rendering or waiting, held jobs included,
asks first: "Rendering will stop". **Stop and quit** cancels every job at
once and removes their unfinished files, leaving any file they would have
replaced as it was; unsaved changes are asked about next. If you then stay,
the jobs remain cancelled and can be rendered again with Retry.
**Keep rendering**, Escape or a click beside the question keeps the window
open and the jobs rendering.

### Files are never half-written

FFmpeg writes to a hidden file, `.<name>.<job>.part`, in the destination
folder. Only when FFmpeg has finished without error and the file has been
checked for the streams and length expected is it renamed over the
destination, so an existing file there stays as it was until the new one is
complete. A subtitle file beside it is written the same way. The job's
working files, its rendered sound among them, go in a hidden folder,
`.aster-render-<job>`, beside the destination, or in the system's temporary
folder when that one cannot be made. Cancelling or a failure removes the
hidden file and the working folder; quitting during a render removes them
too, unless the editor is stopped before it has done so. What a crash leaves
is removed later: when a job starts, the part files and working folders in
its folder that have not changed for an hour and belong to no listed job, and
when the editor starts, such working folders in the temporary folder.

While FFmpeg finishes the file (MP4 and MOV rewrite it to put their index
first), it is waited for at least 10 minutes plus a second for every 8 MiB
written, and after that for as long as the file keeps changing. Two minutes
without a change fails the job with "FFmpeg stopped finishing the file:
nothing was written to it for 120 seconds".

When the encoder is Auto and a hardware encoder fails, the job starts again
once from the beginning with the software encoder, and its message says so,
for example "NVIDIA NVENC failed (…); finished with libx264 software
encoding". If the software encoder fails too, the message gives both
reasons. A full disk, a missing permission, a name too long, a folder that
went away or a stalled finish fails the job at once, since a second encode
would meet it again. A hardware encoder chosen by name fails with FFmpeg's
reason instead. Some problems do not stop an export but are listed in the
job's message: a stabilized clip whose camera motion was never analysed is
exported as the camera took it, and a LUT that cannot be read is left out.

If FFmpeg is missing or its check failed, the dialog says the encoders could
not be checked; install or fix FFmpeg and press **Try again**.

### Export limits

- Pictures are 8-bit BT.709 (10-bit for ProRes) at a constant frame rate. HDR,
  10-bit H.264 or H.265, transparency and variable frame rate are not written.
- Sound is mono or stereo, mixed by the playback mixer: at most 16 clips sound
  at once (more fails the export), a speed change alters pitch, frozen,
  reversed and ramped stretches are silent, a transition does not crossfade
  the sound, and a hidden video track still sounds unless it is muted.
  Sound keeps the source file's own time, as the picture does: a sound
  stream that starts after the picture is preceded by silence, and Opus
  sound can move by a few milliseconds to agree with the picture.
- A half-transparent clip with no effects, mask or blend mode can be a
  slightly different shade in the export than in the viewer: the export
  blends it in linear light, the viewer in gamma space. Clips with effects
  match.
- Titles are drawn at most 4096 pixels a side, so a title filling an 8K
  output is softer than the rest of the picture.
- Hardware H.264 encoders take at most 4096 pixels a side; above that Auto
  uses the software encoder.
- Every export reencodes; nothing is copied from a source file unchanged.
- Agents can queue and control exports in a running editor window (see the
  [agent protocol](agent-protocol.md#render-jobs)); the headless `aster-agent`
  cannot export.
- Only NVIDIA NVENC was run on real hardware. AMD AMF, Intel Quick Sync, Apple
  VideoToolbox, VA-API and Media Foundation are checked and driven by the same
  code but have not been tried, and their quality levels are set from
  NVENC's measurements, not their own. The AV1 and VP9 quality levels and
  size estimates are not measured on any encoder.
- There is no check for free space before an export starts; a disk that
  fills fails the job with the system's reason.

## Preview quality and proxies

**Settings → Preview** holds two choices that change only what the viewer
shows, never an export.

**Quality** sets how much detail the viewer decodes and composites: **Full**
is every pixel the screen shows, **Half** and **Quarter** a half or a quarter
of that on the long side, for smoother playback. **Auto**, the default, is
Full until frames come late during playback, then Half until playback stops.

**Proxies** are smaller copies of videos made for the preview: at most 1280
pixels on the long side, with the same frames at the same times as the
original, counted from the start of the file, and no sound. A video with
transparency gets none: its proxy fails with "it has transparency, which a
proxy cannot keep, so the original is read".

- **Off** makes none and reads none.
- **Auto**, the default, makes them for videos larger than 1920 pixels on a
  side or above 50 Mbit/s.
- **Always** makes them for every video.

A video's context menu in the library has **Create proxy** and
**Remove proxy** (with Proxies Off, "Create proxy (used when Proxies is
on)"), and its tile shows a badge while one is waiting, being made (with its
progress), ready or failed; with Proxies Off a made proxy is said not to be
read. The viewer reads a proxy only when it is ready, the video file has not
changed since, and the proxy has at least the pixels the clip is drawn at:
the size the viewer's budget lets it have, and during playback no more than
the size it already has. A large viewer at Full quality may still read the
original; Half and Quarter use proxies more often. A proxy file that cannot be
read is not used again until it is removed or made again, and the original is
shown meanwhile. Sound, tracking, stabilization, captions from speech and
every export read the original file.

Proxies are kept in a cache in the user's cache directory (or in `proxies/`
beside the settings file when Aster is started with `--settings`), not in the
project. **Cache size** chooses its limit, from 5 to 500 GB (20 GB unless
changed); the oldest unused proxies are removed first, never one the viewer
has open. A proxy removed to keep the limit goes back to having none, and
is not made again until the library, the project or the Proxies choice
changes. **Clear proxies** removes them all. After you remove a proxy or
clear the cache, Auto and Always do not make that video's proxy again until you
ask or the project is opened again. Without FFmpeg proxies cannot be made:
each one asked for is marked failed in the library, and the viewer reads the
originals. Closing the window stops a proxy being made, waiting at most
750 ms for it. Whether proxies make a given video play smoothly has not been
measured.

## Keyboard controls

Use Command on macOS and Ctrl on Linux/Windows. Editable fields own their usual
text shortcuts; timeline controls resume when text editing finishes. While
keyframes are selected the arrows, Delete and the clipboard chords act on them
(see [keyframes](#keyframes-and-the-graph-editor)). Delete on a selected
transition marking removes the transition, not the clip. Space toggles
playback even while a button has the keyboard. The bindings were exercised on
Linux only; the Alt chords are untested as Option on macOS.

| Shortcut | Action |
| --- | --- |
| Command/Ctrl+N / O / S | New / Open / Save project |
| Command/Ctrl+Shift+S | Save As |
| Command/Ctrl+I / Command/Ctrl+Shift+I | Import files / folder |
| Command/Ctrl+Z / Command/Ctrl+Shift+Z | Undo / Redo |
| Command/Ctrl+C / X / V | Copy / Cut / Paste clips |
| Command/Ctrl+D | Duplicate selected clips |
| Command/Ctrl+G / Command/Ctrl+Shift+G | Group / Ungroup |
| Space | Toggle active timeline/source playback |
| Left / Right | Step one active timeline/source frame |
| Shift+Left / Right | Step approximately one second |
| Home / End | Beginning / End |
| S / M / T / N | Split / Marker / Text / Toggle snapping |
| C | Select the clip under the playhead; again for the next one down |
| Alt+Left / Right | Select the previous / next clip on the track |
| Alt+Up / Down | Select the clip on the track above / below |
| Command/Ctrl+A | Select all clips |
| Command/Ctrl+T | Add a transition to the selected clips' nearest edges |
| F | Freeze the frame under the playhead |
| U / G | Keyframe lane of the selected clip / Graph editor |
| Enter / Shift+Enter on a diamond | Add or remove a keyframe / open its menu |
| J / K / L | Step back one second / Pause / Play, then double the speed up to 8× |
| Command/Ctrl+L | Toggle loop |
| Delete / Shift+Delete | Delete / Ripple delete |
| Command/Ctrl+scroll, pinch, or = / - | Timeline zoom about the pointer |
| Command/Ctrl+Plus / Minus / 0 | Interface size larger / smaller / 100% |
| Shift+Z | Fit timeline |
| Command/Ctrl+E | Export |
| Command/Ctrl+Shift+E | Render queue |
| Command/Ctrl+K | Open local command panel |
| Command/Ctrl+Shift+K | Open native AI Ask panel |
| Command/Ctrl+Comma | Preferences |

The panel's Command mode interprets supported editing phrases or raw command
JSON locally. Ask mode invokes an installed Claude or Codex CLI using its own
sign-in. Select the provider, model, effort or executable override in the panel
menu or Settings; credentials are not stored in Aster preferences or projects.
Windows Claude requires its native `claude.exe`, because batch launchers cannot
carry the editing schema.

An Ask request sends names, local source paths, text, timing and other project
metadata. It sends no source video/image/audio bytes. Review the summary,
commands and before/after changes before Apply; a changed proposal commits that
exact validated state as one undo step. A proposal already matching the project
shows "No changes" and preserves existing undo/redo. A later edit or reopening
makes the proposal
stale, and Cancel stops its owned request. Provider errors leave the project
unchanged. Scene understanding, transcription and beat detection are outside
this metadata planner; captions from speech are made locally (see
[captions from speech](#captions-from-speech)), and the planner sees caption
text, not sound. See [native AI editing](native-ai.md) for the runtime
contract and evidence.

External agents connect through the [authenticated agent protocol](agent-protocol.md),
with project, context and revision guards and the same Undo/Redo behavior.
