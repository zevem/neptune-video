use crate::{Time, look::ParamSpec, retime::scale};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MAX_ANIM_TRACKS: usize = 64;
pub const MAX_KEYFRAMES: usize = 256;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ParamKey {
    PositionX,
    PositionY,
    ScaleX,
    ScaleY,
    Rotation,
    Opacity,
    CropLeft,
    CropTop,
    CropRight,
    CropBottom,
    Gain,
    Amount,
    Radius,
    Angle,
    Threshold,
    Softness,
    Frequency,
    OffsetX,
    OffsetY,
    Exposure,
    Contrast,
    Saturation,
    Temperature,
    Tint,
    Lift,
    Gamma,
    GainLuma,
    Hue,
    HueWidth,
    HueShift,
    SatShift,
    LumShift,
    KeyR,
    KeyG,
    KeyB,
    Tolerance,
    Spill,
    Smoothness,
    CropZoom,
    CenterX,
    CenterY,
    Width,
    Height,
    Feather,
    Roundness,
}

// Shapes the segment leaving its key, as a CSS-style cubic through (0,0) and (1,1).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Ease {
    Hold,
    Linear,
    Bezier { x1: f32, y1: f32, x2: f32, y2: f32 },
}

fn cubic(a: f64, b: f64, s: f64) -> f64 {
    let r = 1.0 - s;
    3.0 * r * r * s * a + 3.0 * r * s * s * b + s * s * s
}

// Curve parameter whose x reaches linear progress `t`.
fn solve(x1: f32, x2: f32, t: f64) -> f64 {
    let (mut low, mut high) = (0.0_f64, 1.0_f64);
    for _ in 0..24 {
        let mid = (low + high) / 2.0;
        if cubic(f64::from(x1), f64::from(x2), mid) < t {
            low = mid;
        } else {
            high = mid;
        }
    }
    (low + high) / 2.0
}

// One axis of the cubic 0, a, b, 1 as its blossom: equal arguments give the
// curve, mixed ones the control values of the part between them.
fn blossom(a: f64, b: f64, u: f64, v: f64, w: f64) -> f64 {
    let lerp = |from: f64, to: f64, s: f64| from + (to - from) * s;
    let (q0, q1, q2) = (lerp(0.0, a, u), lerp(a, b, u), lerp(b, 1.0, u));
    lerp(lerp(q0, q1, v), lerp(q1, q2, v), w)
}

// A control value re-expressed on a part running `from..to`, when the part
// can carry it. Only float noise is clamped.
fn renormalize(value: f64, from: f64, to: f64, range: (f64, f64)) -> Option<f32> {
    const NOISE: f64 = 1e-6;
    let span = to - from;
    let ratio = if span.abs() < 1e-12 {
        if (value - from).abs() > 1e-9 {
            return None;
        }
        0.0
    } else {
        (value - from) / span
    };
    (ratio >= range.0 - NOISE && ratio <= range.1 + NOISE)
        .then(|| ratio.clamp(range.0, range.1) as f32)
}

// A part of a subdivided segment: the linear and eased progress it ends on and
// the ease that shapes it.
pub(crate) struct Piece {
    pub x: f64,
    pub y: f64,
    pub ease: Ease,
}

struct Divider<'a> {
    x: [f64; 2],
    y: [f64; 2],
    range: (f64, f64),
    adjust: &'a dyn Fn(f64) -> f64,
    pieces: Vec<Piece>,
}
impl Divider<'_> {
    // The curve over parameters `s0..s1`, which runs between eased progress
    // `y0` and `y1`, halved until every part's control values are in range.
    fn divide(&mut self, s0: f64, s1: f64, y0: f64, y1: f64, depth: u32) -> bool {
        let ([xa, xb], [ya, yb]) = (self.x, self.y);
        let (x0, x3) = (blossom(xa, xb, s0, s0, s0), blossom(xa, xb, s1, s1, s1));
        let fit = |s: [f64; 3], range: (f64, f64), from: f64, to: f64, a: f64, b: f64| {
            renormalize(blossom(a, b, s[0], s[1], s[2]), from, to, range)
        };
        if let (Some(xa), Some(xb), Some(ya), Some(yb)) = (
            fit([s0, s0, s1], (0.0, 1.0), x0, x3, xa, xb),
            fit([s0, s1, s1], (0.0, 1.0), x0, x3, xa, xb),
            fit([s0, s0, s1], self.range, y0, y1, ya, yb),
            fit([s0, s1, s1], self.range, y0, y1, ya, yb),
        ) {
            self.pieces.push(Piece {
                x: x3,
                y: y1,
                ease: Ease::Bezier {
                    x1: xa,
                    y1: ya,
                    x2: xb,
                    y2: yb,
                },
            });
            return true;
        }
        let inside = |part: f64| (self.adjust)(cubic(ya, yb, s0 + (s1 - s0) * part));
        let mid = (s0 + s1) / 2.0;
        let middle = inside(0.5);
        // A stretch `adjust` holds on one value throughout is a level line.
        if [y1, inside(0.25), middle, inside(0.75)]
            .iter()
            .all(|y| (y - y0).abs() < 1e-12)
        {
            self.pieces.push(Piece {
                x: x3,
                y: y1,
                ease: Ease::Linear,
            });
            return true;
        }
        if depth == 0 {
            return false;
        }
        self.divide(s0, mid, y0, middle, depth - 1) && self.divide(mid, s1, middle, y1, depth - 1)
    }
}

