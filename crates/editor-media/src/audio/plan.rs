//! Sample-aligned admission before decoder I/O. Scratch belongs to one mixer.
use super::{
    AssetId, BTreeSet, CHUNK_FRAMES, ClipContent, ClipId, MAX_VOICES, MediaError, MediaKind,
    Project, Time, TrackKind, VecDeque,
};
use editor_core::{Clip, Ease, ParamKey, TimeKey};

/// Whether keyframes drive the clip's gain, so its stored gain is only a base.
pub(super) fn gain_animated(clip: &Clip) -> bool {
    clip.animation
        .iter()
        .any(|track| track.target.is_none() && track.key == ParamKey::Gain)
}

// The speed of a straight forward segment of a time map, which is all of one
// a sequential decoder can play.
fn straight(from: &TimeKey, to: &TimeKey) -> Option<f64> {
    let speed = (to.source.0 - from.source.0) as f64 / (to.at.0 - from.at.0) as f64;
    (from.ease == Ease::Linear && speed > 0.0).then_some(speed)
}

/// The source speed one sequential decoder can hold at `position`. A time map
/// is audible only on its straight forward segments: a freeze, a reverse and a
/// ramp are silent.
pub(super) fn steady_speed(clip: &Clip, position: Time) -> Option<f64> {
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
/// is silent from the sample it starts on, and the decoder after it starts at
/// that segment's own source position.
fn audible_run(clip: &Clip, from: usize) -> Option<(Time, Time, usize)> {
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

const MAX_INTERVALS: usize = MAX_VOICES * CHUNK_FRAMES;

pub(super) fn voice_limit() -> MediaError {
    MediaError::Invalid(
        "Preview audio supports at most 16 simultaneous audible clips; mute extra tracks".into(),
    )
}

#[derive(Clone, Copy)]
pub(super) struct Window {
    pub start: Time,
    pub frames: usize,
    pub rate: u32,
    pub speed: f64,
}

/// Indices address the same captured immutable project for this whole chunk.
/// The descriptor is 32 bytes on the supported 64-bit desktop targets.
#[derive(Clone, Copy)]
pub(super) struct Interval {
    pub track: usize,
    pub clip: usize,
    pub first: usize,
    pub last: usize,
}
impl Interval {
    pub fn id(self, project: &Project) -> ClipId {
        project.tracks[self.track].clips[self.clip].id
    }
}

pub(super) struct Plan {
    intervals: VecDeque<Interval>,
    changes: [i32; CHUNK_FRAMES + 1],
}
impl Default for Plan {
    fn default() -> Self {
        Self {
            intervals: VecDeque::new(),
            changes: [0; CHUNK_FRAMES + 1],
        }
    }
}
impl Plan {
    pub fn intervals(&self) -> impl Iterator<Item = &Interval> {
        self.intervals.iter()
    }

    pub fn prepare(
        &mut self,
        project: &Project,
        audio_owners: &BTreeSet<(AssetId, [u8; 16])>,
        window: Window,
    ) -> Result<(), MediaError> {
        self.intervals.clear();
        self.changes.fill(0);
        if window.frames == 0
            || window.frames > CHUNK_FRAMES
            || window.rate == 0
            || !window.speed.is_finite()
            || window.speed <= 0.0
        {
            return Err(MediaError::Invalid(
                "Invalid preview audio buffer timing".into(),
            ));
        }
        let end = window.start
            + Time::from_seconds(window.frames as f64 / f64::from(window.rate) * window.speed);
        let interval_limit = MAX_VOICES * window.frames;
        for (track_index, track) in project.tracks.iter().enumerate() {
            if track.muted {
                continue;
            }
            for (clip_index, clip) in track.clips.iter().enumerate() {
                // Validated tracks are start-sorted; future clips cannot overlap.
                if clip.start >= end {
                    break;
                }
                if (clip.gain == 0.0 && !gain_animated(clip)) || clip.end() <= window.start {
                    continue;
                }
                let ClipContent::Media { asset_id } = &clip.content else {
                    continue;
                };
                if track.kind == TrackKind::Video
                    && clip
                        .linked_id
                        .is_some_and(|link| audio_owners.contains(&(*asset_id, link.into_bytes())))
                {
                    continue;
                }
                let Some(asset) = project.assets.get(asset_id) else {
                    continue;
                };
                if !asset.has_audio && asset.kind != MediaKind::Audio {
                    continue;
                }
                let mut segment = 0;
                while let Some((from, to, after)) = audible_run(clip, segment) {
                    segment = after;
                    // Runs are in timeline order; a later one is past the window too.
                    if from >= end {
                        break;
                    }
                    let first = if from > window.start {
                        ((from - window.start).0 as f64 * f64::from(window.rate)
                            / (1_000_000.0 * window.speed))
                            .ceil() as usize
                    } else {
                        0
                    };
                    let last = (((to.min(end) - window.start).0 as f64 * f64::from(window.rate)
                        / (1_000_000.0 * window.speed))
                        .ceil() as usize)
                        .min(window.frames);
                    if to <= window.start || first >= last {
                        continue;
                    }
                    // Every interval owns at least one sample. More than 16*frames
                    // intervals must exceed peak 16 somewhere, so reject before
                    // growing scratch or touching a source. Counters stay <=16384.
                    if self.intervals.len() == interval_limit {
                        return Err(voice_limit());
                    }
                    if self.intervals.len() == self.intervals.capacity() {
                        let capacity =
                            (self.intervals.capacity().max(MAX_VOICES) * 2).min(MAX_INTERVALS);
                        self.intervals
                            .try_reserve_exact(capacity - self.intervals.len())
                            .map_err(|_| {
                                MediaError::Invalid(
                                    "Preview audio scheduling exceeds its memory budget".into(),
                                )
                            })?;
                    }
                    self.changes[first] += 1;
                    self.changes[last] -= 1;
                    self.intervals.push_back(Interval {
                        track: track_index,
                        clip: clip_index,
                        first,
                        last,
                    });
                }
            }
        }
        let mut audible = 0;
        for change in &self.changes[..window.frames] {
            audible += change;
            if audible > MAX_VOICES as i32 {
                return Err(voice_limit());
            }
        }
        self.intervals
            .make_contiguous()
            .sort_unstable_by_key(|interval| (interval.first, interval.last, interval.id(project)));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::{Asset, Clip, Track};
    use std::{path::PathBuf, sync::Arc};

    fn project_with_intervals(intervals: &[(usize, usize)]) -> Project {
        let asset = Asset::new(
            PathBuf::from("owned-planner-fixture.wav"),
            MediaKind::Audio,
            Time(1_000_000),
        );
        let mut project = Project::new("Pure sample interval fixture");
        Arc::make_mut(&mut project.assets).insert(asset.id, asset.clone());
        project.tracks.clear();
        for (index, &(first, last)) in intervals.iter().enumerate() {
            let mut track = Track::new(format!("Audio {index}"), TrackKind::Audio);
            let mut clip = Clip::from_asset(&asset, Time(first as i64 * 1000));
            clip.duration = Time((last - first) as i64 * 1000);
            track.clips.push(clip);
            project.tracks.push(Arc::new(track));
        }
        project.validate().unwrap();
        project
    }

    fn window(frames: usize) -> Window {
        Window {
            start: Time::ZERO,
            frames,
            rate: 1000,
            speed: 1.0,
        }
    }

    #[test]
    fn admission_matches_independent_per_frame_oracle_and_track_permutations() {
        let mut seed = 0x5a17_2026_u64;
        let mut plan = Plan::default();
        for trial in 0..256 {
            let count = trial % 25;
            let mut intervals = Vec::new();
            for _ in 0..count {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let first = (seed >> 32) as usize % 8;
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let last = first + 1 + (seed >> 32) as usize % (8 - first);
                intervals.push((first, last));
            }
            let expected: Vec<_> = (0..8)
                .map(|frame| {
                    intervals
                        .iter()
                        .filter(|&&(first, last)| first <= frame && frame < last)
                        .count()
                })
                .collect();
            let accepted = expected.iter().all(|&count| count <= MAX_VOICES);
            let mut project = project_with_intervals(&intervals);
            for _ in 0..2 {
                let result = plan.prepare(&project, &BTreeSet::new(), window(8));
                assert_eq!(result.is_ok(), accepted);
                if accepted {
                    let actual: Vec<_> = (0..8)
                        .map(|frame| {
                            plan.intervals()
                                .filter(|interval| interval.first <= frame && frame < interval.last)
                                .count()
                        })
                        .collect();
                    assert_eq!(actual, expected);
                }
                project.tracks.reverse();
            }
        }
    }

    #[test]
    fn a_silent_base_gain_is_planned_when_keyframes_raise_it() {
        let mut project = project_with_intervals(&[(0, 8)]);
        let clip = &mut Arc::make_mut(&mut project.tracks[0]).clips[0];
        clip.gain = 0.0;
        let mut plan = Plan::default();
        plan.prepare(&project, &BTreeSet::new(), window(8)).unwrap();
        assert_eq!(plan.intervals().count(), 0);

        let clip = &mut Arc::make_mut(&mut project.tracks[0]).clips[0];
        clip.animation.push(editor_core::AnimTrack {
            target: None,
            key: ParamKey::Gain,
            keys: vec![editor_core::Keyframe {
                at: Time::ZERO,
                value: 1.0,
                ease: Ease::Linear,
            }],
        });
        project.validate().unwrap();
        plan.prepare(&project, &BTreeSet::new(), window(8)).unwrap();
        assert_eq!(plan.intervals().count(), 1);
    }

    #[test]
    fn a_time_map_is_audible_only_on_straight_forward_segments() {
        let mut project = project_with_intervals(&[(0, 800)]);
        let clip = &mut Arc::make_mut(&mut project.tracks[0]).clips[0];
        assert_eq!(steady_speed(clip, Time(100_000)), Some(1.0));
        clip.playback_rate = 1.25;
        assert_eq!(steady_speed(clip, Time(100_000)), Some(1.25));
        clip.playback_rate = 1.0;
        let key = |at: i64, source: i64, ease| editor_core::TimeKey {
            at: Time(at),
            source: Time(source),
            ease,
        };
        let ramp = Ease::Bezier {
            x1: 0.42,
            y1: 0.0,
            x2: 0.58,
            y2: 1.0,
        };
        clip.retime = Some(editor_core::Retime {
            keys: vec![
                key(0, 0, Ease::Linear),
                key(200_000, 400_000, Ease::Linear),
                key(400_000, 400_000, Ease::Linear),
                key(600_000, 200_000, ramp),
                key(800_000, 400_000, Ease::Linear),
            ],
        });
        project.validate().unwrap();
        let clip = &project.tracks[0].clips[0];
        assert_eq!(steady_speed(clip, Time::ZERO), Some(2.0));
        assert_eq!(steady_speed(clip, Time(199_999)), Some(2.0));
        assert_eq!(steady_speed(clip, Time(200_000)), None, "freeze");
        assert_eq!(steady_speed(clip, Time(500_000)), None, "reverse");
        assert_eq!(steady_speed(clip, Time(700_000)), None, "ramp");
    }

    #[test]
    fn a_time_map_is_planned_sample_by_sample_as_its_segments_are_audible() {
        let key = |at: i64, source: i64, ease| editor_core::TimeKey {
            at: Time(at),
            source: Time(source),
            ease,
        };
        let ramp = Ease::Bezier {
            x1: 0.42,
            y1: 0.0,
            x2: 0.58,
            y2: 1.0,
        };
        // Keys on and off the millisecond the samples of this window fall on.
        let maps = [
            (
                "linear",
                vec![key(0, 0, Ease::Linear), key(8000, 4000, Ease::Linear)],
            ),
            (
                "two straight segments at one speed",
                vec![
                    key(0, 0, Ease::Linear),
                    key(3500, 3500, Ease::Linear),
                    key(8000, 8000, Ease::Linear),
                ],
            ),
            (
                "freeze",
                vec![
                    key(0, 0, Ease::Linear),
                    key(2500, 2500, Ease::Linear),
                    key(5000, 2500, Ease::Linear),
                    key(8000, 5500, Ease::Linear),
                ],
            ),
            (
                "hold",
                vec![key(0, 1000, Ease::Hold), key(8000, 1000, Ease::Linear)],
            ),
            (
                "reverse",
                vec![
                    key(0, 0, Ease::Linear),
                    key(3000, 6000, Ease::Linear),
                    key(6001, 2999, Ease::Linear),
                    key(8000, 4998, Ease::Linear),
                ],
            ),
            (
                "all backwards",
                vec![key(0, 8000, Ease::Linear), key(8000, 0, Ease::Linear)],
            ),
            (
                "eased",
                vec![
                    key(0, 0, Ease::Linear),
                    key(2000, 2000, ramp),
                    key(5999, 6000, Ease::Linear),
                    key(8000, 7000, Ease::Linear),
                ],
            ),
        ];
        let mut plan = Plan::default();
        for (name, keys) in maps {
            for (start, speed) in [(0, 1.0), (3000, 1.0), (-2000, 1.0), (1000, 2.0), (0, 0.5)] {
                let mut project = project_with_intervals(&[(10, 18)]);
                let clip = &mut Arc::make_mut(&mut project.tracks[0]).clips[0];
                clip.retime = Some(editor_core::Retime { keys: keys.clone() });
                project.validate().unwrap();
                let clip = &project.tracks[0].clips[0];
                let window = Window {
                    start: Time(10_000 + start),
                    frames: 12,
                    rate: 1000,
                    speed,
                };
                // Independently of the intervals: a sample sounds when the clip
                // is on at its time and a decoder can hold the speed there.
                let expected: Vec<_> = (0..window.frames)
                    .map(|frame| {
                        let at = window.start + Time((frame as f64 * 1000.0 * speed) as i64);
                        clip.start <= at && at < clip.end() && steady_speed(clip, at).is_some()
                    })
                    .collect();
                plan.prepare(&project, &BTreeSet::new(), window).unwrap();
                let actual: Vec<_> = (0..window.frames)
                    .map(|frame| {
                        plan.intervals()
                            .filter(|interval| interval.first <= frame && frame < interval.last)
                            .count()
                    })
                    .collect();
                assert_eq!(
                    actual,
                    expected
                        .iter()
                        .map(|on| usize::from(*on))
                        .collect::<Vec<_>>(),
                    "{name} from {start} at {speed}"
                );
                // One decoder plays an interval: the speed it starts at holds
                // to its last sample, and where the next interval of the clip
                // follows at once it is at another speed.
                let speed_at = |frame: usize| {
                    let at = window.start + Time((frame as f64 * 1000.0 * speed) as i64);
                    steady_speed(clip, at)
                };
                let intervals: Vec<_> = plan.intervals().copied().collect();
                for interval in &intervals {
                    let held = speed_at(interval.first);
                    assert!(
                        (interval.first..interval.last).all(|frame| speed_at(frame) == held),
                        "{name} from {start} at {speed}"
                    );
                }
                for pair in intervals.windows(2) {
                    assert!(
                        pair[0].last < pair[1].first
                            || speed_at(pair[0].first) != speed_at(pair[1].first),
                        "{name} from {start} at {speed}"
                    );
                }
            }
        }
    }

    #[test]
    fn half_open_cuts_admit_sixteen_replacements_and_reject_one_frame_overload() {
        let mut intervals = vec![(0, 4); MAX_VOICES];
        intervals.extend([(4, 8); MAX_VOICES]);
        let mut plan = Plan::default();
        plan.prepare(
            &project_with_intervals(&intervals),
            &BTreeSet::new(),
            window(8),
        )
        .unwrap();
        intervals.push((3, 4));
        assert!(matches!(
            plan.prepare(
                &project_with_intervals(&intervals),
                &BTreeSet::new(),
                window(8)
            ),
            Err(MediaError::Invalid(_))
        ));
    }

    #[test]
    fn scratch_stays_bounded_and_reuses_capacity_at_the_largest_admitted_workload() {
        let intervals: Vec<_> = (0..CHUNK_FRAMES)
            .flat_map(|frame| std::iter::repeat_n((frame, frame + 1), MAX_VOICES))
            .collect();
        let project = project_with_intervals(&intervals);
        let mut plan = Plan::default();
        plan.prepare(&project, &BTreeSet::new(), window(CHUNK_FRAMES))
            .unwrap();
        assert_eq!(plan.intervals.len(), MAX_INTERVALS);
        assert!(plan.intervals.capacity() <= MAX_INTERVALS);
        let scratch = plan.intervals.as_slices().0.as_ptr();
        let capacity = plan.intervals.capacity();
        plan.prepare(&project, &BTreeSet::new(), window(CHUNK_FRAMES))
            .unwrap();
        assert_eq!(plan.intervals.capacity(), capacity);
        assert_eq!(plan.intervals.as_slices().0.as_ptr(), scratch);

        let overloaded = project_with_intervals(&[(0, 1); MAX_VOICES + 1]);
        assert!(
            plan.prepare(&overloaded, &BTreeSet::new(), window(1))
                .is_err()
        );
        assert_eq!(plan.intervals.len(), MAX_VOICES);
        assert!(plan.intervals.capacity() <= MAX_INTERVALS);
        assert!(plan.prepare(&project, &BTreeSet::new(), window(0)).is_err());
        assert!(
            plan.prepare(&project, &BTreeSet::new(), window(CHUNK_FRAMES + 1))
                .is_err()
        );
    }

    #[test]
    fn sample_empty_positive_duration_clips_do_not_consume_voice_slots() {
        let mut project = project_with_intervals(&[(0, 1); MAX_VOICES + 1]);
        for track in &mut project.tracks {
            let clip = &mut Arc::make_mut(track).clips[0];
            clip.start = Time(1);
            clip.duration = Time(1);
        }
        project.validate().unwrap();
        let mut plan = Plan::default();
        plan.prepare(&project, &BTreeSet::new(), window(4)).unwrap();
        assert_eq!(plan.intervals().count(), 0);
    }

    #[test]
    fn microsecond_boundaries_match_integer_sample_oracle_at_supported_preview_speeds() {
        for (speed, speed_numerator, speed_denominator) in
            [(0.5, 1_i128, 2_i128), (1.0, 1, 1), (2.0, 2, 1)]
        {
            for offset in [0, 1, 7000, 7001, 17_000] {
                let mut project = project_with_intervals(&[(0, 1)]);
                let clip = &mut Arc::make_mut(&mut project.tracks[0]).clips[0];
                clip.start = Time(5_000_000 + offset);
                clip.duration = Time(1000);
                project.validate().unwrap();
                let denominator = 1_000_000 * speed_numerator;
                let sample_at = |microseconds: i64| {
                    let numerator = i128::from(microseconds) * 48_000 * speed_denominator;
                    ((numerator + denominator - 1) / denominator) as usize
                };
                let first = sample_at(offset);
                let last = sample_at(offset + 1000).min(CHUNK_FRAMES);
                let mut plan = Plan::default();
                plan.prepare(
                    &project,
                    &BTreeSet::new(),
                    Window {
                        start: Time(5_000_000),
                        frames: CHUNK_FRAMES,
                        rate: 48_000,
                        speed,
                    },
                )
                .unwrap();
                let bounds: Vec<_> = plan
                    .intervals()
                    .map(|interval| (interval.first, interval.last))
                    .collect();
                let expected = if first < last {
                    vec![(first, last)]
                } else {
                    Vec::new()
                };
                assert_eq!(bounds, expected);
            }
        }
    }
}
