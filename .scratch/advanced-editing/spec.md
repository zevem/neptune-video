# Advanced editing

Status: ready-for-human
Progress: resolved
Type: task

Tickets 01 to 15 and 17 are implemented and resolved with their known limits
in [issues/](issues/); ticket 16 is answered research that waits on a product
decision. What remains is for a person: that decision, and native acceptance
on macOS, Windows, a hardware GPU in a windowed session and HiDPI, none of
which was attempted. See [verification](../../docs/verification.md#advanced-editing).

Keyframe animation, transitions, visual effects, filters, speed controls,
masks/compositing, motion tracking, color grading, stabilization, background
removal and picture-in-picture, added to the core editor without weakening its
contracts: validated reversible commands, pure core, bounded workers, quiet idle.

This document is the binding design. Where a ticket and this file disagree, this
file wins; change it deliberately and note the change under Comments.

## Decisions

1. **Small generic durable data.** One flat `ParamKey` enum and one flat
   `EffectKind` enum; effects are `{kind, params}` checked against a static spec
   table. This keeps the provider schema within its 24 KiB cap and makes every
   parameter keyframable by the same mechanism.
2. **No clip-to-clip references.** A transition belongs to the clip it brings
   in or takes out; its partner is found at evaluation time. Trackers are
   project-level and keyed in source time. Delete, move, split and paste need no
   referential cleanup.
3. **Clip-relative animation time.** Keyframe and retime `at` values are integer
   microseconds from the clip's start. Moves and ripples leave them untouched;
   split, start-trim and rate change rebase them.
4. **Static values stay where they are.** `Transform`, `gain` and effect
   `params` hold the base value; an animation track overrides it while present.
   Existing files and `SetTransform` keep their meaning.
5. **One pure evaluator.** `editor_core::resolve(project, time, overrides)`
   returns a `RenderPlan`. Wall clock is never an input; grain/glitch/shake are
   seeded from clip ID and canvas frame index so a paused viewer has nothing to
   repaint. The plan is the headless test oracle and the seam a future export
   renderer would consume.
6. **GPU compositor, plain fallback.** An offscreen wgpu compositor in the
   desktop crate draws non-plain plans and publishes one canvas texture to
   egui. A plan whose layers are all plain keeps today's mesh path unchanged.
   Without a wgpu device the mesh path draws transform/opacity only and shows a
   quiet "Effects not shown" state. Pixel math exists once, in WGSL, verified by
   device readback tests that print a named `SKIP:` when no adapter exists.
7. **Derived analysis data is not project data.** Stabilization trajectories
   live in a bounded on-disk cache keyed by source fingerprint and algorithm
   version. Tracker paths are small, decimated and durable (they are an editing
   decision the person can adjust), entering through a command.
8. **Schema version 2.** Read accepts 1 and 2 (v1 is a strict subset through
   serde defaults); write is always 2, so an older build refuses the file
   through its future-schema protection rather than stripping data on save.
9. **Strict now, relax later.** Validation rejects what the renderer cannot
   draw (for example effects on text clips), so loosening never invalidates a
   saved file.

## Domain model (`crates/editor-core/src/`)

All new types derive `Debug, Clone, PartialEq, Serialize, Deserialize,
JsonSchema`. Enums are unit `snake_case` or internally tagged with `type`. No
`flatten`, no untagged enums, no maps inside `Clip`. Every new field on an
existing struct is `#[serde(default, skip_serializing_if = …)]`.

```rust
// model.rs
pub struct Clip { /* existing fields */
    pub blend: BlendMode,                  // default Normal
    pub effects: Vec<Effect>,              // ≤ 16, stack order = render order
    pub masks: Vec<Mask>,                  // ≤ 8
    pub animation: Vec<AnimTrack>,         // ≤ 64, (target, key) unique
    pub retime: Option<Retime>,
    pub transition_in: Option<Transition>,
    pub transition_out: Option<Transition>,
    pub follow: Option<Follow>,
}
pub struct Project { /* existing */
    pub trackers: Arc<BTreeMap<Uuid, Arc<Tracker>>>,   // ≤ 64
}

// animate.rs
pub enum ParamKey { PositionX, PositionY, ScaleX, ScaleY, Rotation, Opacity,
    CropLeft, CropTop, CropRight, CropBottom, Gain,
    Amount, Radius, Angle, Threshold, Softness, Frequency, OffsetX, OffsetY,
    Exposure, Contrast, Saturation, Temperature, Tint, Lift, Gamma, GainLuma,
    Hue, HueWidth, HueShift, SatShift, LumShift,
    KeyR, KeyG, KeyB, Tolerance, Spill, Smoothness, CropZoom,
    CenterX, CenterY, Width, Height, Feather, Roundness }
pub struct AnimTrack { pub target: Option<Uuid>, pub key: ParamKey, pub keys: Vec<Keyframe> }
pub struct Keyframe { pub at: Time, pub value: f32, pub ease: Ease }
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Ease { Hold, Linear, Bezier { x1: f32, y1: f32, x2: f32, y2: f32 } }

// look.rs
pub enum BlendMode { Normal, Multiply, Screen, Overlay, SoftLight, Add, Darken, Lighten, Difference }
pub enum EffectKind { Glow, Shake, Distort, MotionBlur, Grain, Sharpen, Shadow, Blur, Vignette,
    Glitch, Grade, HslBand, CurveMaster, CurveRed, CurveGreen, CurveBlue, Lut,
    ChromaKey, DifferenceMatte, Stabilize }
pub struct ParamValue { pub key: ParamKey, pub value: f32 }
pub struct Effect { pub id: Uuid, pub kind: EffectKind, pub enabled: bool, pub mix: f32,
    pub params: Vec<ParamValue>, pub points: Vec<[f32; 2]>, pub asset_id: Option<AssetId> }
pub enum MaskShape { Rectangle, Ellipse }
pub enum MaskOp { Add, Subtract, Intersect }
pub struct Mask { pub id: Uuid, pub shape: MaskShape, pub op: MaskOp, pub invert: bool,
    pub center: [f32; 2], pub size: [f32; 2], pub rotation: f32, pub feather: f32, pub roundness: f32 }
pub enum TransitionKind { Fade, Dissolve, SlideLeft, SlideRight, SlideUp, SlideDown,
    ZoomIn, ZoomOut, WipeLeft, WipeRight, WipeUp, WipeDown, Glitch, Blur }
pub struct Transition { pub kind: TransitionKind, pub duration: Time, pub ease: Ease, pub params: Vec<ParamValue> }
pub struct Tracker { pub id: Uuid, pub name: String, pub asset_id: AssetId, pub keys: Vec<TrackKey> } // ≤ 2048
pub struct TrackKey { pub source: Time, pub center: [f32; 2], pub scale: f32, pub rotation: f32 }
pub struct Follow { pub tracker_id: Uuid, pub position: bool, pub scale: bool, pub rotation: bool, pub offset: [f32; 2] }
pub enum LookPreset { Cinematic, TealOrange, Vintage, Faded, Noir, Warm, Cool }

// retime.rs
pub struct Retime { pub keys: Vec<TimeKey> }                           // 2..=64
pub struct TimeKey { pub at: Time, pub source: Time, pub ease: Ease }  // source = offset from source_in
```

### Rules

- **Animation.** Keys strictly increasing in `at`, within `0..=duration`, at
  most 256 per track, values finite and inside the parameter's range.
  `target: None` addresses the clip (transform and gain keys only);
  `Some(id)` addresses an effect or mask on the same clip and the key must be
  animatable there. `ease` shapes the segment leaving its key as a CSS-style
  cubic: `x1, x2 ∈ 0..=1`, `y1, y2 ∈ -4..=4`. Named easings (ease in/out, back,
  and so on) are UI constants over `Bezier`.
- **Effects.** `look::spec(kind) -> &'static [ParamSpec { key, min, max,
  default }]` is the single table that validates, fills defaults, drives the
  inspector and documents the agent surface. A key foreign to the kind is
  invalid; a missing key takes its default; `mix ∈ 0..=1`. Lengths are fractions
  of the layer box or canvas height, never pixels, because preview frames vary
  in size. `points` is used only by `Curve*` (2..=16 points, x strictly
  increasing, both in `0..=1`). `asset_id` is required for `Lut` and
  `DifferenceMatte` and forbidden elsewhere; `RemoveAssets` refuses an asset
  still referenced by an effect or tracker.
- **Filters** are not a type. `ApplyPreset` expands a `LookPreset` into ordinary
  adjustable `Grade`/`Grain`/`Vignette`/curve effects.
- **Text clips** accept transform/gain animation, transitions and follow;
  effects, masks and non-`Normal` blend are rejected until text is rasterized
  into the compositor.
- **Retime.** `None` behaves exactly as today. With `Some`, `playback_rate` must
  be `1.0`; the first key is at 0, the last at `duration`, `at` strictly
  increasing. Equal sources freeze, decreasing sources reverse, an eased segment
  ramps (`y1, y2 ∈ 0..=1`, so each segment is monotone). Mean segment speed is 0
  or within `0.05..=16`. The seam is `Clip::source_time(playhead)`,
  `Clip::source_span() -> (Time, Time)` and `Clip::speed_at(playhead) -> f64`;
  every linear `source_in + Δ·rate` consumer moves to it.
- **Transitions.** `transition_in` on clip B covers `[B.start, B.start +
  duration)`. Its outgoing partner is the same-track clip whose end equals
  `B.start`, sampled past its end (source clamped to the asset, so a missing
  handle holds the last frame). With no partner B transitions from
  transparency. `transition_out` applies only when nothing abuts. Duration is at
  least one canvas frame and at most `min(clip.duration, 10 s)`. "Custom"
  transitions are parameterized kinds, not user-authored shaders.
- **Tracking.** Trackers bind an asset and are keyed in source time, so they
  survive split, trim, move and retime. `Follow` resolves against the topmost
  active clip showing the tracker's asset; when none is on screen it is inert.
- **Stabilization** is an effect holding `Smoothness` and `CropZoom`; motion
  data is a regenerable cache.
- **Background removal** is `ChromaKey` or `DifferenceMatte` (clean plate);
  the replacement is a clip on a lower track. Automatic subject segmentation
  needs a neural runtime and model and is a separate research ticket.
- **Picture-in-picture** needs no type: tracks already layer bottom to top with
  `Transform`; this epic adds blend, mask, shadow and layout presets.

## Commands

Whole-value setters; snapshot history makes them reversible without inverses.
All go through `clip_mut`, so a locked track refuses them. All bounds live in
`Project::validate`.

| Command | Rule |
| --- | --- |
| `SetEffects { clip_id, effects }` | Replaces the stack; IDs unique; prunes animation tracks whose target vanished |
| `SetMasks { clip_id, masks }` | Same pruning |
| `SetBlendMode { clip_id, mode }` | |
| `SetKeyframes { clip_id, target, key, keys }` | Replaces one track; empty removes it |
| `SetRetime { clip_id, retime, duration }` | Follows links like `SetPlaybackRate`; bakes each linked clip's rate into its key sources then sets rate 1.0; `None` clears |
| `SetTransition { clip_id, edge, transition }` | `edge` is `in` or `out`; `None` removes |
| `ApplyPreset { clip_ids, preset, strength }` | Appends effects with fresh IDs |
| `SetTracker { tracker }` / `RemoveTracker { tracker_id }` | Upsert; removal clears every `Follow` naming it |
| `SetFollow { clip_id, follow }` | Tracker must exist |

Existing arms change: `SplitClip` partitions animation and retime keys at the
cut with synthesized boundary keys (de Casteljau subdivision), gives the left
half `transition_in` and the right half `transition_out`, and regenerates effect
and mask IDs on the right with targets remapped. `TrimClip` rebases keys on a
start trim and crops a retime map. `SetPlaybackRate` rescales key times with the
duration; on a retimed clip it scales the map. Copy/paste remaps effect and
mask IDs.

## Evaluation (`render.rs`)

```rust
pub fn resolve(project: &Project, at: Time, overrides: &[ClipOverride]) -> RenderPlan;
pub struct RenderPlan { pub canvas: [u32; 2], pub background: [u8; 4], pub layers: Vec<Layer> } // bottom → top
pub struct Layer { pub clip: ClipId, pub track: TrackId, pub source: LayerSource,
    pub transform: Transform, pub blend: BlendMode, pub effects: Vec<ResolvedEffect>,
    pub masks: Vec<Mask>, pub transition: Option<ResolvedTransition>, pub seed: u64 }
```

Per clip: evaluate animation tracks, apply follow, emit transition pairs (the
outgoing clip appears as a layer past its end), drop disabled effects.
`Layer::is_plain()` selects the unchanged mesh path. `RenderPlan: PartialEq` is
the compositor's dirty check.

## Rendering, workers and UI

- `src/compose/`: GPU compositor (linear-light math, premultiplied sRGB out,
  ping-pong `Rgba8Unorm` targets at canvas-fit size), a `ComposeBudget` of
  logical bytes separate from `ViewerBudget` and released when idle, composition
  only in `ui`. Never described as a measured GPU-memory bound.
- `crates/editor-media/src/analysis.rs`: one bounded cancellable analysis lane
  with its own processes; results carry generation, request, asset and source
  fingerprint and are dropped when stale.
- Inspector: an inner segmented control (Video / Look / Color / Speed / Motion)
  generated from `look::spec`, with per-row keyframe toggles capturing clip ID
  and playhead at click. Timeline: keyframe lane, graph-editor mode, transition
  handles with a menu/keyboard route. Library: Media / Effects / Transitions.

## Acceptance

| Area | Proof |
| --- | --- |
| Model and commands | Core tests: bounds, NaN, reversal, atomic failure, split-invariance of sampled values, v1 JSON decode |
| Store | v1 fixtures load; writer emits 2; a v1-pinned reader refuses v2 |
| Evaluation | Plan tests: order, keyframe values, transition pairing, follow, seed determinism |
| Agent | Provider schema under cap; capability list, planner text and diffs cover new commands |
| Rendering | Device readback tests per effect/blend/mask/transition; existing viewer tests unchanged; native captures |
| Speed | Reverse/freeze/ramp bounds in core; audio plan tests; measured reverse preview rate |
| Analysis | Stale-result and cancellation tests; synthetic-motion tracking accuracy; synthetic-jitter stabilization residual |
| UI | One history entry per gesture; Escape cancels; captures at normal and narrow widths |

## Not claimed

Export or final-quality rendering (no export path exists: everything here is
preview at preview resolution); real-time playback of effect stacks; measured
GPU-memory bounds; automatic subject segmentation; warp-quality stabilization
or rolling-shutter correction; tracking through occlusion; pitch-preserving or
ramped audio; effects on text; user-authored shader transitions; macOS/Windows
acceptance; parity with any established editor.

## Tickets

| Id | Title | Depends |
| --- | --- | --- |
| 01 | Types, defaults and validation | – |
| 02 | Commands and split/trim/rate/paste rebasing | 01 |
| 03 | `resolve` and `RenderPlan` | 02 |
| 04 | Store schema v2 with v1 read | 01 |
| 05 | Agent capability list, planner text, context projection, schema cap | 02 |
| 06 | Linear-time callers move to the retime seam (rules, viewer, timeline, audio) | 02 |
| 07 | GPU compositor backbone, budget, viewer consumes the plan | 03 |
| 08 | Effect, color, mask, key and transition shaders | 07 |
| 09 | Inspector tabs, drafts and keyed rows | 02, 07 |
| 10 | Keyframe lane and graph editor | 09 |
| 11 | Transition handles and effects/transitions browser | 08, 10 |
| 12 | LUT asset (`.cube`) | 08 |
| 13 | Analysis lane and motion tracking | 03 |
| 14 | Stabilization | 13, 08 |
| 15 | Speed UI and reverse preview buffering | 06, 09 |
| 16 | Automatic subject segmentation (research) | 13 |
| 17 | ADR 0006, glossary, usage and verification docs | all |

## Comments

2026-10-13: Spec synthesized from three independent architecture proposals over
seven subsystem maps. Nothing implemented or measured yet.


2026-10-13: Review of tickets 01-05 changed these rules. Clips on audio tracks
accept gain keyframes and retime only, and a sound cannot be an effect asset or
a tracker asset (decision 9). A cut inside an eased segment may synthesize more
than one key, so each part keeps the curve; a part no keys can carry is refused.
`SetSettings` lengthens a transition shorter than the new frame to one frame.
`TrimClip` reads a change of `source_in` on a title or a still as a move of the
start edge and keeps `source_in` at or above zero. `SetRetime` scales the map
for a linked clip of another length and leaves a linked title without a map.
The recovery archive stores each tracker once. The clipboard text is version 2
and version 1 is still read. Preview audio follows gain keyframes, and plays a
time map only on its straight forward segments (silent on a freeze, a reverse
and a ramp).

2026-10-13: Ticket 06. The viewer snaps a clip's source time to the asset's
frame grid with 18 µs of slack, because grid times floored to the microsecond
could add up to just before the frame they are on. In a backwards segment a
timeline frame shows the source reached by its own end, so a reversed span
shows the frames it shows forwards, last first. Neither rule is in `resolve`:
the plan's `source_time` is unsnapped and the consumer applies them. Preview
audio splits a clip's chunk interval at the time map's segment boundaries, so
silence starts on the sample a freeze, reverse or ramp does. A trim that would
leave a ramp slower than 0.05 is refused by validation, not prevented by
`rules::trim_range`. Reverse preview through the sequential decoder starts one
FFmpeg process per uncached step back (measured, not yet buffered: ticket 15).

2026-10-13: Ticket 07. The compositor's intermediate canvases are
`Rgba8UnormSrgb`, not `Rgba8Unorm`: shaders read and write linear light and the
format stores it encoded, because eight linear bits band in the shadows. Only
the published canvas is `Rgba8Unorm`. Text is not composited, so it parts the
plan into runs; a run is composited from its lowest layer through its highest
one that is not plain, on a canvas of its own (at most four a frame), and only
the lowest run has the canvas colour under it. A blended layer above a text
layer therefore blends with the media between the two and nothing lower. A
blend over a clear backdrop shows the layer as it is. Masks are evaluated in
one pass after the effects: the first adds to nothing or takes from the whole
layer, add is the greater coverage, feather is the width of a band centred on
the edge as a fraction of the layer box's shorter side. The mesh path blends
in gamma space as before, so a half-transparent layer changes shade when its
plan stops being plain. No pass draws a transition yet: both layers of a pair
are requested and shown whole. "Effects not shown" sits in the margin over the
canvas, since the transport's facts are hidden in a narrow window.

2026-10-13: Ticket 08a. `Layer` gained `time`: the start of the canvas frame
from the clip's start. Noise with a rate of its own is drawn from it and the
clip's ID (a shake eases between places drawn `Frequency` times a second, a
glitch keeps one pattern for each such stretch), and grain from `seed`. A
grade works on linear light in the order exposure, white balance, contrast,
lift/gain/gamma, saturation: contrast is a power of `2^Contrast` about 0.18,
temperature and tint are gains that keep white as bright as it was, and the
result is raised to `1/Gamma`, so a gamma above one lifts the middle. Curves,
the hue band, the keys and grain work on gamma-encoded values. A curve is
evaluated in the shader from its points with monotone slopes, not from a
256-entry table. Lengths are fractions of the canvas's height; a blur's
deviation is half its `Radius`. `Distort` is a wave (`OffsetX` is its phase in
waves). `MotionBlur` streaks the composited layer along `Angle` and
accumulates no frames. Effects are drawn on the placed layer in canvas
space, so a shake or a wave can bring in clear canvas, and a spatial effect
is not turned with the layer. The lifts of the looks were made smaller for
linear light. `DifferenceMatte` is drawn when the compositor is given a clean
plate, which the viewer does not decode yet; `Lut` and `Stabilize` are drawn
by no pass.

