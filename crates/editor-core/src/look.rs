use crate::{AssetId, Ease, ParamKey, Time};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MAX_EFFECTS: usize = 16;
pub const MAX_MASKS: usize = 8;
pub const MAX_CURVE_POINTS: usize = 16;
pub const MAX_TRACKERS: usize = 64;
pub const MAX_TRACK_KEYS: usize = 2048;
pub const MAX_TRANSITION: Time = Time(10_000_000);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    SoftLight,
    Add,
    Darken,
    Lighten,
    Difference,
}
impl BlendMode {
    pub fn is_normal(&self) -> bool {
        *self == Self::Normal
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    Glow,
    Shake,
    Distort,
    MotionBlur,
    Grain,
    Sharpen,
    Shadow,
    Blur,
    Vignette,
    Glitch,
    Grade,
    HslBand,
    CurveMaster,
    CurveRed,
    CurveGreen,
    CurveBlue,
    Lut,
    ChromaKey,
    DifferenceMatte,
    Stabilize,
}
impl EffectKind {
    pub const ALL: [Self; 20] = [
        Self::Glow,
        Self::Shake,
        Self::Distort,
        Self::MotionBlur,
        Self::Grain,
        Self::Sharpen,
        Self::Shadow,
        Self::Blur,
        Self::Vignette,
        Self::Glitch,
        Self::Grade,
        Self::HslBand,
        Self::CurveMaster,
        Self::CurveRed,
        Self::CurveGreen,
        Self::CurveBlue,
        Self::Lut,
        Self::ChromaKey,
        Self::DifferenceMatte,
        Self::Stabilize,
    ];
    pub fn is_curve(self) -> bool {
        matches!(
            self,
            Self::CurveMaster | Self::CurveRed | Self::CurveGreen | Self::CurveBlue
        )
    }
    pub fn needs_asset(self) -> bool {
        matches!(self, Self::Lut | Self::DifferenceMatte)
    }
}

/// One parameter of an effect, mask, transition or clip: its inclusive range
/// and the value used when none is stored.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamSpec {
    pub key: ParamKey,
    pub min: f32,
    pub max: f32,
    pub default: f32,
}
impl ParamSpec {
    pub fn contains(&self, value: f32) -> bool {
        // A NaN compares false and is rejected with every other out-of-range value.
        value >= self.min && value <= self.max
    }
}

const fn p(key: ParamKey, min: f32, max: f32, default: f32) -> ParamSpec {
    ParamSpec {
        key,
        min,
        max,
        default,
    }
}

