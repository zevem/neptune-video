use crate::{Ease, Time, animate::pieces};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const MAX_TIME_KEYS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TimeKey {
    // Microseconds from the clip's start.
    pub at: Time,
    // Offset from the clip's source_in.
    pub source: Time,
    pub ease: Ease,
}

/// A clip's time map. Equal sources freeze, decreasing sources reverse and an
/// eased segment ramps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Retime {
    pub keys: Vec<TimeKey>,
}

// `delta * numerator / denominator`, rounded half away from zero like `Time::scaled`.
pub(crate) fn scale(delta: i64, numerator: i64, denominator: i64) -> i64 {
    if denominator <= 0 {
        return 0;
    }
    let product = i128::from(delta) * i128::from(numerator);
    let denominator = i128::from(denominator);
    let rounded = (2 * product.abs() + denominator) / (2 * denominator);
    (rounded * product.signum()).clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

// Source distance covered `elapsed` into a segment of `span` that covers `moved`.
fn travelled(ease: Ease, moved: i64, elapsed: i64, span: i64) -> i64 {
    match ease {
        Ease::Hold => 0,
        Ease::Linear => scale(moved, elapsed, span),
        ease => (moved as f64 * ease.eval(elapsed as f64 / span as f64)).round() as i64,
    }
}

// Drops the keys before (`head`) or after `at`, which lies inside the map,
// leaving a key on the cut that keeps the remaining curve. `false` when the
// cut segment cannot be carried by keys.
fn cut(keys: &mut Vec<TimeKey>, at: Time, head: bool) -> bool {
    let next = keys.partition_point(|k| k.at < at);
    if next == 0 || next == keys.len() {
        return true;
    }
    if keys[next].at == at {
        if head {
            keys.drain(..next);
        } else {
            keys.truncate(next + 1);
        }
        return true;
    }
    let (from, to) = (keys[next - 1], keys[next]);
    let (elapsed, span) = (at.0 - from.at.0, to.at.0 - from.at.0);
    let moved = to.source.0.saturating_sub(from.source.0);
    let source = |offset: i64| Time(from.source.0.saturating_add(offset));
    let on_cut = source(travelled(from.ease, moved, elapsed, span));
    let part = if matches!(from.ease, Ease::Bezier { .. }) {
        let offset = |y: f64| (moved as f64 * y).round() as i64;
        let adjust = |y: f64| {
            if moved == 0 {
                y
            } else {
                offset(y) as f64 / moved as f64
            }
        };
        let t = elapsed as f64 / span as f64;
        let window = if head { (t, 1.0) } else { (0.0, t) };
        let Some((_, pieces)) = pieces(from.ease, window.0, window.1, (0.0, 1.0), &adjust) else {
            return false;
        };
        let (start, end) = if head { (at, to.at) } else { (from.at, at) };
        let mut part = vec![TimeKey {
            at: start,
            source: if head { on_cut } else { from.source },
            ease: pieces[0].ease,
        }];
        for pair in pieces.windows(2) {
            let time = Time(from.at.0 + (pair[0].x * span as f64).round() as i64);
            if part.last().is_some_and(|key| key.at < time) && time < end {
                part.push(TimeKey {
                    at: time,
                    source: source(offset(pair[0].y)),
                    ease: pair[1].ease,
                });
            }
        }
        part
    } else {
        vec![TimeKey {
            at: if head { at } else { from.at },
            source: if head { on_cut } else { from.source },
            ease: from.ease,
        }]
    };
    if head {
        keys.splice(..next, part);
    } else {
        keys.splice(next - 1.., part);
        keys.push(TimeKey {
            at,
            source: on_cut,
            ease: Ease::Linear,
        });
    }
    true
}

impl Retime {
    /// The map equivalent to a constant playback rate over `duration`.
    pub fn constant(duration: Time, rate: f64) -> Self {
        let key = |at, source| TimeKey {
            at,
            source,
            ease: Ease::Linear,
        };
        Self {
            keys: vec![
                key(Time::ZERO, Time::ZERO),
                key(duration, duration.scaled(rate)),
            ],
        }
    }

    pub fn validate(&self, duration: Time) -> bool {
        let (Some(first), Some(last)) = (self.keys.first(), self.keys.last()) else {
            return false;
        };
        (2..=MAX_TIME_KEYS).contains(&self.keys.len())
            && first.at == Time::ZERO
            && last.at == duration
            && self
                .keys
                .iter()
                .all(|k| k.source.is_valid() && k.ease.is_monotone())
            && self.keys.windows(2).all(|pair| {
                let span = i128::from(pair[1].at.0) - i128::from(pair[0].at.0);
                let moved = (i128::from(pair[1].source.0) - i128::from(pair[0].source.0)).abs();
                // Two microseconds of slack absorb rounding at the rate limits.
                span > 0 && (moved == 0 || ((moved + 2) * 20 >= span && moved <= span * 16 + 2))
            })
    }

    // The segment containing `local`, or the nearest one outside the map.
    fn segment(&self, local: Time) -> Option<(&TimeKey, &TimeKey)> {
        let last = self.keys.len().checked_sub(2)?;
        let index = self
            .keys
            .partition_point(|k| k.at <= local)
            .saturating_sub(1)
            .min(last);
        Some((&self.keys[index], &self.keys[index + 1]))
    }

    /// Source offset from `source_in` at clip-relative `local`. Outside the
    /// map the nearest segment continues at its mean speed.
    pub fn source_at(&self, local: Time) -> Time {
        let Some((from, to)) = self.segment(local) else {
            return self.keys.first().map_or(Time::ZERO, |k| k.source);
        };
        let moved = to.source.0.saturating_sub(from.source.0);
        let span = to.at.0.saturating_sub(from.at.0);
        let mean = |origin: &TimeKey| match from.ease {
            Ease::Hold => 0,
            _ => scale(moved, local.0.saturating_sub(origin.at.0), span),
        };
        let offset = if local >= to.at {
            to.source.0.saturating_add(mean(to))
        } else if local <= from.at {
            from.source.0.saturating_add(mean(from))
        } else {
            from.source
                .0
                .saturating_add(travelled(from.ease, moved, local.0 - from.at.0, span))
        };
        Time(offset)
    }

    /// Signed source seconds per timeline second; 0 on a freeze.
    pub fn speed_at(&self, local: Time) -> f64 {
        let Some((from, to)) = self.segment(local) else {
            return 0.0;
        };
        let span = to.at.0.saturating_sub(from.at.0);
        if span <= 0 || from.ease == Ease::Hold {
            return 0.0;
        }
        let mean = to.source.0.saturating_sub(from.source.0) as f64 / span as f64;
        if local < from.at || local > to.at {
            return mean;
        }
        mean * from.ease.slope((local.0 - from.at.0) as f64 / span as f64)
    }

    /// Smallest and largest source offsets the map reaches.
    pub fn span(&self) -> (Time, Time) {
        let sources = || self.keys.iter().map(|k| k.source);
        (
            sources().min().unwrap_or(Time::ZERO),
            sources().max().unwrap_or(Time::ZERO),
        )
    }

    /// The map over `from..to`, rebased to start at `from` with its lowest
    /// source at zero, and the amount that lowest source moved: add it to the
    /// clip's `source_in`. `None` when the range leaves the map. A cut inside
    /// a segment synthesizes a boundary key by subdivision.
    pub fn crop(&self, from: Time, to: Time) -> Option<(Self, Time)> {
        let (first, last) = (self.keys.first()?, self.keys.last()?);
        if from < first.at || to > last.at || from >= to {
            return None;
        }
        let mut keys = self.keys.clone();
        if !(cut(&mut keys, to, false) && cut(&mut keys, from, true)) {
            return None;
        }
        let shift = keys.iter().map(|k| k.source).min()?;
        for key in &mut keys {
            key.at = key.at - from;
            key.source = key.source - shift;
        }
        Some((Self { keys }, shift))
    }

    /// Where the map shows the source offset `source`: an instant for each
    /// moving segment that passes it and the whole stretch of each segment
    /// that holds it, as inclusive clip-relative `(from, to)` in time order.
    pub fn shows(&self, source: Time) -> Vec<(Time, Time)> {
        let mut found: Vec<(Time, Time)> = Vec::new();
        for pair in self.keys.windows(2) {
            let (from, to) = (pair[0], pair[1]);
            let span = to.at.0.saturating_sub(from.at.0);
            let moved = to.source.0.saturating_sub(from.source.0);
            let stretch = if moved == 0 || from.ease == Ease::Hold || span <= 0 {
                (from.source == source).then_some((from.at, to.at))
            } else {
                // Measured along the segment's own direction, so a reversed
                // one is searched like a forward one.
                let target = source.0.saturating_sub(from.source.0) * moved.signum();
                (0..=moved.abs()).contains(&target).then(|| {
                    let reached = |elapsed: i64| {
                        travelled(from.ease, moved, elapsed, span) * moved.signum() >= target
                    };
                    let elapsed = match from.ease {
                        Ease::Linear => scale(target, span, moved.abs()),
                        _ => {
                            // An eased segment is monotone: the first
                            // microsecond that reaches the source.
                            let (mut low, mut high) = (0, span);
                            while low < high {
                                let middle = low + (high - low) / 2;
                                if reached(middle) {
                                    high = middle;
                                } else {
                                    low = middle + 1;
                                }
                            }
                            low
                        }
                    };
                    let at = Time(from.at.0 + elapsed.clamp(0, span));
                    (at, at)
                })
            };
            let Some(stretch) = stretch else { continue };
            match found.last_mut() {
                // A source on a shared key belongs to both of its segments.
                Some(last) if last.1 >= stretch.0 => last.1 = last.1.max(stretch.1),
                _ => found.push(stretch),
            }
        }
        found
    }

    /// How long the map has held its source when it reaches `local`, when
    /// `local` lies in a segment that holds.
    pub fn held_for(&self, local: Time) -> Option<Time> {
        let (from, to) = self.segment(local)?;
        let holds = from.source == to.source || from.ease == Ease::Hold;
        (holds && from.at <= local && local <= to.at).then(|| local - from.at)
    }

    /// Stretches key times from a clip lasting `old` to one lasting `new`, so
    /// the same source span plays faster or slower.
    pub fn rescale(&mut self, old: Time, new: Time) {
        for key in &mut self.keys {
            key.at = Time(scale(key.at.0, new.0, old.0));
        }
    }
}
