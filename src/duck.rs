//! Duck under speech: the selected clips are lowered wherever the other
//! clips are heard, as ordinary volume keyframes the person can see, edit
//! and undo.
//!
//! The gate is an energy gate over the peaks the sound lane reads, not
//! speech detection: any sound over [`SIGNAL_DB`] on another clip opens it.
//! The voices are the clips that are not selected and that `sound::sounds`
//! admits to the mix (a selected clip passed over is no voice either), so link ownership, media without sound and a keyed
//! clip at volume 0 are taken as the mixer takes them; a freeze, a reverse or
//! a ramp, which the mixer keeps silent, opens nothing. The level read is the
//! source's, before the voice's volume, fades or effects.
//!
//! Their peaks are taken one media file at a time, through
//! [`HeldPeaks::want_work`], reduced to spans in timeline time and let go,
//! so the gate never needs two files' peaks at once and works with more
//! voices than the held peaks have room for. When every voice has been
//! taken the spans are united, and `sound::duck` turns them into the keys of
//! one `Batch` of `SetKeyframes`, a clip each: one entry of the history.
//!
//! [`Ducking`] holds the one run under way. It is given up on Escape and
//! when the project changes. Nothing here reads a file or waits.
use crate::{
    normalize,
    peaks::HeldPeaks,
    ui::{DuckProgress, fades},
};
use editor_core::{
    AssetId, Clip, ClipId, Command, ParamKey, Project, Time, Track,
    sound::{self, Sounding},
};
use editor_media::{Peak, Peaks};
use std::collections::VecDeque;
use uuid::Uuid;

/// What the history calls a duck, and what its routes are named.
pub const LABEL: &str = "Duck under speech";
/// How far a clip is lowered under the others: a quarter of its volume,
/// -12 dB.
pub const DEPTH: f32 = 0.25;
/// How long the level takes to go down before the others are heard, and
/// to come back after.
pub const ATTACK: Time = Time(150_000);
pub const RELEASE: Time = Time(400_000);
/// How often the gate looks: a peak bucket.
pub const STEP: Time = Time(10_000);
/// The level of a voice's source above which the gate opens, in dBFS.
pub const SIGNAL_DB: f32 = -35.0;
/// Sound shorter than this is no reason to duck.
pub const SHORTEST: Time = Time(200_000);
/// How long a span is held open after its sound.
pub const HANGOVER: Time = Time(300_000);
/// Spans closer than this are one.
pub const BRIDGED: Time = Time(1_000_000);
/// The spans one media file gives at most; the closest are merged beyond.
pub const MOST_SPANS: usize = 4096;
/// The depths a person may ask for, in dB below the clip's volume.
pub const SHALLOWEST_DB: f32 = 1.0;
pub const DEEPEST_DB: f32 = 60.0;

/// The depth of a dip `db` below the volume, either sign; `None` outside
/// [`SHALLOWEST_DB`] to [`DEEPEST_DB`].
pub fn depth_of_db(db: f32) -> Option<f32> {
    let db = db.abs();
    (SHALLOWEST_DB..=DEEPEST_DB)
        .contains(&db)
        .then(|| 10_f64.powf(-f64::from(db) / 20.0) as f32)
}

fn volume_keys(clip: &Clip) -> Option<&[editor_core::Keyframe]> {
    (clip.animation.iter())
        .find(|track| track.target.is_none() && track.key == ParamKey::Gain)
        .map(|track| track.keys.as_slice())
        .filter(|keys| !keys.is_empty())
}

/// The level a ducked clip comes back to between its dips: its volume, or
/// for a clip whose volume is keyed, its loudest key, since those keys are
/// replaced.
pub fn base(clip: &Clip) -> f32 {
    match volume_keys(clip) {
        Some(keys) => (keys.iter())
            .map(|key| key.value)
            .fold(f32::NEG_INFINITY, f32::max),
        None => clip.gain,
    }
}

/// Why `clip` is not ducked, when it is not: a clip that does not sound
/// (`sound::sounds`), a picture whose linked audio clip owns its sound among
/// them, a clip on a locked track and a clip whose every volume key is 0.
pub fn refusal(project: &Project, clip: ClipId) -> Option<String> {
    let Some((track, clip)) = project.clip(clip) else {
        return Some("The clip no longer exists.".to_owned());
    };
    if !fades::has_sound(project, clip) {
        return Some("That clip has no sound to duck.".to_owned());
    }
    if sound::owned(project, clip) {
        return Some(format!("{}.", fades::OWNED));
    }
    if !sound::sounds(project, track, clip) {
        return Some(format!(
            "That clip is not heard: {}.",
            normalize::unheard(project, track)
        ));
    }
    if track.locked {
        return Some("Unlock the clip's track to duck its sound.".to_owned());
    }
    (base(clip) <= 0.0)
        .then(|| "Every volume key of that clip is at 0%: there is nothing to duck.".to_owned())
}

/// The clips of `clips` that would be ducked, or why none would be.
pub fn takers(project: &Project, clips: &[ClipId]) -> Result<Vec<ClipId>, String> {
    let mut refused = None;
    let taken: Vec<ClipId> = (clips.iter().copied())
        .filter(|id| match refusal(project, *id) {
            Some(reason) => {
                refused.get_or_insert(reason);
                false
            }
            None => true,
        })
        .collect();
    if taken.is_empty() {
        return Err(refused.unwrap_or_else(|| "Select a clip first.".to_owned()));
    }
    Ok(taken)
}

/// How many of `clips` that would be ducked have volume keys, which a duck
/// replaces: the person is asked first.
pub fn keyed(project: &Project, clips: &[ClipId]) -> usize {
    (takers(project, clips).unwrap_or_default().iter())
        .filter(|id| (project.clip(**id)).is_some_and(|(_, clip)| volume_keys(clip).is_some()))
        .count()
}

/// What the person is asked before keys are replaced.
pub fn question(keyed: usize) -> String {
    match keyed {
        1 => "Replace this clip's volume keyframes?".to_owned(),
        count => format!("Replace the volume keyframes of {count} clips?"),
    }
}

fn name_of(clip: &Clip) -> String {
    match clip.name.trim() {
        "" => "The clip".to_owned(),
        name => name.to_owned(),
    }
}

// Sorts `spans` and makes one of each run whose gaps `joins` says close.
fn join(spans: &mut Vec<(Time, Time)>, joins: impl Fn(Time) -> bool) {
    spans.sort_unstable();
    let mut joined: Vec<(Time, Time)> = Vec::with_capacity(spans.len());
    for (from, to) in spans.drain(..) {
        match joined.last_mut() {
            Some(last) if joins(from - last.1) => last.1 = last.1.max(to),
            _ => joined.push((from, to)),
        }
    }
    *spans = joined;
}

