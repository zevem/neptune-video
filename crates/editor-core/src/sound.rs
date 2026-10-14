//! What sounds, and how loud: the one definition the mixer and the interface share.
use crate::{
    AssetId, Clip, Ease, EffectKind, Keyframe, MediaKind, ParamKey, Project, Time, TimeKey, Track,
    TrackKind, animate::MAX_KEYFRAMES, gain_at,
};
use std::{collections::BTreeSet, ops::RangeInclusive};
use uuid::Uuid;

// One edge of the equal-power curve: 0 at the edge, 1 from `length` on. An
// edge without a fade is 1 on both sides of it.
fn ramp(elapsed: Time, length: Time) -> f32 {
    if length <= Time::ZERO || elapsed >= length {
        return 1.0;
    }
    if elapsed <= Time::ZERO {
        return 0.0;
    }
    (std::f64::consts::FRAC_PI_2 * elapsed.0 as f64 / length.0 as f64).sin() as f32
}

/// The clip's fades `local` after its start, in 0..=1: `sin(pi/2 * t)` up
/// over `fade_in` and the same down over `fade_out`. A clip without fades is
/// 1 throughout, and a faded edge is 0 beyond the clip.
pub fn fade_at(clip: &Clip, local: Time) -> f32 {
    ramp(local, clip.fade_in) * ramp(clip.duration - local, clip.fade_out)
}

/// Track volume x clip volume (stored or keyed) x the edge fades.
pub fn level_at(track: &Track, clip: &Clip, playhead: Time) -> f32 {
    track.gain * gain_at(clip, playhead) * fade_at(clip, playhead - clip.start)
}

/// Whether keyframes drive the clip's volume, so its stored volume is only a base.
pub fn volume_keyed(clip: &Clip) -> bool {
    clip.animation
        .iter()
        .any(|track| track.target.is_none() && track.key == ParamKey::Gain)
}

/// Whether [`level_at`] changes inside the clip: volume keys or a fade.
pub fn level_varies(clip: &Clip) -> bool {
    volume_keyed(clip) || !clip.fade_in.is_zero() || !clip.fade_out.is_zero()
}

// The speed of a straight forward segment of a time map, which is all of one
// a sequential decoder can play.
fn straight(from: &TimeKey, to: &TimeKey) -> Option<f64> {
    let speed = (to.source.0 - from.source.0) as f64 / (to.at.0 - from.at.0) as f64;
    (from.ease == Ease::Linear && speed > 0.0).then_some(speed)
}

/// The source speed one sequential decoder can hold at `position`. A time map
/// is audible only on its straight forward segments: a freeze, a reverse and a
/// ramp are silent. A clip without a map is one such segment.
pub fn steady_speed(clip: &Clip, position: Time) -> Option<f64> {
    let Some(retime) = &clip.retime else {
        return Some(clip.playback_rate);
    };
    let local = position - clip.start;
    let index = retime
        .keys
        .partition_point(|key| key.at <= local)
        .checked_sub(1)?;
    straight(retime.keys.get(index)?, retime.keys.get(index + 1)?)
}

/// The next stretch of timeline one decoder plays `clip` over, looking from
/// segment `from` of its time map, and the segment to look from after it. A
/// clip without a map is one stretch. A map gives one for each run of
/// straight forward segments at one speed, so a freeze, a reverse or a ramp
/// is silent from the sample it starts on.
pub fn audible_run(clip: &Clip, from: usize) -> Option<(Time, Time, usize)> {
    let Some(retime) = &clip.retime else {
        return (from == 0).then(|| (clip.start, clip.end(), 1));
    };
    let speed_of = |index: usize| straight(retime.keys.get(index)?, retime.keys.get(index + 1)?);
    let segments = retime.keys.len().saturating_sub(1);
    let first = (from..segments).find(|index| speed_of(*index).is_some())?;
    let speed = speed_of(first);
    let after = (first + 1..segments)
        .find(|index| speed_of(*index) != speed)
        .unwrap_or(segments);
    Some((
        clip.start + retime.keys[first].at,
        clip.start + retime.keys[after].at,
        after,
    ))
}

