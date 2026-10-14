//! The pure evaluator: what the canvas shows at one timeline time.
//!
//! The plan depends on the project, the time and the drafts alone, so equal
//! plans mean there is nothing to repaint.
use crate::{
    AnimTrack, AssetId, BlendMode, Clip, ClipContent, ClipId, Effect, EffectKind, Mask, MediaKind,
    ParamKey, ParamSpec, ParamValue, Project, TextStyle, Time, Track, TrackId, TrackKey, TrackKind,
    Tracker, Transform, Transition, TransitionKind, look,
    title::{self, ElementLayer, Motion, WordState},
};
use uuid::Uuid;

/// A draft standing in for one clip's stored values while it is being edited.
/// `None` keeps the stored value; animation still plays over a draft.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClipOverride {
    pub clip: ClipId,
    pub transform: Option<Transform>,
    pub effects: Option<Vec<Effect>>,
    pub masks: Option<Vec<Mask>>,
    pub blend: Option<BlendMode>,
    /// The clip's animation tracks with a keyframe being edited in place, so
    /// a value keyframes drive shows its draft too.
    pub animation: Option<Vec<AnimTrack>>,
    /// How the clip's picture is read by what draws it, when that is not as
    /// it is: a stabilized one. It is no part of the project and changes no
    /// layer of the clip's own; a point tracked on the clip's media is put
    /// where this reading shows it.
    pub reading: Option<Reading>,
    /// What a title or graphic says and how it looks, while being edited. A
    /// media clip keeps its own content whatever this holds.
    pub content: Option<ClipContent>,
}

/// How a layer's box reads its picture when it does not read it as it is:
/// the place `uv` of what is shown is read at `0.5 + matrix * (uv - 0.5) +
/// offset` of the frame, in fractions of the frame from its left top.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    /// Row by row: `[xx, xy, yx, yy]`.
    pub matrix: [f32; 4],
    pub offset: [f32; 2],
}

impl Reading {
    pub const IDENTITY: Self = Self {
        matrix: [1.0, 0.0, 0.0, 1.0],
        offset: [0.0, 0.0],
    };

    /// Where of the frame the place `uv` of what is shown is read.
    pub fn read(&self, uv: [f32; 2]) -> [f32; 2] {
        let [x, y] = [uv[0] - 0.5, uv[1] - 0.5];
        let [xx, xy, yx, yy] = self.matrix;
        [
            0.5 + xx * x + xy * y + self.offset[0],
            0.5 + yx * x + yy * y + self.offset[1],
        ]
    }

    /// Where of what is shown the place `source` of the frame is: the
    /// reading undone. A reading that shows no picture leaves it where it is.
    pub fn shown(&self, source: [f32; 2]) -> [f32; 2] {
        let [xx, xy, yx, yy] = self.matrix;
        let det = xx * yy - xy * yx;
        if !det.is_finite() || det.abs() < 1e-9 {
            return source;
        }
        let [x, y] = [
            source[0] - 0.5 - self.offset[0],
            source[1] - 0.5 - self.offset[1],
        ];
        [0.5 + (yy * x - xy * y) / det, 0.5 + (xx * y - yx * x) / det]
    }
}

/// The picture's uv at the left top and right bottom of a layer's box, as
/// its crop and flips leave them.
pub fn corners(transform: &Transform) -> [f32; 4] {
    let [left, top, right, bottom] = transform.crop;
    let (mut u0, mut u1) = (left, 1.0 - right);
    let (mut v0, mut v1) = (top, 1.0 - bottom);
    if transform.flip_x {
        std::mem::swap(&mut u0, &mut u1);
    }
    if transform.flip_y {
        std::mem::swap(&mut v0, &mut v1);
    }
    [u0, v0, u1, v1]
}

/// The size a media layer takes on a canvas, in canvas pixels, before the
/// clip's own scale: its cropped picture fitted inside the canvas.
pub fn fitted(source: [f32; 2], crop: [f32; 4], canvas: [f32; 2]) -> [f32; 2] {
    let cropped = [
        source[0] * (1.0 - crop[0] - crop[2]).max(0.001),
        source[1] * (1.0 - crop[1] - crop[3]).max(0.001),
    ];
    if cropped[0] <= 0.0 || cropped[1] <= 0.0 {
        return canvas;
    }
    let fit = (canvas[0] / cropped[0]).min(canvas[1] / cropped[1]);
    [cropped[0] * fit, cropped[1] * fit]
}

