//! Pure timeline rules the interface applies before it sends a command.
//!
//! The core accepts any arrangement that is ordered by start; it does not
//! keep clips of one track from overlapping. The interface does: a move lands
//! in the nearest place with room, a trim stops at its neighbours and at the
//! ends of its source, and everything falls on a frame.
use editor_core::{
    Asset, Clip, ClipContent, ClipId, FrameRate, MediaKind, Project, Time, Track, TrackId,
    TrackKind,
};

/// The nearest frame boundary.
pub fn quantize(time: Time, rate: FrameRate) -> Time {
    let frame = rate.frame_at(time.max(Time::ZERO));
    let before = rate.frame_time(frame);
    let after = rate.frame_time(frame + 1);
    if (time.0 - before.0).abs() <= (after.0 - time.0).abs() {
        before
    } else {
        after
    }
}

/// One frame: the shortest a clip may become.
pub fn frame(rate: FrameRate) -> Time {
    rate.frame_time(1).max(Time(1))
}

/// The clips that move, split or disappear with `ids`: everything linked to
/// them and, when `groups` is set, everything grouped with them.
pub fn related(project: &Project, ids: &[ClipId], groups: bool) -> Vec<ClipId> {
    let mut selected: Vec<ClipId> = ids.to_vec();
    loop {
        let mut links = Vec::new();
        let mut group_ids = Vec::new();
        for id in &selected {
            if let Some((_, clip)) = project.clip(*id) {
                links.extend(clip.linked_id);
                if groups {
                    group_ids.extend(clip.group_id);
                }
            }
        }
        let before = selected.len();
        for clip in project.tracks.iter().flat_map(|track| &track.clips) {
            let joined = clip.linked_id.is_some_and(|id| links.contains(&id))
                || clip.group_id.is_some_and(|id| group_ids.contains(&id));
            if joined && !selected.contains(&clip.id) {
                selected.push(clip.id);
            }
        }
        if selected.len() == before {
            return selected;
        }
    }
}

/// Whether a clip can be made of this asset at all. A lookup table is the
/// asset of an effect and goes on no track.
pub fn placeable(asset: &Asset) -> bool {
    asset.kind != MediaKind::Lut
}

/// Whether a track of this kind can hold a clip of this asset.
pub fn track_accepts(kind: TrackKind, asset: &Asset) -> bool {
    placeable(asset)
        && match kind {
            TrackKind::Video => asset.kind != MediaKind::Audio,
            TrackKind::Audio => asset.has_audio,
            TrackKind::Text => false,
        }
}

/// The kind of track a new clip of this asset belongs on.
pub fn home_track(asset: &Asset) -> TrackKind {
    if asset.kind == MediaKind::Audio {
        TrackKind::Audio
    } else {
        TrackKind::Video
    }
}

fn overlaps(a: (Time, Time), b: (Time, Time)) -> bool {
    a.0 < b.1 && b.0 < a.1
}

/// The start nearest to `start` at which a clip of `duration` has room on
/// `track`, ignoring the clips in `ignore`.
pub fn free_start(track: &Track, start: Time, duration: Time, ignore: &[ClipId]) -> Time {
    let start = start.max(Time::ZERO);
    let others: Vec<(Time, Time)> = track
        .clips
        .iter()
        .filter(|clip| !ignore.contains(&clip.id))
        .map(|clip| (clip.start, clip.end()))
        .collect();
    let fits = |at: Time| {
        at >= Time::ZERO
            && !others
                .iter()
                .any(|other| overlaps((at, at + duration), *other))
    };
    if fits(start) {
        return start;
    }
    others
        .iter()
        .flat_map(|(from, to)| [*to, *from - duration])
        .filter(|candidate| fits(*candidate))
        .min_by_key(|candidate| (candidate.0 - start.0).abs())
        // After the last clip there is always room.
        .unwrap_or_else(|| others.iter().map(|other| other.1).max().unwrap_or(start))
}