/// The clip speeds over the pitch ratio at which a `Pitch` holds its pitch:
/// what one tempo stage takes. Beyond them the clip still plays at its speed
/// and what is left over is heard as a change of pitch.
pub const PITCH_KEPT: RangeInclusive<f64> = 0.5..=2.0;

/// The semitones of the clip's first enabled `Pitch`, the one that is read.
/// A clip that has one keeps its pitch when its speed changes, and these
/// semitones are its only shift; a clip without one has its pitch changed by
/// its speed. A `Pitch` at 0 is "keep pitch".
pub fn pitch(clip: &Clip) -> Option<f32> {
    (clip.effects.iter())
        .find(|effect| effect.enabled && effect.kind == EffectKind::Pitch)
        .map(|effect| effect.param(ParamKey::Amount).unwrap_or_default())
}

/// The frequency ratio of a shift of `semitones`: 2 an octave up.
pub fn pitch_ratio(semitones: f32) -> f64 {
    2_f64.powf(f64::from(semitones) / 12.0)
}

/// Whether a clip at `speed` with a `Pitch` of `semitones` is heard at that
/// pitch: its speed over the pitch ratio is within [`PITCH_KEPT`].
pub fn pitch_kept(speed: f64, semitones: f32) -> bool {
    PITCH_KEPT.contains(&(speed / pitch_ratio(semitones)))
}

// A soloed text track is ignored: nothing on it sounds.
fn any_solo(project: &Project) -> bool {
    project
        .tracks
        .iter()
        .any(|track| track.solo && track.kind != TrackKind::Text)
}

/// Whether another track's solo keeps `track` out of the mix: a video or
/// audio track is soloed and this one is not. The solo part of [`sounds`]
/// alone, for a track head to show.
pub fn soloed_out(project: &Project, track: &Track) -> bool {
    !track.solo && any_solo(project)
}

// Everything but ownership, which is asked last because it reads the project.
fn admitted(
    project: &Project,
    track: &Track,
    clip: &Clip,
    soloed: bool,
    owned: impl FnOnce() -> bool,
) -> bool {
    // Mute wins over solo.
    if track.muted || (soloed && !track.solo) || track.gain <= 0.0 {
        return false;
    }
    if clip.gain == 0.0 && !volume_keyed(clip) {
        return false;
    }
    let has_sound = clip
        .asset_id()
        .and_then(|id| project.assets.get(&id))
        .is_some_and(|asset| asset.has_audio || asset.kind == MediaKind::Audio);
    has_sound && !owned()
}

/// What a project-wide pass needs to ask [`sounds`] of many clips: the links
/// whose sound an audio-track clip owns, each with the media it owns the
/// sound of, and whether any track is soloed. An owner counts whether or not
/// its own track is muted or soloed out, so neither reveals embedded sound.
#[derive(Debug, Clone)]
pub struct Sounding {
    owners: BTreeSet<(AssetId, Uuid)>,
    soloed: bool,
}

impl Sounding {
    pub fn of(project: &Project) -> Self {
        Self {
            owners: project
                .tracks
                .iter()
                .filter(|track| track.kind == TrackKind::Audio)
                .flat_map(|track| track.clips.iter())
                .filter_map(|clip| clip.asset_id().zip(clip.linked_id))
                .collect(),
            soloed: any_solo(project),
        }
    }

    /// [`owned`] for a clip of `track`.
    pub fn owned(&self, track: &Track, clip: &Clip) -> bool {
        track.kind != TrackKind::Audio
            && clip
                .asset_id()
                .zip(clip.linked_id)
                .is_some_and(|pair| self.owners.contains(&pair))
    }

