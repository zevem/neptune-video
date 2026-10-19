# Core editor guide

Neptune Video's current core is a native editing workspace. It keeps source files
unchanged and records durable edits through the same validated history used by
external agents. The [verification record](verification.md) identifies tested
flows and remaining platform/media gates.


AI agents in MCP clients such as Claude Code and Codex can edit the same
projects, live in the window or on their own: see [Neptune Video for MCP clients](mcp.md).
By default, Ask in the [Agent panel](#the-agent-panel) uses the same tools
inside the window.

## Projects and recovery

The project menu creates, opens, saves and duplicates `.nvp` documents.
Duplicate writes a new file with a fresh project identity and keeps the original
file unchanged; a duplicate cannot replace its own source path. Preferences
persist the window's library/inspector layout, appearance, interface size,
recent projects, snapping, autosave and library sort mode.

Interface size (Settings, or Command/Ctrl+Plus / Minus / 0) scales the whole
window from 50% to 300% in 10% steps. A size larger than leaves the editor its
720 × 520 point layout is held to the largest the window has room for: about
150% at 1280 × 800 and 200% at 1920 × 1080. The chosen size returns as the
window grows.

Autosave retains recoverable editing state. The ordinary `.nvp` file stores
the current project; its `.nvp.recovery` sidecar preserves undo/redo chains.
Opening the ordinary file starts fresh history; accepting recovery restores the
session's history. Unsaved projects have an isolated recovery destination under
the settings directory. Save and recovery errors remain visible.

Checkpoints are named project snapshots beside the project file. Save the
project first, then open Checkpoints from its menu. Restoring a checkpoint is
one reversible edit in the current project: Undo/Redo remains available, and
the project path and live context stay intact. Checkpoints from another project
identity are rejected.

An OS lock on `.nvp.lock` owns the recovery sidecar. A second editor does not
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
A row under the search lists All, Video, Sound or Images only, in whatever
folder and order are in view; a GIF is with the images and a LUT is under All
alone. The folders stay listed to be walked into, a search looks through
every folder within the kind, and a list left empty says there is nothing of
that kind there. The choice lasts for the session and is kept in no file.

Add as background music, in the context menu of a sound file in the library,
lays that sound under the whole project as one step of the history: a new
audio track named Music, placed last, with the sound end to end from the
start until the project's length is covered. Every clip is at 50% volume; the
last is cut at the project's end and fades out over 2 seconds, or over its
whole length when it is shorter. It is always a new track, so a second bed
does not write on the first. The bed is as long as the project is when it is
made and does not follow later edits; a repeat is a hard cut at the file's
ends, with no loop point sought. It is refused, with the reason, when nothing
is on the timeline, when the sound is shorter than a tenth of a second, when
covering the project would take more than 256 clips, and for media that is
not a sound file. A bed is one more clip sounding at every moment, and more
than sixteen clips sounding at once stop the mix with a message.
No sound or music comes with the editor.

Double-click an asset to add it to the timeline, or drag it to an appropriate
track/time. Video, audio and text tracks have no fixed model count limit.
Timeline rows and library entries are virtualized. Zoom, snapping and markers
help place edits; performance at very large project sizes still needs measured
acceptance.

Select clips to trim their handles, drag/move, split, copy/paste, duplicate or
delete. Ripple deletion/trimming shifts following eligible content. Linking
preserves editing relationships and grouping moves a selection together. Track
locks block clip edits; hide suppresses picture and mute suppresses sound.
Clipboard copies work between native Neptune Video windows and carry source references;
unrelated text on the OS clipboard does not silently create clips.

Detach audio, in a video clip's menu and under Clip in the project menu, puts
that clip's sound on an audio track as a clip of its own, linked to the
picture: the same media, span, speed, time map, volume and volume keyframes.
It uses the first unlocked audio track with room over the clip's span and adds
a track when there is none. It is one step of the history, writes no file and
reads only the first audio stream. The audio clip then owns the sound, and the
pair trims, splits, moves and is deleted together. Unlinking gives the picture
its sound back, so both are heard; set one clip's volume to 0 for a silent
picture. It is refused, with the reason, for a clip without sound, on a locked
track, whose sound is already on a linked audio clip, or that is linked to
another video clip of the same media: sound is owned by link and media, so
unlink such clips first and detach each for itself. The audio clip draws the
video's sound as a waveform, as every clip on an audio track does. Adding a
video to the timeline does not detach its audio by itself.

Save audio as…, in the context menu of a library file with sound, in a clip's
menu and under Clip in the project menu (for the one selected clip), writes
that sound to a WAV file you name in a save dialog; the name it offers is the
media's with `.wav`. A library file gives its whole sound; a clip gives the
span of its media it plays, from the lowest source time it reaches to the
highest, without its volume, fades, effects or speed. What is written is the
file's first sound stream as it is: 16-bit PCM at its own rate and channels.
It edits nothing and is no step of the history. The file is written beside
its target under the target's name with `.part` and renamed over it once it
is whole, so a file that was there is replaced only by a whole one; a failure
or Cancel removes the `.part` and leaves the target as it was, and a `.part`
that is already there is refused and left alone. A name without an extension
gets `.wav`; another extension, the media file itself or a file of the
project's media is refused, by whatever name the path reaches it (another
case, a link or `..`). While it is written a card at the top of the viewer
shows its progress with Cancel; once written the card offers Add to project,
an ordinary import, for twelve seconds. Saving the same span again stops the
earlier file and says so. Once a file is being put in place both come too
late: Cancel says so, its card stays, and its answer follows; a newer save of
the span is written beside it. Opening or creating another project does not
stop a file being written: it is still written and its notice says where, but
nothing is offered to the new project. A long source keeps the
sound lane, so waveforms and loudness measurements wait behind it.