/// Where on the canvas the place `source` of a media clip's picture is drawn,
/// in canvas pixels from the canvas centre: through the reading, the crop
/// and flips, the box the picture is fitted in, its scale, turn and place.
/// `source` is in fractions of the frame, and `size` the picture's in
/// pixels (the canvas stands in for one that is not known).
pub fn picture_point(
    canvas: [u32; 2],
    size: [u32; 2],
    transform: &Transform,
    reading: Option<&Reading>,
    source: [f32; 2],
) -> [f32; 2] {
    let canvas = canvas.map(|side| side as f32);
    let size = if size[0] > 0 && size[1] > 0 {
        size.map(|side| side as f32)
    } else {
        canvas
    };
    let fitted = fitted(size, transform.crop, canvas);
    let shown = reading.map_or(source, |reading| reading.shown(source));
    let [u0, v0, u1, v1] = corners(transform);
    let along = |at: f32, from: f32, to: f32| {
        let span = to - from;
        (at - from)
            / if span.abs() < f32::EPSILON {
                f32::EPSILON
            } else {
                span
            }
            - 0.5
    };
    let x = along(shown[0], u0, u1) * fitted[0] * transform.scale[0];
    let y = along(shown[1], v0, v1) * fitted[1] * transform.scale[1];
    let (sin, cos) = transform.rotation.to_radians().sin_cos();
    [
        transform.position[0] + x * cos - y * sin,
        transform.position[1] + x * sin + y * cos,
    ]
}

#[derive(Debug, Clone, PartialEq)]
pub enum LayerSource {
    Media {
        asset_id: AssetId,
        source_time: Time,
        /// Signed source seconds per timeline second; 0 while a frame is held.
        speed: f64,
    },
    /// A title as it shows at the layer's frame-snapped time.
    Text {
        text: String,
        font_size: f32,
        color: [u8; 4],
        style: TextStyle,
        enter: Option<Motion>,
        exit: Option<Motion>,
        /// Where speech is in a timed title with a highlight.
        words: Option<WordState>,
    },
    /// A graphic's design box and its elements, each with its motions.
    Graphic {
        size: [f32; 2],
        elements: Vec<ElementLayer>,
    },
}

/// An enabled effect with every parameter of its kind present, in table order.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedEffect {
    pub id: Uuid,
    pub kind: EffectKind,
    pub mix: f32,
    pub params: Vec<ParamValue>,
    pub points: Vec<[f32; 2]>,
    pub asset_id: Option<AssetId>,
}
impl ResolvedEffect {
    pub fn param(&self, key: ParamKey) -> Option<f32> {
        self.params.iter().find(|p| p.key == key).map(|p| p.value)
    }
}

