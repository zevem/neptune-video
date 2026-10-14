# Video Editing

The project describes an editable audiovisual sequence and the media it uses.
Human editors and AI agents work with the same editing concepts.

## Projects and media

**Project**:
A document containing an editable audiovisual sequence, its media library and
canvas settings.
_Avoid_: Workspace, session

**Editing session**:
Work on a project together with its undo/redo history and saved-state boundary.
An interrupted editing session can be restored from recovery.

**Asset**:
A source video, image, animated image, audio item or colour lookup table in the
media library. One asset can be used by several clips without changing the
source media.
_Avoid_: Clip, footage item

**Media library**:
The project's collection of assets and their organization into folders.
_Avoid_: Timeline, bin when referring to the whole library

**Library folder**:
A collection used to organize assets inside a project, independent of the source
files' folders on disk.

**Source range**:
The interval of an asset used by a clip, from its source in-point to its source
out-point.

**Proxy**:
A lower-cost representation of an asset used for interactive editing while its
original source remains authoritative.

**Offline asset**:
An asset whose source media is currently unavailable. Its clips and editing
decisions remain part of the project.

## Timeline and editing

**Timeline**:
The ordered set of tracks that determines when project content appears and
sounds.
_Avoid_: Sequence when referring to the project's timeline

**Track**:
A layer of clips of a compatible kind: video, audio or text.
_Avoid_: Lane when referring to a persisted track

**Clip**:
A placed use of an asset or generated text on a track, with a timeline interval,
source range and editing properties.
_Avoid_: Asset, media file

**Playhead**:
The current timeline position used for preview and frame stepping.
_Avoid_: Cursor

**Marker**:
A named position on the timeline that helps navigate or align an edit.

**Trim**:
A change to a clip's boundary and source range without moving unrelated clips.

**Split**:
An edit dividing one clip at a timeline position into two clips that together
retain the original content.

**Ripple edit**:
An edit that closes or opens time and moves subsequent eligible clips by the
same amount.

**Linked clips**:
Clips that share an editing relationship, such as the picture and sound from
the same source, so selected operations preserve their alignment.

**Clip group**:
A set of clips selected and manipulated as one editing unit.
_Avoid_: Folder

**Snapping**:
Alignment of an edit to a nearby meaningful position, such as a playhead,
marker or clip boundary.

**Locked track**:
A track protected from editing until it is unlocked.

**Hidden track**:
A visual track excluded from preview composition while its clips remain in the
timeline.

**Muted track**:
A track whose sound is excluded from playback while its clips remain in the
timeline.

**Solo**:
A track switch that, while any video or audio track has it, leaves only the
soloed tracks heard. It is saved with the project and undone like mute; mute
wins over solo, and a text track has none.
_Avoid_: Preview-only or session state; solo is project data

**Track volume**:
A factor over the volume of every clip on a track, 0 to 800 percent, where
100 leaves the clips as they are.
_Avoid_: Gain alone, which is a clip's own volume

## Picture and playback

**Canvas**:
The project's picture dimensions, aspect ratio and background.
_Avoid_: Viewport when referring to the final picture

**Transform**:
A clip's position, scale, rotation, opacity and reflection within the canvas.

**Crop**:
The visible source region of a clip before it is placed on the canvas.

**Scrubbing**:
Moving the playhead interactively to inspect picture and sound at another time.

**Frame stepping**:
Moving the playhead to an adjacent project frame without continuous playback.

**Preview consumer**:
A transient owner of a stream of preview pictures, separate from the project.
Clips using the same asset can have independent consumers. A continuing clip
keeps its consumer, and a contiguous cut can transfer it to its successor.
_Avoid_: Track, asset or durable clip identifier when referring to this owner

**Preview retention hint**:
An optional transient subset of up to three preview consumers whose compatible
video decoder pipes are protected from ordinary eviction. It does not request
frames, make a result current or describe every active consumer. The caller
replaces or clears the subset when the desired owners change.

**Playback speed**:
The rate at which preview advances through the timeline. It does not itself
retime a clip's content.
_Avoid_: Clip speed, which is the clip's own rate or time map

## Looks and compositing

**Effect**:
One adjustable picture operation on a clip, such as a glow, a blur, a grade or
a chroma key: a kind, its parameters, a mix and an on/off switch. A clip's
effects form a stack applied in order, sixteen at most.
_Avoid_: Filter, plugin; audio effect, which works on a clip's sound