    /// [`sounds`], without reading the project's links again.
    pub fn sounds(&self, project: &Project, track: &Track, clip: &Clip) -> bool {
        admitted(project, track, clip, self.soloed, || {
            self.owned(track, clip)
        })
    }

    /// [`audible_at`], without reading the project's links again.
    pub fn audible_at(&self, project: &Project, track: &Track, clip: &Clip, at: Time) -> bool {
        on_straight(clip, at) && self.sounds(project, track, clip)
    }
}

fn on_straight(clip: &Clip, at: Time) -> bool {
    clip.start <= at && at < clip.end() && steady_speed(clip, at).is_some()
}

/// The ownership part alone: a picture clip silenced by an audio-track clip
/// of the same media in its link. A clip on an audio track is never owned.
pub fn owned(project: &Project, clip: &Clip) -> bool {
    let Some(pair) = clip.asset_id().zip(clip.linked_id) else {
        return false;
    };
    let mut owner = false;
    for other in project
        .tracks
        .iter()
        .filter(|track| track.kind == TrackKind::Audio)
        .flat_map(|track| track.clips.iter())
    {
        if other.id == clip.id {
            return false;
        }
        owner |= other.asset_id().zip(other.linked_id) == Some(pair);
    }
    owner
}

/// Whether this clip is admitted to the mix at all: its track is not muted,
/// not soloed out and not at volume 0; its asset has sound; no linked
/// audio-track clip of the same asset owns that sound; and its volume is not
/// 0 without volume keys. The one definition; the mixer and the UI share it.
/// `track` is the clip's own track. [`Sounding`] answers the same for many
/// clips.
pub fn sounds(project: &Project, track: &Track, clip: &Clip) -> bool {
    admitted(project, track, clip, any_solo(project), || {
        track.kind != TrackKind::Audio && owned(project, clip)
    })
}

/// [`sounds`], and `at` is inside the clip on a straight forward stretch of
/// its time map.
pub fn audible_at(project: &Project, track: &Track, clip: &Clip, at: Time) -> bool {
    on_straight(clip, at) && sounds(project, track, clip)
}

/// Merges the closest neighbours of `spans` (sorted, apart) until at most
/// `most` are left: the shortest gaps are closed first, the earlier of two
/// equal ones first. Hands back how many were merged away.
pub fn merge_closest(spans: &mut Vec<(Time, Time)>, most: usize) -> usize {
    let excess = spans.len().saturating_sub(most.max(1));
    if excess == 0 {
        return 0;
    }
    let mut gaps: Vec<(Time, usize)> = (spans.windows(2).enumerate())
        .map(|(index, pair)| (pair[1].0 - pair[0].1, index))
        .collect();
    gaps.sort_unstable();
    // Whether the span after each gap joins the one before it.
    let mut joined = vec![false; spans.len()];
    for (_, index) in &gaps[..excess] {
        joined[index + 1] = true;
    }
    let mut merged: Vec<(Time, Time)> = Vec::with_capacity(spans.len() - excess);
    for (span, joins) in spans.iter().zip(joined) {
        match merged.last_mut() {
            Some(last) if joins => last.1 = last.1.max(span.1),
            _ => merged.push(*span),
        }
    }
    *spans = merged;
    excess
}

/// What [`duck`] makes of some spans.
#[derive(Debug, Clone, PartialEq)]
pub struct Duck {
    /// The volume keys, from the clip's start, strictly increasing, all
    /// linear and at most [`MAX_KEYFRAMES`]; none when no span reaches the
    /// clip.
    pub keys: Vec<Keyframe>,
    /// The dips they make.
    pub dips: usize,
    /// The dips merged into a neighbour so that the keys fit.
    pub merged: usize,
}

// The four corners of each dip: the level leaves `base` `attack` before the
// span, is at `low` over it, and is back at `base` `release` after it.
fn corners(
    spans: &[(Time, Time)],
    base: f32,
    low: f32,
    attack: Time,
    release: Time,
) -> Vec<(Time, f32)> {
    (spans.iter())
        .flat_map(|(from, to)| {
            [
                (*from - attack, base),
                (*from, low),
                (*to, low),
                (*to + release, base),
            ]
        })
        .collect()
}