/// The timeline the clips of `selected` that are ducked play over, as
/// sorted stretches apart.
fn window(project: &Project, selected: &[ClipId]) -> Vec<(Time, Time)> {
    let targets = takers(project, selected).unwrap_or_default();
    let mut window: Vec<(Time, Time)> = (targets.iter())
        .filter_map(|id| project.clip(*id))
        .map(|(_, clip)| (clip.start, clip.end()))
        .collect();
    join(&mut window, |gap| gap <= Time::ZERO);
    window
}

fn overlaps(window: &[(Time, Time)], clip: &Clip) -> bool {
    (window.iter()).any(|(from, to)| *from < clip.end() && clip.start < *to)
}

/// The clips heard under the ducked clips, with their tracks, in the order
/// of the tracks: not selected, admitted to the mix by `sound::sounds`, and
/// playing while a ducked clip does. A selected clip that is passed over is
/// no voice either.
fn voices<'a>(
    project: &'a Project,
    selected: &[ClipId],
    window: &[(Time, Time)],
) -> Vec<(&'a Track, &'a Clip)> {
    let sounding = Sounding::of(project);
    (project.tracks.iter())
        .flat_map(|track| track.clips.iter().map(move |clip| (&**track, clip)))
        .filter(|(track, clip)| {
            !selected.contains(&clip.id)
                && overlaps(window, clip)
                && sounding.sounds(project, track, clip)
        })
        .collect()
}

/// The media of the voices under the clips of `selected` that are ducked,
/// each once, in the order the gate takes them.
pub fn voice_media(project: &Project, selected: &[ClipId]) -> Vec<AssetId> {
    let mut media: Vec<AssetId> = Vec::new();
    for (_, clip) in voices(project, selected, &window(project, selected)) {
        if let Some(id) = clip.asset_id()
            && !media.contains(&id)
        {
            media.push(id);
        }
    }
    media
}

/// Where the voices of one media file are heard under the clips of
/// `selected` that are ducked, read from its peaks: in steps of [`STEP`], where the clip plays a straight
/// stretch of its time map and the level of the source it plays there
/// passes [`SIGNAL_DB`]. In timeline time, sorted and apart, at most
/// [`MOST_SPANS`].
pub fn asset_spans(
    project: &Project,
    selected: &[ClipId],
    asset: AssetId,
    peaks: &Peaks,
) -> Vec<(Time, Time)> {
    let window = window(project, selected);
    let mut raw: Vec<(Time, Time)> = Vec::new();
    for (_, clip) in voices(project, selected, &window) {
        if clip.asset_id() != Some(asset) {
            continue;
        }
        for (from, to) in &window {
            let (from, to) = ((*from).max(clip.start), (*to).min(clip.end()));
            let mut open: Option<Time> = None;
            let mut at = from;
            while at < to {
                // `sound::audible_at`, whose admission does not change along
                // the clip and was asked of it once, above.
                let heard = sound::steady_speed(clip, at).is_some()
                    && (peaks.at(clip.source_time(at)))
                        .is_some_and(|peak| Peak::decibels(peak.rms) > SIGNAL_DB);
                match (heard, open) {
                    (true, None) => open = Some(at),
                    (false, Some(start)) => {
                        raw.push((start, at));
                        open = None;
                    }
                    _ => {}
                }
                at = at + STEP;
            }
            if let Some(start) = open {
                raw.push((start, to));
            }
        }
    }
    join(&mut raw, |gap| gap <= Time::ZERO);
    sound::merge_closest(&mut raw, MOST_SPANS);
    raw
}

/// The spans of every voice made one: those shorter than [`SHORTEST`] are
/// dropped, each is held open [`HANGOVER`] longer, and those less than
/// [`BRIDGED`] apart are merged.
pub fn unite(mut raw: Vec<(Time, Time)>) -> Vec<(Time, Time)> {
    join(&mut raw, |gap| gap <= Time::ZERO);
    raw.retain(|(from, to)| *to - *from >= SHORTEST);
    for span in &mut raw {
        span.1 = span.1 + HANGOVER;
    }
    join(&mut raw, |gap| gap < BRIDGED);
    raw
}

/// The one step that ducks `targets` under `spans` by `depth`, and what to
/// say of it; or why nothing is written. A target that nothing is heard
/// under is left as it is.
pub fn command(
    project: &Project,
    targets: &[ClipId],
    spans: &[(Time, Time)],
    depth: f32,
) -> Result<(Command, String), String> {
    let mut commands = Vec::new();
    let (mut ducked, mut quiet): (Vec<(String, usize)>, Vec<String>) = (Vec::new(), Vec::new());
    let mut merged = 0;
    for id in targets {
        let Some((_, clip)) = project.clip(*id) else {
            continue;
        };
        let local: Vec<(Time, Time)> = (spans.iter())
            .map(|(from, to)| (*from - clip.start, *to - clip.start))
            .collect();
        let duck = sound::duck(&local, clip.duration, base(clip), depth, ATTACK, RELEASE);
        if duck.keys.is_empty() {
            quiet.push(name_of(clip));
            continue;
        }
        merged += duck.merged;
        ducked.push((name_of(clip), duck.dips));
        commands.push(Command::SetKeyframes {
            clip_id: clip.id,
            target: None,
            key: ParamKey::Gain,
            keys: duck.keys,
        });
    }
    let said = match ducked.as_slice() {
        [] => {
            let name = quiet.first().map_or("the clip", String::as_str);
            return Err(format!(
                "Nothing is heard under {name}: no volume keyframes were written."
            ));
        }
        [(name, 1)] => format!("{name} is ducked in one place where other clips are heard."),
        [(name, dips)] => format!("{name} is ducked in {dips} places where other clips are heard."),
        several => format!(
            "{} clips are ducked where other clips are heard.",
            several.len()
        ),
    };
    let merged = match merged {
        0 => String::new(),
        1 => " One dip was merged into its neighbour to stay within 256 volume keys.".to_owned(),
        count => format!(
            " {count} dips were merged into their neighbours to stay within 256 volume keys."
        ),
    };
    let quiet = match quiet.as_slice() {
        [] => String::new(),
        [name] => format!(" Nothing is heard under {name}; it was left as it was."),
        names => format!(
            " Nothing is heard under {} of them; they were left as they were.",
            names.len()
        ),
    };
    let command = Command::Batch {
        label: LABEL.to_owned(),
        commands,
    };
    Ok((command, format!("{said}{merged}{quiet}")))
}

/// What one step of a run came to.
#[derive(Debug)]
pub enum Stepped {
    /// One media file's peaks were taken, or it was left out: the next is
    /// wanted in the next frame.
    Took,
    /// Every voice was taken: the step to take and what to say, or why
    /// nothing is written. The step is boxed: it may be large.
    Done(Result<(Box<Command>, String), String>),
    /// The project changed under it: what to say, and the lane's read to
    /// give up, if one was under way.
    GivenUp { said: String, cancel: Option<u64> },
}

