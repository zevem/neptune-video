//! What a pass of the compositor is told: one uniform record, laid out as
//! `shaders/common.wgsl` declares it, and the passes each effect is drawn in.
//!
//! Nothing here touches a device, so the packing and the chain of a layer
//! are tested without one. What a pass's numbers mean is said where it is
//! drawn, in `shaders/effect.wgsl`; what is worked out here is whatever a
//! shader cannot: pixels from fractions, the slopes of a curve, and the
//! noise that moves with time.
use super::Placement;
use editor_core::{
    BlendMode, ClipId, EffectKind, Layer, Mask, MaskOp, MaskShape, ParamKey, ResolvedEffect,
    ResolvedTransition, Transform, TransitionKind, look,
};
use editor_media::Sampling;
use uuid::Uuid;

/// The vectors of a pass's own in a [`Uniform`].
pub const VECTORS: usize = 28;
/// Masks a layer may have: `editor_core` holds a clip to as many.
pub const MASKS: usize = 8;

/// The targets a layer's chain may go between: the layer so far and two to
/// work on, for an effect that needs the layer again once it has blurred it.
pub const SLOTS: usize = 3;

/// The operation of an effect pass: `OP_*` in `shaders/effect.wgsl`, where
/// nought is the pass that changes nothing.
pub const OP_MASK: u32 = 1;
pub const OP_GRADE: u32 = 2;
pub const OP_HSL_BAND: u32 = 3;
pub const OP_CURVE: u32 = 4;
pub const OP_CHROMA_KEY: u32 = 5;
pub const OP_DIFFERENCE_MATTE: u32 = 6;
pub const OP_GRAIN: u32 = 7;
pub const OP_VIGNETTE: u32 = 8;
pub const OP_SHAKE: u32 = 9;
pub const OP_DISTORT: u32 = 10;
pub const OP_GLITCH: u32 = 11;
pub const OP_STREAK: u32 = 12;
pub const OP_SHRINK: u32 = 13;
pub const OP_GAUSS: u32 = 14;
pub const OP_SOFT: u32 = 15;
pub const OP_BRIGHT: u32 = 16;
pub const OP_LUT: u32 = 17;

/// The variants of [`OP_SOFT`]: what is made of the layer and its blur.
const SOFT_BLUR: u32 = 0;
const SOFT_GLOW: u32 = 1;
const SOFT_SHARPEN: u32 = 2;
const SOFT_SHADOW: u32 = 3;

/// What a transition pass makes of its two layers: `KIND_*` in
/// `shaders/transition.wgsl`.
pub const KIND_DISSOLVE: u32 = 1;
pub const KIND_DIP: u32 = 2;
pub const KIND_SLIDE: u32 = 3;
pub const KIND_ZOOM: u32 = 4;
pub const KIND_WIPE: u32 = 5;
pub const KIND_GLITCH: u32 = 6;

/// The widest a gaussian is drawn at, in texels of the picture it is drawn
/// over: a wider one is drawn over a smaller picture.
const WIDEST: f32 = 4.0;
/// The samples of one streak pass.
const STREAK: f32 = 24.0;

/// One pass's uniform record. Every field is four wide, as in the shader.
#[derive(Clone, Debug, PartialEq)]
pub struct Uniform {
    op: [u32; 4],
    size: [f32; 4],
    seed: [f32; 4],
    p: [[f32; 4]; VECTORS],
}

impl Uniform {
    pub const BYTES: usize = 16 * (3 + VECTORS);

    /// A pass over a target of `size` pixels that mixes its result in whole
    /// and draws at full opacity.
    pub fn new([width, height]: [u32; 2]) -> Self {
        let (width, height) = (width.max(1) as f32, height.max(1) as f32);
        Self {
            op: [0; 4],
            size: [width, height, 1.0 / width, 1.0 / height],
            seed: [0.0, 0.0, 1.0, 1.0],
            p: [[0.0; 4]; VECTORS],
        }
    }

    pub fn op(mut self, op: u32) -> Self {
        self.op[0] = op;
        self
    }

    pub fn variant(mut self, variant: u32) -> Self {
        self.op[1] = variant;
        self
    }