/// `progress` runs 0 → 1 over the transition for both layers of a pair: the
/// incoming layer is fully shown at 1, the outgoing layer fully gone.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTransition {
    pub kind: TransitionKind,
    pub progress: f32,
    pub outgoing: bool,
    pub partner: Option<ClipId>,
    pub params: Vec<ParamValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub clip: ClipId,
    pub track: TrackId,
    pub source: LayerSource,
    /// `position` is in canvas pixels from the canvas centre, y down.
    pub transform: Transform,
    pub blend: BlendMode,
    pub effects: Vec<ResolvedEffect>,
    pub masks: Vec<Mask>,
    pub transition: Option<ResolvedTransition>,
    /// Stable per clip and canvas frame; the only source of noise.
    pub seed: u64,
    /// The start of the canvas frame, from the clip's start: what noise with
    /// a rate of its own (shake, glitch) moves by. Wall clock never does.
    pub time: Time,
}
impl Layer {
    /// Drawable by the mesh path: transform and opacity only.
    pub fn is_plain(&self) -> bool {
        self.blend.is_normal()
            && self.effects.is_empty()
            && self.masks.is_empty()
            && self.transition.is_none()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderPlan {
    pub canvas: [u32; 2],
    pub background: [u8; 4],
    /// Bottom → top.
    pub layers: Vec<Layer>,
}
impl RenderPlan {
    pub fn is_plain(&self) -> bool {
        self.layers.iter().all(Layer::is_plain)
    }
}

/// The clip's gain at `playhead`, with its animation applied.
pub fn gain_at(clip: &Clip, playhead: Time) -> f32 {
    let spec = look::clip_spec().iter().find(|s| s.key == ParamKey::Gain);
    let local = playhead - clip.start;
    let value = animated(&clip.animation, None, ParamKey::Gain, local).unwrap_or(clip.gain);
    spec.map_or(value, |spec| bound(value, spec))
}

/// Layers in the viewer's order: hidden tracks skipped, audio tracks silent
/// here, tracks bottom → top. A transition's outgoing clip is added directly
/// below the clip it leads into.
pub fn resolve(project: &Project, at: Time, overrides: &[ClipOverride]) -> RenderPlan {
    let settings = &project.settings;
    let cx = Context {
        project,
        at,
        frame: settings.frame_rate.frame_at(at),
        overrides,
    };
    let mut layers = Vec::new();
    for track in &project.tracks {
        if track.hidden || track.kind == TrackKind::Audio {
            continue;
        }
        for clip in track.active_clips(at) {
            let (transition, outgoing) = cx.transition(track, clip);
            if let Some((partner, transition)) = outgoing {
                layers.push(cx.layer(track, partner, Some(transition), true));
            }
            layers.push(cx.layer(track, clip, transition, false));
        }
    }
    RenderPlan {
        canvas: [settings.width, settings.height],
        background: settings.background,
        layers,
    }
}

struct Context<'a> {
    project: &'a Project,
    at: Time,
    frame: i64,
    overrides: &'a [ClipOverride],
}

impl Context<'_> {
    // The transition on `clip` now, and its outgoing partner with the
    // matching half when one abuts.
    fn transition<'t>(
        &self,
        track: &'t Track,
        clip: &Clip,
    ) -> (
        Option<ResolvedTransition>,
        Option<(&'t Clip, ResolvedTransition)>,
    ) {
        if let Some(transition) = &clip.transition_in
            && let Some(progress) = progress(transition, clip.start, self.at)
        {
            let before = track.clips.partition_point(|c| c.start <= clip.start);
            let partner = track.clips[..before]
                .iter()
                .rev()
                .find(|c| c.id != clip.id && c.end() == clip.start);
            let half = |outgoing, partner| ResolvedTransition {
                kind: transition.kind,
                progress,
                outgoing,
                partner,
                params: transition_params(transition),
            };
            return (
                Some(half(false, partner.map(|c| c.id))),
                partner.map(|c| (c, half(true, Some(clip.id)))),
            );
        }
        let end = clip.end();
        if let Some(transition) = &clip.transition_out
            && let Some(progress) = progress(transition, end - transition.duration, self.at)
        {
            let next = track.clips.partition_point(|c| c.start < end);
            if track.clips.get(next).is_none_or(|c| c.start != end) {
                return (
                    Some(ResolvedTransition {
                        kind: transition.kind,
                        progress,
                        outgoing: true,
                        partner: None,
                        params: transition_params(transition),
                    }),
                    None,
                );
            }
        }
        (None, None)
    }

    // `held` marks an outgoing partner drawn past its end: its source stops at
    // the asset's bounds.
    fn layer(
        &self,
        track: &Track,
        clip: &Clip,
        transition: Option<ResolvedTransition>,
        held: bool,
    ) -> Layer {
        let draft = self.overrides.iter().find(|o| o.clip == clip.id);
        let local = self.at - clip.start;
        let content = draft
            .and_then(|o| o.content.as_ref())
            .filter(|content| content.is_generated() && clip.content.is_generated())
            .unwrap_or(&clip.content);
        let source = match content {
            ClipContent::Media { asset_id } => {
                let wanted = clip.source_time(self.at);
                let limit = self
                    .project
                    .assets
                    .get(asset_id)
                    .filter(|asset| held && asset.kind != MediaKind::Image)
                    .map_or(Time::MAX, |asset| asset.duration.max(Time::ZERO));
                let source_time = wanted.clamp(Time::ZERO, limit);
                LayerSource::Media {
                    asset_id: *asset_id,
                    source_time,
                    speed: if held && source_time != wanted {
                        0.0
                    } else {
                        clip.speed_at(self.at)
                    },
                }
            }
            ClipContent::Text {
                text,
                font_size,
                color,
                style,
                enter,
                exit,
                words,
            } => {
                // Steps once a frame: a paused viewer has nothing to redraw.
                let at = self.frame_local(clip);
                LayerSource::Text {
                    text: text.clone(),
                    font_size: *font_size,
                    color: *color,
                    style: style.clone(),
                    enter: title::entering(enter.as_ref(), at),
                    exit: title::leaving(exit.as_ref(), at, clip.duration),
                    words: style
                        .highlight
                        .as_ref()
                        .filter(|_| !words.is_empty())
                        .map(|highlight| title::word_state(words, highlight, at)),
                }
            }
            ClipContent::Graphic { size, elements } => {
                let at = self.frame_local(clip);
                LayerSource::Graphic {
                    size: *size,
                    elements: elements
                        .iter()
                        .map(|element| ElementLayer {
                            element: element.clone(),
                            enter: title::entering(element.enter.as_ref(), at),
                            exit: title::leaving(element.exit.as_ref(), at, clip.duration),
                        })
                        .collect(),
                }
            }
        };
        let tracks = draft
            .and_then(|o| o.animation.as_deref())
            .unwrap_or(&clip.animation);
        let transform = self.transform(clip, 0);
        let effects = draft
            .and_then(|o| o.effects.as_deref())
            .unwrap_or(&clip.effects);
        let masks = draft
            .and_then(|o| o.masks.as_deref())
            .unwrap_or(&clip.masks);
        Layer {
            clip: clip.id,
            track: track.id,
            source,
            transform,
            blend: draft.and_then(|o| o.blend).unwrap_or(clip.blend),
            effects: effects
                .iter()
                .filter(|effect| effect.enabled && effect.mix > 0.0)
                .map(|effect| resolve_effect(tracks, effect, local))
                .collect(),
            masks: masks
                .iter()
                .map(|mask| resolve_mask(tracks, mask, local))
                .collect(),
            transition,
            seed: seed(clip.id, self.frame),
            time: self.frame_local(clip),
        }
    }

    // The start of the canvas frame, from the clip's start.
    fn frame_local(&self, clip: &Clip) -> Time {
        self.project.settings.frame_rate.frame_time(self.frame) - clip.start
    }

    // The clip's transform now: its draft or stored one, animated, moved with
    // the tracker it follows, and back inside what a transform may be.
    // `depth` counts the followers this is worked out for: a clip that
    // follows a point on a clip that follows another is placed through both,
    // and past `FOLLOW_DEPTH` (clips following each other) a clip stays put.
    fn transform(&self, clip: &Clip, depth: u8) -> Transform {
        let draft = self.overrides.iter().find(|o| o.clip == clip.id);
        let base = draft
            .and_then(|o| o.transform.as_ref())
            .unwrap_or(&clip.transform);
        let tracks = draft
            .and_then(|o| o.animation.as_deref())
            .unwrap_or(&clip.animation);
        let mut transform = animate_transform(tracks, base, self.at - clip.start);
        if depth < FOLLOW_DEPTH {
            self.follow(clip, &mut transform, depth);
        }
        bound_transform(transform)
    }

    // Moves the layer with its tracker, read where the topmost other layer
    // showing the tracked asset is in its source, and put where that layer
    // draws that place of its picture. Inert when none is on screen.
    fn follow(&self, clip: &Clip, transform: &mut Transform, depth: u8) {
        let Some(follow) = &clip.follow else {
            return;
        };
        let Some(tracker) = self.project.trackers.get(&follow.tracker_id) else {
            return;
        };
        let Some(shown) = tracked_clip(self.project, self.at, clip.id, tracker) else {
            return;
        };
        let Some(key) = tracker.at(shown.source_time(self.at)) else {
            return;
        };
        if follow.position {
            let point = self.point(shown, key.center, depth + 1);
            for (axis, at) in point.into_iter().enumerate() {
                transform.position[axis] += at + follow.offset[axis];
            }
        }
        if follow.scale {
            transform.scale[0] *= key.scale;
            transform.scale[1] *= key.scale;
        }
        if follow.rotation {
            transform.rotation += key.rotation;
        }
    }

    // Where the place `source` of the picture of `shown` is on the canvas, in
    // canvas pixels from its centre.
    fn point(&self, shown: &Clip, source: [f32; 2], depth: u8) -> [f32; 2] {
        let settings = &self.project.settings;
        let size = (shown.asset_id())
            .and_then(|id| self.project.assets.get(&id))
            .map_or([0; 2], |asset| [asset.width, asset.height]);
        let reading = (self.overrides.iter())
            .find(|o| o.clip == shown.id)
            .and_then(|o| o.reading.as_ref());
        picture_point(
            [settings.width, settings.height],
            size,
            &self.transform(shown, depth),
            reading,
            source,
        )
    }
}