/// The part of a segment between linear progress `from` and `to`, as the
/// eased progress it starts on and the pieces that reproduce it by de
/// Casteljau subdivision. A part whose control values fall outside `range` is
/// subdivided further; `None` when that does not bring them in. `adjust`
/// rounds an eased progress to one a key can hold.
pub(crate) fn pieces(
    ease: Ease,
    from: f64,
    to: f64,
    range: (f64, f64),
    adjust: &dyn Fn(f64) -> f64,
) -> Option<(f64, Vec<Piece>)> {
    let edge = |t: f64| match t {
        t if t <= 0.0 => 0.0,
        t if t >= 1.0 => 1.0,
        t => adjust(ease.eval(t)),
    };
    let (start, end) = (edge(from), edge(to));
    let Ease::Bezier { x1, y1, x2, y2 } = ease else {
        let piece = Piece {
            x: to,
            y: end,
            ease,
        };
        return Some((start, vec![piece]));
    };
    let parameter = |t: f64| match t {
        t if t <= 0.0 => 0.0,
        t if t >= 1.0 => 1.0,
        t => solve(x1, x2, t),
    };
    let mut divider = Divider {
        x: [f64::from(x1), f64::from(x2)],
        y: [f64::from(y1), f64::from(y2)],
        range,
        adjust,
        pieces: Vec::new(),
    };
    divider
        .divide(parameter(from), parameter(to), start, end, 6)
        .then_some((start, divider.pieces))
}

impl Ease {
    pub fn validate(&self) -> bool {
        match *self {
            Self::Hold | Self::Linear => true,
            Self::Bezier { x1, y1, x2, y2 } => {
                [x1, x2].iter().all(|v| (0.0..=1.0).contains(v))
                    && [y1, y2].iter().all(|v| (-4.0..=4.0).contains(v))
            }
        }
    }

    /// Valid and never leaving `0..=1`, so the eased segment cannot reverse.
    pub fn is_monotone(&self) -> bool {
        match *self {
            Self::Hold | Self::Linear => true,
            Self::Bezier { x1, y1, x2, y2 } => {
                [x1, y1, x2, y2].iter().all(|v| (0.0..=1.0).contains(v))
            }
        }
    }

    /// Eased progress for linear progress `t`. Uses only `+ - * /` and a fixed
    /// bisection, so every host computes the same value.
    pub fn eval(&self, t: f64) -> f64 {
        if t >= 1.0 {
            return 1.0;
        }
        if t.is_nan() || t <= 0.0 {
            return 0.0;
        }
        match *self {
            Self::Hold => 0.0,
            Self::Linear => t,
            Self::Bezier { x1, y1, x2, y2 } => {
                cubic(f64::from(y1), f64::from(y2), solve(x1, x2, t))
            }
        }
    }