    pub fn count(mut self, count: usize) -> Self {
        self.op[2] = count as u32;
        self
    }

    /// Two numbers in `0..1` from the layer's seed: 24 bits each.
    pub fn seed(mut self, seed: u64) -> Self {
        let part = |bits: u64| (bits & 0x00ff_ffff) as f32 / 16_777_216.0;
        self.seed[0] = part(seed);
        self.seed[1] = part(seed >> 24);
        self
    }

    pub fn mix(mut self, mix: f32) -> Self {
        self.seed[2] = mix;
        self
    }

    pub fn opacity(mut self, opacity: f32) -> Self {
        self.seed[3] = opacity;
        self
    }

    /// Says that `second` holds the layer as the effect found it: what the
    /// pass's result is mixed over.
    pub fn over_second(mut self) -> Self {
        self.op[3] = 1;
        self
    }

    pub fn set(mut self, index: usize, vector: [f32; 4]) -> Self {
        self.p[index] = vector;
        self
    }

    /// Scalars four to a vector from `p[from]` on, in the order given: how
    /// an effect's parameters reach its shader function.
    pub fn scalars(mut self, from: usize, values: impl IntoIterator<Item = f32>) -> Self {
        for (index, value) in values.into_iter().take((VECTORS - from) * 4).enumerate() {
            self.p[from + index / 4][index % 4] = value;
        }
        self
    }

    /// The box a layer is placed in: `p[0]` and `p[1]` of the passes that
    /// need to know where the layer is.
    pub fn place(self, place: &Placement) -> Self {
        self.place_at(0, place)
    }

    /// The same box in `p[index]` and the vector after it.
    pub fn place_at(self, index: usize, place: &Placement) -> Self {
        let (sin, cos) = place.angle.sin_cos();
        let [x, y] = place.center;
        let [across, down] = place.half;
        self.set(index, [x, y, across, down])
            .set(index + 1, [cos, sin, 0.0, 0.0])
    }

    pub fn bytes(&self) -> [u8; Self::BYTES] {
        let mut bytes = [0u8; Self::BYTES];
        let words = (self.op.iter().map(|word| word.to_ne_bytes()))
            .chain(self.size.iter().map(|word| word.to_ne_bytes()))
            .chain(self.seed.iter().map(|word| word.to_ne_bytes()))
            .chain(self.p.iter().flatten().map(|word| word.to_ne_bytes()));
        for (to, word) in bytes.chunks_exact_mut(4).zip(words) {
            to.copy_from_slice(&word);
        }
        bytes
    }
}

/// The blend pass's variant: the order of the `case`s in `blend.wgsl`.
pub fn blend_mode(mode: BlendMode) -> u32 {
    match mode {
        BlendMode::Normal => 0,
        BlendMode::Multiply => 1,
        BlendMode::Screen => 2,
        BlendMode::Overlay => 3,
        BlendMode::SoftLight => 4,
        BlendMode::Add => 5,
        BlendMode::Darken => 6,
        BlendMode::Lighten => 7,
        BlendMode::Difference => 8,
    }
}

/// What a pass is given as `second`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Second {
    Blank,
    /// One of the chain's own targets.
    Slot(usize),
    /// An uploaded picture, by its serial: the clean plate of a matte.
    Plate(u64),
    /// An uploaded lookup table, by its serial.
    Lut(u64),
}

/// What a pass is told of the lookup table it is given as `second`: its
/// name, its points along an edge, the tiles in a row of its texture (see
/// `lut::layout`) and the inputs it covers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Table {
    pub serial: u64,
    pub size: u32,
    pub across: u32,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
}

/// One pass of a layer's chain: from the target `first` to the target
/// `into`, both among the chain's [`SLOTS`]. The placed layer starts in
/// slot nought.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub uniform: Uniform,
    pub first: usize,
    pub second: Second,
    pub into: usize,
}

/// The picture's uv at the left top and right bottom of a layer's box, as
/// its crop and flips leave them.
pub fn corners(transform: &Transform) -> [f32; 4] {
    editor_core::render::corners(transform)
}

