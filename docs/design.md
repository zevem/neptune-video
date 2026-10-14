# Interface direction

The editor should feel like a focused native creative tool: picture and
timeline dominate, with restrained chrome and one distinctive accent. Neptune
is a reference for spacing, native rendering and attention to state, rather
than an interface to copy.

## Surfaces and controls

Use a compact media library, a central preview, a contextual inspector and a
timeline that keeps clip boundaries readable. Make panels resizable and
collapsible. At narrow sizes, reduce simultaneous panels before compressing
interactive targets: a window too narrow for both side panels shows the one
asked for last, never neither. Empty projects offer the real import action; sample media
is an explicit demo action rather than pretending to be the person's project.

Use rounded, optically aligned vector icons for familiar actions. Every
icon-only control has an accessible name, tooltip and keyboard equivalent where
appropriate. Keep text for meaningful names, values, errors and unfamiliar
operations. Avoid decorative labels, promotional copy and repeated headings
inside the editor.

Use the pinned [Tabler icon subset](../assets/icons/tabler/README.md) for UI
glyphs, including dropdowns, layout presets and window controls. Outline is the
default; transport controls and the on-key diamond use Tabler's filled style.
Keep the library's optical spacing inside its 24-point grid and recolor glyphs
for state. Timeline marks, curve handles and effect previews describe editing
content and retain their own geometry.

Distinguish hover, focus, selection, disabled, pressed and busy states using a
consistent surface and contrast hierarchy. Track lock, visibility, mute and
solo controls show their actual state: a soloed track's headphones take the
accent, and the mute control of a track that another's solo silences is
dimmer than one that is only off and says why. A head keeps 22-point targets
and its track's name whole: three controls stand side by side, four in two
columns, never four in a row of a 156-point head. A track's volume is a
percent field in its head's menu, and a head says a volume that is not 100%
under the name. Focused fields own text input and IME;
timeline shortcuts resume after editing finishes. An Escape closes the
innermost active interaction once, without also triggering an underlying edit.

## Editing feedback

Clip dragging, snapping, trim handles and the playhead should produce immediate
local feedback, then one committed history entry when the gesture ends.
Gesture cancellation restores its initial state. Show snap guides briefly at
the actual target. Treat linked/grouped movement as a visible editing unit.

Preview and decoding states are honest: distinguish waiting, failed, offline
media and a frame actually decoded from the source. Loading and save state
fit into quiet existing surfaces. Persistent failures remain discoverable;
brief success feedback does not compete with playback.

The timeline's transport has a record control just before speed, at every
width: a red disc at rest (R), dimmed while the microphone opens and grey
while a take is being closed. A running take shows a red stop square where
the disc was and, in speed's place, the whole seconds recorded over a small
level bar, red within 1 dB of full scale; the pair is one control that ends
the take, and speed is not offered until it ends. The play button does not
end a take; pressed during one it says to stop recording first. The source monitor records
nothing and has no such control. The window is drawn ten times a second only
while a take runs.

Transforms and canvas settings use concise numeric fields with sensible
increments and reset controls. Controls reflect the selected clip and retain
their target if asynchronous work or focus changes before completion. A multi-
selection's mixed values remain distinguishable from ordinary defaults.

## Looks, animation and motion surfaces

The clip inspector is sectioned: Video, Look, Color, Speed and Motion for a
picture, Video and Motion for a title, and one, Audio, for a clip on an audio
track: its Playback card, then a card for each audio effect in the order the
sound goes through them. The clip's name and the section control stay above
the scrolled rows; one section has no control. Rows for an effect, a mask or a
transition are generated from the core's parameter tables, so a new parameter
needs no panel of its own; an audio effect's rows carry no diamond, because
its values take no keyframes, and its levels read in dB and its frequencies
in Hz, dragged on a logarithmic scale. A kind the session's FFmpeg does not run
keeps its card, says "Not available in this FFmpeg build" and disables its
rows; until the probe has answered, every kind is shown as available. The
library has Media, Effects, Transitions and Looks tabs; a tile is given by
double-click or Return to the selected clips and by drag to the clip under
it. The Media tab's kind filter (All, Video, Sound, Images) is one 22-point
row under the search, drawn as the tabs are, and an empty list says what it
lacks and where. The Effects tab's last heading, Audio, lists the audio kinds, which go
only to clips on audio tracks; a picture is told in words why it takes none.
"Audio effect" names these filters everywhere in the interface; "sound
effect" means an imported sound clip and names nothing here.

picture, Video, Text, Animate and Motion for a title, Video, Graphic, Animate
and Motion for a graphic, none for sound. The clip's name and the section
control stay above the scrolled rows. Rows for an effect, a mask or a
transition are generated from the core's parameter tables, so a new parameter
needs no panel of its own. The library has Media, Text, Captions, Effects,
Transitions and Looks tabs; a tile is given by double-click or Return to the
selected clips and by drag to the clip under it. When the tab names would
touch they shrink, then become icons named on hover.

Text tiles are drawn by the title lane at their own size, so a tile shows the
real font, outline and colour emoji, and it plays its entrance only while the
pointer rests on it; an untouched tab asks for no repaints. Outline, shadow
and background are switch cards whose rows appear when switched on, and
lengths kept in ems are shown in pixels at the title's size. The Captions tab
is a list first: a caption's words and times are edited where they are shown,
and the row follows the playhead except while it is being typed into. Speech
and translation say where the work happens: the speech panel says nothing is
uploaded, and an AI action is marked "AI" and opens a plan for review rather
than editing.