2026-10-13: Ticket 08b. A transition is one pass that makes one layer of its
two clips, which is then blended with the incoming clip's blend mode; each
clip's opacity, effects and masks are applied before it. Progress 0 is the
outgoing clip alone and 1 the incoming alone. `Dissolve` is `(1 - p)` of one
and `p` of the other in linear light. `Fade` between two clips is a dip
through transparency: the outgoing clip fades out over the first half and the
incoming in over the second, so half way what is under the track shows (the
canvas colour on the lowest track). A `Fade` with no partner is the same plain
fade a `Dissolve` is. A slide moves both clips by the canvas, the incoming
pushing the outgoing in the direction it is named for, in whole pixels. A
wipe's edge crosses the canvas in the direction it is named for; `Softness` is
the width of its eased band as a fraction of the canvas along that direction.
A zoom scales each clip about the centre of its own box, as one picture pushed
in to (`ZoomIn`: the outgoing by `1 + Amount * p`, the incoming by that over
`1 + Amount`) or drawn back from, and mixes them by `p`. `Blur` is a dissolve
of two clips blurred by a deviation of `Radius / 2` of the canvas's height at
most, half way, and not at all at the ends. `Glitch` cuts 24 bands of rows
from one clip to the other as `p` passes a number drawn for each from the
frame's seed, and tears rows and parts colours by `Amount`, most half way. A
clip whose partner has no picture yet is drawn against nothing. Without a
device every kind is a cross-fade of the two quads, and text clips always
fade. While a transition holds a clip past its end, that clip and the one it
leads into are left out of each other's cut run, so each keeps a layer and a
decoder of its own; a cut with no transition is handed over as before. Six
live video consumers (a transition on each of three stacked tracks) are one
more than the decoder bank: the unprotected three restart FFmpeg for each
next frame until the transitions end.