/// The pass that places a layer's picture in its box on a target of `size`
/// pixels: `shaders/place.wgsl`. A stabilized layer whose camera's path is
/// known reads its picture through `steady`; any other reads it as it is.
pub fn placed(
    size: [u32; 2],
    place: &Placement,
    transform: &Transform,
    steady: Option<&Sampling>,
) -> Uniform {
    let steady = steady.unwrap_or(&Sampling::IDENTITY);
    let [x, y] = steady.offset;
    (Uniform::new(size).place(place))
        .set(2, corners(transform))
        .set(3, steady.matrix)
        .set(4, [x, y, 0.0, 0.0])
}

/// The slope of a curve at each of its points, so that the cubic between two
/// of them never turns back: the weighted harmonic mean of the slopes of the
/// straight lines either side, nought where they disagree, and the line's
/// own slope at the ends.
pub fn slopes(points: &[[f32; 2]]) -> Vec<f32> {
    let line = |k: usize| {
        let (h, rise) = (
            points[k + 1][0] - points[k][0],
            points[k + 1][1] - points[k][1],
        );
        (h.max(f32::EPSILON), rise / h.max(f32::EPSILON))
    };
    (0..points.len())
        .map(|k| {
            if points.len() < 2 {
                0.0
            } else if k == 0 {
                line(0).1
            } else if k + 1 == points.len() {
                line(k - 1).1
            } else {
                let ((h0, d0), (h1, d1)) = (line(k - 1), line(k));
                if d0 * d1 <= 0.0 {
                    0.0
                } else {
                    3.0 * (h0 + h1) / ((2.0 * h1 + h0) / d0 + (h1 + 2.0 * h0) / d1)
                }
            }
        })
        .collect()
}

/// The value of a curve at `v`, as `effect.wgsl` draws it from the same
/// points and `slopes`: flat outside the points it has, and between two of
/// them the cubic that leaves and arrives at their slopes. What draws a curve
/// for a person to edit draws it with this.
pub fn curve_at(points: &[[f32; 2]], slopes: &[f32], v: f32) -> f32 {
    let Some(last) = points.len().checked_sub(1).filter(|last| *last > 0) else {
        return v;
    };
    if v <= points[0][0] {
        return points[0][1];
    }
    if v >= points[last][0] {
        return points[last][1];
    }
    let k = (1..last).rev().find(|k| v >= points[*k][0]).unwrap_or(0);
    let h = (points[k + 1][0] - points[k][0]).max(0.000_001);
    let t = (v - points[k][0]) / h;
    let (t2, t3) = (t * t, t * t * t);
    (2.0 * t3 - 3.0 * t2 + 1.0) * points[k][1]
        + (t3 - 2.0 * t2 + t) * h * slopes[k]
        + (3.0 * t2 - 2.0 * t3) * points[k + 1][1]
        + (t3 - t2) * h * slopes[k + 1]
}

/// A number for a clip, a moment and a purpose, the same on every host:
/// FNV-1a, then stirred so that neighbouring moments are unlike.
fn drawn(clip: ClipId, knot: i64, purpose: u8) -> u64 {
    let hash = (clip.as_bytes().iter())
        .chain(&knot.to_le_bytes())
        .chain(&[purpose])
        .fold(0xcbf2_9ce4_8422_2325, |hash: u64, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        });
    let hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    let hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    hash ^ (hash >> 31)
}

/// A value in `-1..=1` that wanders as `moment` goes on: a new one drawn at
/// every whole moment, eased between.
fn wander(clip: ClipId, moment: f64, purpose: u8) -> f32 {
    let knot = moment.floor();
    let at = |knot: f64| (drawn(clip, knot as i64, purpose) >> 40) as f64 / 8_388_608.0 - 1.0;
    let t = moment - knot;
    let t = t * t * (3.0 - 2.0 * t);
    (at(knot) + (at(knot + 1.0) - at(knot)) * t) as f32
}

