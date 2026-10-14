//! Pure timeline rules the interface applies before it sends a command.
//!
//! The core accepts any arrangement that is ordered by start; it does not
//! keep clips of one track from overlapping. The interface does: a move lands
//! in the nearest place with room, a trim stops at its neighbours and at the
//! ends of its source, and everything falls on a frame.
use editor_core::{
    Asset, AssetId, BlendMode, Clip, ClipContent, ClipId, Command, FrameRate, MediaKind, ParamKey,
    Project, Time, Track, TrackId, TrackKind, Transform,
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

/// Whether a clip's sound could be put on an audio track: a picture clip
/// with sound in its media. A title, a still, a silent video and a clip that
/// is on an audio track already have nothing to detach.
pub fn carries_sound(project: &Project, track: &Track, clip: &Clip) -> bool {
    track.kind == TrackKind::Video
        && (clip.asset_id())
            .and_then(|asset| project.assets.get(&asset))
            .is_some_and(|asset| asset.has_audio)
}

/// The picture clip "Detach audio" is asked of, with its track, or why its
/// sound stays where it is.
fn detachable(project: &Project, clip: ClipId) -> Result<(&Track, &Clip), String> {
    let (track, clip) = project.clip(clip).ok_or("The clip no longer exists.")?;
    if track.kind == TrackKind::Audio {
        return Err("That clip is on an audio track already.".to_owned());
    }
    if !carries_sound(project, track, clip) {
        return Err("That clip has no sound to detach.".to_owned());
    }
    if track.locked {
        return Err("Unlock the clip's track to detach its audio.".to_owned());
    }
    // As the mixer has it: an audio-track clip of the same media in the
    // clip's link owns the sound.
    let owned = clip.linked_id.is_some_and(|link| {
        (project.tracks.iter())
            .filter(|track| track.kind == TrackKind::Audio)
            .flat_map(|track| &track.clips)
            .any(|other| other.linked_id == Some(link) && other.asset_id() == clip.asset_id())
    });
    if owned {
        return Err("Its sound is on the linked audio clip already.".to_owned());
    }
    // The sound is owned by link and media, not by span: the new audio clip
    // would silence every other picture clip of this media in the link, and
    // nothing would sound over their spans.
    let shared = clip.linked_id.is_some_and(|link| {
        (project.tracks.iter())
            .filter(|track| track.kind == TrackKind::Video)
            .flat_map(|track| &track.clips)
            .any(|other| {
                other.id != clip.id
                    && other.linked_id == Some(link)
                    && other.asset_id() == clip.asset_id()
            })
    });
    if shared {
        return Err(
            "Unlink the clip from the other clips of its media to detach its audio.".to_owned(),
        );
    }
    Ok((track, clip))
}

/// Why "Detach audio" would be refused of `clip`, when it would be.
pub fn detach_refusal(project: &Project, clip: ClipId) -> Option<String> {
    detachable(project, clip).err()
}

/// "Detach audio": the sound of a picture clip as a clip of its own on an
/// audio track, linked to the picture, as one step. The new clip plays the
/// same span of the same media at the same speed and volume; being on an
/// audio track and in the picture's link, it is the one that sounds. It goes
/// on the first unlocked audio track with room over the clip's span, or on a
/// new one.
pub fn detach_audio(project: &Project, clip: ClipId) -> Result<Command, String> {
    let (_, picture) = detachable(project, clip)?;
    let sound = Clip {
        id: uuid::Uuid::new_v4(),
        transform: Transform::default(),
        // The link is made below, with whatever the picture is linked to.
        linked_id: None,
        blend: BlendMode::Normal,
        effects: Vec::new(),
        masks: Vec::new(),
        // An audio clip animates its volume and nothing else.
        animation: (picture.animation.iter())
            .filter(|track| track.target.is_none() && track.key == ParamKey::Gain)
            .cloned()
            .collect(),
        transition_in: None,
        transition_out: None,
        follow: None,
        ..picture.clone()
    };
    let free = (project.tracks.iter()).find(|track| {
        track.kind == TrackKind::Audio
            && !track.locked
            && free_start(track, sound.start, sound.duration, &[]) == sound.start
    });
    let mut commands = Vec::with_capacity(3);
    let track_id = match free {
        Some(track) => track.id,
        None => {
            let count = (project.tracks.iter())
                .filter(|track| track.kind == TrackKind::Audio)
                .count();
            let track = Track::new(format!("Audio {}", count + 1), TrackKind::Audio);
            let id = track.id;
            commands.push(Command::AddTrack { track });
            id
        }
    };
    let clip_ids = vec![picture.id, sound.id];
    commands.push(Command::AddClip {
        track_id,
        clip: sound,
    });
    commands.push(Command::LinkClips { clip_ids });
    Ok(Command::Batch {
        label: "Detach audio".to_owned(),
        commands,
    })
}

/// The most clips one music bed is made of.
pub const BED_CLIPS: usize = 256;
/// The shortest sound a bed repeats: a tenth of a second.
const BED_SHORTEST: Time = Time(100_000);
/// How loud a bed is under what it accompanies.
const BED_VOLUME: f32 = 0.5;
/// How long a bed takes to fade out at the project's end.
const BED_FADE: Time = Time(2_000_000);

/// The sound "Add as background music" is asked of, the length it has to
/// cover and how many clips of it that takes, or why no bed is made.
fn bed(project: &Project, asset: AssetId) -> Result<(&Asset, Time, usize), String> {
    let asset = (project.assets.get(&asset)).ok_or("That media is no longer in the library.")?;
    if asset.kind != MediaKind::Audio || !asset.has_audio {
        return Err("Background music is made of a sound file.".to_owned());
    }
    let length = project.duration();
    if length <= Time::ZERO {
        return Err(
            "Put a clip on the timeline first: background music is as long as the project."
                .to_owned(),
        );
    }
    if asset.duration < BED_SHORTEST {
        return Err(
            "That sound is shorter than a tenth of a second, too short to repeat as music."
                .to_owned(),
        );
    }
    let count = (length.0 as u64).div_ceil(asset.duration.0 as u64);
    if count > BED_CLIPS as u64 {
        return Err(format!(
            "That sound would repeat {count} times to cover the project; background music is {BED_CLIPS} clips at most."
        ));
    }
    Ok((asset, length, count as usize))
}

/// Why "Add as background music" would be refused of `asset`, when it would.
pub fn bed_refusal(project: &Project, asset: AssetId) -> Option<String> {
    bed(project, asset).err()
}

/// "Add as background music": a new audio track named "Music", placed last,
/// with the sound end to end from the start until the project's length is
/// covered, as one step. Every clip is at half volume; the last is cut at the
/// project's end and fades out over two seconds, or over its whole length
/// when it is shorter. The bed is as long as the project is now and does not
/// follow it afterwards.
pub fn background_music(project: &Project, asset: AssetId) -> Result<Command, String> {
    let (asset, length, count) = bed(project, asset)?;
    let track = Track::new("Music", TrackKind::Audio);
    let track_id = track.id;
    let mut commands = Vec::with_capacity(count + 1);
    commands.push(Command::AddTrack { track });
    for index in 0..count {
        let mut clip = Clip::from_asset(asset, Time(asset.duration.0 * index as i64));
        clip.gain = BED_VOLUME;
        if index + 1 == count {
            clip.duration = length - clip.start;
            clip.fade_out = BED_FADE.min(clip.duration);
        }
        commands.push(Command::AddClip { track_id, clip });
    }
    Ok(Command::Batch {
        label: "Add background music".to_owned(),
        commands,
    })
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

    // ---- Detach audio ----------------------------------------------------

    /// An editor whose first video track holds a four second clip of a ten
    /// second video with sound, from two seconds into the source.
    fn editor_with_shot(change: impl FnOnce(&mut Clip)) -> (editor_core::Editor, ClipId) {
        let mut asset = Asset::new("/media/shot.mp4".into(), MediaKind::Video, seconds(10.0));
        asset.has_audio = true;
        let mut clip = Clip::from_asset(&asset, seconds(1.0));
        clip.source_in = seconds(2.0);
        clip.duration = seconds(4.0);
        change(&mut clip);
        let id = clip.id;
        let mut editor = editor_core::Editor::default();
        let track_id = editor.project().tracks[0].id;
        for command in [
            Command::AddAssets {
                assets: vec![asset],
            },
            Command::AddClip { track_id, clip },
        ] {
            editor.dispatch(command).unwrap();
        }
        (editor, id)
    }

    /// Every clip on an audio track, with the track it is on.
    fn sound_clips(project: &Project) -> Vec<(&Track, &Clip)> {
        (project.tracks.iter())
            .filter(|track| track.kind == TrackKind::Audio)
            .flat_map(|track| track.clips.iter().map(move |clip| (&**track, clip)))
            .collect()
    }

    fn set_locked(editor: &mut editor_core::Editor, track_id: TrackId, locked: bool) {
        editor
            .dispatch(Command::SetTrackState {
                track_id,
                locked: Some(locked),
                hidden: None,
                muted: None,
                name: None,
                gain: None,
                solo: None,
            })
            .unwrap();
    }

    #[test]
    fn detached_audio_is_one_linked_clip_of_the_same_span_and_one_step() {
        let (mut editor, id) = editor_with_shot(|clip| {
            clip.playback_rate = 1.5;
            clip.gain = 0.6;
            clip.fade_in = seconds(0.5);
            clip.fade_out = seconds(1.0);
            // What a picture has and a sound does not.
            clip.transform.opacity = 0.5;
            clip.blend = BlendMode::Screen;
            clip.effects
                .push(editor_core::Effect::new(editor_core::EffectKind::Glow));
        });
        let before = editor.project().clone();
        let steps = editor.history().count();
        let command = detach_audio(editor.project(), id).unwrap();
        // The free audio track is used, so no track is made.
        let Command::Batch { commands, .. } = &command else {
            panic!("{command:?}");
        };
        assert!(matches!(
            commands.as_slice(),
            [Command::AddClip { .. }, Command::LinkClips { .. }]
        ));
        editor.dispatch(command).unwrap();
        assert_eq!(editor.history().count(), steps + 1);
        assert_eq!(editor.undo_label(), Some("Detach audio"));

        let project = editor.project();
        let picture = project.clip(id).unwrap().1;
        let sounds = sound_clips(project);
        let [(track, sound)] = sounds.as_slice() else {
            panic!("one audio clip, not {}", sounds.len());
        };
        assert_eq!(track.name, "Audio 1");
        assert_ne!(sound.id, picture.id);
        assert_eq!(sound.asset_id(), picture.asset_id());
        assert_eq!(
            (sound.start, sound.source_in, sound.duration),
            (picture.start, picture.source_in, picture.duration)
        );
        assert_eq!((sound.playback_rate, sound.gain), (1.5, 0.6));
        // The fades go with the sound they shape.
        assert_eq!(
            (sound.fade_in, sound.fade_out),
            (seconds(0.5), seconds(1.0))
        );
        assert!(picture.linked_id.is_some() && sound.linked_id == picture.linked_id);
        assert_eq!(sound.transform, Transform::default());
        assert!(sound.effects.is_empty() && sound.blend.is_normal());
        // The picture is as it was but for its link.
        assert_eq!(picture.effects.len(), 1);
        assert_eq!(picture.transform.opacity, 0.5);
        // The pair moves as one from here on.
        assert_eq!(related(project, &[id], false).len(), 2);

        // Undo takes the whole of it away, and redo brings the same pair back.
        let after = project.clone();
        assert!(editor.undo());
        assert_eq!(editor.project(), &before);
        assert!(editor.redo());
        assert_eq!(editor.project(), &after);
    }

    #[test]
    fn detached_audio_keeps_the_time_map_and_the_volume_keyframes() {
        use editor_core::{AnimTrack, Ease, Keyframe, Retime};
        let key = |at: f64, value: f32| Keyframe {
            at: seconds(at),
            value,
            ease: Ease::Linear,
        };
        let volume = AnimTrack {
            target: None,
            key: ParamKey::Gain,
            keys: vec![key(0.0, 1.0), key(2.0, 0.25)],
        };
        let opacity = AnimTrack {
            target: None,
            key: ParamKey::Opacity,
            keys: vec![key(0.0, 0.0), key(1.0, 1.0)],
        };
        let (mut editor, id) = editor_with_shot(|clip| {
            clip.retime = Some(Retime::constant(clip.duration, 0.5));
            clip.animation = vec![opacity, volume.clone()];
        });
        editor
            .dispatch(detach_audio(editor.project(), id).unwrap())
            .unwrap();
        let project = editor.project();
        let picture = project.clip(id).unwrap().1;
        let sounds = sound_clips(project);
        let sound = sounds[0].1;
        assert!(sound.retime.is_some() && sound.retime == picture.retime);
        assert_eq!(sound.source_span(), picture.source_span());
        // The volume's keys go with the sound; the picture's own stay behind.
        assert_eq!(sound.animation, [volume]);
        assert_eq!(picture.animation.len(), 2);
    }

    #[test]
    fn detached_audio_takes_a_free_lane_or_makes_one() {
        // Something else is on the only audio track over the clip's span.
        let (mut editor, id) = editor_with_shot(|_| {});
        let mut music = Asset::new("/media/song.wav".into(), MediaKind::Audio, seconds(30.0));
        music.has_audio = true;
        let bed = Clip::from_asset(&music, Time::ZERO);
        let audio = editor.project().tracks[1].id;
        for command in [
            Command::AddAssets {
                assets: vec![music],
            },
            Command::AddClip {
                track_id: audio,
                clip: bed,
            },
        ] {
            editor.dispatch(command).unwrap();
        }
        let before = editor.project().clone();
        editor
            .dispatch(detach_audio(editor.project(), id).unwrap())
            .unwrap();
        let made = editor.project().tracks.last().unwrap();
        assert_eq!(
            (made.name.as_str(), made.kind),
            ("Audio 2", TrackKind::Audio)
        );
        assert_eq!(made.clips.len(), 1);
        assert_eq!(
            editor.project().tracks[1].clips.len(),
            1,
            "the bed is alone"
        );
        // The track goes with the clip when the step is undone.
        assert!(editor.undo());
        assert_eq!(editor.project(), &before);

        // A locked lane is not written on, however empty.
        let (mut editor, id) = editor_with_shot(|_| {});
        let audio = editor.project().tracks[1].id;
        set_locked(&mut editor, audio, true);
        editor
            .dispatch(detach_audio(editor.project(), id).unwrap())
            .unwrap();
        assert!(editor.project().tracks[1].clips.is_empty());
        assert_eq!(editor.project().tracks.last().unwrap().name, "Audio 2");

        // A lane with room beside what it holds is used where the clip is.
        let (mut editor, id) = editor_with_shot(|_| {});
        let audio = editor.project().tracks[1].id;
        let shot = editor.project().assets.values().next().unwrap().clone();
        let mut early = Clip::from_asset(&shot, Time::ZERO);
        early.duration = seconds(1.0);
        editor
            .dispatch(Command::AddClip {
                track_id: audio,
                clip: early,
            })
            .unwrap();
        let tracks = editor.project().tracks.len();
        editor
            .dispatch(detach_audio(editor.project(), id).unwrap())
            .unwrap();
        assert_eq!(editor.project().tracks.len(), tracks);
        let lane = &editor.project().tracks[1];
        assert_eq!(lane.clips.len(), 2);
        assert_eq!(lane.clips[1].start, seconds(1.0));
    }

    #[test]
    fn audio_that_cannot_be_detached_is_refused_in_words() {
        // Once detached, the sound is on the linked audio clip.
        let (mut editor, id) = editor_with_shot(|_| {});
        assert_eq!(detach_refusal(editor.project(), id), None);
        editor
            .dispatch(detach_audio(editor.project(), id).unwrap())
            .unwrap();
        let again = detach_audio(editor.project(), id).unwrap_err();
        assert_eq!(again, "Its sound is on the linked audio clip already.");
        assert_eq!(detach_refusal(editor.project(), id), Some(again));
        // The audio clip itself has nothing to detach.
        let sound = sound_clips(editor.project())[0].1.id;
        let lane = detach_audio(editor.project(), sound).unwrap_err();
        assert_eq!(lane, "That clip is on an audio track already.");
        // Unlinked, the picture sounds again and can be detached again.
        editor
            .dispatch(Command::UnlinkClips { clip_ids: vec![id] })
            .unwrap();
        assert_eq!(detach_refusal(editor.project(), id), None);

        // A link to sound of other media owns nothing of this clip.
        let (mut editor, id) = editor_with_shot(|_| {});
        let mut music = Asset::new("/media/song.wav".into(), MediaKind::Audio, seconds(30.0));
        music.has_audio = true;
        let bed = Clip::from_asset(&music, Time::ZERO);
        let (audio, bed_id) = (editor.project().tracks[1].id, bed.id);
        for command in [
            Command::AddAssets {
                assets: vec![music],
            },
            Command::AddClip {
                track_id: audio,
                clip: bed,
            },
            Command::LinkClips {
                clip_ids: vec![id, bed_id],
            },
        ] {
            editor.dispatch(command).unwrap();
        }
        editor
            .dispatch(detach_audio(editor.project(), id).unwrap())
            .unwrap();
        // The three are one link now.
        assert_eq!(related(editor.project(), &[bed_id], false).len(), 3);

        // Two cuts of one shot linked to move together: the audio clip of
        // one would own the sound of both, so neither is detached while they
        // are linked.
        let (mut editor, id) = editor_with_shot(|_| {});
        let shot = editor.project().assets.values().next().unwrap().clone();
        let mut cut = Clip::from_asset(&shot, seconds(6.0));
        cut.duration = seconds(2.0);
        let (video, cut_id) = (editor.project().tracks[0].id, cut.id);
        for command in [
            Command::AddClip {
                track_id: video,
                clip: cut,
            },
            Command::LinkClips {
                clip_ids: vec![id, cut_id],
            },
        ] {
            editor.dispatch(command).unwrap();
        }
        let shared = detach_audio(editor.project(), id).unwrap_err();
        assert_eq!(
            shared,
            "Unlink the clip from the other clips of its media to detach its audio."
        );
        assert_eq!(detach_refusal(editor.project(), cut_id), Some(shared));
        // Unlinked, each is detached for itself and the other keeps its sound.
        editor
            .dispatch(Command::UnlinkClips {
                clip_ids: vec![id, cut_id],
            })
            .unwrap();
        editor
            .dispatch(detach_audio(editor.project(), id).unwrap())
            .unwrap();
        assert_eq!(editor.project().clip(cut_id).unwrap().1.linked_id, None);
        assert_eq!(detach_refusal(editor.project(), cut_id), None);

        // No sound in the media, and a title.
        let silent = project_of_video(|_| {});
        let clip = silent.tracks[0].clips[0].id;
        let none = detach_audio(&silent, clip).unwrap_err();
        assert_eq!(none, "That clip has no sound to detach.");
        let (titled, titles) = project_with(&[(0.0, 2.0)]);
        assert_eq!(detach_audio(&titled, titles[0]).unwrap_err(), none);
        // A locked track, and a clip that is gone.
        let (mut editor, id) = editor_with_shot(|_| {});
        let video = editor.project().tracks[0].id;
        set_locked(&mut editor, video, true);
        let locked = detach_audio(editor.project(), id).unwrap_err();
        assert_eq!(locked, "Unlock the clip's track to detach its audio.");
        let gone = detach_audio(editor.project(), uuid::Uuid::new_v4()).unwrap_err();
        assert_eq!(gone, "The clip no longer exists.");
    }

    #[test]
    fn a_detach_that_fails_changes_nothing_and_one_that_holds_is_saved_and_read() {
        // The batch was made, and then the lane it was made for was locked:
        // the project refuses the whole of it.
        let (mut editor, id) = editor_with_shot(|_| {});
        let command = detach_audio(editor.project(), id).unwrap();
        let audio = editor.project().tracks[1].id;
        set_locked(&mut editor, audio, true);
        let before = editor.project().clone();
        let steps = editor.history().count();
        assert!(editor.dispatch(command.clone()).is_err());
        assert_eq!(editor.project(), &before);
        assert_eq!(editor.history().count(), steps);
        assert!(sound_clips(editor.project()).is_empty());
        // So is one whose picture clip is on a track locked since.
        let (mut editor, id) = editor_with_shot(|_| {});
        let command = detach_audio(editor.project(), id).unwrap();
        let video = editor.project().tracks[0].id;
        set_locked(&mut editor, video, true);
        let before = editor.project().clone();
        assert!(editor.dispatch(command).is_err());
        assert_eq!(editor.project(), &before);

        // The batch is ordinary commands: as text it is the same edit, which
        // is how an agent sends it.
        let (mut editor, id) = editor_with_shot(|clip| {
            clip.retime = Some(editor_core::Retime::constant(clip.duration, 2.0));
        });
        let command = detach_audio(editor.project(), id).unwrap();
        let text = serde_json::to_string(&command).unwrap();
        let read: Command = serde_json::from_str(&text).unwrap();
        editor.dispatch(read).unwrap();
        let project = editor.project().clone();
        assert_eq!(sound_clips(&project).len(), 1);
        // And the pair is in the file as it is in the window.
        let dir = std::env::temp_dir().join(format!("aster-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("detached.aster");
        editor_store::save_project(&path, &project).unwrap();
        let loaded = editor_store::load_project(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(loaded, project);
        assert_eq!(
            detach_refusal(&loaded, id).as_deref(),
            Some("Its sound is on the linked audio clip already.")
        );
    }

    // ---- Background music ------------------------------------------------

    /// An editor whose project is `length` seconds long (one title) and
    /// whose library holds a sound of `sound` seconds.
    fn editor_with_sound(length: f64, sound: f64) -> (editor_core::Editor, AssetId) {
        let music = Asset::new("/media/song.wav".into(), MediaKind::Audio, seconds(sound));
        let id = music.id;
        let mut editor = editor_core::Editor::default();
        editor
            .dispatch(Command::AddAssets {
                assets: vec![music],
            })
            .unwrap();
        if length > 0.0 {
            let track_id = (editor.project().tracks.iter())
                .find(|track| track.kind == TrackKind::Text)
                .unwrap()
                .id;
            let clip = Clip::text("Title", Time::ZERO, seconds(length));
            editor
                .dispatch(Command::AddClip { track_id, clip })
                .unwrap();
        }
        (editor, id)
    }

    /// The clips of the last track, which a bed is.
    fn bed_of(project: &Project) -> &[Clip] {
        let track = project.tracks.last().unwrap();
        assert_eq!(
            (track.name.as_str(), track.kind),
            ("Music", TrackKind::Audio)
        );
        &track.clips
    }

    #[test]
    fn a_bed_repeats_the_sound_to_the_projects_end_as_one_step() {
        let (mut editor, music) = editor_with_sound(25.0, 10.0);
        let before = editor.project().clone();
        let steps = editor.history().count();
        let command = background_music(editor.project(), music).unwrap();
        let Command::Batch { commands, .. } = &command else {
            panic!("{command:?}");
        };
        assert!(matches!(commands[0], Command::AddTrack { .. }));
        assert_eq!(commands.len(), 4);
        editor.dispatch(command).unwrap();
        assert_eq!(editor.history().count(), steps + 1);
        assert_eq!(editor.undo_label(), Some("Add background music"));

        let project = editor.project();
        project.validate().unwrap();
        assert_eq!(project.tracks.len(), before.tracks.len() + 1);
        let clips = bed_of(project);
        let spans: Vec<(Time, Time)> = (clips.iter())
            .map(|clip| (clip.start, clip.duration))
            .collect();
        assert_eq!(
            spans,
            [
                (seconds(0.0), seconds(10.0)),
                (seconds(10.0), seconds(10.0)),
                (seconds(20.0), seconds(5.0)),
            ]
        );
        for clip in clips {
            assert_eq!(clip.asset_id(), Some(music));
            assert_eq!((clip.gain, clip.source_in), (0.5, Time::ZERO));
            assert_eq!(clip.fade_in, Time::ZERO);
            assert!(clip.linked_id.is_none() && clip.group_id.is_none());
        }
        assert_eq!(
            [clips[0].fade_out, clips[1].fade_out, clips[2].fade_out],
            [Time::ZERO, Time::ZERO, seconds(2.0)]
        );
        // The project is no longer than it was.
        assert_eq!(project.duration(), before.duration());

        // Undo takes the track and its clips away, and redo brings them back.
        let after = project.clone();
        assert!(editor.undo());
        assert_eq!(editor.project(), &before);
        assert!(editor.redo());
        assert_eq!(editor.project(), &after);

        // A second bed is a second track: the first is never written on.
        let again = background_music(editor.project(), music).unwrap();
        editor.dispatch(again).unwrap();
        let tracks = &editor.project().tracks;
        assert_eq!(tracks.len(), before.tracks.len() + 2);
        assert_eq!(tracks[tracks.len() - 2].clips.len(), 3);
        assert_eq!(bed_of(editor.project()).len(), 3);
    }

    #[test]
    fn a_bed_ends_with_the_project_and_fades_out_there() {
        // Longer than the project: one clip, cut at the project's end.
        let (mut editor, music) = editor_with_sound(25.0, 180.0);
        editor
            .dispatch(background_music(editor.project(), music).unwrap())
            .unwrap();
        let [only] = bed_of(editor.project()) else {
            panic!("one clip");
        };
        assert_eq!((only.start, only.duration), (Time::ZERO, seconds(25.0)));
        assert_eq!((only.gain, only.fade_out), (0.5, seconds(2.0)));

        // A last clip shorter than the fade fades over all of itself.
        let (mut editor, music) = editor_with_sound(21.0, 10.0);
        editor
            .dispatch(background_music(editor.project(), music).unwrap())
            .unwrap();
        let clips = bed_of(editor.project());
        assert_eq!(clips.len(), 3);
        assert_eq!(
            (clips[2].duration, clips[2].fade_out),
            (seconds(1.0), seconds(1.0))
        );
        assert_eq!(clips[1].fade_out, Time::ZERO);

        // A sound that divides the project is not cut, and still fades.
        let (mut editor, music) = editor_with_sound(20.0, 10.0);
        editor
            .dispatch(background_music(editor.project(), music).unwrap())
            .unwrap();
        let clips = bed_of(editor.project());
        assert_eq!(clips.len(), 2);
        assert_eq!(
            (clips[1].duration, clips[1].fade_out),
            (seconds(10.0), seconds(2.0))
        );

        // The project's length is its last clip's end, on any track.
        let (mut editor, music) = editor_with_sound(4.0, 10.0);
        let audio = editor.project().tracks[1].id;
        let asset = editor.project().assets[&music].clone();
        editor
            .dispatch(Command::AddClip {
                track_id: audio,
                clip: Clip::from_asset(&asset, seconds(2.0)),
            })
            .unwrap();
        editor
            .dispatch(background_music(editor.project(), music).unwrap())
            .unwrap();
        let clips = bed_of(editor.project());
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[1].end(), seconds(12.0));
    }

    #[test]
    fn a_bed_that_cannot_be_made_is_refused_in_words() {
        // Nothing on the timeline: there is no length to cover.
        let (editor, music) = editor_with_sound(0.0, 10.0);
        let empty = background_music(editor.project(), music).unwrap_err();
        assert_eq!(
            empty,
            "Put a clip on the timeline first: background music is as long as the project."
        );
        assert_eq!(bed_refusal(editor.project(), music), Some(empty));

        // 256 clips are made, as one step that is taken back whole; 257 are
        // not.
        let (mut editor, music) = editor_with_sound(256.0, 1.0);
        assert_eq!(bed_refusal(editor.project(), music), None);
        let command = background_music(editor.project(), music).unwrap();
        let Command::Batch { commands, .. } = &command else {
            panic!("a batch");
        };
        assert_eq!(commands.len(), BED_CLIPS + 1);
        let before = editor.project().clone();
        let steps = editor.history().count();
        editor.dispatch(command).unwrap();
        editor.project().validate().unwrap();
        assert_eq!(editor.history().count(), steps + 1);
        let clips = bed_of(editor.project());
        assert_eq!(clips.len(), BED_CLIPS);
        assert_eq!(clips[BED_CLIPS - 1].end(), editor.project().duration());
        assert_eq!(editor.project().duration(), before.duration());
        assert!(editor.undo());
        assert_eq!(editor.project(), &before);
        let (editor, music) = editor_with_sound(256.5, 1.0);
        let many = background_music(editor.project(), music).unwrap_err();
        assert_eq!(
            many,
            "That sound would repeat 257 times to cover the project; background music is 256 clips at most."
        );

        // Under a tenth of a second, however few clips it would take.
        let (editor, music) = editor_with_sound(1.0, 0.1);
        assert_eq!(bed_refusal(editor.project(), music), None);
        let (editor, music) = editor_with_sound(1.0, 0.099);
        let short = background_music(editor.project(), music).unwrap_err();
        assert_eq!(
            short,
            "That sound is shorter than a tenth of a second, too short to repeat as music."
        );

        // A video, with sound or without, is not a sound file; nor is media
        // that has left the library.
        let (editor, shot) = editor_with_shot(|_| {});
        let asset = editor.project().clip(shot).unwrap().1.asset_id().unwrap();
        let picture = background_music(editor.project(), asset).unwrap_err();
        assert_eq!(picture, "Background music is made of a sound file.");
        let gone = background_music(editor.project(), uuid::Uuid::new_v4()).unwrap_err();
        assert_eq!(gone, "That media is no longer in the library.");
    }

    #[test]
    fn a_bed_that_fails_changes_nothing_and_one_that_holds_is_saved_and_read() {
        // The batch was made, and then its sound left the library: the
        // project refuses the whole of it, track and all.
        let (mut editor, music) = editor_with_sound(25.0, 10.0);
        let command = background_music(editor.project(), music).unwrap();
        editor
            .dispatch(Command::RemoveAssets {
                asset_ids: vec![music],
            })
            .unwrap();
        let before = editor.project().clone();
        let steps = editor.history().count();
        assert!(editor.dispatch(command).is_err());
        assert_eq!(editor.project(), &before);
        assert_eq!(editor.history().count(), steps);

        // The batch is ordinary commands: as text it is the same edit, which
        // is how an agent sends it.
        let (mut editor, music) = editor_with_sound(25.0, 10.0);
        let command = background_music(editor.project(), music).unwrap();
        let text = serde_json::to_string(&command).unwrap();
        let read: Command = serde_json::from_str(&text).unwrap();
        editor.dispatch(read).unwrap();
        let project = editor.project().clone();
        assert_eq!(bed_of(&project).len(), 3);
        // And the bed is in the file as it is in the window.
        let dir = std::env::temp_dir().join(format!("aster-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bed.aster");
        editor_store::save_project(&path, &project).unwrap();
        let loaded = editor_store::load_project(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(loaded, project);
        let last = bed_of(&loaded).last().unwrap();
        assert_eq!((last.gain, last.fade_out), (0.5, seconds(2.0)));
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