// Lengths are fractions of the layer box or canvas height; angles are degrees.
const GLOW: &[ParamSpec] = &[
    p(ParamKey::Amount, 0.0, 4.0, 0.5),
    p(ParamKey::Radius, 0.0, 0.25, 0.02),
    p(ParamKey::Threshold, 0.0, 1.0, 0.6),
];
const SHAKE: &[ParamSpec] = &[
    p(ParamKey::Amount, 0.0, 0.25, 0.01),
    p(ParamKey::Frequency, 0.1, 60.0, 8.0),
    p(ParamKey::Angle, 0.0, 45.0, 0.0),
];
const DISTORT: &[ParamSpec] = &[
    p(ParamKey::Amount, 0.0, 0.25, 0.02),
    p(ParamKey::Frequency, 0.1, 64.0, 4.0),
    p(ParamKey::Angle, -360.0, 360.0, 0.0),
    p(ParamKey::OffsetX, -1000.0, 1000.0, 0.0),
];
const MOTION_BLUR: &[ParamSpec] = &[
    p(ParamKey::Amount, 0.0, 0.5, 0.02),
    p(ParamKey::Angle, -360.0, 360.0, 0.0),
];
const GRAIN: &[ParamSpec] = &[
    p(ParamKey::Amount, 0.0, 1.0, 0.15),
    p(ParamKey::Radius, 0.0005, 0.02, 0.0015),
];
const SHARPEN: &[ParamSpec] = &[
    p(ParamKey::Amount, 0.0, 4.0, 0.5),
    p(ParamKey::Radius, 0.0, 0.02, 0.002),
];
const SHADOW: &[ParamSpec] = &[
    p(ParamKey::Amount, 0.0, 1.0, 0.5),
    p(ParamKey::Radius, 0.0, 0.25, 0.02),
    p(ParamKey::OffsetX, -1.0, 1.0, 0.01),
    p(ParamKey::OffsetY, -1.0, 1.0, 0.01),
];
const BLUR: &[ParamSpec] = &[p(ParamKey::Radius, 0.0, 0.25, 0.01)];
const VIGNETTE: &[ParamSpec] = &[
    p(ParamKey::Amount, 0.0, 1.0, 0.4),
    p(ParamKey::Radius, 0.0, 2.0, 0.75),
    p(ParamKey::Softness, 0.0, 1.0, 0.5),
];
const GLITCH: &[ParamSpec] = &[
    p(ParamKey::Amount, 0.0, 1.0, 0.3),
    p(ParamKey::Frequency, 0.1, 60.0, 6.0),
    p(ParamKey::OffsetX, 0.0, 0.25, 0.01),
];
const GRADE: &[ParamSpec] = &[
    p(ParamKey::Exposure, -5.0, 5.0, 0.0),
    p(ParamKey::Contrast, -1.0, 1.0, 0.0),
    p(ParamKey::Saturation, 0.0, 2.0, 1.0),
    p(ParamKey::Temperature, -1.0, 1.0, 0.0),
    p(ParamKey::Tint, -1.0, 1.0, 0.0),
    p(ParamKey::Lift, -1.0, 1.0, 0.0),
    p(ParamKey::Gamma, 0.1, 4.0, 1.0),
    p(ParamKey::GainLuma, 0.0, 4.0, 1.0),
];
const HSL_BAND: &[ParamSpec] = &[
    p(ParamKey::Hue, 0.0, 360.0, 0.0),
    p(ParamKey::HueWidth, 1.0, 180.0, 30.0),
    p(ParamKey::Softness, 0.0, 1.0, 0.5),
    p(ParamKey::HueShift, -180.0, 180.0, 0.0),
    p(ParamKey::SatShift, -1.0, 1.0, 0.0),
    p(ParamKey::LumShift, -1.0, 1.0, 0.0),
];
const CHROMA_KEY: &[ParamSpec] = &[
    p(ParamKey::KeyR, 0.0, 1.0, 0.0),
    p(ParamKey::KeyG, 0.0, 1.0, 1.0),
    p(ParamKey::KeyB, 0.0, 1.0, 0.0),
    p(ParamKey::Tolerance, 0.0, 1.0, 0.3),
    p(ParamKey::Softness, 0.0, 1.0, 0.1),
    p(ParamKey::Spill, 0.0, 1.0, 0.5),
];
const DIFFERENCE_MATTE: &[ParamSpec] = &[
    p(ParamKey::Tolerance, 0.0, 1.0, 0.1),
    p(ParamKey::Softness, 0.0, 1.0, 0.1),
];
const STABILIZE: &[ParamSpec] = &[
    p(ParamKey::Smoothness, 0.0, 1.0, 0.5),
    p(ParamKey::CropZoom, 1.0, 2.0, 1.1),
];
// Mask centre and size are fractions of the layer box, origin top left.
const MASK: &[ParamSpec] = &[
    p(ParamKey::CenterX, -1.0, 2.0, 0.5),
    p(ParamKey::CenterY, -1.0, 2.0, 0.5),
    p(ParamKey::Width, 0.0, 4.0, 0.5),
    p(ParamKey::Height, 0.0, 4.0, 0.5),
    p(ParamKey::Rotation, -36_000.0, 36_000.0, 0.0),
    p(ParamKey::Feather, 0.0, 1.0, 0.0),
    p(ParamKey::Roundness, 0.0, 1.0, 0.0),
];
// Mirrors `Transform::validate` and the clip gain bound.
const CLIP: &[ParamSpec] = &[
    p(ParamKey::PositionX, -1_000_000.0, 1_000_000.0, 0.0),
    p(ParamKey::PositionY, -1_000_000.0, 1_000_000.0, 0.0),
    p(ParamKey::ScaleX, 0.001, 100.0, 1.0),
    p(ParamKey::ScaleY, 0.001, 100.0, 1.0),
    p(ParamKey::Rotation, -36_000.0, 36_000.0, 0.0),
    p(ParamKey::Opacity, 0.0, 1.0, 1.0),
    p(ParamKey::CropLeft, 0.0, 0.99, 0.0),
    p(ParamKey::CropTop, 0.0, 0.99, 0.0),
    p(ParamKey::CropRight, 0.0, 0.99, 0.0),
    p(ParamKey::CropBottom, 0.0, 0.99, 0.0),
    p(ParamKey::Gain, 0.0, 8.0, 1.0),
];
const SOFT_EDGE: &[ParamSpec] = &[p(ParamKey::Softness, 0.0, 1.0, 0.05)];
const ZOOM: &[ParamSpec] = &[p(ParamKey::Amount, 0.0, 4.0, 0.5)];
const GLITCH_TRANSITION: &[ParamSpec] = &[p(ParamKey::Amount, 0.0, 1.0, 0.5)];
const BLUR_TRANSITION: &[ParamSpec] = &[p(ParamKey::Radius, 0.0, 0.25, 0.05)];