/// The mask pass of a layer placed at `place`: every mask in one pass, so
/// that they combine before any of them cuts the picture.
fn masks(pass: Uniform, masks: &[Mask], place: &Placement) -> Uniform {
    let mut pass = pass.op(OP_MASK).place(place).count(masks.len().min(MASKS));
    for (index, mask) in masks.iter().take(MASKS).enumerate() {
        let (sin, cos) = mask.rotation.to_radians().sin_cos();
        let [x, y] = mask.center;
        let [width, height] = mask.size;
        let shape = match mask.shape {
            MaskShape::Rectangle => 0.0,
            MaskShape::Ellipse => 1.0,
        };
        let op = match mask.op {
            MaskOp::Add => 0.0,
            MaskOp::Subtract => 1.0,
            MaskOp::Intersect => 2.0,
        };
        let invert = f32::from(u8::from(mask.invert));
        pass = pass
            .set(2 + 3 * index, [x, y, width, height])
            .set(3 + 3 * index, [cos, sin, mask.feather, mask.roundness])
            .set(4 + 3 * index, [shape, op, invert, 0.0]);
    }
    pass
}

/// One layer of a transition pass, as the pass needs to know it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Side {
    pub opacity: f32,
    /// The centre of the layer's box in target pixels: what a zoom is about.
    pub centre: [f32; 2],
}

/// Whether `below` and `above` are the two layers of one transition: the
/// clip that is over and, directly over it, the clip it leads into.
pub fn paired(below: &Layer, above: &Layer) -> bool {
    let (Some(out), Some(into)) = (&below.transition, &above.transition) else {
        return false;
    };
    out.outgoing
        && !into.outgoing
        && below.track == above.track
        && out.partner == Some(above.clip)
        && into.partner == Some(below.clip)
}

/// The pass that makes one layer of the two of a transition, or of the one
/// there is: `outgoing` is given as `first` and `incoming` as `second`, and
/// a side that is not there is blank. What each kind draws is said in
/// `shaders/transition.wgsl`. `seed` is that of the layer the transition is
/// on.
///
/// A fade between two clips dips through nothing; a fade with no partner has
/// nothing to dip to and is the dissolve from or to nothing that every other
/// kind also is at its ends. A blur is a dissolve of two layers whose chains
/// were given a blur: see [`chain_with`].
pub fn transition(
    size: [u32; 2],
    transition: &ResolvedTransition,
    seed: u64,
    outgoing: Option<Side>,
    incoming: Option<Side>,
) -> Uniform {
    let progress = transition.progress.clamp(0.0, 1.0);
    let value = |key: ParamKey| {
        (transition.params.iter())
            .find(|param| param.key == key)
            .map(|param| param.value)
            .or_else(|| {
                (look::transition_spec(transition.kind).iter())
                    .find(|spec| spec.key == key)
                    .map(|spec| spec.default)
            })
            .unwrap_or(0.0)
    };
    let grown = 1.0 + value(ParamKey::Amount).max(0.0);
    let zoomed = 1.0 + value(ParamKey::Amount).max(0.0) * progress;
    // The kind, its parameter, the way it moves and the scale of each side.
    let (kind, param, toward, scale) = match transition.kind {
        TransitionKind::Fade if transition.partner.is_some() => (KIND_DIP, 0.0, [0.0; 2], [1.0; 2]),
        TransitionKind::Fade | TransitionKind::Dissolve | TransitionKind::Blur => {
            (KIND_DISSOLVE, 0.0, [0.0; 2], [1.0; 2])
        }
        TransitionKind::SlideLeft => (KIND_SLIDE, 0.0, [-1.0, 0.0], [1.0; 2]),
        TransitionKind::SlideRight => (KIND_SLIDE, 0.0, [1.0, 0.0], [1.0; 2]),
        TransitionKind::SlideUp => (KIND_SLIDE, 0.0, [0.0, -1.0], [1.0; 2]),
        TransitionKind::SlideDown => (KIND_SLIDE, 0.0, [0.0, 1.0], [1.0; 2]),
        // Both grow by the same step, as one picture pushed in to: the
        // outgoing from its size, the incoming up to its size.
        TransitionKind::ZoomIn => (KIND_ZOOM, 0.0, [0.0; 2], [zoomed, zoomed / grown]),
        TransitionKind::ZoomOut => (KIND_ZOOM, 0.0, [0.0; 2], [1.0 / zoomed, grown / zoomed]),
        TransitionKind::WipeLeft => (KIND_WIPE, value(ParamKey::Softness), [-1.0, 0.0], [1.0; 2]),
        TransitionKind::WipeRight => (KIND_WIPE, value(ParamKey::Softness), [1.0, 0.0], [1.0; 2]),
        TransitionKind::WipeUp => (KIND_WIPE, value(ParamKey::Softness), [0.0, -1.0], [1.0; 2]),
        TransitionKind::WipeDown => (KIND_WIPE, value(ParamKey::Softness), [0.0, 1.0], [1.0; 2]),
        TransitionKind::Glitch => (KIND_GLITCH, value(ParamKey::Amount), [0.0; 2], [1.0; 2]),
    };
    let opacity = |side: Option<Side>| side.map_or(0.0, |side| side.opacity.clamp(0.0, 1.0));
    let centre = |side: Option<Side>| side.map_or([0.0; 2], |side| side.centre);
    let (from, to) = (centre(outgoing), centre(incoming));
    Uniform::new(size)
        .op(kind)
        .seed(seed)
        .set(0, [progress, param, opacity(outgoing), opacity(incoming)])
        .set(1, [toward[0], toward[1], scale[0], scale[1]])
        .set(2, [from[0], from[1], to[0], to[1]])
}