/// A duck under way: what was asked, what is left to take, and what was
/// taken.
#[derive(Debug)]
struct Run {
    instance: Uuid,
    revision: u64,
    /// The clips selected, none of which is a voice.
    selected: Vec<ClipId>,
    /// Those of them that are ducked.
    targets: Vec<ClipId>,
    depth: f32,
    /// The media still to be taken, the one wanted now first.
    voices: VecDeque<AssetId>,
    total: usize,
    raw: Vec<(Time, Time)>,
    /// The media whose peaks could not be read, by name.
    left_out: Vec<String>,
}

/// The one duck under way, if any.
#[derive(Debug, Default)]
pub struct Ducking {
    run: Option<Run>,
}

impl Ducking {
    /// Whether a duck is under way.
    pub fn busy(&self) -> bool {
        self.run.is_some()
    }

    /// Starts ducking each of `clips`, the clips selected, that [`refusal`]
    /// lets through by `depth`, under the clips that are not selected, in
    /// the project `instance` at `revision`: nothing is asked
    /// of the lane here, and the first media is wanted by the first
    /// [`Self::step`]. Hands back what to say, or why nothing was started.
    /// A run under way is to be given up first.
    pub fn start(
        &mut self,
        project: &Project,
        instance: Uuid,
        revision: u64,
        clips: &[ClipId],
        depth: f32,
    ) -> Result<String, String> {
        if !(0.0..1.0).contains(&depth) {
            return Err("A duck lowers a clip below its volume.".to_owned());
        }
        let targets = takers(project, clips)?;
        let passed = match clips.len() - targets.len() {
            0 => String::new(),
            count => {
                let reason = (clips.iter())
                    .find_map(|id| refusal(project, *id))
                    .unwrap_or_default();
                match count {
                    1 => format!(" One was passed over: {reason}"),
                    count => format!(" {count} were passed over: {reason}"),
                }
            }
        };
        let media = voice_media(project, clips);
        if media.is_empty() {
            return Err(match targets.as_slice() {
                [one] => format!(
                    "No other clip is heard under {}: there is nothing to duck it under.",
                    project
                        .clip(*one)
                        .map_or_else(|| "it".to_owned(), |(_, clip)| name_of(clip))
                ),
                _ => {
                    "No other clip is heard under these clips: there is nothing to duck them under."
                        .to_owned()
                }
            });
        }
        let said = format!(
            "Reading the sound of {} to duck under. Escape stops it.{passed}",
            match media.len() {
                1 => "one clip's media".to_owned(),
                count => format!("{count} media files"),
            }
        );
        self.run = Some(Run {
            instance,
            revision,
            selected: clips.to_vec(),
            targets,
            depth,
            total: media.len(),
            voices: media.into(),
            raw: Vec::new(),
            left_out: Vec::new(),
        });
        Ok(said)
    }

    /// Takes the next media's peaks when they are held, or wants them of
    /// `peaks`; once every one is taken, hands back the step. `None` while
    /// it waits. Given up when the project is no longer `instance` at
    /// `revision`.
    pub fn step(
        &mut self,
        project: &Project,
        instance: Uuid,
        revision: u64,
        peaks: &mut HeldPeaks,
    ) -> Option<Stepped> {
        let run = self.run.as_mut()?;
        if run.instance != instance || run.revision != revision {
            let cancel = self.give_up(peaks).flatten();
            return Some(Stepped::GivenUp {
                said: format!(
                    "{LABEL} was stopped: the project changed while its sound was read. \
                     Nothing was changed."
                ),
                cancel,
            });
        }
        let Some(&id) = run.voices.front() else {
            let run = self.run.take()?;
            let done = finish(project, &run).map(|(command, said)| (Box::new(command), said));
            return Some(Stepped::Done(done));
        };
        let Some(asset) = project.assets.get(&id) else {
            run.voices.pop_front();
            return Some(Stepped::Took);
        };
        if let Some(held) = peaks.held(asset).cloned() {
            run.raw
                .extend(asset_spans(project, &run.selected, id, &held));
            peaks.let_go(id);
            run.voices.pop_front();
            return Some(Stepped::Took);
        }
        if peaks.failed(asset) {
            run.left_out.push(asset.name.clone());
            peaks.let_go(id);
            run.voices.pop_front();
            return Some(Stepped::Took);
        }
        peaks.want_work(asset);
        None
    }

    /// Gives up the run under way, if one was: the media it wanted is let
    /// go, and the lane's read of it, when one was under way, is handed
    /// back to be given up.
    pub fn give_up(&mut self, peaks: &mut HeldPeaks) -> Option<Option<u64>> {
        let run = self.run.take()?;
        Some(run.voices.front().and_then(|id| peaks.let_go(*id)))
    }

    /// Forgets the run: the project is another, and the peaks with it.
    pub fn clear(&mut self) {
        self.run = None;
    }

    /// How far the run is, for the inspector to show.
    pub fn progress(&self, project: &Project) -> Option<DuckProgress> {
        let run = self.run.as_ref()?;
        let reading = (run.voices.front())
            .and_then(|id| project.assets.get(id))
            .map(|asset| asset.name.clone());
        Some(DuckProgress {
            clips: run.targets.clone(),
            reading,
            taken: run.total - run.voices.len(),
            total: run.total,
        })
    }
}