/// Where a moved selection lands: `delta` itself when nothing is in the way,
/// otherwise the nearest shift that leaves every moved clip room on its
/// track. `selected` are the clips the person holds; what is linked or
/// grouped with them moves too. Only `selected` go to `target`.
pub fn resolve_move(
    project: &Project,
    selected: &[ClipId],
    delta: Time,
    target: Option<TrackId>,
) -> Option<Time> {
    let moving = related(project, selected, true);
    // Each moving clip with the track it ends on.
    let mut movers: Vec<(TrackId, Time, Time)> = Vec::with_capacity(moving.len());
    for id in &moving {
        let (track, clip) = project.clip(*id)?;
        let destination = match target {
            Some(target) if selected.contains(id) => target,
            _ => track.id,
        };
        movers.push((destination, clip.start, clip.end()));
    }
    let earliest = movers.iter().map(|mover| mover.1).min()?;
    let obstacles = |track: TrackId| {
        project
            .track(track)
            .into_iter()
            .flat_map(|track| &track.clips)
            .filter(|clip| !moving.contains(&clip.id))
            .map(|clip| (clip.start, clip.end()))
    };
    let fits = |delta: Time| {
        earliest + delta >= Time::ZERO
            && movers.iter().all(|(track, start, end)| {
                !obstacles(*track).any(|other| overlaps((*start + delta, *end + delta), other))
            })
    };
    let delta = delta.max(Time::ZERO - earliest);
    if fits(delta) {
        return Some(delta);
    }
    // Rest against either side of whatever is in the way, nearest first.
    let mut candidates: Vec<Time> = movers
        .iter()
        .flat_map(|(track, start, end)| {
            obstacles(*track).flat_map(move |(from, to)| [to - *start, from - *end])
        })
        .collect();
    candidates.sort_by_key(|candidate| (candidate.0 - delta.0).abs());
    candidates.dedup();
    candidates
        .into_iter()
        .take(64)
        .find(|candidate| fits(*candidate))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Start,
    End,
}

/// Whether a clip is cut from a source with a beginning and an end. Stills
/// and text can be stretched either way.
pub fn source_bound(project: &Project, clip: &Clip) -> Option<Time> {
    match &clip.content {
        ClipContent::Media { asset_id } => project
            .assets
            .get(asset_id)
            .filter(|asset| asset.kind != MediaKind::Image)
            .map(|asset| asset.duration),
        ClipContent::Text { .. } | ClipContent::Graphic { .. } => None,
    }
}

/// The timeline time each edge of a clip can be let out by before its source
/// runs out, start edge first. Nothing for a clip that is not cut from a
/// source.
///
/// A time map has none to give: a trim crops the map, and the map says
/// nothing of the source outside it. Reversed, the start edge of such a clip
/// is the late end of its source, which is one more reason the room is not
/// read from `source_in`.
fn handles(project: &Project, clip: &Clip) -> Option<(Time, Time)> {
    if clip.retime.is_some() {
        return Some((Time::ZERO, Time::ZERO));
    }
    let length = source_bound(project, clip)?;
    // Without a map the clip plays its span forwards at one speed.
    let speed = clip.speed_at(clip.start).max(0.05);
    let (first, _) = clip.source_span();
    Some((
        first.scaled(1.0 / speed),
        (length - first).scaled(1.0 / speed) - clip.duration,
    ))
}

/// How far an edge may move, as the least and greatest change of that edge's
/// time: it keeps one frame of clip, stays off its neighbours and inside its
/// source.
pub fn trim_range(project: &Project, track: &Track, clip: &Clip, edge: Edge) -> (Time, Time) {
    let rate = project.settings.frame_rate;
    let shortest = frame(rate);
    let handles = handles(project, clip);
    match edge {
        Edge::Start => {
            let previous = track
                .clips
                .iter()
                .filter(|other| other.id != clip.id && other.end() <= clip.start)
                .map(Clip::end)
                .max()
                .unwrap_or(Time::ZERO);
            let mut least = previous - clip.start;
            if let Some((head, _)) = handles {
                // The source has nothing before its first frame.
                least = least.max(Time::ZERO - head);
            }
            (
                least.min(Time::ZERO),
                (clip.duration - shortest).max(Time::ZERO),
            )
        }
        Edge::End => {
            let next = track
                .clips
                .iter()
                .filter(|other| other.id != clip.id && other.start >= clip.end())
                .map(|other| other.start)
                .min();
            let mut most = next.map_or(Time::MAX, |next| next - clip.end());
            if let Some((_, tail)) = handles {
                most = most.min(tail);
            }
            (
                (shortest - clip.duration).min(Time::ZERO),
                most.max(Time::ZERO),
            )
        }
    }
}