/// The single table that validates, fills defaults, drives the inspector and
/// documents the agent surface.
pub fn spec(kind: EffectKind) -> &'static [ParamSpec] {
    match kind {
        EffectKind::Glow => GLOW,
        EffectKind::Shake => SHAKE,
        EffectKind::Distort => DISTORT,
        EffectKind::MotionBlur => MOTION_BLUR,
        EffectKind::Grain => GRAIN,
        EffectKind::Sharpen => SHARPEN,
        EffectKind::Shadow => SHADOW,
        EffectKind::Blur => BLUR,
        EffectKind::Vignette => VIGNETTE,
        EffectKind::Glitch => GLITCH,
        EffectKind::Grade => GRADE,
        EffectKind::HslBand => HSL_BAND,
        EffectKind::CurveMaster
        | EffectKind::CurveRed
        | EffectKind::CurveGreen
        | EffectKind::CurveBlue
        | EffectKind::Lut => &[],
        EffectKind::ChromaKey => CHROMA_KEY,
        EffectKind::DifferenceMatte => DIFFERENCE_MATTE,
        EffectKind::Stabilize => STABILIZE,
    }
}

pub fn mask_spec() -> &'static [ParamSpec] {
    MASK
}

/// Transform and gain parameters addressed by an animation track without a target.
pub fn clip_spec() -> &'static [ParamSpec] {
    CLIP
}

pub fn transition_spec(kind: TransitionKind) -> &'static [ParamSpec] {
    match kind {
        TransitionKind::Fade
        | TransitionKind::Dissolve
        | TransitionKind::SlideLeft
        | TransitionKind::SlideRight
        | TransitionKind::SlideUp
        | TransitionKind::SlideDown => &[],
        TransitionKind::ZoomIn | TransitionKind::ZoomOut => ZOOM,
        TransitionKind::WipeLeft
        | TransitionKind::WipeRight
        | TransitionKind::WipeUp
        | TransitionKind::WipeDown => SOFT_EDGE,
        TransitionKind::Glitch => GLITCH_TRANSITION,
        TransitionKind::Blur => BLUR_TRANSITION,
    }
}

/// What an animation track addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimTarget {
    Clip,
    Effect(EffectKind),
    Mask,
}

pub fn target_spec(target: AnimTarget) -> &'static [ParamSpec] {
    match target {
        AnimTarget::Clip => CLIP,
        AnimTarget::Effect(kind) => spec(kind),
        AnimTarget::Mask => MASK,
    }
}

pub fn param_spec(target: AnimTarget, key: ParamKey) -> Option<&'static ParamSpec> {
    target_spec(target).iter().find(|spec| spec.key == key)
}