/// How many clips deep a followed point is placed through the clip it is on.
const FOLLOW_DEPTH: u8 = 4;

// Eased progress of a transition starting at `start`, while it is running.
fn progress(transition: &Transition, start: Time, at: Time) -> Option<f32> {
    let duration = transition.duration;
    if duration <= Time::ZERO || at < start || at >= start + duration {
        return None;
    }
    let linear = (at.0 - start.0) as f64 / duration.0 as f64;
    Some((transition.ease.eval(linear) as f32).clamp(0.0, 1.0))
}

fn transition_params(transition: &Transition) -> Vec<ParamValue> {
    look::transition_spec(transition.kind)
        .iter()
        .map(|spec| ParamValue {
            key: spec.key,
            value: bound(transition.param(spec.key).unwrap_or(spec.default), spec),
        })
        .collect()
}

fn animated(tracks: &[AnimTrack], target: Option<Uuid>, key: ParamKey, local: Time) -> Option<f32> {
    tracks
        .iter()
        .find(|track| track.target == target && track.key == key && !track.keys.is_empty())
        .map(|track| track.sample(local))
}

// Into the parameter's range; a value that is not a number takes the default.
fn bound(value: f32, spec: &ParamSpec) -> f32 {
    if value.is_nan() {
        spec.default
    } else {
        value.clamp(spec.min, spec.max)
    }
}

