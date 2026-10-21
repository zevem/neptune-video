# Interface direction

The editor should feel like a focused native creative tool: picture and
timeline dominate, with restrained chrome and one accent. Neptune Video shares
Neptune's window, materials, controls, type, menus, sheets and toasts with
Terminal and Finder, so the apps read as one family; it owns the stage, the
timecode and the timeline, which an editor needs and the other apps do not
have.

## Neptune's window and controls

Tokens live in [src/theme.rs](../src/theme.rs) (`Palette`, `metrics`,
`motion`) with Terminal's names and values. Use them rather than literal
colours or sizes. The window is one quiet material, `chrome`, shared by the
toolbar, the gutters, the library and the inspector, which sit flush on it
with no card of their own. The viewer and the timeline are rounded pane wells
inside 6-point gutters; the window's 16-point corner is the 10-point pane
corner plus the gutter. Menus, sheets, tooltips and toasts are `elevated`
with a hairline `border` and a soft shadow. Depth comes from elevation and
hairlines, not fills. The window is square where the platform draws it
(Windows) or where it fills the screen, except that macOS keeps a zoomed
window's corners.

The controls are Terminal's: [controls.rs](../src/ui/controls.rs) and
[panel.rs](../src/ui/panel.rs) are ported from it and stay as Terminal has
them, with each departure marked `Neptune Video:` in the code.
[kit.rs](../src/ui/kit.rs) holds what Terminal has no counterpart for (pane
wells, number fields, the key column, select lists, chips, empty states) and
draws it from the same tokens and primitives. Buttons, fields and selects are
30 points tall, compact controls and menu rows 28, controls inside an
inspector row 26, and icon buttons 28 around a 16-point glyph. Head cells,
diamonds and card actions are 22-point small targets. Type is Geist in
Regular, Medium and SemiBold, in sentence case; JetBrains Mono, Terminal's
mono, is kept for the timecode and readouts, whose digits must not shift.
Every egui `ComboBox` is drawn as a select with a menu list.