pub fn animatable(target: AnimTarget, key: ParamKey) -> bool {
    param_spec(target, key).is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ParamValue {
    pub key: ParamKey,
    pub value: f32,
}

// Every stored key belongs to the table once, with a value inside its range.
fn params_valid(params: &[ParamValue], specs: &[ParamSpec]) -> bool {
    params.iter().enumerate().all(|(index, param)| {
        params[..index]
            .iter()
            .all(|earlier| earlier.key != param.key)
            && specs
                .iter()
                .any(|spec| spec.key == param.key && spec.contains(param.value))
    })
}

fn param(params: &[ParamValue], specs: &[ParamSpec], key: ParamKey) -> Option<f32> {
    let spec = specs.iter().find(|spec| spec.key == key)?;
    Some(
        params
            .iter()
            .find(|param| param.key == key)
            .map_or(spec.default, |param| param.value),
    )
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Effect {
    pub id: Uuid,
    pub kind: EffectKind,
    pub enabled: bool,
    pub mix: f32,
    pub params: Vec<ParamValue>,
    // Curve kinds only: control points with x strictly increasing.
    pub points: Vec<[f32; 2]>,
    // Lut and DifferenceMatte only.
    pub asset_id: Option<AssetId>,
}
impl Effect {
    pub fn new(kind: EffectKind) -> Self {
        Self {
            id: Uuid::new_v4(),
            kind,
            enabled: true,
            mix: 1.0,
            params: Vec::new(),
            points: if kind.is_curve() {
                vec![[0.0, 0.0], [1.0, 1.0]]
            } else {
                Vec::new()
            },
            asset_id: None,
        }
    }

    /// The stored value or the kind's default; `None` when the key is foreign.
    pub fn param(&self, key: ParamKey) -> Option<f32> {
        param(&self.params, spec(self.kind), key)
    }

    /// Everything except that the referenced asset exists.
    pub fn validate(&self) -> bool {
        let unit = |value: &f32| (0.0..=1.0).contains(value);
        let points = if self.kind.is_curve() {
            (2..=MAX_CURVE_POINTS).contains(&self.points.len())
                && self.points.iter().all(|point| point.iter().all(unit))
                && self.points.windows(2).all(|pair| pair[0][0] < pair[1][0])
        } else {
            self.points.is_empty()
        };
        points
            && unit(&self.mix)
            && params_valid(&self.params, spec(self.kind))
            && self.asset_id.is_some() == self.kind.needs_asset()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MaskShape {
    Rectangle,
    Ellipse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MaskOp {
    Add,
    Subtract,
    Intersect,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Mask {
    pub id: Uuid,
    pub shape: MaskShape,
    pub op: MaskOp,
    pub invert: bool,
    // Fractions of the layer box, origin top left.
    pub center: [f32; 2],
    pub size: [f32; 2],
    pub rotation: f32,
    pub feather: f32,
    pub roundness: f32,
}
impl Mask {
    pub fn new(shape: MaskShape) -> Self {
        Self {
            id: Uuid::new_v4(),
            shape,
            op: MaskOp::Add,
            invert: false,
            center: [0.5, 0.5],
            size: [0.5, 0.5],
            rotation: 0.0,
            feather: 0.0,
            roundness: 0.0,
        }
    }

    pub fn param(&self, key: ParamKey) -> Option<f32> {
        Some(match key {
            ParamKey::CenterX => self.center[0],
            ParamKey::CenterY => self.center[1],
            ParamKey::Width => self.size[0],
            ParamKey::Height => self.size[1],
            ParamKey::Rotation => self.rotation,
            ParamKey::Feather => self.feather,
            ParamKey::Roundness => self.roundness,
            _ => return None,
        })
    }

    pub fn validate(&self) -> bool {
        MASK.iter().all(|spec| {
            self.param(spec.key)
                .is_some_and(|value| spec.contains(value))
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    Fade,
    Dissolve,
    SlideLeft,
    SlideRight,
    SlideUp,
    SlideDown,
    ZoomIn,
    ZoomOut,
    WipeLeft,
    WipeRight,
    WipeUp,
    WipeDown,
    Glitch,
    Blur,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Transition {
    pub kind: TransitionKind,
    pub duration: Time,
    pub ease: Ease,
    pub params: Vec<ParamValue>,
}
impl Transition {
    pub fn param(&self, key: ParamKey) -> Option<f32> {
        param(&self.params, transition_spec(self.kind), key)
    }

    /// `frame` is one canvas frame; the transition spans at least that and at
    /// most the clip or ten seconds.
    pub fn validate(&self, frame: Time, clip_duration: Time) -> bool {
        self.duration >= frame
            && self.duration > Time::ZERO
            && self.duration <= clip_duration.min(MAX_TRANSITION)
            && self.ease.validate()
            && params_valid(&self.params, transition_spec(self.kind))
    }
}

// The clip edge a transition belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransitionEdge {
    In,
    Out,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TrackKey {
    pub source: Time,
    // Fractions of the source frame, origin top left.
    pub center: [f32; 2],
    pub scale: f32,
    pub rotation: f32,
}

/// A tracked path through one asset, keyed in source time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Tracker {
    pub id: Uuid,
    pub name: String,
    pub asset_id: AssetId,
    pub keys: Vec<TrackKey>,
}
impl Tracker {
    /// Everything except that the asset exists and contains the keys.
    pub fn validate(&self) -> bool {
        !self.name.trim().is_empty()
            && self.name.len() <= 256
            && (1..=MAX_TRACK_KEYS).contains(&self.keys.len())
            && self
                .keys
                .windows(2)
                .all(|pair| pair[0].source < pair[1].source)
            && self.keys.iter().all(|key| {
                key.source.is_valid()
                    && key.center.iter().all(|v| (-4.0..=4.0).contains(v))
                    && (0.001..=100.0).contains(&key.scale)
                    && (-36_000.0..=36_000.0).contains(&key.rotation)
            })
    }

    /// The tracked point at `source`, linear between keys and held outside
    /// them.
    pub fn at(&self, source: Time) -> Option<TrackKey> {
        let next = self.keys.partition_point(|key| key.source <= source);
        let before = next.checked_sub(1).and_then(|index| self.keys.get(index));
        let (from, to) = match (before, self.keys.get(next)) {
            (Some(from), Some(to)) => (from, to),
            (Some(key), None) | (None, Some(key)) => return Some(*key),
            (None, None) => return None,
        };
        let t = ((source.0 - from.source.0) as f64 / (to.source.0 - from.source.0) as f64) as f32;
        let mix = |a: f32, b: f32| a + (b - a) * t;
        Some(TrackKey {
            source,
            center: [
                mix(from.center[0], to.center[0]),
                mix(from.center[1], to.center[1]),
            ],
            scale: mix(from.scale, to.scale),
            rotation: mix(from.rotation, to.rotation),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Follow {
    pub tracker_id: Uuid,
    pub position: bool,
    pub scale: bool,
    pub rotation: bool,
    pub offset: [f32; 2],
}
impl Follow {
    /// Everything except that the tracker exists.
    pub fn validate(&self) -> bool {
        self.offset
            .iter()
            .all(|v| (-1_000_000.0..=1_000_000.0).contains(v))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LookPreset {
    Cinematic,
    TealOrange,
    Vintage,
    Faded,
    Noir,
    Warm,
    Cool,
}
impl LookPreset {
    pub const ALL: [Self; 7] = [
        Self::Cinematic,
        Self::TealOrange,
        Self::Vintage,
        Self::Faded,
        Self::Noir,
        Self::Warm,
        Self::Cool,
    ];
}

/// The look a stack of effects holds: `len` effects from `at` that are what
/// `preset` expands into at `strength`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AppliedLook {
    pub preset: LookPreset,
    pub strength: f32,
    pub at: usize,
    pub len: usize,
}

// An effect whose listed parameters move from a neutral value to the look's
// value as `strength` rises.
fn tuned(kind: EffectKind, strength: f32, params: &[(ParamKey, f32, f32)]) -> Effect {
    let mut effect = Effect::new(kind);
    effect.params = params
        .iter()
        .map(|&(key, neutral, value)| ParamValue {
            key,
            value: neutral + (value - neutral) * strength,
        })
        .collect();
    effect
}

// A curve whose points move from the identity line to the look's curve.
fn curve(kind: EffectKind, strength: f32, points: &[[f32; 2]]) -> Effect {
    let mut effect = Effect::new(kind);
    effect.points = points
        .iter()
        .map(|&[x, y]| [x, x + (y - x) * strength])
        .collect();
    effect
}

fn grain(strength: f32, amount: f32) -> Effect {
    tuned(
        EffectKind::Grain,
        strength,
        &[(ParamKey::Amount, 0.0, amount)],
    )
}

fn vignette(strength: f32, amount: f32) -> Effect {
    tuned(
        EffectKind::Vignette,
        strength,
        &[
            (ParamKey::Amount, 0.0, amount),
            (ParamKey::Radius, 0.75, 0.75),
            (ParamKey::Softness, 0.6, 0.6),
        ],
    )
}

/// The ordinary, adjustable effects a look expands into, with fresh IDs.
/// `strength` in `0..=1` scales the look from neutral to full.
///
/// A grade works on linear light, where a lift of one hundredth already
/// raises black to a dark grey: the lifts here are that small. The curves
/// work on gamma-encoded values, as they are drawn.
pub fn preset_effects(preset: LookPreset, strength: f32) -> Vec<Effect> {
    use ParamKey::{Contrast, Lift, Saturation, Temperature, Tint};
    let strength = if strength.is_nan() {
        0.0
    } else {
        strength.clamp(0.0, 1.0)
    };
    let grade = |params: &[(ParamKey, f32, f32)]| tuned(EffectKind::Grade, strength, params);
    match preset {
        LookPreset::Cinematic => vec![
            grade(&[
                (Contrast, 0.0, 0.2),
                (Saturation, 1.0, 0.85),
                (Temperature, 0.0, 0.08),
                (Lift, 0.0, -0.004),
            ]),
            curve(
                EffectKind::CurveMaster,
                strength,
                &[[0.0, 0.0], [0.25, 0.2], [0.75, 0.8], [1.0, 1.0]],
            ),
            vignette(strength, 0.35),
            grain(strength, 0.08),
        ],
        LookPreset::TealOrange => vec![
            grade(&[
                (Contrast, 0.0, 0.15),
                (Saturation, 1.0, 1.15),
                (Temperature, 0.0, 0.15),
                (Tint, 0.0, -0.05),
            ]),
            curve(
                EffectKind::CurveRed,
                strength,
                &[[0.0, 0.0], [0.3, 0.25], [0.7, 0.76], [1.0, 1.0]],
            ),
            curve(
                EffectKind::CurveBlue,
                strength,
                &[[0.0, 0.06], [0.3, 0.36], [0.7, 0.64], [1.0, 0.92]],
            ),
        ],
        LookPreset::Vintage => vec![
            grade(&[
                (Contrast, 0.0, -0.1),
                (Saturation, 1.0, 0.7),
                (Temperature, 0.0, 0.25),
                (Tint, 0.0, 0.05),
                (Lift, 0.0, 0.008),
            ]),
            curve(EffectKind::CurveBlue, strength, &[[0.0, 0.08], [1.0, 0.88]]),
            vignette(strength, 0.45),
            grain(strength, 0.25),
        ],
        LookPreset::Faded => vec![
            grade(&[
                (Contrast, 0.0, -0.2),
                (Saturation, 1.0, 0.8),
                (Lift, 0.0, 0.01),
            ]),
            curve(
                EffectKind::CurveMaster,
                strength,
                &[[0.0, 0.1], [0.5, 0.5], [1.0, 0.94]],
            ),
        ],
        LookPreset::Noir => vec![
            grade(&[(Saturation, 1.0, 0.0), (Contrast, 0.0, 0.35)]),
            vignette(strength, 0.5),
            grain(strength, 0.2),
        ],
        LookPreset::Warm => vec![grade(&[
            (Temperature, 0.0, 0.3),
            (Tint, 0.0, 0.05),
            (Saturation, 1.0, 1.08),
        ])],
        LookPreset::Cool => vec![grade(&[
            (Temperature, 0.0, -0.3),
            (Tint, 0.0, -0.03),
            (Saturation, 1.0, 0.95),
        ])],
    }
}

/// The least strength a look is told apart at: weaker, every look is the
/// same neutral effects.
pub const MIN_LOOK_STRENGTH: f32 = 0.01;

// Every number of an effect that a look's strength moves.
fn look_values(effect: &Effect) -> impl Iterator<Item = f32> + '_ {
    (effect.params.iter().map(|param| param.value))
        .chain(effect.points.iter().map(|point| point[1]))
}

// Whether `found` is `made` but for the numbers a strength moves.
fn same_shape(found: &Effect, made: &Effect) -> bool {
    found.kind == made.kind
        && found.mix == made.mix
        && found.asset_id == made.asset_id
        && found.params.len() == made.params.len()
        && found.points.len() == made.points.len()
        && (found.params.iter().zip(&made.params)).all(|(a, b)| a.key == b.key)
        && (found.points.iter().zip(&made.points)).all(|(a, b)| a[0] == b[0])
}

/// The look `effects` holds, read from the effects themselves so that it is
/// the same after a save, an undo or an edit from anywhere: the first run of
/// effects that are exactly what a look expands into at some strength of at
/// least [`MIN_LOOK_STRENGTH`]. An effect of the run that was adjusted, taken
/// out or moved leaves ordinary effects and no look. Whether an effect of the
/// run is enabled is not part of it.
pub fn applied_look(effects: &[Effect]) -> Option<AppliedLook> {
    const SLACK: f32 = 2e-4;
    let made: Vec<_> = LookPreset::ALL
        .into_iter()
        .map(|preset| {
            (
                preset,
                preset_effects(preset, 0.0),
                preset_effects(preset, 1.0),
            )
        })
        .collect();
    for at in 0..effects.len() {
        for (preset, neutral, full) in &made {
            let Some(run) = effects.get(at..at + full.len()) else {
                continue;
            };
            if !run
                .iter()
                .zip(full)
                .all(|(found, made)| same_shape(found, made))
            {
                continue;
            }
            // (found, neutral, full) for every number of the run.
            let values: Vec<(f32, f32, f32)> = (run.iter().zip(neutral.iter().zip(full)))
                .flat_map(|(found, (neutral, full))| {
                    look_values(found)
                        .zip(look_values(neutral).zip(look_values(full)))
                        .map(|(found, (neutral, full))| (found, neutral, full))
                })
                .collect();
            // The strength is read off the number it moves furthest.
            let Some(&(found, neutral, full_value)) = values
                .iter()
                .max_by(|a, b| (a.2 - a.1).abs().total_cmp(&(b.2 - b.1).abs()))
            else {
                continue;
            };
            let strength = (found - neutral) / (full_value - neutral);
            if !(MIN_LOOK_STRENGTH - SLACK..=1.0 + SLACK).contains(&strength) {
                continue;
            }
            let strength = strength.min(1.0);
            let fits = values.iter().all(|(found, neutral, full)| {
                (found - (neutral + (full - neutral) * strength)).abs() <= SLACK
            });
            if fits {
                return Some(AppliedLook {
                    preset: *preset,
                    strength,
                    at,
                    len: full.len(),
                });
            }
        }
    }
    None
}

/// `effects` given `look`, at a strength in `0..=1`. A clip holds one look:
/// the one [`applied_look`] reads is taken out and the new one goes where it
/// was, or at the end when there was none. The same look made again at
/// another strength keeps its effects' IDs, and with them their keyframes.
/// `None` takes the held look out.
pub fn with_look(effects: &[Effect], look: Option<(LookPreset, f32)>) -> Vec<Effect> {
    let mut fresh = look.map_or_else(Vec::new, |(preset, strength)| {
        preset_effects(preset, strength)
    });
    let mut stack = effects.to_vec();
    match applied_look(effects) {
        Some(held) => {
            let run = held.at..held.at + held.len;
            if look.is_some_and(|(preset, _)| preset == held.preset) {
                for (fresh, old) in fresh.iter_mut().zip(&effects[run.clone()]) {
                    fresh.id = old.id;
                    fresh.enabled = old.enabled;
                }
            }
            stack.splice(run, fresh);
        }
        None => stack.extend(fresh),
    }
    stack
}