2026-10-13: Review of tickets 06-12 changed these rules. A canvas is composited
at the screen pixels it covers, a texel to a pixel, and steps down only to fit
`ComposeBudget` or the device's longest texture: a window dragged through sizes
makes and draws its canvases again at each. A layer a canvas shows keeps its
decoded frame and no texture of its own, so it is uploaded once, by the
compositor; a layer drawn as a quad gets its texture back from that frame.
Decoded frames are kept only while the plan has a run to composite, and a plan
that stops being plain while paused asks for the frames it shows again; until
they come the run stays quads, and "Effects not shown" is said only of a layer
that is not plain and was drawn as a quad because nothing composites or the
compositor refused its run. Canvases, tables and kept frames are given up three
seconds after the last frame that composited, on the clock, with one wake to do
it in. Whatever the device refuses while pipelines are built or a canvas is
drawn (validation, a driver's internal failure, memory) is caught: everything
is released and the mesh path draws from then on. A layer's decode box follows
the scale stored with its clip, or held in a drag, and is the whole canvas when
the scale is keyed or follows a tracker. A video frame with any texel that is
not opaque is straight alpha on both paths. The viewer's handles take no hold
of a value keyframes drive, and the centre snaps a moved layer where it is
shown. A paste is refused in words when an effect's asset is missing, as when
the clip's own media is.

2026-10-13: Ticket 09a. `ClipOverride` carries
`animation: Option<Vec<AnimTrack>>`: a keyframe being edited is previewed by
handing the plan the clip's tracks with that keyframe written, and a keyed
value's draft is that keyframe, never the stored value. A keyframe is written
at the playhead held inside the clip, and the playhead is "on" a keyframe
within half a canvas frame of it. A layout preset sets position, scale and
crop and leaves rotation, flips and opacity; it is refused while any of the
three is keyed. The Look section's add menu lists the Stylize, Blur, Distort
and Key kinds from the tables; Color, Speed and Motion kinds belong to their
sections, and the difference matte is left out until the viewer decodes a
clean plate. Masks and the constant speed live in Video and Speed. A sound
clip has no sections and a title has Video and Motion. A clip's name and
sections stay above its scrolled rows. Escape gives up a value the pointer
holds or a field being typed and leaves the selection; with a menu open it
closes the menu only. Shift+Enter on a diamond opens its menu.

2026-10-13: Ticket 09b. The Color section edits the first effect of each of
its kinds in the clip's stack and makes one on first touch. A new one goes
after the last key or colour effect that comes no later than it in the order
key, grade, hue bands, curves, LUT, and first in the stack when there is
none: keys are pulled from the ungraded picture and the Look section's
effects follow the colour. A grade or a curve the section touched is taken
out when it is back where it changes nothing, unless keyframes drive it; hue
bands, the LUT and the two keys are added and removed by name. A curve point
that has the keyboard owns the arrows, Delete and Backspace. The editor draws
`compose::pass::curve_at` over `compose::pass::slopes`, which a device test
holds to the shader. The viewer asks for a difference matte's plate as
`Viewing::Plate(clip, effect)` in the same budget as the layers: it is kept
as a decoded frame for the compositor and never becomes a texture, and it is
asked for only while a canvas is composited. The eyedropper reads the mean
of three by three texels of the keyed clip's own decoded frame under the
pointer, before its effects, and frames are kept while it is armed; Escape,
a pick, or the effect going ends it. "Remove background" holds one method at
a time; a clean plate makes no effect until its still is chosen, and stays
out of the Look menu. A background is one batch that puts the asset on the
nearest free unlocked video track under the clip's, or on a new track made
directly under it; a still lasts as long as the clip and a video as much of
that as it has. There is no solid clip: the solid behind everything is the
project's canvas colour, set from the same group.

2026-10-13: Ticket 15. The Speed section edits a clip's time map as a speed
curve (`src/speed.rs`): stretches of source between points, each held, flat,
or ramped from one speed to another. An even ramp is written as a Bezier with
handles at a third of the stretch and a smooth one with handles at half; a
flat stretch is written `Linear` so its sound plays; any other ease is kept
as it is until its points are touched. The graph draws how fast, not which
way: a stretch keeps its direction, and Reverse turns the whole clip. A point
takes 0 to 16×; what the project refuses (`Invalid("time map")`) is not sent,
the last valid draft stays and a line says why. A retime, a freeze and a ramp
are cut to the room on the track as a rate change is; later clips are not
moved. A named speed lets a map go when its source starts at the asset's
first frame and it plays forwards, and writes a straight map otherwise, so a
reversed clip stays reversed. The speed field of a mapped clip is its mean
speed and writes a relative `SetPlaybackRate`. Freeze is F, the inspector's
button and the clip menu: the selected clip under the playhead, else the top
clip there; the hold is the inspector's length, 2 s by default. A point drag
is a draft shown in the graph and the duration only: `ClipOverride` carries
no map, so the viewer and timeline follow on release. A trim gesture stops at
the furthest whole frame whose trial `Retime::crop` validates
(`rules::trim_delta`). A step back behind a decoder's pipe starts the
replacement up to 24 frames early, or as many as half the frame cache holds,
and keeps the frames it reads on the way; a frame too large for two is
stepped one at a time as before.

2026-10-13: Ticket 10. A clip's keyframes are edited in the timeline
(`src/ui/keyframes.rs`). A chevron on the tile of a selected clip that has
animation, the tool strip and U open its lane: a row of 18 points for each
animation track, under the clip's track and scrolled with the tracks; opening
it scrolls it into view. G and the tool strip put the graph in the tracks'
place for the one selected clip: every track on one value axis in stored
units (one curve alone is labelled in its inspector unit), fitted to the
curves shown unless the wheel has moved it, with labels from 64 points of
height. Each key in view is a widget named for its parameter and timecode.
Keys move on the frame grid and snap to the playhead when snapping is on,
stay inside the clip, and replace a key they land on; a value is held to
`look::param_spec` and lands on the finest power of ten a point of the graph
tells apart; Shift keeps a drag in the graph to one axis. The handles of the
segments beside a selected key are the two control points of `Ease::Bezier`
(`x` in 0..=1, `y` in -4..=4); a `Linear` segment shows them a third along
and becomes a cubic when one is moved; `Hold` has none, and a level segment
takes only `x`. The curve is `AnimTrack::sample`, cached until the track's
`Arc`, the zoom, scroll, size, values shown or the held draft change. Every
gesture is one `Batch` of `SetKeyframes` named for it ("Move keyframes",
"Delete keyframes", "Paste keyframes", "Change easing"); a held one is drawn
on the canvas through `ClipOverride.animation`. While keys are selected, and
the keyboard is with them or with nothing, the arrows (a frame; ten with
Shift; a hundredth of the graph's height up and down), Delete and the
clipboard chords are theirs, each press its own entry; Escape gives up a
held drag, then the selected keys, then the clip. A press on the tracks lets
keys go. A paste is of keyframes while they were the last thing copied and a
lane or the graph shows a clip: at the playhead, into the tracks of the same
target and parameter, leaving out what the clip has no place for. A locked
track takes no drag, and what the keyboard asks of it the project refuses.
The time map is not drawn as a curve: the Speed section edits it.

2026-10-13: Ticket 11. A transition is a marking on its edge of the clip's
tile, as wide as it lasts (six points at least), under the name on a tall
tile. Its inner end is a handle: a drag is one `SetTransition`, snapped,
quantized to frames and held to one frame..min(clip, ten seconds); Escape
gives up the drag, then the selected marking, then the clip. A press on a
marking selects it and its clip, and the inspector shows the transition in
the clip's place (kind, duration, easing, the kind's `look::transition_spec`
values, Remove, Save as default) through the clip's draft; Delete removes a
selected marking's transition and leaves the clip. A new transition is the
kind's kept default (app settings, one per kind, dropped when no clip would
take it) or a half second linear Dissolve, cut to its clip. A cut is its
incoming clip's start: asked for at the end of a clip another follows, it
goes to that clip. Asked for with no kind where one is, the one there is
selected; with a kind, it becomes that kind and keeps its length and easing.
`Action::AddTransitions` carries the clip and edge captured when it was
asked for (Cmd+T and the menus: each selected clip's edge nearest the
playhead; the badge a cut or free edge shows under the pointer; a tile let
go over a clip: the nearer edge) and is one entry. The library's Effects,
Transitions and Looks list `EffectKind::ALL` (less the kinds that need a
file, and Stabilize), the transition kinds and the look presets; a double
click or Return gives a tile to the selected clips, a drop to the clip under
it. Effects and looks go to media clips on video tracks; a colour or key
effect goes where the Color section would put it, any other at the end.
Sound takes none of them and a locked track is refused by the project.