The toolbar is the title bar: 44 points of `chrome`, a drag region minus its
controls, with Terminal's three window lights at its leading edge. The
library's toggle travels with the library's edge and is never drawn pressed.
The title button is the Project menu: it shows the project's name and, beside
it, the save state in words ("Saved", "Edited", "Saving…", or "Changed on
disk" after an amber dot). A launcher over the viewer's centre opens the
command line, as Command/Ctrl+K does. Undo, Redo, the render queue's ring
while it has work, a quiet labelled Export and the inspector's toggle follow
at the trailing edge; the toggle is rightmost, pressed while the panel shows,
and carries the attention dot while a proposal waits. Import appears there
only while the library is hidden. Where the toolbar is short of room it folds
in a fixed order: the launcher to its glyph, the name, Undo and Redo into
More, Export to its glyph, the save words to a dot. The toggles and the title
button never fold. The supplied [app icon](../assets/README.md) is the window,
dock and taskbar icon and is drawn as it is: brand artwork is not a glyph and
is never recolored. The toolbar shows the project, not the application.

Sheets are Terminal's: one elevated surface over a rounded scrim that fades in,
its title at the top and its answers at the trailing edge of its foot, the
confirming one furthest out. Escape gives each sheet's own answer; Enter gives
the default only when pressed after the sheet is in sight, and never a
destructive one. Recovery and conflict sheets never default to replacement.
Preferences is a sheet with a source list (Appearance, Editing, Preview,
Agent, Keyboard) and a search, opened from the gear at the library's foot,
the Project menu or Command/Ctrl+Comma. Toasts sit at the window's top
trailing corner. Information (a secondary info mark) and what was done (a
green check, as in Terminal) leave after four seconds; a warning (yellow) or
a problem (red) stays until it is dismissed, and the pointer resting on a
toast stops its clock.
While a sheet is open toasts move to the bottom trailing corner and never
cover it; a refusal caused inside the open sheet is said in that sheet.

## Surfaces and layout

Use a compact media library, a central preview, a contextual inspector and a
timeline that keeps clip boundaries readable. The library and the inspector
share the upper band with the viewer; the timeline runs the window's full
width under them. Panels resize by their splitters, whose grips show only on
hover and while dragged, and collapse by their toggles or Ctrl+Shift+B and
Ctrl+Shift+O (⌘B and ⌥⌘O on macOS), which a focused field keeps, never by
dragging past a minimum. They slide in and out over 0.16 s, reversibly. A
double-click on a splitter restores its default, and Preferences › Appearance
› Reset layout restores them all. A panel that hides with the keyboard hands
it to the timeline.

At narrow sizes, reduce simultaneous panels before compressing interactive
targets. Both side panels show from a window 820 points wide, Terminal's
narrowest window with its sidebar; as the window narrows they give way
equally to keep the viewer 480 wide, then the viewer gives way down to 360.
A window too narrow for both shows the one asked for last, never neither; a
fresh narrow window shows the inspector, and which was asked for last is kept
for the session only. A panel docks beside the viewer at every width; there
is no drawer over the picture.

Empty projects offer the real import action; sample media is an explicit demo
action rather than pretending to be the person's project. An empty region
says what it lacks and where it comes from, without decoration.

Use rounded, optically aligned vector icons for familiar actions. Every
icon-only control has an accessible name, tooltip and keyboard equivalent where
appropriate. Keep text for meaningful names, values, errors and unfamiliar
operations. Avoid decorative labels, promotional copy and repeated headings
inside the editor.

Use the pinned [Tabler icon subset](../assets/icons/tabler/README.md) for UI
glyphs, including dropdowns, layout presets and window controls. Outline is the
default; transport controls, the record disc and the on-key diamond use
Tabler's filled style. Keep the library's optical spacing inside its 24-point
grid and recolor glyphs for state; never swap outline and filled for state,
except for the diamond. Timeline marks, curve handles and effect previews
describe editing content and retain their own geometry.

Distinguish hover, focus, selection, disabled, pressed and busy states using a
consistent surface and contrast hierarchy. Hover lifts a fill; a control that
stays on is `pressed`, not the accent; a disabled control is dimmed and says
why on hover. Track lock, visibility, mute and solo controls show their
actual state: a soloed track's headphones take the accent on a pressed cell,
and the mute control of a track that another's solo silences is dimmer than
one that is only off and says why. A head keeps 22-point targets and its
track's name whole: three controls stand side by side, four in two columns,
never four in a row of a 156-point head. A track's volume is a percent field
in its head's menu, and a head says a volume that is not 100% under the name.
Focused fields own text input and IME; timeline shortcuts resume after
editing finishes. A focused tab strip or segmented control takes Left and
Right to change its choice, and an arrow a control owns is never also the
playhead's. An Escape closes the innermost active interaction once, without
also triggering an underlying edit.

## The accent and the status colours

The accent marks nothing at rest. It is kept for keyboard focus, selected
content (tiles, library cards, keyframes, markers, transitions, the canvas
frame), the highlighted row of an open menu, a drag in progress, the one
primary button in view, a switch that is on and solo's headphones, a
slider's fill, progress and the text cursor. The playhead is ink with a halo,
play is a neutral disc, and the toolbar's Export is quiet; the accented
Export is the one that confirms in the export sheet.

Red is destructive, failure, record and clipping, and record keeps its red
under the Red accent: its shape and place tell it apart. Yellow is a warning
and the snap guide, which turns to ink when the accent is Yellow. Amber,
`attention`, is what waits for the person: an agent's proposal, its count on
the Agent tab, "Changed on disk". It turns yellow when the accent is Orange.
In the light appearance yellow and amber used as text, glyphs or thin lines
go through `Palette::legible`, which darkens them where they would not read.
`muted` is for placeholders, disabled ink and minor glyphs, never for notes or
body copy. A media kind's tone is decoration: a clip also says what it is
with its icon and its name, and its selection with a ring, a keyline and a
lifted body.

## The stage, the timecode and the timeline

The viewer is a stage: its whole pane, transport included, is #101012 in both
appearances and is drawn with the dark stage palette, so a picture is always
judged against a neutral dark surround and its overlays read on it. Menus
opened from it use the app's palette. Posters and title previews in the
library and the inspector sit in the same dark wells. The picture keeps 18
points of stage on every side and a hairline keyline. Status about the
picture, prompts and "Effects not shown" are capsules on the picture.

The transport owns time. Its timecode is JetBrains Mono at 15 points with the
leading groups that are still zero quieter than the rest, so the time that
moves is what reads, and it is read out as one label ("Playhead 00:00:02:14
of 00:00:16:00"). The timeline has no corner timecode of its own.

The timeline sits on the content surface, `bg`, in its own pane: a tool
strip, a ruler with its labels in the upper half and its ticks rising from
the foot, heads on the leading side and lanes as faint rows. A selected tile
takes a 2-point accent ring with a contrast keyline inside it and a lifted
body; the bars at its edges show where they can be pulled. Markers keep the
colour the project stores for them and give their names in the tooltip. Tool
strip toggles are pressed when on; Snapping keeps one name and says its state
as a checkbox. Where the strip is short of room, the zoom slider shrinks,
then hides, then Insert and Edit fold into a More menu; the keyframe and graph
toggles, Snapping, the zoom buttons and Fit never fold.

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
end a take; pressed during one it says to stop recording first. The source
monitor records nothing and has no such control. The window is drawn ten
times a second only while a take runs.

Transforms and canvas settings use concise numeric fields with sensible
increments and reset controls. Controls reflect the selected clip and retain
their target if asynchronous work or focus changes before completion. A multi-
selection's mixed values remain distinguishable from ordinary defaults.

## Looks, animation and motion surfaces

The inspector and the agent share the trailing panel under one tab strip. The
clip inspector is sectioned: Video, Look, Color, Speed and Motion for a
picture; Video, Text, Animate and Motion for a title; Video, Graphic, Animate
and Motion for a graphic; and one, Audio, for a clip on an audio track: its
Playback card, then a card for each audio effect in the order the sound goes
through them. The clip's header, its name beside a thumbnail, and the section
control, a segmented control, stay above the scrolled rows; one section has
no control. Rows sit in cards, 32 points each, with labels in the primary ink
and notes in the secondary. Rows for an effect, a mask or a transition are
generated from the core's parameter tables, so a new parameter needs no
panel of its own; an audio effect's rows carry no diamond, because its values
take no keyframes, and its levels read in dB and its frequencies in Hz,
dragged on a logarithmic scale. A kind the session's FFmpeg does not run
keeps its card, says "Not available in this FFmpeg build" and disables its
rows; until the probe has answered, every kind is shown as available.

The library has Media, Text, Captions, Effects, Transitions and Looks tabs in
Terminal's tab strip; when the names would touch they shrink, then become
icons named on hover. Under the tabs, one search row carries the tab's own
actions, and Import… and the Preferences gear stand in its footer. A tile is
given by double-click or Return to the selected clips and by drag to the clip
under it. The Media tab's kind filter (All, Video, Sound, Images) is one
22-point row under the search, drawn as the tabs are, and an empty list says
what it lacks and where. The Effects tab's last heading, Audio, lists the
audio kinds, which go only to clips on audio tracks; a picture is told in
words why it takes none. "Audio effect" names these filters everywhere in the
interface; "sound effect" means an imported sound clip and names nothing
here.

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

An animatable row carries a key column at its trailing edge: a step back, the
diamond and a step on, reserved on every keyed row so the diamonds line up
down a card. The diamond is hollow with no keyframe at the playhead, filled on
one, and filled for a paired row when either value is keyed there; it is never
the accent. The row keeps its full label when keyed, and Tab moves through its
controls in reading order. The diamond's menu opens from the keyboard with
Shift+Enter, takes focus on its first choice and returns focus when it closes.
A diamond pressed with the playhead off the clip explains itself and edits
nothing; an easing that could have no effect (on a track's last keyframe) is
not offered.

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
elsewhere is the same: a held drag, then an open menu, then selected
keyframes or a selected marking, then the clip. One Escape in a submenu
closes only that submenu.

Say what the picture is not. "Effects not shown" names a layer drawn without
its effects and why; a layer that cannot be decoded is named on the stage
("Can't decode …"), and frames on their way are said only after a pause
("Waiting for frames…"); a stabilized clip without an analysis is shown as
the camera took it and the section says so; a refused speed curve keeps the
last valid one and says why; background removal states that there is no
automatic subject cut-out. Background work (tracking, analysis, a duck being
read, a sound being saved, a separation) shows progress with a named cancel
and never blocks a gesture; a separator that has not yet said how far it is
shows "…" and a bar without a value rather than an invented figure, and the
window does not animate for it. A loudness measurement changes nothing until
its figures come and then says what it measured and set. A control that
depends on a host tool (an FFmpeg filter, the stem separator) is shown and
says why it cannot be used, rather than being hidden. A speed graph or value
axis holds still while a point is held.

Known departures: clip tiles are not individually focusable (selection by
keyboard is C and the Alt arrows, with a focus ring and scroll into view);
track-head buttons show a focus ring but the head itself has no Enter action;
global shortcuts are read before the interface, so Space on a focused button
also toggles playback; a mask is edited from its rows and not dragged on the
canvas; missing or offline media has no mark of its own yet in the library,
on its tiles or on the stage; at 760x560 the inspector, shown alone beside
the viewer, shows about 190 points of rows.

## The agent panel

The Agent tab is laid out as Terminal's chat: a session row that says
whether an agent can connect, the conversation, and one composer surface at
the foot holding the field, the Ask or Command choice, who answers, and a
round send button. The session row's More menu holds "Copy the connect
command" and, while a conversation goes on, "New conversation". Who answers,
the effort and how edits are made are chosen in the composer's model menu;
Preferences › Agent holds the same choices with the model's name and the
folders the agent may use. The person's requests sit on `control`, not on an
accent tint, and the panel's introduction keeps the words that say what is
sent and seen.

## Light appearance

The light appearance has the same layout, components and states; only token
values change. The window is #ececef and the timeline #ffffff, cards stay the
translucent `control` rather than white, and floating surfaces are white with
a border and a lighter shadow. The stage and the poster wells stay dark, and
the stage's text is struck twice so it keeps the weight the dark appearance's
glyph curve gives it. Over a picture a tile's name sits on a light frost under
dark ink. Appearance is Light, Dark or System; System follows the platform and
is dark where the platform does not say.

## Motion and performance

Use short motion only when it clarifies a state or spatial relationship:
hover fills and glyph reveals over 0.12 s, switches, panels and sheets over
0.16 s, eased out and reversible. Editing feedback (the playhead, scrubbing,
zoom, drags, trims, snap guides and selection) never eases. Preferences ›
Appearance › Reduce motion settles every eased change at once. Paused
projects need no ambient animation or continuous repaint; a settled window
requests no frames, and a capsule that holds still lets the window sleep.
Large timelines draw visible rows and intervals; media grids virtualize
entries. GPU backing alone does not establish interaction quality.

## Accessibility names

Every control reports a native role and a stable name, and a name does not
change with state: the state is the role's value. "Snapping (N)" is a checkbox
that is checked or not; the panel toggles "Media library" and "Inspector and
agent" are checkboxes; tabs, kind filters, segmented choices, Preferences'
source rows and menu choices that pick one of several (the model menu's
providers, efforts and edit modes) are radio buttons with a selected state;
selects, among them "Model: …", are combo boxes. A name may end with its
shortcut in parentheses, which the tooltip draws as keycaps. The title button
is "Project menu", with the project's name beside it as a label; the
launcher is "Ask or type a command (Ctrl+K)"; the zoom slider is "Timeline
zoom"; the composer's field is "Request" or "Command line". Painted words
that carry meaning are labels too: the timecode, the save state, menu
headings and why the agent's session failed. A toast is not an
accessibility node, but its Dismiss is. Accessible names are what the tests
and the capture harness find controls by: change one only with the design,
and update them together.

## Visual acceptance

Review screenshots of the running final native build, then correct clipping,
contrast, alignment, focus and cramped interactions, and compare the parts
Neptune shares with Neptune Terminal's own captures. Cover an empty project,
an imported asset, a selected/trimmed clip, track controls, Preferences,
history, offline media and an error, at normal and narrow window widths and in
both appearances. For looks, animation and motion, cover each inspector
section, the keyframe lane, the graph editor, a transition marking, each
library tab and a background job in progress at 1440x900 and 760x560; for
sound, cover the waveform and its level line before the peaks arrive, fade
handles at rest and held, the track head's solo and volume menu, the volume
line and the empty Volume row, the Audio section with a preset and an
unavailable kind, beat ticks and a snap to a beat, the record control at rest
and during a take, and the cards of a sound being saved and a separation, on
a private display with an isolated D-Bus session so no dialog reaches the
person's desktop. Demonstrate dragging, scrubbing, panel slides and playback
with a short native recording when available. The acceptance record
identifies host, window scale, build and unverified platforms; a static
mockup is design reference rather than runtime evidence.