An animatable row carries a diamond: hollow with no keyframe at the playhead,
filled on one, and filled for a paired row when either value is keyed there.
The row keeps its full label when keyed, and Tab moves through its controls in
reading order. The diamond's menu opens from the keyboard with Shift+Enter,
takes focus on its first choice and returns focus when it closes. A diamond
pressed with the playhead off the clip explains itself and edits nothing; an
easing that could have no effect (on a track's last keyframe) is not offered.

The timeline carries animation where the clip is. The keyframe lane opens
under the clip's track; the graph editor takes the tracks' place for one
selected clip. A transition is a marking on its clip's edge, as wide as it
lasts and hit where it is drawn, with a handle clear of both trim grips. A
retimed clip says so in a badge on its tile, and the name gives way to it.
A tile on an audio lane draws its sound under its name, in the audio tone and
as one shape a point of width a column; a tile on a video lane draws its
poster there and no sound. A waveform never waits for the frame: until the
sound is read the tile carries a level line where silence would be drawn, and
no spinner or progress.
A fade of a clip's sound is drawn over that sound as the curve it follows,
with what it takes away darkened above the line. Its handle is a dot at the
top corner of the sound, under the name, where the fade ends: shown on a
selected tile and on the one under the pointer, kept clear of both trim grips
and of the other handle as a transition's is, and absent from a tile narrower
than 40 points. A held handle says its length beside itself. A picture clip
whose sound its linked audio clip owns shows one line, "Sound is on the
linked audio clip", where its volume and fades would be: a control that
changes nothing heard is not offered.
A sound tile carries one more line, thin and in the ink of the fade curves,
at its clip's volume: 100% at two thirds of the sound's height and 200% at
its top, so there is room to raise it, level without keyframes and bent by
them with a dot at each. It is taken hold of only on a selected tile, so a
press on an unselected clip is always the clip's; held, it takes the accent
and says its percent beside the point taken hold of, inside the tile and
clear of the grips and of both fades' curves and handles; over a fade it is
said just past that fade, and where the fades leave no room between them it
is said at the trailing end of the name's row, which gives way to it,
wholly where not even its ellipsis would fit beside the percent. A
keyed line is a reading and is not dragged as a whole. The keyframe lane of
a clip with sound and no keyframe is one empty Volume row and no other
chrome: the row is where the first key goes.
A tile whose media's beats are shown carries a tick a point wide and seven
tall at the foot of its sound at each beat where it is heard, in the ink of
the fade curves; ticks closer than four points to another are left out
rather than drawn as a smear, and a snap to a beat shows the ordinary guide.
Every keyframe in view, every marking handle and every curve point is a named
widget, so each pointer gesture has a keyboard route: arrows nudge, Delete
removes, and the clip, timeline or library menu adds.

Choosing on the canvas is a visible mode with one way out. The key-colour
eyedropper, the track region and a mask outline are drawn on the picture only
while their inspector section is shown, and Escape ends the eyedropper or
gives up an untracked region before it touches the selection. Escape order
elsewhere is the same: a held drag, then selected keyframes or a selected
marking, then the clip.

Say what the picture is not. "Effects not shown" names a layer drawn without
its effects and why; a stabilized clip without an analysis is shown as the
camera took it and the section says so; a refused speed curve keeps the last
valid one and says why; background removal states that there is no automatic
subject cut-out. Background work (tracking, analysis, a duck being read, a
sound being saved, a separation) shows progress with a named cancel and never
blocks a gesture; a separator that has not yet said how far it is shows "…"
and a bar without a value rather than an invented figure, and the window does
not animate for it. A loudness measurement changes nothing until its figures
come and then says what it measured and set. A control that depends on a
host tool (an FFmpeg filter, the stem separator) is shown and says why it
cannot be used, rather than being hidden. A speed graph or value axis holds
still while a point is held.

Known departures: clip tiles are not individually focusable (selection by
keyboard is C and the Alt arrows, with a focus ring and scroll into view);
track-head buttons show a focus ring but the head itself has no Enter action;
global shortcuts are read before the interface, so Space on a focused button
also toggles playback; a mask is edited from its rows and not dragged on the
canvas; at 760x560 the inspector shows about 105 points of rows, less than
the curve editor is tall.

## Motion and performance

Use short motion only when it clarifies a state or spatial relationship. Respect
reduced motion. Paused projects need no ambient animation or continuous repaint.
Large timelines draw visible rows and intervals; media grids virtualize entries.
GPU backing alone does not establish interaction quality.

## Visual acceptance

Review screenshots of the running final native build, then correct clipping,
contrast, alignment, focus and cramped interactions. Cover an empty project,
an imported asset, a selected/trimmed clip, track controls, settings, history,
offline media and an error, at normal and narrow window widths. For looks,
animation and motion, cover each inspector section, the keyframe lane, the
graph editor, a transition marking, each library tab and a background job in
progress at 1440x900 and 760x560; for sound, cover the waveform and its level
line before the peaks arrive, fade handles at rest and held, the track head's
solo and volume menu, the volume line and the empty Volume row, the Audio
section with a preset and an unavailable kind, beat ticks and a snap to a
beat, the record control at rest and during a take, and the cards of a sound
being saved and a separation, on a private display with an isolated D-Bus
session so no dialog reaches the person's desktop. Demonstrate
dragging, scrubbing and playback with a short native recording when available.
The acceptance record identifies host, window scale, build and unverified
platforms; a static mockup is design reference rather than runtime evidence.