/// Whether the project would take `clip`, and what is linked to it, with one
/// edge moved by `delta`: a time map is cropped by a trim, and what is left
/// of an eased stretch can be slower than a map may be.
fn crop_holds(project: &Project, clip: &Clip, edge: Edge, delta: Time) -> bool {
    let (head, length) = match edge {
        Edge::Start => (delta, clip.duration - delta),
        Edge::End => (Time::ZERO, clip.duration + delta),
    };
    related(project, &[clip.id], false).iter().all(|id| {
        let Some((_, linked)) = project.clip(*id) else {
            return true;
        };
        let Some(retime) = &linked.retime else {
            return true;
        };
        // As `TrimClip` reads it: the start edge moves as far on every
        // linked clip, and each changes length by as much.
        let kept = linked.duration + (length - clip.duration);
        (retime.crop(head, head + kept)).is_some_and(|(map, _)| map.validate(kept))
    })
}

/// How far an edge moves when it is asked to move by `wanted`: as far as
/// [`trim_range`] lets it, and short of a place where what would be left of
/// a time map is one the project refuses. The edge stops on the last frame
/// before such a place, and takes a place beyond it when asked for one.
pub fn trim_delta(project: &Project, track: &Track, clip: &Clip, edge: Edge, wanted: Time) -> Time {
    let (least, most) = trim_range(project, track, clip, edge);
    let wanted = wanted.clamp(least, most);
    if crop_holds(project, clip, edge, wanted) {
        return wanted;
    }
    // Nothing moved is what the project holds now. Between that and what
    // was asked for, the furthest whole frame that still holds.
    let unit = frame(project.settings.frame_rate).0.max(1);
    let (sign, mut near, mut far) = (wanted.0.signum(), 0, wanted.0.abs() / unit);
    let reach = |frames: i64| Time(sign * frames * unit);
    if crop_holds(project, clip, edge, reach(far)) {
        return reach(far);
    }
    while far - near > 1 {
        let middle = near + (far - near) / 2;
        if crop_holds(project, clip, edge, reach(middle)) {
            near = middle;
        } else {
            far = middle;
        }
    }
    reach(near)
}