fn animate_transform(tracks: &[AnimTrack], base: &Transform, local: Time) -> Transform {
    let value = |key, stored| animated(tracks, None, key, local).unwrap_or(stored);
    Transform {
        position: [
            value(ParamKey::PositionX, base.position[0]),
            value(ParamKey::PositionY, base.position[1]),
        ],
        scale: [
            value(ParamKey::ScaleX, base.scale[0]),
            value(ParamKey::ScaleY, base.scale[1]),
        ],
        rotation: value(ParamKey::Rotation, base.rotation),
        opacity: value(ParamKey::Opacity, base.opacity),
        flip_x: base.flip_x,
        flip_y: base.flip_y,
        crop: [
            value(ParamKey::CropLeft, base.crop[0]),
            value(ParamKey::CropTop, base.crop[1]),
            value(ParamKey::CropRight, base.crop[2]),
            value(ParamKey::CropBottom, base.crop[3]),
        ],
    }
}

// Back inside `Transform::validate` after animation, follow or a raw draft.
fn bound_transform(mut transform: Transform) -> Transform {
    let specs = look::clip_spec();
    let fit = |key, value: &mut f32| {
        if let Some(spec) = specs.iter().find(|spec| spec.key == key) {
            *value = bound(*value, spec);
        }
    };
    let [x, y] = &mut transform.position;
    fit(ParamKey::PositionX, x);
    fit(ParamKey::PositionY, y);
    let [x, y] = &mut transform.scale;
    fit(ParamKey::ScaleX, x);
    fit(ParamKey::ScaleY, y);
    fit(ParamKey::Rotation, &mut transform.rotation);
    fit(ParamKey::Opacity, &mut transform.opacity);
    let [left, top, right, bottom] = &mut transform.crop;
    fit(ParamKey::CropLeft, left);
    fit(ParamKey::CropTop, top);
    fit(ParamKey::CropRight, right);
    fit(ParamKey::CropBottom, bottom);
    // Opposite crops must leave some of the picture.
    for (near, far) in [(0, 2), (1, 3)] {
        let sum = transform.crop[near] + transform.crop[far];
        if sum > 0.99 {
            transform.crop[near] *= 0.99 / sum;
            transform.crop[far] *= 0.99 / sum;
        }
    }
    transform
}