**Look preset**:
A named starting point (Cinematic, Teal & orange, Vintage, Faded, Noir, Warm,
Cool) that becomes ordinary effects at a chosen strength. A clip holds one look,
read from its effects; giving another replaces it.
_Avoid_: Filter when referring to the stored data, which is only effects

**LUT asset**:
A colour lookup table imported from a `.cube` file into the media library. It
is read by a LUT effect and is never placed on the timeline as a clip.

**Mask**:
A rectangle or ellipse, with feather and roundness, that limits where a clip
shows. A clip's masks combine in order by adding, subtracting or intersecting,
eight at most.

**Blend mode**:
How a clip's picture mixes with what is under it: normal, multiply, screen,
overlay, soft light, add, darken, lighten or difference.

**Transition**:
A timed change from one clip to the next, or from or to nothing, such as a
dissolve, a wipe or a slide. It belongs to one clip and one of that clip's
edges, and finds the clip it meets when the picture is evaluated.
_Avoid_: Effect when referring to a transition

**Edge**:
The side of a clip a transition is on: `in` at its start, `out` at its end. A
cut between two clips is the incoming clip's `in` edge; `out` applies only
where no clip follows.

**Chroma key**:
An effect that makes one colour of a clip transparent, with tolerance,
softness and spill control.

**Clean plate**:
A still of a scene without its subject. A difference matte effect compares
each frame with it and keeps what differs.
_Avoid_: Background when referring to the plate; the background is the clip
placed under the keyed one

**Picture-in-picture**:
A clip on a higher track shown smaller over another. It is an arrangement of
transform, crop, mask, blend and shadow, not a stored type; layout presets set
position, scale and crop.

**Layer**:
One clip as the canvas shows it at one time: its source frame, resolved
transform, blend mode, enabled effects, masks and transition progress. Layers
exist only in a render plan and are never saved.
_Avoid_: Track, clip

**Render plan**:
The layers of the canvas at one timeline time, bottom first, with the canvas
size and colour. It is computed from the project, the time and any drafts
alone, so equal plans mean there is nothing to redraw.

**Plain layer**:
A layer with normal blend and no effects, masks or transition, which the
window can draw directly as one quad.

**Compositor**:
The off-screen GPU renderer that draws the layers a quad cannot: it places
each layer, runs its effects and masks, blends it and publishes one canvas
picture to the window. Without a graphics device those layers are drawn plain
and the viewer says "Effects not shown".

## Animation and speed

**Keyframe**:
A value of one parameter at one time of its clip, with the easing of the
stretch that leaves it. Its time is counted from the clip's start, so moving
the clip does not change it.

**Animation track**:
The keyframes of one parameter of a clip, or of one effect or mask on it. While
a track exists it overrides the stored value; removing it returns to that value.
_Avoid_: Track alone, which is a timeline track; lane, which is where the
timeline shows it

**Easing**:
The shape of the change between two keyframes or across a transition: hold,
linear, or a cubic curve with two handles. Named easings (ease in, ease out,
ease in and out, overshoot) are fixed handle positions.

**Keyframe lane**:
The rows the timeline opens under a clip's track to show and edit its
animation tracks. A view, not project data.

**Graph editor**:
The timeline mode that shows the selected clip's animation tracks as curves
over time, with easing handles.

**Time map**:
The keys that say which source moment a clip shows at each moment of its own
time, when its speed is not one constant rate. In code and commands it is the
clip's `retime`.
_Avoid_: Playback speed, which is the preview clock's rate

**Retime**:
To change how a clip moves through its source: a constant rate, a reverse, a
freeze or a ramp. A retime changes the clip's length and moves its keyframes
so each stays on the source moment it was on.

**Freeze**:
A stretch of a time map that holds one source frame. The clip grows by the
length of the hold where the track has room.

**Ramp**:
A stretch of a time map whose speed changes smoothly from one rate to another.

**Reverse**:
A time map, or a stretch of one, whose source runs backwards.

## Motion analysis

**Tracker**:
A named path of one region through an asset's frames, keyed in source time and
stored in the project. It belongs to the asset, so it survives splitting,
trimming, moving and retiming of the clips that show it.
_Avoid_: Track, which is a timeline track

**Track region**:
The box drawn on the picture that a tracker follows from the playhead onwards.
It is a choice in progress, not project data. It is held in fractions of the
source frame: on a cropped, flipped or stabilized picture it is the place of
the frame that is drawn under it.

**Follow**:
A clip's instruction to take its position, and optionally its scale and
rotation, from a tracker, with an offset. It has effect while another clip
showing the tracker's asset is on screen, and the point is where that clip
draws that place of its picture.