/// The spacing of labelled ruler ticks, in seconds, and how many parts each
/// is divided into, so that labels stay about `spacing` points apart.
pub fn ruler_step(points_per_second: f32, fps: f64, spacing: f32) -> (f64, u32) {
    let frame = 1.0 / fps.max(1.0);
    let steps: [(f64, u32); 17] = [
        (frame, 1),
        (frame * 2.0, 2),
        (frame * 5.0, 5),
        (frame * 10.0, 5),
        (0.5, 5),
        (1.0, 5),
        (2.0, 4),
        (5.0, 5),
        (10.0, 5),
        (15.0, 3),
        (30.0, 6),
        (60.0, 6),
        (120.0, 4),
        (300.0, 5),
        (600.0, 5),
        (1800.0, 6),
        (3600.0, 6),
    ];
    let mut previous = 0.0;
    for (step, parts) in steps {
        // Frame multiples can pass the half second at low rates; keep the
        // list increasing.
        if step <= previous {
            continue;
        }
        previous = step;
        if step * f64::from(points_per_second) >= f64::from(spacing) {
            return (step, parts);
        }
    }
    (7200.0, 4)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn seconds(value: f64) -> Time {
        Time::from_seconds(value)
    }

    /// A project whose first track holds text clips at the given spans.
    fn project_with(spans: &[(f64, f64)]) -> (Project, Vec<ClipId>) {
        let mut project = Project::new("Test");
        let mut track = Track::new("Text", TrackKind::Text);
        let mut ids = Vec::new();
        for (start, length) in spans {
            let clip = Clip::text("x", seconds(*start), seconds(*length));
            ids.push(clip.id);
            track.clips.push(clip);
        }
        project.tracks = vec![Arc::new(track)];
        (project, ids)
    }

    #[test]
    fn quantize_rounds_to_the_nearest_frame() {
        let rate = FrameRate::default();
        assert_eq!(quantize(seconds(0.049), rate), rate.frame_time(1));
        assert_eq!(quantize(seconds(0.051), rate), rate.frame_time(2));
    }

    #[test]
    fn a_move_into_free_space_keeps_its_delta() {
        let (project, ids) = project_with(&[(0.0, 2.0), (10.0, 2.0)]);
        assert_eq!(
            resolve_move(&project, &[ids[0]], seconds(3.0), None),
            Some(seconds(3.0))
        );
    }

    #[test]
    fn a_move_onto_a_neighbour_rests_against_its_nearer_side() {
        let (project, ids) = project_with(&[(0.0, 2.0), (10.0, 2.0)]);
        // Asked to start at 9: that overlaps 10..12, and 8 is nearer than 12.
        assert_eq!(
            resolve_move(&project, &[ids[0]], seconds(9.0), None),
            Some(seconds(8.0))
        );
    }

    #[test]
    fn a_move_never_starts_before_zero() {
        let (project, ids) = project_with(&[(1.0, 2.0)]);
        assert_eq!(
            resolve_move(&project, &[ids[0]], seconds(-5.0), None),
            Some(seconds(-1.0))
        );
    }

    #[test]
    fn free_start_finds_the_nearest_gap() {
        let (project, _) = project_with(&[(0.0, 4.0), (5.0, 4.0)]);
        // Two seconds do not fit in the one-second gap at 4; after 9 they do.
        assert_eq!(
            free_start(&project.tracks[0], seconds(3.0), seconds(2.0), &[]),
            seconds(9.0)
        );
    }

    #[test]
    fn a_trim_stops_at_its_neighbours() {
        let (project, _) = project_with(&[(0.0, 2.0), (3.0, 2.0), (6.0, 2.0)]);
        let track = &project.tracks[0];
        let middle = &track.clips[1];
        let (least, _) = trim_range(&project, track, middle, Edge::Start);
        assert_eq!(least, seconds(-1.0));
        let (_, most) = trim_range(&project, track, middle, Edge::End);
        assert_eq!(most, seconds(1.0));
    }

    #[test]
    fn a_time_map_is_trimmed_inwards_only() {
        let (mut project, _) = project_with(&[(3.0, 2.0)]);
        let clip = &mut Arc::make_mut(&mut project.tracks[0]).clips[0];
        clip.retime = Some(editor_core::Retime::constant(clip.duration, 0.5));
        let track = &project.tracks[0];
        let clip = &track.clips[0];
        let shortest = frame(project.settings.frame_rate);
        assert_eq!(
            trim_range(&project, track, clip, Edge::Start),
            (Time::ZERO, seconds(2.0) - shortest)
        );
        assert_eq!(
            trim_range(&project, track, clip, Edge::End),
            (shortest - seconds(2.0), Time::ZERO)
        );
    }

    /// A project whose first track holds one clip of a ten second video.
    fn project_of_video(change: impl FnOnce(&mut Clip)) -> Project {
        let asset =
            editor_core::Asset::new("/media/shot.mp4".into(), MediaKind::Video, seconds(10.0));
        let mut clip = Clip::from_asset(&asset, seconds(20.0));
        change(&mut clip);
        let mut project = Project::new("Test");
        Arc::make_mut(&mut project.assets).insert(asset.id, asset);
        Arc::make_mut(&mut project.tracks[0]).clips.push(clip);
        project.validate().unwrap();
        project
    }

    fn ranges(project: &Project) -> ((Time, Time), (Time, Time)) {
        let track = &project.tracks[0];
        let clip = &track.clips[0];
        (
            trim_range(project, track, clip, Edge::Start),
            trim_range(project, track, clip, Edge::End),
        )
    }

    #[test]
    fn a_trim_at_one_rate_is_let_out_as_far_as_the_source_reaches() {
        // Source 2 s to 8 s at double speed: 1 s of timeline before, 1 s after.
        let project = project_of_video(|clip| {
            clip.source_in = seconds(2.0);
            clip.duration = seconds(3.0);
            clip.playback_rate = 2.0;
        });
        let shortest = frame(project.settings.frame_rate);
        let (start, end) = ranges(&project);
        assert_eq!(start, (seconds(-1.0), seconds(3.0) - shortest));
        assert_eq!(end, (shortest - seconds(3.0), seconds(1.0)));
        // And at half speed 4 s before, and 12 s after the 4 s it plays.
        let project = project_of_video(|clip| {
            clip.source_in = seconds(2.0);
            clip.duration = seconds(4.0);
            clip.playback_rate = 0.5;
        });
        let (start, end) = ranges(&project);
        assert_eq!(start.0, seconds(-4.0));
        assert_eq!(end.1, seconds(12.0));
    }

    #[test]
    fn a_time_map_is_never_let_out_whichever_way_it_plays() {
        use editor_core::Ease::{Bezier, Linear};
        let ramp = Bezier {
            x1: 0.42,
            y1: 0.0,
            x2: 0.58,
            y2: 1.0,
        };
        // Linear, a freeze, a reverse and a ramp, each with source to spare
        // on both sides of what it plays.
        let maps: [&[(f64, f64, editor_core::Ease)]; 4] = [
            &[(0.0, 0.0, Linear), (4.0, 2.0, Linear)],
            &[(0.0, 0.0, Linear), (2.0, 2.0, Linear), (4.0, 2.0, Linear)],
            &[(0.0, 4.0, Linear), (4.0, 0.0, Linear)],
            &[(0.0, 0.0, ramp), (4.0, 4.0, Linear)],
        ];
        for keys in maps {
            let project = project_of_video(|clip| {
                clip.source_in = seconds(3.0);
                clip.duration = seconds(4.0);
                clip.retime = Some(editor_core::Retime {
                    keys: keys
                        .iter()
                        .map(|&(at, source, ease)| editor_core::TimeKey {
                            at: seconds(at),
                            source: seconds(source),
                            ease,
                        })
                        .collect(),
                });
            });
            let shortest = frame(project.settings.frame_rate);
            let (start, end) = ranges(&project);
            assert_eq!(start, (Time::ZERO, seconds(4.0) - shortest), "{keys:?}");
            assert_eq!(end, (shortest - seconds(4.0), Time::ZERO), "{keys:?}");
        }
    }

    #[test]
    fn a_trim_keeps_one_frame_of_clip() {
        let (project, _) = project_with(&[(0.0, 2.0)]);
        let track = &project.tracks[0];
        let clip = &track.clips[0];
        let (_, most) = trim_range(&project, track, clip, Edge::Start);
        assert_eq!(most, seconds(2.0) - frame(project.settings.frame_rate));
    }

    #[test]
    fn related_follows_links_and_groups() {
        let (mut project, ids) = project_with(&[(0.0, 1.0), (2.0, 1.0), (4.0, 1.0)]);
        let link = uuid::Uuid::new_v4();
        let track = Arc::make_mut(&mut project.tracks[0]);
        track.clips[0].linked_id = Some(link);
        track.clips[1].linked_id = Some(link);
        assert_eq!(related(&project, &[ids[0]], true).len(), 2);
    }

    #[test]
    fn ruler_labels_keep_their_distance() {
        let (step, _) = ruler_step(100.0, 30.0, 90.0);
        assert_eq!(step, 1.0);
        let (step, _) = ruler_step(2.0, 30.0, 90.0);
        assert_eq!(step, 60.0);
    }
}