A clip on an audio track draws its media's sound as a waveform under its
name, through the clip's trim, speed and time map: a reversed stretch is
mirrored and a held frame is flat. The sound is read once in the background
when a tile first comes into view and kept in a bounded cache outside the
project, so the next session draws it without reading the media again; until
it is read the tile shows a level line. The waveform is the source's first
audio stream mixed to one channel, a hundred points a second, so it is blocky
at the closest zoom; the waveform itself does not show the clip's volume (the
volume line over it does), a muted track still draws it, and clips on video
tracks and library tiles have none. Media longer
than 5.8 hours, or whose sound cannot be read, keeps the level line and says
so once.

A clip's sound can fade in from its start and out to its end. On an audio
track, a selected clip and the clip under the pointer carry a handle at each
top corner of their sound, under the name, on tiles at least 40 points wide:
drag one along the clip to set how long that edge fades. The fade grows or
shrinks by as far as the handle is dragged. The drag lands on
frames, or on a clip edge, marker or the playhead when snapping is on, stops
at the clip's other fade and at the clip, shows its length beside the handle
and is one step of the history when let go; Escape gives it up. The fade is
drawn over the waveform as the curve the sound follows. The inspector has
Fade in and Fade out fields in seconds beside Volume, for a clip on an audio
track and, in the Sound card of its Video section, for a video clip with
sound; a typed length longer than the room is held to the room. The clip menu
has Fade in 0.5 s, Fade out 0.5 s and Remove fades for the selected clips.
Each clip has its own fades: a linked clip's are not changed with it. A video
clip whose sound is on its linked audio clip shows "Sound is on the linked
audio clip" in place of its volume and fade rows, and its menu rows and the
command panel's phrases refuse it in the same words, because that clip's own
sound is not heard; fade the audio clip, or unlink the pair. A video-track
tile has no handles and draws no curve. The picture and the sound follow a
handle when it is let go, not while it is held.

A tile on an audio track also carries a line at its clip's volume. 100% stands
at two thirds of the height of the tile's sound and 200% at its top; a higher
volume is drawn at the top and read in the inspector. Select the clip and drag
the line up or down to set its volume: the line follows the pointer, the
volume is shown beside it and lands on a whole percent from 0 to 200, and
letting go is one step of the history; Escape gives the drag up. A 44-point
lane has about six percent to a point, so a finer volume is typed in the
inspector. A line let go where it was taken hold of keeps the volume the clip
had, so one above 200% is not turned down by lifting it, and a finer one is
not rounded. The line is taken hold of within four points of it, between the
grips that trim the clip, on tiles at least 40 points wide and on a track that
is not locked; a press anywhere else on the tile moves the clip as before.
Alt-click on the line, of a selected clip or not, writes a volume keyframe at
the frame under the pointer, at the volume the clip has there, so nothing
heard changes, and opens the clip's keyframe lane. A keyframe written between
two others divides their stretch as a split does: a held stretch stays held
and an eased one keeps its shape, with a further keyframe where the curve
needs one to do so. Once the volume has
keyframes the line is the curve they make, with a dot at each, and it is no
longer dragged as a whole: move the keyframes in the lane or the graph. "Key
volume" in the clip menu writes the same keyframe at the playhead, for a video
clip with sound too, and opens the lane; it is shown and cannot be chosen
while the playhead is off the clip, where the volume is keyed already, on a
locked track and on a video clip whose sound is on its linked audio clip. The
line is in percent, not decibels, it is on audio-track tiles only, and the
sound follows a drag when it is let go.