2026-10-13: Review of tickets 09-15 changed these rules. The viewer's
handles do take hold of a value keyframes drive: the drag writes a keyframe
at the playhead of the press (`SetKeyframe`s in the drag's one entry), which
replaces the rule above. An Escape in an inspector field being typed in
leaves the field and gives up what was typed; the selection stays. A button
in the Speed section edits the clip a held typed value leaves. Keyframes
stay selected only while the clips selected are the ones they were selected
with, and selected keyframes select their clip alone; a paste is of
keyframes only while the clipboard still holds what the copy put there. In
the graph a key keeps its value until it is lifted three points, values land
on the finest power of ten a point tells apart (no coarser than a hundredth
of the lane's range) and Up/Down move a hundredth of the narrowest selected
range. A press on the "+" badge is the clip's own press (trim or move) and
adds a transition only when let go in place. A marking is hit where it is
drawn, under the name, and its handle stays clear of both trim grips. A look
given in the inspector takes the place of the look given there before, and
Strength makes that look again as one entry; the library and the agent
still append, and which look was given is session state. A dragged field
moves at most one unit a point, half a degree for an angle.

2026-10-13: Wave 3 acceptance defects changed these rules. `SetRetime` moves
a clip's keyframes, transitions aside, to where the new map shows the source
moment each was on: a freeze shifts later keys by its length, a reverse
mirrors keys and the easing of each stretch, a moment shown twice takes the
showing nearest the key's old time, one never shown the map key nearest in
source; a linked title still scales. A clip holds one look, read from its
effects (the first run equal to a preset at some strength of 0.01 or more);
`ApplyPreset` replaces it in place for the inspector, the library and the
agent alike, which replaces "appends" in the table and the session state
above. A window too narrow for both side panels shows the one asked for
last. C selects the clip under the playhead (again: the next down the
stack), Alt+Left/Right the clip before or after, Alt+Up/Down the one on the
track above or below; the playhead goes to a clip it was off. The speed
graph is linear to 2, 4, 8 or 16 times and holds that top while a point is
held. Scrubbing one value of a keyed pair keys both at that time, and a
row's diamond is filled when either is keyed there. A diamond pressed with
the playhead off the clip says so and keys nothing; easing is not offered on
a track's last keyframe. "remove transition" takes the selected clip's own
transition, the edge nearest the playhead first, and reaches the next
clip's only when "at end" is said. A press in a trim grip that does not move
trims nothing.

2026-10-13: Ticket 13. Tracking is template matching of one region and finds
where it moved, nothing else: zero-mean normalized cross-correlation of a luma
patch in a window around where the region is expected (its last place plus its
last step, 8 to 32 analysis pixels each way), then least squares between
pixels. Frames are read at the asset's rate, 384 pixels at most a side. A
match under 0.5 ends the path on the frame before ("lost"), a match under 0.8
takes the patch again from where it was found, and an even patch is refused.
`scale` is 1 and `rotation` 0 in every key; nothing is detected, turned,
scaled or found again after it is lost. A path runs forwards from the frame
under the playhead to the end of what the clip shows of its source, is
thinned to the keys a straight line between its neighbours does not say
(a twentieth of an analysis pixel) and then evenly to 2048, and enters as one
`SetTracker`, named "Tracker n". The answer carries the request, the editor's
instance and the source's path, size, length and dimensions; one that matches
none of them is dropped, and one whose media changed is said and adds
nothing. One region is tracked at a time and a second takes its place. The
region is session state held by the inspector (Escape gives it up); the
canvas shows it and the paths only while the Motion section is shown. A clip
set to follow keeps its place at the playhead: the offset is what takes the
point back out, and "Centre on point" makes it the clip's position negated.
Following is position only in the interface. `ClipDraft` holds the follow, a
held offset is drawn by moving the override's position by the difference,
and `Tracker::at`, `render::tracked_point` and `render::tracked_clip` are
the one reading of a path. "follow NAME" and "stop following" are agent
phrases. Only video is tracked (not GIFs or stills).

2026-10-13: Ticket 14. Stabilization is a 2D deshake: the camera's global
motion (shift, turn and size, from block matching on luma 384 pixels a side
with outlier rejection; shift alone when fewer than six blocks or fewer than
half of them agree, when they are close together, or when they keep to the
fitted turn and size by more than 0.35 pixels) is integrated
to a path, smoothed by a gaussian of Smoothness x 0.5 s, and the difference
is applied with a crop. No rolling-shutter correction, no 3D or warp. The
project holds only the `Stabilize` effect (Smoothness shown as "Strength",
CropZoom); the path is derived, of the whole asset and so shared by every
clip of the file, kept at `<user cache>/stabilization` (beside the settings
when a window is told where those are), one file a media path, 64 MiB and 256
files, least lately read removed first. A path whose stamp (file length and
modification time, and the asset's size, duration, picture size and rate)
differs is stale and not used. The correction is applied in the place pass on
the layer's source sampling, for the source time of the frame the layer
shows, scaled by the effect's mix and scaled down per frame so that it never
reads past what the crop hides; until a path is held the layer is drawn
uncorrected and uncropped and the Motion section says so. One video is
measured at a time; a newer request gives the older up. Tracker overlays and
a difference matte's plate are not steadied with the layer. Stabilize stays
out of the effects browser: its home is the Motion section.

2026-10-13: Ticket 17. Docs written against the code; one issue file a
ticket. Where this file's body and the code differ, the code and the dated
entries above stand: intermediate canvases are `Rgba8UnormSrgb` (not
`Rgba8Unorm`); `ApplyPreset` replaces a clip's look (the command table says
appends); `Layer` has `time` and `ClipOverride` has `animation`; `MediaKind`
has `Lut`; the library has four tabs (Looks too); the stabilization cache
file is named for the media path and stamped, not keyed by a fingerprint;
analysis answers carry request, project, target, asset and `SourceIdentity`
and the app checks its editor instance, with no generation counter; the
compositor also releases after 120 painted frames without a job, besides the
three seconds. Ticket 16 is answered in its issue file and needs the product
owner's decisions on licence, download size and scope before any work.

2026-10-13: Final review changed these rules. A followed point is put where
the tracked clip draws that place of its picture: `render::picture_point`
through its fitted box, crop, flips, scale, turn and place (animated, drafted,
and its own follow up to four clips deep), which replaces "read as a canvas
fraction" above; `render::tracked_place` is that for the inspector's and the
agent's offsets. `ClipOverride` carries `reading: Option<Reading>`, how a
steadied layer reads its picture: derived, handed in by the viewer, changing
no layer of its own clip, and the viewer resolves again when a frame has both
a reading and a follower. The region, the point and the path are drawn
through the reading of the frame shown, the path's other keys through that
same frame's. A path starts on the frame the viewer shows
(`viewer::source_frame`, with its slack) and runs to the furthest source the
rest of the clip shows; where the clip plays backwards Track is disabled. A
path that arrives while there is something to redo or a plan under review is
held ("Tracker n is ready": Add tracker, Discard) and added when neither is
left. Removing media no clip uses removes its trackers in the same step and
names them; "remove tracker NAME" is an agent phrase. Alt and an arrow reach
for no clip while a control owns the arrows. A loss is said once and not kept.

2026-10-13: Final acceptance defects changed these rules. A parameter row's
fields are made under an id taken from the row (`widgets::keyed_row_with`), so
a reset button that comes or goes, or a discarded pass, does not move the
field being typed in. `ui::DraftHold` replaces the one-second rest: a draft is
held while the pointer is in use or a text field has the keyboard, unless the
draft was changed by the pointer, and is committed when neither holds; Return
or leaving the field is one entry, Escape none. Media tiles, folders, the
"All media" crumb and the Project menu draw the focus ring; panel splitters
take no keyboard focus. A press is a transition offer only on the "+" disc
that was drawn the frame before (`TimelineState::badge`); elsewhere in the
edge zone it selects or trims. A lane stepping backwards has a look-behind:
`DecoderPool::look_behind` starts the earlier run on its own thread after a
frame is published, throttled to the room the frame cache has, on a free
decoder slot only; runs are two frames short of half the cache; a step
forwards, a jump, another source, a project change or a dropped lane ends it,
and a request it covers waits for it while that request is current. Measured:
180 distinct pictures in each of three 6 s windows of reversed 1920x1080
30 fps H.264 (keyframe every 15 frames) in the release build on the RTX 4060
under headless Weston, against 144 to 147 before; nothing is claimed for
distant keyframes, effects, larger preview frames or other adapters.