/// The deviation, in pixels of a target `height` high, of the blur a blur
/// transition gives both its layers: none at either end and the most half
/// way, where it is half of `Radius` as an effect's is.
fn transition_blur(layer: &Layer, height: f32) -> f32 {
    let Some(transition) =
        (layer.transition.as_ref()).filter(|transition| transition.kind == TransitionKind::Blur)
    else {
        return 0.0;
    };
    let radius = (transition.params.iter())
        .find(|param| param.key == ParamKey::Radius)
        .map_or(0.0, |param| param.value);
    let progress = transition.progress.clamp(0.0, 1.0);
    0.5 * radius.max(0.0) * height * (1.0 - (2.0 * progress - 1.0).abs())
}

fn blurs(layer: &Layer) -> bool {
    (layer.transition.as_ref()).is_some_and(|transition| transition.kind == TransitionKind::Blur)
}

/// A layer's chain as it is built: the passes so far and the slot the layer
/// is in after them.
struct Chain<'a> {
    layer: &'a Layer,
    place: &'a Placement,
    size: [u32; 2],
    links: Vec<Link>,
    cur: usize,
}

impl Chain<'_> {
    /// A pass over this chain's target that mixes its result in whole.
    fn pass(&self) -> Uniform {
        Uniform::new(self.size).seed(self.layer.seed)
    }

    /// The lowest slot that holds neither the layer nor `other`.
    fn spare(&self, other: usize) -> usize {
        (0..SLOTS)
            .find(|slot| *slot != self.cur && *slot != other)
            .unwrap_or(0)
    }

    /// One pass from the layer to the layer.
    fn single(&mut self, uniform: Uniform, second: Second) {
        let into = self.spare(self.cur);
        self.links.push(Link {
            uniform,
            first: self.cur,
            second,
            into,
        });
        self.cur = into;
    }

    /// A pass from `from` to a slot the layer is not in, which it returns.
    fn aside(&mut self, from: usize, uniform: Uniform) -> usize {
        let into = self.spare(from);
        self.links.push(Link {
            uniform,
            first: from,
            second: Second::Blank,
            into,
        });
        into
    }

    /// The passes that blur what is in `from` by a gaussian of `sigma`
    /// pixels and make `last` of it and the layer: `last` is given the blur,
    /// small, as `first` and the layer as `second`, and what it draws is the
    /// layer from then on.
    ///
    /// A gaussian wider than [`WIDEST`] is drawn over the picture shrunk by
    /// a power of two into the left top of a target, and enlarged again by
    /// `last`: twelve texels either side then reach three deviations.
    fn soften(&mut self, mut from: usize, sigma: f32, last: Uniform) {
        let mut side = 1u32;
        while sigma / side as f32 > WIDEST && side < 64 {
            side *= 2;
        }
        let small = [
            self.size[0].div_ceil(side) as f32,
            self.size[1].div_ceil(side) as f32,
        ];
        let shrunk = [side as f32, small[0], small[1], 0.0];
        if side > 1 {
            from = self.aside(from, self.pass().op(OP_SHRINK).set(4, shrunk));
        }
        // A deviation of nothing would divide by it.
        let sigma = (sigma / side as f32).max(0.1);
        let reach = (3.0 * sigma).ceil().clamp(1.0, 12.0);
        for axis in 0..2 {
            let gauss = (self.pass().op(OP_GAUSS).variant(axis))
                .set(4, shrunk)
                .set(5, [sigma, reach, 0.0, 0.0]);
            from = self.aside(from, gauss);
        }
        let into = self.spare(from);
        self.links.push(Link {
            uniform: last.op(OP_SOFT).over_second().set(4, shrunk),
            first: from,
            second: Second::Slot(self.cur),
            into,
        });
        self.cur = into;
    }

    /// The passes of one effect, if it is one that is drawn and does
    /// anything as it is set.
    fn effect(&mut self, effect: &ResolvedEffect, plates: &[(Uuid, u64)], luts: &[(Uuid, Table)]) {
        let spec = look::spec(effect.kind);
        let value = |key: ParamKey| {
            effect
                .param(key)
                .or_else(|| spec.iter().find(|s| s.key == key).map(|s| s.default))
                .unwrap_or(0.0)
        };
        // Its parameters in the order of its table, from `p[0]`.
        let pass = (self.pass().mix(effect.mix)).scalars(0, spec.iter().map(|s| value(s.key)));
        let height = self.size[1] as f32;
        let amount = value(ParamKey::Amount);
        // Half the radius is the deviation: the blur all but ends a radius
        // and a half away.
        let sigma = 0.5 * value(ParamKey::Radius) * height;
        match effect.kind {
            EffectKind::Grade => self.single(pass.op(OP_GRADE), Second::Blank),
            EffectKind::HslBand => self.single(pass.op(OP_HSL_BAND), Second::Blank),
            EffectKind::CurveMaster
            | EffectKind::CurveRed
            | EffectKind::CurveGreen
            | EffectKind::CurveBlue => {
                let points = &effect.points[..effect.points.len().min(look::MAX_CURVE_POINTS)];
                if points.len() < 2 {
                    return;
                }
                let channels = match effect.kind {
                    EffectKind::CurveRed => 1,
                    EffectKind::CurveGreen => 2,
                    EffectKind::CurveBlue => 4,
                    _ => 7,
                };
                let mut pass = pass.op(OP_CURVE).variant(channels).count(points.len());
                for (index, (point, slope)) in points.iter().zip(slopes(points)).enumerate() {
                    pass = pass.set(index, [point[0], point[1], slope, 0.0]);
                }
                self.single(pass, Second::Blank);
            }
            EffectKind::ChromaKey => {
                let key = [ParamKey::KeyR, ParamKey::KeyG, ParamKey::KeyB].map(value);
                let strongest = (0..3).fold(0, |best, c| if key[c] > key[best] { c } else { best });
                let mut own = [0.0; 4];
                own[strongest] = 1.0;
                self.single(pass.op(OP_CHROMA_KEY).set(4, own), Second::Blank);
            }
            EffectKind::DifferenceMatte => {
                // Without its clean plate there is nothing to differ from.
                let Some((_, plate)) = plates.iter().find(|(id, _)| *id == effect.id) else {
                    return;
                };
                let pass = (pass.op(OP_DIFFERENCE_MATTE))
                    .place_at(4, self.place)
                    .set(6, corners(&self.layer.transform));
                self.single(pass, Second::Plate(*plate));
            }
            EffectKind::Lut => {
                // Until its table has come, a lookup changes nothing.
                let Some((_, table)) = luts.iter().find(|(id, _)| *id == effect.id) else {
                    return;
                };
                let [r, g, b] = table.domain_min;
                let per = [0, 1, 2]
                    .map(|c| 1.0 / (table.domain_max[c] - table.domain_min[c]).max(f32::EPSILON));
                let pass = (pass.op(OP_LUT))
                    .variant(table.across.max(1))
                    .count(table.size as usize)
                    .set(4, [r, g, b, 0.0])
                    .set(5, [per[0], per[1], per[2], 0.0]);
                self.single(pass, Second::Lut(table.serial));
            }
            EffectKind::Grain if amount > 0.0 => {
                let cell = (value(ParamKey::Radius) * height).max(1.0);
                self.single(
                    pass.op(OP_GRAIN).set(4, [cell, 0.0, 0.0, 0.0]),
                    Second::Blank,
                );
            }
            EffectKind::Vignette if amount > 0.0 => {
                self.single(pass.op(OP_VIGNETTE).place_at(4, self.place), Second::Blank);
            }
            EffectKind::Shake => {
                let turn = value(ParamKey::Angle);
                if amount <= 0.0 && turn <= 0.0 {
                    return;
                }
                // A new place is drawn `Frequency` times a second of the
                // clip, and the layer eases from one to the next.
                let moment =
                    self.layer.time.0 as f64 / 1_000_000.0 * f64::from(value(ParamKey::Frequency));
                let clip = self.layer.clip;
                let reach = amount * height;
                let (sin, cos) = (turn * wander(clip, moment, 2)).to_radians().sin_cos();
                let moved = [
                    reach * wander(clip, moment, 0),
                    reach * wander(clip, moment, 1),
                    cos,
                    sin,
                ];
                self.single(
                    (pass.op(OP_SHAKE).place_at(4, self.place)).set(6, moved),
                    Second::Blank,
                );
            }
            EffectKind::Distort if amount > 0.0 => {
                let (sin, cos) = value(ParamKey::Angle).to_radians().sin_cos();
                self.single(
                    (pass.op(OP_DISTORT).place_at(4, self.place)).set(6, [cos, sin, 0.0, 0.0]),
                    Second::Blank,
                );
            }
            EffectKind::Glitch if amount > 0.0 => {
                // One pattern for each stretch of the clip's time.
                let moment =
                    self.layer.time.0 as f64 / 1_000_000.0 * f64::from(value(ParamKey::Frequency));
                let salt = Uniform::new(self.size)
                    .seed(drawn(self.layer.clip, moment.floor() as i64, 3))
                    .seed;
                self.single(
                    pass.op(OP_GLITCH).set(4, [salt[0], salt[1], 0.0, 0.0]),
                    Second::Blank,
                );
            }
            EffectKind::MotionBlur if amount > 0.0 => {
                // A blur along one direction of what is on the canvas, as
                // long as asked: not the frames before and after, which are
                // never accumulated.
                let (sin, cos) = value(ParamKey::Angle).to_radians().sin_cos();
                let length = amount * height;
                let step = |by: f32| [cos * length / by, sin * length / by, 0.0, 0.0];
                if length <= STREAK {
                    self.single(pass.op(OP_STREAK).set(4, step(STREAK)), Second::Blank);
                    return;
                }
                // Long steps, then short ones that fill what is between.
                let far = self.pass().op(OP_STREAK).set(4, step(STREAK));
                let from = self.aside(self.cur, far);
                let into = self.spare(from);
                self.links.push(Link {
                    uniform: (pass.op(OP_STREAK).over_second()).set(4, step(STREAK * STREAK)),
                    first: from,
                    second: Second::Slot(self.cur),
                    into,
                });
                self.cur = into;
            }
            EffectKind::Blur if sigma > 0.0 => {
                self.soften(self.cur, sigma, pass.variant(SOFT_BLUR));
            }
            EffectKind::Glow if amount > 0.0 => {
                let lit = self.aside(
                    self.cur,
                    self.pass()
                        .op(OP_BRIGHT)
                        .scalars(0, [0.0, 0.0, value(ParamKey::Threshold)]),
                );
                self.soften(lit, sigma, pass.variant(SOFT_GLOW));
            }
            // Half a pixel at the least, or a small preview would show no
            // sharpening at all.
            EffectKind::Sharpen if amount > 0.0 => {
                self.soften(self.cur, sigma.max(0.5), pass.variant(SOFT_SHARPEN));
            }
            EffectKind::Shadow if amount > 0.0 => {
                let offset = [ParamKey::OffsetX, ParamKey::OffsetY].map(|key| value(key) * height);
                self.soften(
                    self.cur,
                    sigma,
                    pass.variant(SOFT_SHADOW)
                        .set(6, [offset[0], offset[1], 0.0, 0.0]),
                );
            }
            // Set to do nothing, or drawn by no pass of the chain: a
            // stabilized picture is steadied where it is placed, see
            // [`placed`].
            _ => {}
        }
    }
}

