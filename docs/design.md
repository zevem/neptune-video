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

Distinguish hover, focus, selection, disabled, pressed and busy states using a
consistent surface and contrast hierarchy. Track lock, visibility and mute
controls show their actual state. Focused fields own text input and IME;
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

Transforms and canvas settings use concise numeric fields with sensible
increments and reset controls. Controls reflect the selected clip and retain
their target if asynchronous work or focus changes before completion. A multi-
selection's mixed values remain distinguishable from ordinary defaults.

## Looks, animation and motion surfaces

The clip inspector is sectioned: Video, Look, Color, Speed and Motion for a
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
subject cut-out. Background work (tracking, analysis) shows progress with a
named cancel and never blocks a gesture. A speed graph or value axis holds
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
progress at 1440x900 and 760x560, on a private display with an isolated D-Bus
session so no dialog reaches the person's desktop. Demonstrate
dragging, scrubbing and playback with a short native recording when available.
The acceptance record identifies host, window scale, build and unverified
platforms; a static mockup is design reference rather than runtime evidence.