/// The step a run comes to once every voice is taken.
fn finish(project: &Project, run: &Run) -> Result<(Command, String), String> {
    let left_out = match run.left_out.as_slice() {
        [] => String::new(),
        [name] => format!(" {name} was left out: its sound could not be read."),
        names => format!(
            " {} were left out: their sound could not be read.",
            names.join(", ")
        ),
    };
    if run.left_out.len() == run.total {
        return Err(format!("Nothing was ducked.{left_out}"));
    }
    let spans = unite(run.raw.clone());
    match command(project, &run.targets, &spans, run.depth) {
        Ok((command, said)) => Ok((command, format!("{said}{left_out}"))),
        Err(said) => Err(format!("{said}{left_out}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        peaks::{Answer, Taken},
        rules,
    };
    use editor_core::{
        Asset, Ease, Editor, Keyframe, MediaKind, Retime, TimeKey, TrackId, TrackKind,
    };
    use editor_media::{KeptPeaks, SourceIdentity};
    use std::{collections::HashMap, sync::Arc};

    fn seconds(value: f64) -> Time {
        Time::from_seconds(value)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    /// Peaks of `length` seconds of sound, silent but for `loud` stretches
    /// of source at -20 dBFS.
    fn peaks(length: f64, loud: &[(f64, f64)]) -> Peaks {
        // `round((dBFS + 96) * 255 / 96)`, as the lane writes a level.
        let level = ((-20.0_f64 + 96.0) * 255.0 / 96.0).round() as u8;
        let bucket = |at: f64| (at * 100.0).round() as usize;
        let mut buckets = vec![[0, 0, 0, 0]; bucket(length)];
        for (from, to) in loud {
            for each in &mut buckets[bucket(*from)..bucket(*to)] {
                *each = [0xc0, 0x40, level, 0];
            }
        }
        Peaks { buckets }
    }

    fn span(from: f64, to: f64) -> (Time, Time) {
        (seconds(from), seconds(to))
    }

    /// Whether two steps are the same, as they are written.
    fn same(one: &Command, other: &Command) -> bool {
        serde_json::to_value(one).unwrap() == serde_json::to_value(other).unwrap()
    }

    /// `music.wav`, sixty seconds, with a twenty second clip of it from 0
    /// on the first audio track: the clip ducked in these tests.
    struct Bench {
        editor: Editor,
        music: ClipId,
    }

    impl Bench {
        fn new() -> Self {
            let mut editor = Editor::default();
            let music = sound("music.wav");
            let mut clip = Clip::from_asset(&music, Time::ZERO);
            clip.duration = seconds(20.0);
            let id = clip.id;
            editor
                .dispatch(Command::AddAssets {
                    assets: vec![music],
                })
                .unwrap();
            let track_id = track_of(&editor, TrackKind::Audio);
            editor
                .dispatch(Command::AddClip { track_id, clip })
                .unwrap();
            Self { editor, music: id }
        }

        fn project(&self) -> &Project {
            self.editor.project()
        }

        /// A clip of `asset` from `start` on a new audio track, shaped by
        /// `shape`, from the start of its media.
        fn voice(&mut self, asset: &Asset, start: f64, shape: impl FnOnce(&mut Clip)) -> ClipId {
            if !self.project().assets.contains_key(&asset.id) {
                self.editor
                    .dispatch(Command::AddAssets {
                        assets: vec![asset.clone()],
                    })
                    .unwrap();
            }
            let name = format!("Voice {}", self.project().tracks.len());
            self.editor
                .dispatch(Command::AddTrack {
                    track: Track::new(&name, TrackKind::Audio),
                })
                .unwrap();
            let track_id = (self.project().tracks.iter())
                .find(|track| track.name == name)
                .unwrap()
                .id;
            self.add(track_id, asset, start, shape)
        }

        fn add(
            &mut self,
            track_id: TrackId,
            asset: &Asset,
            start: f64,
            shape: impl FnOnce(&mut Clip),
        ) -> ClipId {
            let mut clip = Clip::from_asset(asset, seconds(start));
            clip.duration = seconds(10.0);
            shape(&mut clip);
            let id = clip.id;
            self.editor
                .dispatch(Command::AddClip { track_id, clip })
                .unwrap();
            id
        }

        /// The spans the gate finds under the music with every voice's
        /// peaks to hand: the run that is not bounded.
        fn gate(&self, peaks: &HashMap<AssetId, Peaks>) -> Vec<(Time, Time)> {
            let project = self.project();
            let targets = [self.music];
            let mut raw = Vec::new();
            for id in voice_media(project, &targets) {
                if let Some(peaks) = peaks.get(&id) {
                    raw.extend(asset_spans(project, &targets, id, peaks));
                }
            }
            unite(raw)
        }
    }

    fn sound(name: &str) -> Asset {
        let mut asset = Asset::new(
            format!("/media/{name}").into(),
            MediaKind::Audio,
            seconds(60.0),
        );
        asset.name = name.to_owned();
        asset.has_audio = true;
        asset.file_size = 4321;
        asset
    }

    fn track_of(editor: &Editor, kind: TrackKind) -> TrackId {
        (editor.project().tracks.iter())
            .find(|track| track.kind == kind)
            .unwrap()
            .id
    }

    fn track_state(track_id: TrackId, muted: Option<bool>, locked: Option<bool>) -> Command {
        Command::SetTrackState {
            track_id,
            locked,
            hidden: None,
            muted,
            name: None,
            gain: None,
            solo: None,
        }
    }

    // ---- The gate ----------------------------------------------------------

    #[test]
    fn a_short_burst_is_ignored_and_bursts_half_a_second_apart_are_one_span() {
        let mut bench = Bench::new();
        let voice = sound("voice.wav");
        bench.voice(&voice, 2.0, |_| {});
        // 150 ms at 1 s of source, then two half seconds 0.5 s apart.
        let read = peaks(60.0, &[(1.0, 1.15), (4.0, 4.5), (5.0, 5.5)]);
        let raw = asset_spans(bench.project(), &[bench.music], voice.id, &read);
        assert_eq!(raw, [span(3.0, 3.15), span(6.0, 6.5), span(7.0, 7.5)]);
        let peaks = HashMap::from([(voice.id, read)]);
        assert_eq!(bench.gate(&peaks), [span(6.0, 7.8)]);
        // Under the threshold is silence: -36 dBFS opens nothing.
        let mut quiet = peaks[&voice.id].clone();
        let below = ((-36.0_f64 + 96.0) * 255.0 / 96.0).floor() as u8;
        for bucket in &mut quiet.buckets {
            bucket[2] = bucket[2].min(below);
        }
        assert!(asset_spans(bench.project(), &[bench.music], voice.id, &quiet).is_empty());
    }

    #[test]
    fn a_muted_voice_track_is_no_voice() {
        let mut bench = Bench::new();
        let voice = sound("voice.wav");
        let clip = bench.voice(&voice, 2.0, |_| {});
        assert_eq!(voice_media(bench.project(), &[bench.music]), [voice.id]);
        let track_id = bench.project().clip(clip).unwrap().0.id;
        bench
            .editor
            .dispatch(track_state(track_id, Some(true), None))
            .unwrap();
        assert!(voice_media(bench.project(), &[bench.music]).is_empty());
        let read = peaks(60.0, &[(0.0, 10.0)]);
        assert!(asset_spans(bench.project(), &[bench.music], voice.id, &read).is_empty());
        // With no voice left there is nothing to start.
        let mut ducking = Ducking::default();
        let refused = ducking.start(
            bench.project(),
            bench.editor.instance_id(),
            bench.editor.revision(),
            &[bench.music],
            DEPTH,
        );
        assert_eq!(
            refused.unwrap_err(),
            "No other clip is heard under music.wav: there is nothing to duck it under."
        );
        assert!(!ducking.busy());
    }

    #[test]
    fn a_voice_at_twice_its_speed_is_read_at_its_source_time() {
        let mut bench = Bench::new();
        let voice = sound("voice.wav");
        bench.voice(&voice, 2.0, |clip| {
            clip.playback_rate = 2.0;
            clip.duration = seconds(5.0);
        });
        // Source 4 s to 5 s plays from 2 + 4 / 2 s to 2 + 5 / 2 s.
        let read = peaks(60.0, &[(4.0, 5.0)]);
        let raw = asset_spans(bench.project(), &[bench.music], voice.id, &read);
        assert_eq!(raw, [span(4.0, 4.5)]);
    }

    #[test]
    fn the_picture_of_a_detached_pair_is_no_voice_and_its_audio_clip_is() {
        let mut bench = Bench::new();
        let mut shot = sound("shot.mp4");
        shot.kind = MediaKind::Video;
        bench
            .editor
            .dispatch(Command::AddAssets {
                assets: vec![shot.clone()],
            })
            .unwrap();
        let track_id = track_of(&bench.editor, TrackKind::Video);
        let picture = bench.add(track_id, &shot, 2.0, |_| {});
        let targets = [bench.music];
        let heard = |bench: &Bench| -> Vec<ClipId> {
            let project = bench.project();
            (voices(project, &targets, &window(project, &targets)).iter())
                .map(|(_, clip)| clip.id)
                .collect()
        };
        assert_eq!(heard(&bench), [picture], "its sound is its own");
        let detach = rules::detach_audio(bench.project(), picture).unwrap();
        bench.editor.dispatch(detach).unwrap();
        let heard = heard(&bench);
        assert_eq!(heard.len(), 1);
        assert_ne!(heard[0], picture, "the picture's sound is owned");
        let (track, owner) = bench.project().clip(heard[0]).unwrap();
        assert_eq!(track.kind, TrackKind::Audio);
        assert_eq!(owner.asset_id(), Some(shot.id));
        // Read through the audio clip, the same stretch is found.
        let read = peaks(60.0, &[(1.0, 2.0)]);
        let raw = asset_spans(bench.project(), &targets, shot.id, &read);
        assert_eq!(raw, [span(3.0, 4.0)]);
    }

    #[test]
    fn a_voice_at_volume_zero_counts_only_with_volume_keys() {
        let mut bench = Bench::new();
        let voice = sound("voice.wav");
        let clip = bench.voice(&voice, 2.0, |_| {});
        bench
            .editor
            .dispatch(Command::SetGain {
                clip_id: clip,
                gain: 0.0,
            })
            .unwrap();
        assert!(voice_media(bench.project(), &[bench.music]).is_empty());
        bench
            .editor
            .dispatch(Command::SetKeyframes {
                clip_id: clip,
                target: None,
                key: ParamKey::Gain,
                keys: vec![Keyframe {
                    at: Time::ZERO,
                    value: 1.0,
                    ease: Ease::Linear,
                }],
            })
            .unwrap();
        assert_eq!(voice_media(bench.project(), &[bench.music]), [voice.id]);
    }

    #[test]
    fn loud_source_under_a_freeze_opens_nothing() {
        let mut bench = Bench::new();
        let voice = sound("voice.wav");
        let key = |at: f64, source: f64| TimeKey {
            at: seconds(at),
            source: seconds(source),
            ease: Ease::Linear,
        };
        // Source 0 s to 1 s, then 1 s held for two, then 1 s to 2 s.
        bench.voice(&voice, 2.0, |clip| {
            clip.duration = seconds(4.0);
            clip.retime = Some(Retime {
                keys: vec![key(0.0, 0.0), key(1.0, 1.0), key(3.0, 1.0), key(4.0, 2.0)],
            });
        });
        // The held frame's source is loud: only where it plays forward does
        // it open the gate.
        let read = peaks(60.0, &[(1.0, 1.5)]);
        let raw = asset_spans(bench.project(), &[bench.music], voice.id, &read);
        assert_eq!(raw, [span(5.0, 5.5)]);
    }

    // ---- One media file at a time ----------------------------------------------

    /// The lane, standing in: every question is answered in the frame it is
    /// put. The cache keeps the peaks of those `kept` says, and reads the
    /// rest, failing those `broken` says.
    struct Lane {
        peaks: HashMap<AssetId, Peaks>,
        kept: Vec<AssetId>,
        broken: Vec<AssetId>,
        request: u64,
        /// What was asked, in order.
        asked: Vec<(AssetId, crate::peaks::Ask)>,
    }

    impl Lane {
        fn new(peaks: HashMap<AssetId, Peaks>) -> Self {
            Self {
                peaks,
                kept: Vec::new(),
                broken: Vec::new(),
                request: 0,
                asked: Vec::new(),
            }
        }

        fn answer(&mut self, held: &mut HeldPeaks, project: &Project, instance: Uuid) {
            use crate::peaks::Ask;
            for (id, ask) in held.take_wants() {
                self.request += 1;
                held.asked(id, self.request);
                self.asked.push((id, ask));
                let asset = &project.assets[&id];
                let answer = match ask {
                    Ask::Kept if self.kept.contains(&id) => {
                        Answer::Kept(KeptPeaks::Held(Arc::new(self.peaks[&id].clone())))
                    }
                    Ask::Kept => Answer::Kept(KeptPeaks::Missing),
                    Ask::Sound if self.broken.contains(&id) => {
                        Answer::Read(Err("ffmpeg: the sound could not be read".to_owned()))
                    }
                    Ask::Sound => Answer::Read(Ok(Arc::new(self.peaks[&id].clone()))),
                };
                let source = SourceIdentity::of(asset);
                let taken = held.take(
                    instance,
                    Some(asset),
                    self.request,
                    instance,
                    id,
                    &source,
                    answer,
                );
                assert_ne!(taken, Taken::Dropped);
            }
        }
    }

    /// Frames of the application while a duck runs: the run steps, and
    /// what it wanted is asked and answered. Hands back what it came to.
    fn run(
        bench: &Bench,
        ducking: &mut Ducking,
        held: &mut HeldPeaks,
        lane: &mut Lane,
        bound: usize,
    ) -> Result<(Command, String), String> {
        let (project, here) = (bench.project(), bench.editor.instance_id());
        let revision = bench.editor.revision();
        for _ in 0..200 {
            held.begin_frame();
            match ducking.step(project, here, revision, held) {
                Some(Stepped::Done(done)) => return done.map(|(command, said)| (*command, said)),
                Some(Stepped::GivenUp { said, .. }) => panic!("given up: {said}"),
                Some(Stepped::Took) | None => {}
            }
            lane.answer(held, project, here);
            assert!(held.bytes() <= bound, "{} bytes held", held.bytes());
        }
        panic!("the duck did not end");
    }

    #[test]
    fn six_voices_are_taken_one_at_a_time_through_a_map_that_holds_two() {
        let mut bench = Bench::new();
        let mut read = HashMap::new();
        let mut voices = Vec::new();
        for index in 0..6 {
            let voice = sound(&format!("voice {index}.wav"));
            let at = 1.0 + 3.0 * index as f64;
            bench.voice(&voice, at, |clip| clip.duration = seconds(2.0));
            // Twenty seconds each: 8,000 bytes of peaks.
            read.insert(voice.id, peaks(20.0, &[(0.5, 1.5)]));
            voices.push(voice.id);
        }
        // Room for two files' peaks and not three.
        let bound = 2 * 8_000 + 4_000;
        let mut held = HeldPeaks::bounded(bound);
        let mut ducking = Ducking::default();
        let mut lane = Lane::new(read.clone());
        // Half are in the cache; the rest are read.
        lane.kept = voices[..3].to_vec();
        let said = ducking.start(
            bench.project(),
            bench.editor.instance_id(),
            bench.editor.revision(),
            &[bench.music],
            DEPTH,
        );
        assert_eq!(
            said.unwrap(),
            "Reading the sound of 6 media files to duck under. Escape stops it."
        );
        let done = run(&bench, &mut ducking, &mut held, &mut lane, bound).unwrap();
        // Each in turn, in the order of the tracks, each asked once of the
        // cache and the unkept read once.
        let order: Vec<AssetId> = lane.asked.iter().map(|(id, _)| *id).collect();
        let mut expected = Vec::new();
        for (index, id) in voices.iter().enumerate() {
            expected.push(*id);
            if index >= 3 {
                expected.push(*id);
            }
        }
        assert_eq!(order, expected);
        // Every one let go once it was taken.
        assert_eq!(held.bytes(), 0);
        assert!(!ducking.busy());
        // The same as a run with every file to hand.
        let spans = bench.gate(&read);
        assert_eq!(spans.len(), 6);
        let unbounded = command(bench.project(), &[bench.music], &spans, DEPTH).unwrap();
        assert!(same(&done.0, &unbounded.0));
        assert_eq!(done.1, unbounded.1);
        assert_eq!(
            done.1,
            "music.wav is ducked in 6 places where other clips are heard."
        );
    }

    #[test]
    fn a_voice_whose_peaks_fail_is_left_out_by_name_and_the_rest_still_duck() {
        let mut bench = Bench::new();
        let (good, bad) = (sound("good.wav"), sound("bad.wav"));
        bench.voice(&good, 2.0, |_| {});
        bench.voice(&bad, 12.0, |clip| clip.duration = seconds(5.0));
        let peaks = HashMap::from([
            (good.id, peaks(60.0, &[(1.0, 2.0)])),
            (bad.id, peaks(60.0, &[(1.0, 2.0)])),
        ]);
        let mut lane = Lane::new(peaks.clone());
        lane.broken = vec![bad.id];
        let (mut held, mut ducking) = (HeldPeaks::default(), Ducking::default());
        let here = bench.editor.instance_id();
        let revision = bench.editor.revision();
        ducking
            .start(bench.project(), here, revision, &[bench.music], DEPTH)
            .unwrap();
        let bound = crate::peaks::MAX_HELD_PEAK_BYTES;
        let (command, said) = run(&bench, &mut ducking, &mut held, &mut lane, bound).unwrap();
        assert_eq!(
            said,
            "music.wav is ducked in one place where other clips are heard. \
             bad.wav was left out: its sound could not be read."
        );
        let only = HashMap::from([(good.id, peaks[&good.id].clone())]);
        let spans = bench.gate(&only);
        assert_eq!(spans, [span(3.0, 4.3)]);
        let unbounded = super::command(bench.project(), &[bench.music], &spans, DEPTH).unwrap();
        assert!(same(&command, &unbounded.0));

        // Run again, the failure is remembered: the file is not read again,
        // and is left out all the same.
        lane.asked.clear();
        ducking
            .start(bench.project(), here, revision, &[bench.music], DEPTH)
            .unwrap();
        let (_, again) = run(&bench, &mut ducking, &mut held, &mut lane, bound).unwrap();
        assert_eq!(again, said);
        assert!(lane.asked.iter().all(|(id, _)| *id != bad.id));
        assert!(held.failed(&bench.project().assets[&bad.id]));

        // When none can be read, nothing is ducked and each is named.
        let mut lane = Lane::new(peaks);
        lane.broken = vec![good.id, bad.id];
        let (mut held, mut ducking) = (HeldPeaks::default(), Ducking::default());
        ducking
            .start(bench.project(), here, revision, &[bench.music], DEPTH)
            .unwrap();
        let refused = run(&bench, &mut ducking, &mut held, &mut lane, bound).unwrap_err();
        assert_eq!(
            refused,
            "Nothing was ducked. good.wav, bad.wav were left out: their sound could not be read."
        );
    }

    #[test]
    fn a_selected_clip_that_is_passed_over_is_no_voice() {
        let mut bench = Bench::new();
        let (locked, other) = (sound("locked.wav"), sound("other.wav"));
        let passed = bench.voice(&locked, 2.0, |_| {});
        bench.voice(&other, 12.0, |clip| clip.duration = seconds(5.0));
        let track_id = bench.project().clip(passed).unwrap().0.id;
        bench
            .editor
            .dispatch(track_state(track_id, None, Some(true)))
            .unwrap();
        let read = HashMap::from([
            (locked.id, peaks(60.0, &[(1.0, 2.0)])),
            (other.id, peaks(60.0, &[(1.0, 2.0)])),
        ]);
        // Selected beside the music, the clip on the locked track is passed
        // over: it is heard, but is not a voice to duck under.
        let selected = [passed, bench.music];
        assert_eq!(voice_media(bench.project(), &selected), [other.id]);
        assert!(asset_spans(bench.project(), &selected, locked.id, &read[&locked.id]).is_empty());
        let mut lane = Lane::new(read);
        let (mut held, mut ducking) = (HeldPeaks::default(), Ducking::default());
        let (here, revision) = (bench.editor.instance_id(), bench.editor.revision());
        let said = ducking
            .start(bench.project(), here, revision, &selected, DEPTH)
            .unwrap();
        assert_eq!(
            said,
            "Reading the sound of one clip's media to duck under. Escape stops it. \
             One was passed over: Unlock the clip's track to duck its sound."
        );
        let bound = crate::peaks::MAX_HELD_PEAK_BYTES;
        let (command, _) = run(&bench, &mut ducking, &mut held, &mut lane, bound).unwrap();
        assert!(lane.asked.iter().all(|(id, _)| *id != locked.id));
        // Only the other voice, heard from 13 s to 14 s, ducks the music.
        bench.editor.dispatch(command).unwrap();
        let times: Vec<f64> = volume_keys_of(&bench).iter().map(|(at, _)| *at).collect();
        assert_eq!(times, [12.85, 13.0, 14.3, 14.7]);
        // Not selected, the same clip is a voice.
        assert_eq!(
            voice_media(bench.project(), &[bench.music]),
            [locked.id, other.id]
        );
    }

    // ---- What is written ---------------------------------------------------------

    /// The bench with one voice, loud from 1 s to 2 s of its source, from
    /// 2 s on: its run, written. Hands back the step and what was said.
    fn ducked(bench: &mut Bench, depth: f32) -> (Command, String) {
        let voice = sound("voice.wav");
        bench.voice(&voice, 2.0, |_| {});
        let mut lane = Lane::new(HashMap::from([(voice.id, peaks(60.0, &[(1.0, 2.0)]))]));
        let (mut held, mut ducking) = (HeldPeaks::default(), Ducking::default());
        ducking
            .start(
                bench.project(),
                bench.editor.instance_id(),
                bench.editor.revision(),
                &[bench.music],
                depth,
            )
            .unwrap();
        let bound = crate::peaks::MAX_HELD_PEAK_BYTES;
        run(bench, &mut ducking, &mut held, &mut lane, bound).unwrap()
    }

    fn volume_keys_of(bench: &Bench) -> Vec<(f64, f32)> {
        let clip = bench.project().clip(bench.music).unwrap().1;
        (volume_keys(clip).unwrap_or_default().iter())
            .map(|key| (key.at.as_seconds(), key.value))
            .collect()
    }

    #[test]
    fn the_duck_is_one_step_that_undoes_to_the_envelope_before() {
        let mut bench = Bench::new();
        // A clip whose volume is keyed is asked about first; its loudest
        // key is the level it comes back to.
        let before = vec![
            Keyframe {
                at: Time::ZERO,
                value: 0.8,
                ease: Ease::Linear,
            },
            Keyframe {
                at: seconds(10.0),
                value: 0.4,
                ease: Ease::Linear,
            },
        ];
        bench
            .editor
            .dispatch(Command::SetKeyframes {
                clip_id: bench.music,
                target: None,
                key: ParamKey::Gain,
                keys: before.clone(),
            })
            .unwrap();
        assert_eq!(keyed(bench.project(), &[bench.music]), 1);
        assert_eq!(question(1), "Replace this clip's volume keyframes?");
        assert_eq!(question(2), "Replace the volume keyframes of 2 clips?");
        let (command, said) = ducked(&mut bench, DEPTH);
        assert_eq!(
            said,
            "music.wav is ducked in one place where other clips are heard."
        );
        let steps = bench.editor.history().count();
        bench.editor.dispatch(command).unwrap();
        assert_eq!(bench.editor.history().count(), steps + 1);
        assert_eq!(bench.editor.undo_label(), Some(LABEL));
        // Heard from 3 s to 4 s, held 0.3 s longer, with its ramps.
        let keys = volume_keys_of(&bench);
        assert_eq!(keys.len(), 4);
        let times: Vec<f64> = keys.iter().map(|(at, _)| *at).collect();
        assert_eq!(times, [2.85, 3.0, 4.3, 4.7]);
        assert!(close(keys[0].1, 0.8) && close(keys[3].1, 0.8));
        assert!(close(keys[1].1, 0.2) && close(keys[2].1, 0.2));
        assert!(bench.editor.undo());
        let restored = bench.project().clip(bench.music).unwrap().1;
        assert_eq!(volume_keys(restored).unwrap(), before.as_slice());
    }

    #[test]
    fn a_ducked_clip_is_saved_and_read_back_as_written() {
        let mut bench = Bench::new();
        let (command, _) = ducked(&mut bench, depth_of_db(18.0).unwrap());
        bench.editor.dispatch(command).unwrap();
        let dir = std::env::temp_dir().join(format!("aster-duck-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Ducked.aster");
        editor_store::save_project(&file, bench.project()).unwrap();
        let loaded = editor_store::load_project(&file);
        std::fs::remove_dir_all(&dir).unwrap();
        let loaded = loaded.unwrap();
        let read = &loaded.clip(bench.music).unwrap().1.animation;
        assert_eq!(
            *read,
            bench.project().clip(bench.music).unwrap().1.animation
        );
        // 18 dB under its volume of 1.
        assert!(close(volume_keys_of(&bench)[1].1, 0.125_89));
    }

    #[test]
    fn the_depth_is_given_in_db_within_its_bounds() {
        assert!(close(depth_of_db(12.0).unwrap(), 0.251_19));
        assert_eq!(depth_of_db(-12.0), depth_of_db(12.0));
        assert!(close(depth_of_db(60.0).unwrap(), 0.001));
        assert_eq!(depth_of_db(0.5), None);
        assert_eq!(depth_of_db(61.0), None);
        assert!(close(20.0 * DEPTH.log10(), -12.041_2));
        // A depth that does not lower the clip starts nothing.
        let mut bench = Bench::new();
        bench.voice(&sound("voice.wav"), 2.0, |_| {});
        let mut ducking = Ducking::default();
        let here = bench.editor.instance_id();
        for depth in [1.0, -0.5, f32::NAN] {
            let refused = ducking.start(bench.project(), here, 0, &[bench.music], depth);
            assert!(refused.is_err(), "{depth}");
        }
        assert!(!ducking.busy());
    }

    // ---- Refusals and giving up ---------------------------------------------------

    #[test]
    fn a_clip_that_does_not_sound_is_refused_in_words_before_anything_is_asked() {
        let mut bench = Bench::new();
        let mut shot = sound("shot.mp4");
        shot.kind = MediaKind::Video;
        bench
            .editor
            .dispatch(Command::AddAssets {
                assets: vec![shot.clone()],
            })
            .unwrap();
        let picture = bench.add(
            track_of(&bench.editor, TrackKind::Video),
            &shot,
            2.0,
            |_| {},
        );
        let detach = rules::detach_audio(bench.project(), picture).unwrap();
        bench.editor.dispatch(detach).unwrap();
        let (mut held, mut ducking) = (HeldPeaks::default(), Ducking::default());
        let here = bench.editor.instance_id();
        let revision = bench.editor.revision();
        let refused = ducking.start(bench.project(), here, revision, &[picture], DEPTH);
        assert_eq!(refused.unwrap_err(), "Sound is on the linked audio clip.");
        assert!(!ducking.busy());
        assert!(
            ducking
                .step(bench.project(), here, revision, &mut held)
                .is_none()
        );
        assert!(!held.wanting(), "nothing was asked for");

        // A title has no sound; nothing selected is said so.
        let title = Clip::text("Title", Time::ZERO, seconds(2.0));
        let title_id = title.id;
        let track_id = track_of(&bench.editor, TrackKind::Text);
        bench
            .editor
            .dispatch(Command::AddClip {
                track_id,
                clip: title,
            })
            .unwrap();
        assert_eq!(
            refusal(bench.project(), title_id).unwrap(),
            "That clip has no sound to duck."
        );
        assert_eq!(
            takers(bench.project(), &[]).unwrap_err(),
            "Select a clip first."
        );

        // Of several, those that would be ducked are, and the rest said.
        let revision = bench.editor.revision();
        let said = ducking
            .start(
                bench.project(),
                here,
                revision,
                &[picture, bench.music],
                DEPTH,
            )
            .unwrap();
        assert!(
            said.ends_with("One was passed over: Sound is on the linked audio clip."),
            "{said}"
        );
        assert!(ducking.busy());

        // A locked track, and a clip whose keys are all at 0.
        let track_id = bench.project().clip(bench.music).unwrap().0.id;
        bench
            .editor
            .dispatch(track_state(track_id, None, Some(true)))
            .unwrap();
        assert_eq!(
            refusal(bench.project(), bench.music).unwrap(),
            "Unlock the clip's track to duck its sound."
        );
        assert!(bench.editor.undo());
        bench
            .editor
            .dispatch(Command::SetKeyframes {
                clip_id: bench.music,
                target: None,
                key: ParamKey::Gain,
                keys: vec![Keyframe {
                    at: Time::ZERO,
                    value: 0.0,
                    ease: Ease::Linear,
                }],
            })
            .unwrap();
        assert_eq!(
            refusal(bench.project(), bench.music).unwrap(),
            "Every volume key of that clip is at 0%: there is nothing to duck."
        );
    }

    #[test]
    fn a_run_is_given_up_when_the_project_changes_and_on_escape() {
        let mut bench = Bench::new();
        let voice = sound("voice.wav");
        bench.voice(&voice, 2.0, |_| {});
        let here = bench.editor.instance_id();
        let mut held = HeldPeaks::default();
        let mut ducking = Ducking::default();
        ducking
            .start(
                bench.project(),
                here,
                bench.editor.revision(),
                &[bench.music],
                DEPTH,
            )
            .unwrap();
        // The voice is wanted, and the lane asked.
        held.begin_frame();
        let revision = bench.editor.revision();
        assert!(
            ducking
                .step(bench.project(), here, revision, &mut held)
                .is_none()
        );
        assert_eq!(held.take_wants(), [(voice.id, crate::peaks::Ask::Kept)]);
        held.asked(voice.id, 7);
        let progress = ducking.progress(bench.project()).unwrap();
        assert_eq!(
            (progress.reading.as_deref(), progress.taken, progress.total),
            (Some("voice.wav"), 0, 1)
        );
        // Wanted for work, it is not given up when no tile draws it.
        for _ in 0..3 {
            held.begin_frame();
            assert!(held.take_unwanted().is_empty());
        }

        // Any edit is a change: the run is given up, its read handed back
        // to be given up, and nothing is written.
        bench
            .editor
            .dispatch(Command::SetGain {
                clip_id: bench.music,
                gain: 0.5,
            })
            .unwrap();
        let before = bench.project().clone();
        let stepped = ducking.step(bench.project(), here, bench.editor.revision(), &mut held);
        let Some(Stepped::GivenUp { said, cancel }) = stepped else {
            panic!("{stepped:?}");
        };
        assert_eq!(cancel, Some(7));
        assert!(said.starts_with("Duck under speech was stopped: the project changed"));
        assert!(!ducking.busy() && ducking.progress(bench.project()).is_none());
        assert_eq!(*bench.project(), before);
        assert_eq!(held.bytes(), 0);

        // Escape: the same, by the person.
        let revision = bench.editor.revision();
        ducking
            .start(bench.project(), here, revision, &[bench.music], DEPTH)
            .unwrap();
        assert!(
            ducking
                .step(bench.project(), here, revision, &mut held)
                .is_none()
        );
        held.take_wants();
        held.asked(voice.id, 8);
        assert_eq!(ducking.give_up(&mut held), Some(Some(8)));
        assert_eq!(ducking.give_up(&mut held), None, "nothing is under way");
        // Another project: forgotten.
        ducking
            .start(bench.project(), here, revision, &[bench.music], DEPTH)
            .unwrap();
        ducking.clear();
        assert!(
            ducking
                .step(bench.project(), here, revision, &mut held)
                .is_none()
        );
    }

    // ---- Through the real lane ---------------------------------------------------

    fn ffmpeg_available() -> bool {
        let tool = std::env::var_os("ASTER_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
        std::process::Command::new(tool)
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    /// Three seconds of mono 8 kHz sound, silent but for a 1 kHz tone at
    /// -20 dBFS from 1 s to 2 s, written to `dir`.
    fn burst(dir: &std::path::Path) -> Asset {
        let samples: Vec<i16> = (0..24_000)
            .map(|index| {
                if !(8_000..16_000).contains(&index) {
                    return 0;
                }
                let phase = std::f64::consts::TAU * 1000.0 * f64::from(index) / 8000.0;
                (0.1 * 32767.0 * phase.sin()).round() as i16
            })
            .collect();
        let data = samples.len() as u32 * 2;
        let mut bytes = Vec::with_capacity(44 + data as usize);
        bytes.extend(b"RIFF");
        bytes.extend((36 + data).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        // PCM, one channel, 8 kHz, 16 bits.
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(8000u32.to_le_bytes());
        bytes.extend(16_000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(data.to_le_bytes());
        bytes.extend(samples.iter().flat_map(|sample| sample.to_le_bytes()));
        let path = dir.join("speech.wav");
        std::fs::write(&path, &bytes).expect("FIXTURE: the sound file");
        let mut asset = Asset::new(path, MediaKind::Audio, seconds(3.0));
        asset.name = "speech.wav".to_owned();
        asset.has_audio = true;
        asset.file_size = bytes.len() as u64;
        asset
    }

    #[test]
    fn a_tone_read_by_the_lane_ducks_the_clip_under_it() {
        if !ffmpeg_available() {
            eprintln!("SKIP: FFmpeg is unavailable; nothing was read through the lane");
            return;
        }
        let dir = std::env::temp_dir().join(format!("aster-duck-lane-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut bench = Bench::new();
        let voice = burst(&dir);
        bench.voice(&voice, 2.0, |clip| clip.duration = seconds(3.0));
        let cache = editor_media::PeakCache::at(dir.join("peaks"));
        let sound = editor_media::SoundService::new(|| {}).expect("the lane starts");
        let here = bench.editor.instance_id();
        let mut held = HeldPeaks::default();
        let mut ducking = Ducking::default();
        let revision = bench.editor.revision();
        ducking
            .start(bench.project(), here, revision, &[bench.music], DEPTH)
            .unwrap();
        // The frames of the application: the lane's answers taken, the run
        // stepped, and what it wants asked.
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let done = loop {
            while let Some(event) = sound.poll() {
                let said = held.answered(here, bench.project(), event);
                assert_eq!(said, None);
            }
            held.begin_frame();
            match ducking.step(bench.project(), here, revision, &mut held) {
                Some(Stepped::Done(done)) => break done,
                Some(Stepped::GivenUp { said, .. }) => panic!("given up: {said}"),
                Some(Stepped::Took) | None => {}
            }
            assert!(held.ask(&sound, bench.project(), here, &cache).is_empty());
            assert!(std::time::Instant::now() < until, "the duck did not end");
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        drop(sound);
        std::fs::remove_dir_all(&dir).unwrap();
        let (command, said) = done.unwrap();
        assert_eq!(
            said,
            "music.wav is ducked in one place where other clips are heard."
        );
        assert_eq!(held.bytes(), 0, "the voice's peaks were let go");
        bench.editor.dispatch(*command).unwrap();
        // The tone is heard from 3 s to 4 s, held 0.3 s longer; the
        // lane's buckets may put its edges a step either way.
        let keys = volume_keys_of(&bench);
        eprintln!("DUCKED: {keys:?}");
        assert_eq!(keys.len(), 4);
        let expected = [2.85, 3.0, 4.3, 4.7];
        for ((at, _), want) in keys.iter().zip(expected) {
            assert!((at - want).abs() <= 0.011, "{keys:?}");
        }
        assert!(close(keys[1].1, DEPTH) && close(keys[2].1, DEPTH));
    }
}