/// The most passes an effect of `kind` is drawn in.
fn most(kind: EffectKind) -> usize {
    match kind {
        EffectKind::Glow => 5,
        EffectKind::Blur | EffectKind::Sharpen | EffectKind::Shadow => 4,
        EffectKind::MotionBlur => 2,
        EffectKind::Stabilize => 0,
        // Heard, not drawn.
        kind if kind.is_audio() => 0,
        _ => 1,
    }
}

/// The passes between placing `layer` and blending it, in order: its effects
/// as stacked, each over what the ones before it made, then its masks over
/// the result of them all. `plates` names the uploaded clean plate of each
/// difference matte by the effect's ID, and `luts` the table of each lookup
/// that has one; a matte without its plate and a lookup without its table
/// take no pass. A blur transition blurs the layer last of all; the pass
/// that joins the two layers of a transition is not of either's chain: see
/// [`transition`].
///
/// The layer is in the slot the last pass draws into, or in slot nought.
pub fn chain_with(
    layer: &Layer,
    place: &Placement,
    size: [u32; 2],
    plates: &[(Uuid, u64)],
    luts: &[(Uuid, Table)],
) -> Vec<Link> {
    let mut chain = Chain {
        layer,
        place,
        size,
        links: Vec::new(),
        cur: 0,
    };
    for effect in &layer.effects {
        chain.effect(effect, plates, luts);
    }
    if !layer.masks.is_empty() {
        let pass = masks(chain.pass(), &layer.masks, place);
        chain.single(pass, Second::Blank);
    }
    let sigma = transition_blur(layer, size[1] as f32);
    if sigma > 0.0 {
        let pass = chain.pass().variant(SOFT_BLUR);
        chain.soften(chain.cur, sigma, pass);
    }
    chain.links
}