A track's head carries its switches. Every head has lock. A video or audio
track has mute and solo, the headphones: while any track is soloed only the
soloed tracks are heard, and the mute switch of each track that a solo
silences is drawn dimmer and says "silenced by a solo". Mute wins over solo.
A video or text track has hide. An audio head has its three switches side by
side; a video head has four, in two columns, hide under lock and solo under
mute, so that the track's name keeps its room. A text track has no sound and
neither mute nor solo. Each switch is one step of the history and works on a
locked track. Solo, like mute, is saved with the project, so a project opens
sounding as it was left.

A track's volume is in its head's menu (right-click the head): a field in
percent from 0 to 800, where 100 leaves the track's clips as loud as they
are. A typed volume is taken when it is entered and is one step, kept to a
hundredth of a percent; Escape gives it up. A drag on the field is one step
when it is let go and none when Escape ends it, and an Up or Down arrow in it
is a step each to the next whole percent. Reset volume returns to 100. A head
whose track is not at 100% says its volume under the track's name. The volume
is a factor over everything on the track; it is in percent, not decibels, and
there is no pan, meter or automation of it. The sound follows when the edit is
made, not while a field is dragged. The command panel takes
`solo track Audio 1`, `unsolo track Audio 1` and
`set track volume Audio 1 50%`; it refuses a text track and a volume outside
0% to 800% in words.

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
**Video**, **Look**, **Color**, **Speed** and **Motion**. A title has Video and
Motion; a clip on an audio track has one, **Audio**: its Playback card
(speed, Keep pitch, volume, fades and Normalize loudness), its Ducking card,
its [audio effects](#audio-effects) and its timing. A

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
keyframe lane: one row a parameter under the clip's track. A clip with sound
and no keyframe yet has a lane too, opened by U or the tool strip: one empty
Volume row, which "Key volume" in the clip menu, Alt-click on the tile's
volume line, the Volume diamond in the inspector or a paste of copied volume
keyframes fills. Its tile carries the chevron while the lane is open. A still,
a title, a silent video and a video clip whose sound is on its linked audio
clip have no such row; a clip that is muted, soloed out or at volume 0 keeps
it. G or "Graph editor"
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

Effects, masks and blend modes are for pictures on video tracks; titles take
none, and a clip on an audio track takes [audio effects](#audio-effects)
instead.

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
stretches and is silent where the clip is held, reversed or ramped; elsewhere
a picture clip's speed changes its pitch. Its row says "Detach audio to keep
pitch": a clip on an audio track can keep it (see
[Keep pitch and pitch shift](#keep-pitch-and-pitch-shift)). Stepping or playing
backwards through video reads frames in groups and is not smooth at every
size.

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
`reverse`, `freeze frame for 2s`, `detach audio`, `fade in 1s`,
`fade out 2s`, `remove fades`, `pitch up 2`, `pitch down 3`, `keep pitch`,
`enhance voice`, `reduce noise 60%`, `add eq`, `add compressor`,
`add limiter`,
`add keyframe opacity 50%`, `chroma key green`, `follow Tracker 1`,
`stop following` and `remove tracker Tracker 1`; and, of the library,
`add background music song.wav`, or without a name for the sound selected
there; of several media with that name it takes the sound a bed can be made
of. Of the sound, for the selected clips: `normalize`, `duck under speech`
(or `duck 18 db`), `show beats`, `hide beats`, `markers from beats` and
`separate stems`; of a track by its name: `solo track Audio 1`,
`unsolo track Audio 1` and `set track volume Audio 1 50%`. Starting a
tracking or a stabilization analysis has no phrase, and nothing records a
voiceover by phrase.

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
Neptune Video is started with `--settings`, otherwise in the platform's local data
directory (on Linux, `~/.local/share/neptune-video/models`). **Stop making captions**
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
editing data are sent; no sound is. The answer is always shown as a plan to
review, even when Ask is set to make edits as it works:
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
that budget stops the sound with a message that says how many clips sound and
where, for example "17 at 00:01:12". Muting or soloing tracks brings the count
down. For a linked video/audio pair using the same asset, the explicit
Audio-track clip owns the sound, including when its track is muted or soloed
out, so embedded video sound does not double it. Links between different
assets mix normally. A clip's speed changes its pitch unless the clip keeps
it (below); the preview speed always does.

None of the sound described from here on has been listened to: the
[verification record](verification.md#audio-editing) says what each feature's
tests read back and on which host. A clip's fades, its track's volume and solo
are heard in the preview. A fade
follows one equal-power curve. When any track is soloed only soloed tracks
sound, and mute wins over solo. A mix louder than full scale is turned down
by a limiter, where it was cut flat before. A mix that never passes full
scale plays unchanged; after a louder moment the sound stays turned down and
comes back up over about a second and a half, most of the way in the first
half second. The limiter is a safety
stage without lookahead: it works on sample peaks and can colour a hard
transient. Fades are set on the timeline, in the inspector, from the clip
menu and by phrase; a track's volume and solo are set in its head, by phrase
and by serialized commands.

The frame cache retains up to 64 MiB/256 frames and five software video decoders;
GIF sequential working state has a separate 64 MiB/four-state budget.
Preview is RGBA software decoding, with up to 4096 pixels per requested dimension
and 64 MiB per frame. Large videos can be previewed from smaller copies; see
[preview quality and proxies](#preview-quality-and-proxies). Hardware/zero-copy
decoding, HDR and exact variable-frame-rate PTS handling are not current
features.

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
`.neptune-video-render-<job>`, beside the destination, or in the system's temporary
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
- Sound is mono or stereo, mixed by the playback mixer, so fades, track
  volume and solo, audio effects, Keep pitch and the limiter sound as they do
  in the preview. At most 16 clips sound at once (more fails the export), a
  picture clip's speed change alters its pitch, held, reversed and ramped
  stretches are silent, a transition does not crossfade the sound, and a
  hidden video track still sounds unless it is muted. An audio effect this
  FFmpeg cannot run is left out, as in the preview, and the job's message
  says so.
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
  [agent protocol](agent-protocol.md#render-jobs)); the headless `neptune-video-agent`
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
beside the settings file when Neptune Video is started with `--settings`), not in the
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

### Keep pitch and pitch shift

A clip on an audio track has a **Keep pitch** switch under its speed in the
inspector. On, the clip still plays at its speed and its pitch stays where it
was recorded; off, speed changes pitch as it always has. The switch is a
`Pitch` audio effect at 0 semitones, one undo step each way. `pitch up 2` and
`pitch down 3` in the command panel shift the selected audio clip by that
many semitones (-12 to 12) from its own pitch, and its speed then leaves it
there; said twice, the shift is the same. A shifted pitch holds the switch on
until the shift is back at 0. `keep pitch` is the switch by phrase, and an
agent writes the same `pitch` effect with `set_effects`.

Pitch is kept while the clip's speed divided by the pitch ratio is between
0.5 and 2: from 0.5× to 2× with no shift, from 1× to 4× an octave up. Past
that the clip still plays at its speed and the remainder is heard as a change
of pitch (4× keeping pitch plays an octave up), and the inspector says the
speeds that keep it. Keeping or shifting pitch makes the sound arrive
slightly before the picture: in the tests on one Mac with FFmpeg 8.1.2, by 8
to 19 ms at 1.5× and 2×, about 28 ms at 0.84× and up to 59 ms at 0.5×. This
is not corrected. Formants are not preserved, and
how the stretched sound sounds has not been judged by ear. A picture clip
keeps no pitch until its sound is detached to an audio track. Held, reversed
and ramped stretches stay silent.

### Audio effects

A clip on an audio track takes audio effects: a high-pass, equalizer bands,
a compressor, a limiter, noise reduction and centre reduction (and the pitch
of [Keep pitch](#keep-pitch-and-pitch-shift)). They are FFmpeg filters in the
clip's own decoder and are heard in the preview; nothing here has been judged
by ear. The Audio section lists them under the Playback card in the order the
sound goes through them, each with a switch, its values, a reset for a value
off its default, Move up, Move down and Remove. Frequencies are in Hz and
dragged on a logarithmic scale, so a drag moves as many octaves at 100 Hz
as at 10 kHz; levels are in dB, a compressor's ratio as n:1 with
its make-up gain, a limiter's ceiling, and noise reduction's amount and
noise floor; reduction works only with the floor set at or above the
recording's own noise. A value is heard once it is let go, when the sound
restarts, and no value takes keyframes. **Add audio effect** offers two
presets first: **Voice** (a high-pass at 80 Hz, noise reduction at its
defaults, +3 dB at 3.5 kHz one octave wide and a compressor) and
**3-band EQ** (flat bands at 120 Hz, 1 kHz and 8 kHz). Each is one undo step
of ordinary effects; nothing remembers that a preset made them, so a second
Voice adds four more. A clip holds sixteen effects, and a preset that would
pass that is refused with the room there is. A kind the session's FFmpeg
does not run can still be added, by any route; its card says "Not available
in this FFmpeg build", its values are disabled and it is left out of the
sound. The Effects tab of the library lists the same kinds under **Audio**:
a tile goes to the selected clips on audio tracks, or to the one it is
dropped on, and a picture clip is told to detach its audio first, or, once
it is detached, that its sound is on the linked audio clip. `enhance voice`, `add eq` (the 3-band EQ), `add compressor`,
`add limiter`, `add effect high-pass`, `remove effect compressor` and
`reduce noise 60%` (which sets the noise reduction a clip has, or adds one)
do the same for the selected clips. A picture clip's sound takes none until
it is detached.

### Normalize loudness

**Normalize loudness** sets a clip's volume so that the part of its media it
plays reaches -14 LUFS, without its true peak passing -1 dBTP. It is in the
clip menu for the selected clips, as a button under the fades in the Playback
card of the Audio section and in the Sound card of a video clip, and is
`normalize` in the command panel. The sound is measured in the background,
through the clip's audio effects and summed to two channels as a stereo output
plays it; nothing changes until the figures come, and then each clip is one
step of the history. The notice says what was measured and the volume set,
and when the peak, or the most a clip's volume is (800%), kept it short of
-14 LUFS. A clip whose volume has keyframes keeps them: they are scaled so
the loudest is at that volume and every dip keeps its depth. A silent span,
or one under about 0.4 s or too quiet to measure, is said so and nothing
changes. A clip that is removed, trimmed or given other audio effects while
it is measured is not changed, and the notice says to normalize it again;
a picture effect changes nothing measured. It is refused, in words, for a
clip without sound, a video clip whose sound is on its linked audio clip, a
clip that is not heard (its track muted or soloed out, or a volume of 0%)
and a clip on a locked track: before anything is measured, and again when
the figures come, so a video clip whose audio is detached meanwhile is not
changed. This is the loudness of one clip, not of the mix, and no
broadcast or streaming standard is claimed; a time-mapped clip is measured
over all the source it shows, at its speed of 1, without its pitch. At most
64 clips wait to be measured at once. An agent sets a volume with `SetGain`
or `SetKeyframes`; it cannot start a measurement.

### Duck under speech

**Duck under speech** lowers the selected clips wherever the other clips are
heard, as ordinary volume keyframes that can be seen and edited on the clip's
volume line. It is in the clip menu of a clip on an audio track, as a button
with a **Depth** field (in dB, 1 to 60) in the Ducking card of the Audio
section, and is `duck under speech` or `duck` in the command panel, with an
optional depth such as `duck 18 db`. The menu ducks at the default depth.
Each dip goes down to the depth under the clip's level (12 dB by default)
over 150 ms before the sound and comes back over 400 ms after it; the level
is the clip's volume, or its loudest volume key when it has keys.

The other clips are the ones that are heard by the mix and not selected (a
selected clip that is passed over, on a locked track for one, does not count
either): a muted or soloed-out track, a video clip whose sound is on its linked audio
clip and media without sound do not count, and a clip at a volume of 0% with
volume keys does. Their sound is read from the waveforms, one media file at
a time, in 10 ms steps where the clip plays forward (not inside a freeze,
reverse or speed ramp, which the mix keeps silent). A step louder than
-35 dBFS is sound; stretches under 200 ms are dropped, 300 ms is added after
each, and gaps under 1 s are bridged. Nothing is edited while this is read:
the card shows which file is read and offers **Stop ducking**, and Escape or
any change to the project stops it, with nothing changed. A media file whose
waveform cannot be read is left out and named in the notice; the rest still
duck. When every file is read, every ducked clip is written in one step of
the history, "Duck under speech", which Undo takes back whole.

A clip with volume keys is asked about first ("Replace this clip's volume
keyframes?"): its keys are replaced, not added to. It is refused, in words
and before anything is read, for a clip without sound, a video clip whose
sound is on its linked audio clip, a clip that is not heard, a clip on a
locked track, and one whose volume keys are all at 0%. When no other clip is
heard under it, or none of them is loud enough, nothing is written and the
notice says so.

This is an energy gate, not speech detection: music, effects or noise on the
other clips trigger it, and speech quieter than -35 dBFS does not. It reads
the source's level, not the other clip's volume, fades or effects, so a
voice held at 0% by its keys still makes a dip. The duck is written once and
does not follow clips moved or trimmed later: run it again. A clip holds at
most 256 volume keys, about 64 separate dips; beyond that the closest dips
are merged and the notice says how many. An agent can write the keyframes
itself with `SetKeyframes`; it cannot run the gate.

### Beats

**Show beats**, in the clip menu of a clip on an audio track and as `show
beats` in the command panel, finds the beats of that clip's media and draws a
short tick at each at the foot of every tile of that media on an audio
track; **Hide beats** and `hide beats` take them away. Whether a media
file's beats are shown is kept for the session and in no file; a project
opened again shows none. The beats are found in the waveform's peaks once
they are read, on the thread that reads the waveform cache; nothing is shown
while they are found. A sound with no clear steady beat (silence, a held
tone, steady noise) says "No clear beat was found" and shows none.

A tick stands where the clip plays the beat's source time, so a clip at
twice the speed ticks twice as often and a trimmed clip ticks only over what
it plays. Ticks are only where the clip is heard: none on a muted or
soloed-out track or at a volume of 0%, and none inside a freeze, a reverse
or a speed ramp. Ticks closer than 4 points to another are not drawn; zoom in
to see them.

While snapping is on, the beats of the tiles in view are places a moved
clip's edge, a trim, a fade's handle, a transition's handle and the playhead
land on, as clip edges and markers are, with the same guide. A clip being
moved does not land on its own beats, which move with it. Beats out of view,
hidden or not drawn are not landed on.

**Markers from beats**, in the clip menu while that clip's beats are shown,
and `markers from beats` for one selected clip on an audio track, puts a
marker labelled "Beat" on each beat the clip's tile shows over the whole
clip, as one step of the history, "Markers from beats". When there are more
than 256 it marks every second beat, then every fourth, and so on, so that
at most 256 are written. A beat that already has a marker is passed over;
when every one has, nothing is written and the notice says so. Markers are
the only beats a project keeps: an agent and the headless CLI see them, and
neither sees nor finds the beats themselves.

Beats are found by an energy onset detector: music with clear percussion at
a steady tempo is what it reads. A tempo may be found at half or twice the
one felt, and above about 170 BPM it is usually half. There are no
downbeats, bars or time signature, the grid cannot be nudged, and nothing
cuts picture to the beat: landing edits on beats is all of it. No accuracy
is claimed on real music.

### Separate stems

Separate stems, in the menu of a clip that is heard and as "separate stems"
in the command panel (for the one selected clip), splits the whole sound of
the clip's media into two stems, Vocals and Instruments, with Demucs, a stem
separator you install yourself (see [installation](installation.md#stem-separator)).
Neptune Video ships no separator, no model and no runtime, and downloads none;
whatever the tool fetches to run, such as its model on first use, is the
tool's own doing. Without one the row is shown without being able to be
chosen and says "Needs a stem separator; see the documentation"; while the
session is still looking for it, "Looking for a stem separator…". A clip
without sound, one that is not heard (muted, soloed out, at 0%), a picture
whose sound its linked audio clip owns and a clip on a locked track are
refused with the reason, and so is a project without a file: "Save the
project to separate stems".

The stems are written as WAV files to the `<project name> media` folder
beside the project file, named after the media, `song (Vocals).wav` and
`song (Instruments).wav`, with a number added when a name is taken: nothing
there is ever replaced, and the editor never moves or deletes them, also not
on undo. While Demucs runs a card at the top of the viewer shows how far it
says it is, or "…" until it says, with Cancel; Cancel, a failure or a newer
separation of the same media removes the folder the tool was writing into
and keeps no stem. When the stems come, and the clip is still there with
its media unchanged, they are placed as one step, "Separate stems": the two
files are added to the library, each on a new audio track named after it,
with the volume and solo of the original's track, as a clip of the original
clip's span, speed, time map, volume, volume keyframes, fades and audio
effects, linked to the original, and the original is turned down to 0%
without its volume keyframes, so the stems are heard once and not over it. Undo brings the original's volume back and takes the
stems off the timeline and out of the library; their files stay. Because the
whole file is separated, a later trim of the linked clips still finds sound
in the stems. A stem a little shorter than the part the clip plays cuts a
clip of one speed short and is refused for a clip with a time map. Stems
whose clip was deleted, whose media changed, whose clip is no longer heard
(its track muted, or its sound detached to a linked audio clip), or whose
project was closed meanwhile are not placed; the notice names their files.

Separation can be slow: its time is the tool's, its model's and the
machine's (see [performance](performance.md#stem-separation) for the few
runs measured, on tones and not on music). It holds the sound lane, so waveforms,
loudness and beats wait behind it, and it may run for at most fifteen
minutes plus four times the media's length. Opening another project does not
stop it. Each stem is one more sound playing where the clip plays, within the
sixteen-voice bound. Quality is the tool's; nothing was listened to. An agent
and the headless CLI cannot start a separation; the step that places the
stems is ordinary commands they can read and undo.

### Voiceover

The red disc before the timeline's speed control, or R, records a take from
the default microphone while the timeline plays from its playhead at 1×. A
project needs a file first: an unsaved one says "Save the project to record"
and opens nothing. Takes are written as WAV files to the `<project name>
media` folder beside the project file, made on the first take; the editor
never moves or deletes them, also not on undo. While the microphone opens the
disc dims; once the take runs, a stop square with the time recorded and a
small level bar takes the place of record and speed. R, Space, Escape or a
press on it ends the take; the play button, K, L, a double-click on the
picture and the panel's "play" and "pause" do not, and say so. A timeline that
was playing when record was pressed is paused, and plays on from there at 1×
once the microphone runs. "No microphone", a microphone the system does not
let Neptune Video use and a device that will not open are said in words and nothing
plays.

A take is as long as the person speaks. The timeline stops at its end as it
always does and the playhead holds there, but the take goes on, in silence
past the last clip and from 0 in an empty project; loop is not applied while
recording. The playhead cannot be moved and the speed is not changed until the
take ends; edits are allowed. The take is placed where it was heard: the sound
device's first position after the microphone started is paired with the
frames recorded by then, the clip starts at that position and what was
recorded before it is trimmed off the clip's start and stays in the file.
Without a device position in the first second, or with nothing playing, the
transport's position is used instead. The clip goes on the first unlocked
audio track with room over its span, looked for when the take ends, or on a
new "Voiceover" track, and the media, the track and the clip are one step of
the history, "Record voiceover". A take whose placed part is shorter than a
tenth of a second, and one whose project was closed or replaced meanwhile,
is kept on disk and not placed; the notice names its file. Sound the
recorder fell behind on and a take that ended by itself (a device error, a
full disk, the two-hour limit) are said.

The timeline's own sound plays while recording; without headphones it is
recorded too. There is no device choice, monitoring, punch-in, takes,
countdown or echo cancellation, and the device round trip (output and input
latency) is not measured or compensated, so a take may sit a little late.
An agent and the headless CLI cannot start or stop a recording; the step
that places a take is ordinary commands they can read and undo.

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
| R | Record a voiceover take / end it |
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

## The Agent panel

The Agent panel has two modes. **Command** interprets supported editing phrases
or raw command JSON locally; no model is asked. **Ask** sends your request to
the Claude Code or Codex command line installed on this computer, using its own
sign-in. Select the provider, model, effort or executable override in the panel
menu or **Settings › Agent**; credentials are not stored in Neptune Video preferences
or projects. On Windows, Neptune Video refuses a Claude `.cmd` or `.bat` launcher and
needs the native `claude.exe`.

Ask edits in one of two ways, chosen under **Edits** in **Settings › Agent**
(**As it works** or **For review**) or in the **Edits** section of the Ask menu
(**Made as it works** or **Proposed for review**). Made as it works is the
default. [Caption translation](#translating-captions) is always proposed for
review, whichever is chosen. [ADR 0011](adr/0011-the-agent-tab-edits-through-neptune-video-mcp-started-in-its-window.md)
records why.

### Made as it works

Ask starts a conversation with the provider whose only tools are Neptune Video's MCP
server, `neptune-video-mcp`: the same program and the same 39 tools that
[MCP clients](mcp.md) use, started in this window. Every edit to this window's
project goes through the window's agent session, with the same validation and
undo history as your own edits, and appears in the window at once.

- **Steps, live.** Each tool call shows as a step while it runs, then a check
  and the edit's name, or the reason it was refused. The agent's replies show
  between the steps, with bold, code, headings and bullets. A working row
  reads, for example, "Working… 15s · 2 changes", with **Cancel**.
- **Follow-ups.** After a request the field reads "Reply, or ask for more". A
  follow-up continues the same conversation, so "undo the colour you just
  picked" refers to the last request. Each request also tells the agent where
  the playhead is and which clips are selected.
- **Stop.** While a request runs, the Ask button becomes **Stop the request**;
  it and **Cancel** both stop the provider at once. Stop is not a graceful
  interrupt: it ends the conversation, and the next request starts a new one.
  Changes already made stay.
- **New conversation**, in the Ask menu, ends the conversation; the panel says
  "New conversation. The agent remembers nothing of the last one." A
  conversation also ends when you open or create another project in the
  window, on a failure, after 12 hours, or after a request goes 10 minutes
  without a word from the provider.
- **Restart note.** If the provider, executable, model or effort changed since
  the last request, the next request starts a new conversation and the panel
  says "A new conversation started, because the agent's provider, model or
  effort changed. It remembers nothing of the last one." A change to the
  folders the agent may use does not start a new conversation.
- **Notes after a request.** A finished request ends with the provider, the
  model and the count, such as "Codex · gpt-6.1-sol · 2 changes; undo and redo
  take them back", "1 change, which undo or redo takes back" or "no changes".
  A change is an edit or the agent's own undo. A stopped request ends with
  "Stopped. Nothing was changed." or "Stopped. The 2 changes already made
  stay; undo and redo take them back." Changes in projects this window does
  not show are counted apart, such as "· 1 change in another project"; with
  none here the note reads "no changes here" or "Stopped. Nothing was changed
  here." This window's undo does not reach them.

The agent starts on the project open in its window. Like any MCP client, it
can also create, open, save and close projects: a project no window shows is
edited by the agent's server itself and saved after every edit, and the agent
can show it in a new window. When the agent attaches without naming a window,
it comes back to this one. Neptune Video's instructions tell it to say so when it
leaves your project, and to come back unless you ask otherwise. It can save
this project, or save a copy as a new file.

It can read and import files, and save projects, exports and pictures, inside
these folders: the project file's folder, the folder of each imported file,
your Videos (Movies on macOS), Desktop, Documents, Downloads, Music and
Pictures folders, and the folders you add in **Settings › Agent**. Folders
that are too wide, such as your home folder, a drive or a system folder, are
left out. A file elsewhere is refused until you import it yourself or add its
folder. The folders follow the project while the conversation goes on:
importing a file from a new folder, or adding or removing a folder, takes
effect for the agent's later tool calls, without a new conversation.

To add a folder, choose **Add folder…** in the **Folders** row of
**Settings › Agent** and pick it in the folder dialog. Each added folder gets
a row with its name; hover shows the full path, and **Remove** takes it out.
A folder that is too wide is not added, and Neptune Video says "… is too wide for the
agent; choose a folder inside it".

The agent cannot run shell commands, browse the web or use other MCP servers
or tools.

Each of its edits is one step in the window's history, shared with yours.
Command/Ctrl+Z undoes the agent's last edit like your own, and Redo brings it
back. Neptune Video's instructions tell the agent to undo only its own edits unless
you ask; after it leaves this window and comes back, its earlier edits here
count as yours to its undo. While you are in the middle of a gesture or a
dialog is open, its edit is refused, and the agent is told to wait and try
again. An MCP client and the panel's agent can edit the same window at once;
the window applies one request at a time.

Exports the agent starts in this window run in the window's
[render queue](#the-render-queue) like your own, where you can cancel them.
Stopping the request does not cancel an export already queued there: one
queued before Stop finished in the queue.

Unlike a proposal for review, the agent sees what its tools return: rendered
frames and contact sheets as images, transcripts, level and loudness
measurements, file names, paths and other project data inside the folders
above, and the playhead and selection note. Neptune Video does not upload the source
media files themselves. The agent cannot listen to sound; it judges audio
from levels and loudness measurements. Your words reach the provider on its
standard input. Neptune Video's own instructions to the agent are on the provider's
command line, where other programs on this computer can read them; they hold
no project data. Anthropic or OpenAI receive this under your own sign-in and
terms, and sign-in, model access and usage limits are yours.

Made as it works needs:

- Claude Code or the Codex CLI, installed and signed in. Claude Code 2.1.300
  and codex-cli 0.162.1 were checked; other versions are unverified. Codex
  reaches MCP tools through its `code_mode_host` feature, which Neptune Video leaves
  on for the conversation while it turns Codex's other features off.
- `neptune-video-mcp` beside the `neptune-video` executable, or the `NEPTUNE_VIDEO_MCP` environment
  variable naming it. Without it the request fails and names the missing
  file.

This mode was run on one macOS host with debug builds, with Claude and Codex,
in wide and 820 by 640 point windows: live steps, follow-ups, Stop, the
person's undo of an agent edit, an agent export checked with ffprobe, and the
restart note. The [captures](../.scratch/in-app-agent/captures/) show those
states. Leaving the window for projects of the agent's own, coming back,
and folders added while a conversation goes on were run on 2026-10-15 with
release builds on the same macOS host;
[verification](verification.md#in-app-agent) and the
[run notes](../.scratch/in-app-agent/evidence/native-runs-2026-10-15.md)
record them. Linux and Windows windows are not verified.

### Proposed for review

An Ask request for review sends names, local source paths, text, timing and
other project metadata. It sends no source video/image/audio bytes. Review the
summary, commands and before/after changes before Apply; a changed proposal
commits that exact validated state as one undo step. A proposal already
matching the project shows "No changes" and preserves existing undo/redo. A
later edit or reopening makes the proposal stale, and Cancel stops its owned
request. Provider errors leave the project unchanged. Scene understanding,
transcription and beat detection are outside this metadata planner: it reads
markers made from beats, not the beats, and cannot start a measurement, a
duck, a recording or a separation. Captions from speech are made locally (see
[captions from speech](#captions-from-speech)), and the planner sees caption
text, not sound. See [native AI editing](native-ai.md) for the runtime
contract and evidence.

External agents connect through the [authenticated agent protocol](agent-protocol.md),
with project, context and revision guards and the same Undo/Redo behavior.