fn resolve_effect(tracks: &[AnimTrack], effect: &Effect, local: Time) -> ResolvedEffect {
    ResolvedEffect {
        id: effect.id,
        kind: effect.kind,
        mix: effect.mix.min(1.0),
        params: look::spec(effect.kind)
            .iter()
            .map(|spec| {
                let stored = effect.param(spec.key).unwrap_or(spec.default);
                let value = animated(tracks, Some(effect.id), spec.key, local).unwrap_or(stored);
                ParamValue {
                    key: spec.key,
                    value: bound(value, spec),
                }
            })
            .collect(),
        points: effect.points.clone(),
        asset_id: effect.asset_id,
    }
}

fn resolve_mask(tracks: &[AnimTrack], mask: &Mask, local: Time) -> Mask {
    let value = |key| {
        let stored = mask.param(key).unwrap_or(0.0);
        let value = animated(tracks, Some(mask.id), key, local).unwrap_or(stored);
        look::mask_spec()
            .iter()
            .find(|spec| spec.key == key)
            .map_or(value, |spec| bound(value, spec))
    };
    Mask {
        id: mask.id,
        shape: mask.shape,
        op: mask.op,
        invert: mask.invert,
        center: [value(ParamKey::CenterX), value(ParamKey::CenterY)],
        size: [value(ParamKey::Width), value(ParamKey::Height)],
        rotation: value(ParamKey::Rotation),
        feather: value(ParamKey::Feather),
        roundness: value(ParamKey::Roundness),
    }
}

/// Where `tracker`'s point is at timeline time `at`, for the layer of
/// `follower`: read where the topmost other layer showing the tracked asset
/// is in its source. None when no such layer is on screen.
pub fn tracked_point(
    project: &Project,
    at: Time,
    follower: ClipId,
    tracker: &Tracker,
) -> Option<TrackKey> {
    tracked_clip(project, at, follower, tracker).and_then(|shown| tracker.at(shown.source_time(at)))
}

/// Where `tracker`'s point is on the canvas at timeline time `at`, for the
/// layer of `follower`, in canvas pixels from the canvas centre: where the
/// layer it is read from draws that place of its picture, with `overrides`
/// in the place of what they hold. None when no such layer is on screen.
pub fn tracked_place(
    project: &Project,
    at: Time,
    follower: ClipId,
    tracker: &Tracker,
    overrides: &[ClipOverride],
) -> Option<[f32; 2]> {
    let shown = tracked_clip(project, at, follower, tracker)?;
    let key = tracker.at(shown.source_time(at))?;
    let cx = Context {
        project,
        at,
        frame: project.settings.frame_rate.frame_at(at),
        overrides,
    };
    Some(cx.point(shown, key.center, 1))
}

/// The clip a tracker's point is read from at `at`, for the layer of
/// `follower`: the topmost other one on screen that shows the tracked asset.
pub fn tracked_clip<'a>(
    project: &'a Project,
    at: Time,
    follower: ClipId,
    tracker: &Tracker,
) -> Option<&'a Clip> {
    project
        .tracks
        .iter()
        .rev()
        .filter(|track| !track.hidden && track.kind != TrackKind::Audio)
        .find_map(|track| {
            track
                .active_clips(at)
                .filter(|c| c.id != follower && c.asset_id() == Some(tracker.asset_id))
                .last()
        })
}

// FNV-1a over the clip ID and the canvas frame index: the same on every host.
fn seed(clip: ClipId, frame: i64) -> u64 {
    clip.as_bytes()
        .iter()
        .chain(&frame.to_le_bytes())
        .fold(0xcbf2_9ce4_8422_2325, |hash: u64, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}
