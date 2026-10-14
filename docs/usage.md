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

Everything in this section is a preview at preview resolution; there is no
export. It is drawn by a GPU compositor. If the window has no usable graphics
device the picture keeps position, scale and opacity only and the viewer shows
"Effects not shown". The [verification record](verification.md#advanced-editing)
says which of these were driven natively and on what host; none has a measured
playback rate.

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
a moment after it is added or changed. Like everything else in the viewer they
are a preview: there is no export, and captions leave the editor only as SRT
or VTT files. The [verification record](verification.md#titles-captions-and-motion-graphics)
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
and 64 MiB per frame. Hardware/zero-copy decoding, HDR, exact variable-frame-rate
PTS handling, proxy generation, waveforms and export are not current features.
Export was outside the user's first core-feature phase.

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