**Stabilization**:
Smoothing of a shaky camera by moving each frame against the measured camera
motion and cropping in to hide the borders. The project stores the Stabilize
effect with its strength and crop zoom only.

**Stabilization data**:
The camera path measured through a whole video, derived from the media and
kept in a bounded cache outside the project. It can always be measured again.
_Avoid_: Tracker, which is an editing decision stored in the project

**Analysis lane**:
The bounded background worker that tracks regions and measures camera motion.
Its results name the project, request and media they were made for, and are
dropped when any of those has changed.
_Avoid_: Track, keyframe lane

**Subject segmentation**:
Automatic separation of a person or object from its background without a key
colour or clean plate. Not a current feature.

## Sound

**Audio effect**:
One adjustable operation on the sound of a clip on an audio track: a
high-pass, an equalizer band, a compressor, a limiter, noise reduction, pitch
or centre reduction. It sits in the clip's effect stack, at full mix and
without keyframes, and is heard through that clip's own decoder. It is not a
sound effect.
_Avoid_: Sound effect, filter, plugin

**Sound effect**:
An imported sound placed as a clip, such as a whoosh or a hit. It is media,
not an operation on a clip; no sound effects come with the editor.
_Avoid_: Audio effect, which is an operation on a clip's sound

**Fade**:
A length at a clip's start (fade in) or end (fade out) over which its sound
rises from or falls to silence on one equal-power curve. It stays at the
edge when the clip is trimmed or split, and the two together fit in the clip.
_Avoid_: Transition, which belongs to the picture; volume keyframes

**Waveform**:
The drawing of a clip's sound on its tile on an audio track, read from the
media's first sound stream and mixed to one channel. It shows the source, not
the clip's volume, fades or effects.

**Peak file**:
A media file's sound reduced to a hundred levels a second, derived from the
media and kept in a bounded cache outside the project. It feeds the waveform,
the ducking gate and beat finding, and can always be read again.

**Detached audio**:
A video's sound placed as a clip of its own on an audio track, linked to the
picture clip over the same span. It is the same media; no file is written.

**Owned sound**:
The sound of a picture clip that a linked clip of the same media on an audio
track plays instead. An owned picture clip is not heard, and offers no volume,
fade or audio controls of its own.

**Take**:
One voiceover recording: a WAV file written to the project's media folder
and placed as a clip in one history step. The file is the person's and is
never deleted by the editor.
_Avoid_: Clip, which is the take's placement on the timeline

**Media folder**:
The `<document stem> media` folder (the project file's name without `.aster`,
then ` media`) beside a saved project, made on the first take or separation.
It holds takes and stems and is never emptied by the editor.

**Beat grid**:
The beats found in a sound's peak file, held in memory while they are shown.
It is not saved; a marker made from a beat is.

**Stem**:
One part of a sound split by the external stem separator (Vocals or
Instruments), written as a WAV file to the media folder and placed as a clip
linked under the clip it came from.

**Ducking**:
Lowering a clip with written volume keyframes wherever other clips are heard,
found by signal energy, not by recognizing speech.

**Sound lane**:
The bounded background worker that reads peaks, measures loudness, finds
beats, writes a sound to a file and runs the stem separator. Its results name
the project, request and media they were made for.
_Avoid_: Analysis lane, which tracks and stabilizes pictures

## History and agents

**Editing command**:
A targeted editing operation that validates its preconditions and can be
reversed through project history.

**History entry**:
One completed editing operation presented as a single undo or redo step.

**Checkpoint**:
A named project state that the editor can return to during editing or recovery.

**Recovery snapshot**:
A saved editing session retained to restore work and undo/redo history after an
interruption.

**Document baseline**:
The ordinary project file as it was opened or last successfully saved by this
editing session. An intervening change to that file creates a save conflict.

**Recovery binding**:
The original save destination and document baseline retained with a recovery
snapshot. A recovery copy without a usable binding opens as unsaved work.

**Save conflict**:
A disagreement between an editing session's document baseline and the project
file now at its destination, requiring a deliberate choice before saving.

**Agent edit**:
An editing command requested by an AI agent and applied through the project's
normal validation and history rules.

**Captured editing context**:
The project state, selected clips and playhead captured when requesting an AI
edit proposal. Later selection or playhead changes do not retarget that request.

**Focused editing context**:
A declared incomplete view of a captured project that retains the selected
clips, their linked/grouped relatives and source limits.

**Edit proposal**:
A validated set of suggested editing commands and their before/after project
states, awaiting review and application as one history entry.
_Avoid_: Applied edit, generated project