// The level between two corners, `at` of the way.
fn between(before: (Time, f32), after: (Time, f32), at: Time) -> f32 {
    let t = (at.0 - before.0.0) as f64 / (after.0.0 - before.0.0) as f64;
    (f64::from(before.1) + (f64::from(after.1) - f64::from(before.1)) * t) as f32
}

// The corners held to `0..=duration`: one that lies outside is replaced by
// the level where the line through it crosses the edge, so a key at 0 and
// at `duration` carry the level there. Corners at one time are one key.
fn held(corners: &[(Time, f32)], duration: Time) -> Vec<Keyframe> {
    let mut held: Vec<(Time, f32)> = Vec::with_capacity(corners.len());
    for (index, corner) in corners.iter().enumerate() {
        let previous = index.checked_sub(1).map(|index| corners[index]);
        for edge in [Time::ZERO, duration] {
            if let Some(previous) = previous
                && previous.0 < edge
                && edge < corner.0
            {
                held.push((edge, between(previous, *corner, edge)));
            }
        }
        if (Time::ZERO..=duration).contains(&corner.0) {
            held.push(*corner);
        }
    }
    held.dedup_by(|later, earlier| later.0 == earlier.0);
    (held.into_iter())
        .map(|(at, value)| Keyframe {
            at,
            value,
            ease: Ease::Linear,
        })
        .collect()
}

/// Volume keyframes that dip a clip lasting `duration` under `spans`
/// (clip-relative, either way round), with what was merged to make them.
/// For each span the level is `base` at `from - attack`, `base * depth` at
/// `from` and at `to`, and `base` again at `to + release`, all linear and
/// held to `0..=duration`. Spans whose ramps meet are one dip, and spans
/// that do not reach the clip make none. When the keys would pass
/// [`MAX_KEYFRAMES`], the dips closest together are merged until they fit.
/// A ramp of no length is taken as a microsecond, so the keys always rise.
pub fn duck(
    spans: &[(Time, Time)],
    duration: Time,
    base: f32,
    depth: f32,
    attack: Time,
    release: Time,
) -> Duck {
    let (attack, release) = (attack.max(Time(1)), release.max(Time(1)));
    let mut dips: Vec<(Time, Time)> = (spans.iter())
        .map(|(from, to)| (*from.min(to), *from.max(to)))
        .filter(|(from, to)| from < to && *to + release > Time::ZERO && *from - attack < duration)
        .collect();
    dips.sort_unstable();
    // Ramps that would cross are one dip, held low between its spans.
    let mut joined: Vec<(Time, Time)> = Vec::with_capacity(dips.len());
    for (from, to) in dips {
        match joined.last_mut() {
            Some(last) if from - attack < last.1 + release => last.1 = last.1.max(to),
            _ => joined.push((from, to)),
        }
    }
    let low = base * depth;
    let first = joined.len();
    let mut keys = held(&corners(&joined, base, low, attack, release), duration);
    while keys.len() > MAX_KEYFRAMES {
        // A dip merged into its neighbour takes its four corners with it.
        let fewer = (keys.len() - MAX_KEYFRAMES).div_ceil(4);
        let most = joined.len() - fewer.min(joined.len() - 1);
        merge_closest(&mut joined, most);
        keys = held(&corners(&joined, base, low, attack, release), duration);
    }
    Duck {
        keys,
        dips: joined.len(),
        merged: first - joined.len(),
    }
}

/// Volume keyframes that dip a clip under `spans` (clip-relative, sorted):
/// the keys of [`duck`].
pub fn duck_keys(
    spans: &[(Time, Time)],
    duration: Time,
    base: f32,
    depth: f32,
    attack: Time,
    release: Time,
) -> Vec<Keyframe> {
    duck(spans, duration, base, depth, attack, release).keys
}