/// The chain of a layer that looks nothing up in a table: what the tests of
/// every other effect build.
#[cfg(test)]
pub fn chain(
    layer: &Layer,
    place: &Placement,
    size: [u32; 2],
    plates: &[(Uuid, u64)],
) -> Vec<Link> {
    chain_with(layer, place, size, plates, &[])
}

/// The most passes [`chain_with`] comes to, without building them: an effect set
/// to do nothing, a blur narrow enough to need no smaller picture, a matte
/// without its plate and a lookup without its table take fewer.
pub fn chain_len(layer: &Layer) -> usize {
    let effects: usize = layer.effects.iter().map(|effect| most(effect.kind)).sum();
    let blur = if blurs(layer) {
        most(EffectKind::Blur)
    } else {
        0
    };
    effects + usize::from(!layer.masks.is_empty()) + blur
}

/// The targets a layer's chain goes between, the one it is placed on among
/// them: one with no chain, two to go to and fro on, and all [`SLOTS`] for
/// an effect that blurs.
pub fn chain_slots(layer: &Layer) -> usize {
    if blurs(layer) || (layer.effects.iter()).any(|effect| most(effect.kind) > 1) {
        SLOTS
    } else if chain_len(layer) > 0 {
        2
    } else {
        1
    }
}

/// The targets a layer drawn by itself needs: those of its chain, and one
/// to draw its transition into when it has one and no chain made room.
pub fn slots(layer: &Layer) -> usize {
    chain_slots(layer).max(1 + usize::from(layer.transition.is_some()))
}

/// The targets the two layers of a transition need between them: each
/// chain's own, since the first layer waits while the second is drawn, and
/// one that neither ends in for the pass that joins them.
pub fn pair_slots(below: &Layer, above: &Layer) -> usize {
    (chain_slots(below) + chain_slots(above)).max(3)
}