    /// Finite rate of change of [`Ease::eval`] around `t`.
    pub fn slope(&self, t: f64) -> f64 {
        match self {
            Self::Hold => 0.0,
            Self::Linear => 1.0,
            Self::Bezier { .. } => {
                const WINDOW: f64 = 1.0 / 512.0;
                let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
                let (low, high) = ((t - WINDOW).max(0.0), (t + WINDOW).min(1.0));
                (self.eval(high) - self.eval(low)) / (high - low)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Keyframe {
    // Microseconds from the clip's start.
    pub at: Time,
    pub value: f32,
    pub ease: Ease,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AnimTrack {
    // None addresses the clip; Some names an effect or mask on the same clip.
    pub target: Option<Uuid>,
    pub key: ParamKey,
    pub keys: Vec<Keyframe>,
}

impl AnimTrack {
    /// Value at clip-relative `local`; the first and last keys hold outside their span.
    pub fn sample(&self, local: Time) -> f32 {
        let next = self.keys.partition_point(|k| k.at <= local);
        let Some(after) = self.keys.get(next) else {
            return self.keys.last().map_or(0.0, |k| k.value);
        };
        let Some(before) = next.checked_sub(1).map(|i| &self.keys[i]) else {
            return after.value;
        };
        let span = (after.at.0 - before.at.0) as f64;
        let eased = before.ease.eval((local.0 - before.at.0) as f64 / span);
        (f64::from(before.value) + (f64::from(after.value) - f64::from(before.value)) * eased)
            as f32
    }

    // The keys that carry the segment ending at `keys[next]` over the part
    // from `at` (`head`) or up to it, the first of them on the part's start.
    fn part(&self, next: usize, at: Time, range: (f32, f32), head: bool) -> Option<Vec<Keyframe>> {
        let (before, after) = (self.keys[next - 1], self.keys[next]);
        let span = (after.at.0 - before.at.0) as f64;
        let t = (at.0 - before.at.0) as f64 / span;
        let (from, to) = (f64::from(before.value), f64::from(after.value));
        let value = |y: f64| ((from + (to - from) * y) as f32).clamp(range.0, range.1);
        let adjust = |y: f64| {
            if to == from {
                y
            } else {
                (f64::from(value(y)) - from) / (to - from)
            }
        };
        let (window, end) = if head {
            ((t, 1.0), after.at)
        } else {
            ((0.0, t), at)
        };
        let (start, pieces) = pieces(before.ease, window.0, window.1, (-4.0, 4.0), &adjust)?;
        let mut keys = vec![Keyframe {
            at: if head { at } else { before.at },
            value: if head { value(start) } else { before.value },
            ease: pieces.first()?.ease,
        }];
        for pair in pieces.windows(2) {
            let time = Time(before.at.0 + (pair[0].x * span).round() as i64);
            if keys.last().is_some_and(|key| key.at < time) && time < end {
                keys.push(Keyframe {
                    at: time,
                    value: value(pair[0].y),
                    ease: pair[1].ease,
                });
            }
        }
        if !head {
            keys.push(Keyframe {
                at,
                value: value(pieces.last()?.y),
                ease: Ease::Linear,
            });
        }
        Some(keys)
    }

    // Drops the keys before (`head`) or after `at`, leaving a key on the cut
    // that keeps the remaining curve.
    fn cut(&mut self, at: Time, range: (f32, f32), head: bool) -> bool {
        let next = self.keys.partition_point(|k| k.at < at);
        let on_cut = self.keys.get(next).is_some_and(|k| k.at == at);
        let held = |key: &Keyframe| Keyframe {
            at,
            value: key.value,
            ease: Ease::Linear,
        };
        if head {
            if next == 0 {
                return true;
            }
            let keys = match self.keys.get(next) {
                _ if on_cut => Vec::new(),
                Some(_) => match self.part(next, at, range, true) {
                    Some(keys) => keys,
                    None => return false,
                },
                None => vec![held(&self.keys[next - 1])],
            };
            self.keys.splice(..next, keys);
        } else {
            if next + usize::from(on_cut) == self.keys.len() {
                return true;
            }
            if on_cut {
                self.keys.truncate(next + 1);
            } else if next == 0 {
                self.keys = vec![held(&self.keys[0])];
            } else {
                let Some(keys) = self.part(next, at, range, false) else {
                    return false;
                };
                self.keys.splice(next - 1.., keys);
            }
        }
        true
    }

    /// Keeps the curve over `from..=to` and rebases it to start at `from`.
    /// A cut inside a segment synthesizes boundary keys by subdivision, their
    /// values clamped to `range`, so samples inside the span are unchanged.
    /// `false` when a cut segment cannot be carried by keys.
    #[must_use]
    pub fn crop(&mut self, from: Time, to: Time, range: (f32, f32)) -> bool {
        if !(self.cut(to, range, false) && self.cut(from, range, true)) {
            return false;
        }
        for key in &mut self.keys {
            key.at = key.at - from;
        }
        true
    }

    /// Adds a key at `at` on the curve as it is, so that no sample changes:
    /// the segment it lands in is subdivided as a cut would, its values
    /// clamped to `range`, and outside the keys the nearest one's value is
    /// held. `false`, with the track untouched, when a key is there already,
    /// there is no key to take a value from, or the segment cannot be carried
    /// by keys.
    #[must_use]
    pub fn insert(&mut self, at: Time, range: (f32, f32)) -> bool {
        let next = self.keys.partition_point(|k| k.at < at);
        if self.keys.get(next).is_some_and(|k| k.at == at) {
            return false;
        }
        let held = |key: &Keyframe| Keyframe {
            at,
            value: key.value,
            ease: Ease::Linear,
        };
        if next == 0 {
            let Some(first) = self.keys.first().map(held) else {
                return false;
            };
            self.keys.insert(0, first);
        } else if next == self.keys.len() {
            let last = held(&self.keys[next - 1]);
            self.keys.push(last);
        } else {
            let (Some(mut keys), Some(after)) = (
                self.part(next, at, range, false),
                self.part(next, at, range, true),
            ) else {
                return false;
            };
            // Both parts end on `at`; the later one shapes what follows it.
            keys.pop();
            keys.extend(after);
            self.keys.splice(next - 1..next, keys);
        }
        true
    }

    /// Stretches key times from a clip lasting `old` to one lasting `new`.
    /// Keys that land on the same microsecond collapse into the earlier one.
    pub fn rescale(&mut self, old: Time, new: Time) {
        for key in &mut self.keys {
            key.at = Time(scale(key.at.0, new.0, old.0));
        }
        self.keys.dedup_by(|later, earlier| later.at == earlier.at);
    }

    pub fn validate(&self, duration: Time, spec: &ParamSpec) -> bool {
        (1..=MAX_KEYFRAMES).contains(&self.keys.len())
            && self.keys.windows(2).all(|pair| pair[0].at < pair[1].at)
            && self.keys.iter().all(|k| {
                k.at >= Time::ZERO
                    && k.at <= duration
                    && spec.contains(k.value)
                    && k.ease.validate()
            })
    }
}
