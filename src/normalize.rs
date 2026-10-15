//! Normalize loudness: a clip's volume set so that the part of its source it
//! plays reaches one target loudness.
//!
//! The measurement is derived data, taken on the sound lane when asked and
//! kept nowhere: what is written is an ordinary `SetGain`, or, for a clip
//! whose volume is keyed, an ordinary `SetKeyframes`. The clip menu, the
//! inspector and the command panel ask here, and refuse in the same words.
//!
//! [`Normalizing`] holds the clips waiting for their figures. Each was
//! captured when it was asked for, and an answer is written only to a clip
//! that is still what was measured, the same media, source span and audio
//! effects, and that would still be let through. Clips are measured one request each and written as each answers,
//! one step of the history a clip. Nothing here reads a file or waits.
use crate::ui::fades;
use editor_core::{
    AssetId, Clip, ClipId, Command, Effect, Keyframe, ParamKey, Project, Time, Track, sound,
};
use editor_media::{Loudness, LoudnessJob, MediaError, SoundEvent, SourceIdentity};
use std::collections::VecDeque;
use uuid::Uuid;

/// The loudness a clip is brought to, in LUFS (owner decision 6).
pub const TARGET: f32 = -14.0;
/// The true peak the volume may not take a clip above, in dBTP.
pub const CEILING: f32 = -1.0;
/// The most a clip's volume is: the project's bound.
pub const MOST: f32 = 8.0;
/// The clips that may wait for their figures at once, sent or not.
pub const MOST_WAITING: usize = 64;
/// What the history calls the step written for one clip.
pub const LABEL: &str = "Normalize loudness";
/// What a clip whose span is silent is told.
pub const SILENT: &str = "This clip is silent";
/// What a span with no 0.4 s block above the meter's gate is told.
pub const TOO_SHORT: &str = "Too short or too quiet to measure (under 0.4 s)";

/// What held a volume short of the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Held {
    /// The true peak would have passed [`CEILING`].
    Peak,
    /// The volume would have passed [`MOST`].
    Bound,
}

/// The volume a measurement asks for, absolute: the figures are of the
/// source before any volume, so this is what the clip's volume becomes, not
/// a change to it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volume {
    pub gain: f32,
    /// Why it stops short of [`TARGET`], when it does.
    pub held: Option<Held>,
}

fn from_db(db: f32) -> f32 {
    10_f64.powf(f64::from(db) / 20.0) as f32
}

/// `10^((TARGET - integrated) / 20)`, lowered so that `true_peak` plus the
/// volume in dB is at most [`CEILING`], within `0..=MOST`. `None` for figures
/// that are not numbers.
pub fn volume(integrated: f32, true_peak: f32) -> Option<Volume> {
    if !integrated.is_finite() || !true_peak.is_finite() {
        return None;
    }
    let wanted = from_db(TARGET - integrated);
    let ceiling = from_db(CEILING - true_peak);
    let (gain, held) = if ceiling < wanted {
        (ceiling, Some(Held::Peak))
    } else {
        (wanted, None)
    };
    Some(if gain > MOST {
        Volume {
            gain: MOST,
            held: Some(Held::Bound),
        }
    } else {
        Volume {
            gain: gain.max(0.0),
            held,
        }
    })
}

fn volume_track(clip: &Clip) -> Option<&editor_core::AnimTrack> {
    (clip.animation.iter()).find(|track| {
        track.target.is_none() && track.key == ParamKey::Gain && !track.keys.is_empty()
    })
}

/// The step that gives `clip` the volume `gain`, named [`LABEL`]; `None`
/// when it has that volume already. A clip without volume keys has its
/// volume set. A keyed clip's stored volume is not what plays, so its keys
/// are scaled by `gain` over the highest of them: the loudest stretch then
/// stands at `gain` and every dip keeps its depth. Refused in words when
/// every key is at 0, or when a key would pass [`MOST`].
pub fn command(clip: &Clip, gain: f32) -> Result<Option<Command>, String> {
    let made = match volume_track(clip) {
        None => (clip.gain != gain).then_some(Command::SetGain {
            clip_id: clip.id,
            gain,
        }),
        Some(track) => {
            let highest = (track.keys.iter())
                .map(|key| key.value)
                .fold(f32::NEG_INFINITY, f32::max);
            if highest <= 0.0 {
                return Err(
                    "Every volume key of this clip is at 0%: it has no loudest stretch to set."
                        .to_owned(),
                );
            }
            let factor = gain / highest;
            // No key is above the highest, so none is scaled above `gain`;
            // the bound keeps a rounding from saying otherwise.
            let keys: Vec<Keyframe> = (track.keys.iter())
                .map(|key| Keyframe {
                    value: if key.value == highest {
                        gain
                    } else {
                        (key.value * factor).min(gain)
                    },
                    ..*key
                })
                .collect();
            if keys
                .iter()
                .any(|key| key.value > MOST || key.value.is_nan())
            {
                return Err(format!(
                    "A volume key would pass {}%, the most a clip's volume is.",
                    MOST * 100.0
                ));
            }
            (keys != track.keys).then_some(Command::SetKeyframes {
                clip_id: clip.id,
                target: None,
                key: ParamKey::Gain,
                keys,
            })
        }
    };
    Ok(made.map(|command| Command::Batch {
        label: LABEL.to_owned(),
        commands: vec![command],
    }))
}

/// Why the clip is not in the mix, in the order `sound::sounds` asks.
pub fn unheard(project: &Project, track: &Track) -> &'static str {
    if track.muted {
        "its track is muted"
    } else if sound::soloed_out(project, track) {
        "another track is soloed"
    } else if track.gain <= 0.0 {
        "its track's volume is 0%"
    } else {
        "its volume is 0%"
    }
}

/// Why `clip` is not normalized, when it is not: a clip that does not sound
/// (`sound::sounds`), a picture whose linked audio clip owns its sound among
/// them, and a clip on a locked track.
pub fn refusal(project: &Project, clip: ClipId) -> Option<String> {
    let Some((track, clip)) = project.clip(clip) else {
        return Some("The clip no longer exists.".to_owned());
    };
    if !fades::has_sound(project, clip) {
        return Some("That clip has no sound to normalize.".to_owned());
    }
    if sound::owned(project, clip) {
        return Some(format!("{}.", fades::OWNED));
    }
    if !sound::sounds(project, track, clip) {
        return Some(format!(
            "That clip is not heard: {}.",
            unheard(project, track)
        ));
    }
    track
        .locked
        .then(|| "Unlock the clip's track to normalize its sound.".to_owned())
}

/// The clips of `clips` that would be measured, or why none would be.
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

fn percent(gain: f32) -> String {
    format!("{:.0}%", gain * 100.0)
}

fn name_of(clip: &Clip) -> String {
    match clip.name.trim() {
        "" => "The clip".to_owned(),
        name => name.to_owned(),
    }
}

/// One clip as it was when it was asked for: what its answer must still
/// find.
#[derive(Clone, Debug)]
struct Asked {
    clip: ClipId,
    name: String,
    asset: editor_core::Asset,
    source: SourceIdentity,
    /// The lowest and the highest source it plays, as one span.
    from: Time,
    span: Time,
    /// The enabled audio effects, which are all the measurement runs: a
    /// picture effect, or a switched-off one, changes nothing it hears.
    effects: Vec<Effect>,
}

/// What a loudness measurement of `clip` depends on (its media file, the
/// stretch of it heard and its enabled audio effects) as one number: a
/// measurement is for the clip only while this stays the same.
pub fn sound_key(project: &Project, clip: ClipId) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let asked = Asked::of(project, clip)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    format!(
        "{:?}",
        (
            asked.asset.id,
            &asked.source,
            asked.from,
            asked.span,
            &asked.effects
        )
    )
    .hash(&mut hasher);
    Some(hasher.finish())
}

impl Asked {
    fn of(project: &Project, id: ClipId) -> Option<Self> {
        let (_, clip) = project.clip(id)?;
        let asset = project.assets.get(&clip.asset_id()?)?;
        let (from, until) = clip.source_span();
        Some(Self {
            clip: id,
            name: name_of(clip),
            asset: asset.clone(),
            source: SourceIdentity::of(asset),
            from: from.min(until),
            span: Time(from.max(until).0 - from.min(until).0),
            effects: (clip.effects.iter())
                .filter(|effect| effect.enabled && effect.kind.is_audio())
                .cloned()
                .collect(),
        })
    }

    /// The lane's target: a newer job for it stops an older one.
    fn target(&self) -> (AssetId, Time, Time) {
        (self.asset.id, self.from, self.span)
    }

    /// Whether one measurement answers both.
    fn same(&self, other: &Self) -> bool {
        self.target() == other.target()
            && self.source == other.source
            && self.effects == other.effects
    }

    fn job(&self, project: Uuid) -> LoudnessJob {
        LoudnessJob {
            project,
            asset: self.asset.clone(),
            from: self.from,
            span: self.span,
            effects: self.effects.clone(),
        }
    }

    /// The clip as the project has it now, when it is still what was
    /// measured and [`refusal`] still lets it through: a picture whose
    /// sound a detached clip took over meanwhile is not written to.
    fn still<'a>(&self, project: &'a Project) -> Result<&'a Clip, String> {
        let gone = || {
            format!(
                "{} was removed while it was measured; nothing was changed.",
                self.name
            )
        };
        let (_, clip) = project.clip(self.clip).ok_or_else(gone)?;
        let now = Self::of(project, self.clip).ok_or_else(gone)?;
        if !now.same(self) {
            return Err(format!(
                "{} changed while it was measured, so its volume was not set. Normalize it again.",
                self.name
            ));
        }
        if let Some(reason) = refusal(project, self.clip) {
            return Err(format!("{}: {reason}", self.name));
        }
        Ok(clip)
    }
}

/// A measurement the lane was given, and the clips one answer is for.
#[derive(Debug)]
struct Sent {
    request: u64,
    project: Uuid,
    clips: Vec<Asked>,
}

/// What an answer makes of one clip: the step to take, if any, and what to
/// say once it is taken.
#[derive(Debug)]
pub struct Written {
    pub name: String,
    pub command: Option<Command>,
    pub said: String,
    /// Whether what is said is a refusal: then there is no command.
    pub problem: bool,
}

impl Written {
    fn refused(asked: &Asked, said: String) -> Self {
        Self {
            name: asked.name.clone(),
            command: None,
            said,
            problem: true,
        }
    }
}

/// What the figures make of a clip that is still what was measured.
fn written(asked: &Asked, clip: &Clip, integrated: f32, true_peak: f32) -> Written {
    let name = &asked.name;
    let Some(volume) = volume(integrated, true_peak) else {
        return Written::refused(
            asked,
            format!("{name} was not measured: the figures are not numbers."),
        );
    };
    let measured = format!("{name} measured {integrated:.1} LUFS, peak {true_peak:.1} dBFS");
    let set = percent(volume.gain);
    let short = match volume.held {
        None => String::new(),
        Some(Held::Peak) => {
            format!(" It stops short of {TARGET} LUFS so that its peak stays at {CEILING} dBTP.")
        }
        Some(Held::Bound) => format!(
            " It stops short of {TARGET} LUFS at {}, the most a clip's volume is.",
            percent(MOST)
        ),
    };
    let keyed = volume_track(clip).is_some();
    match command(clip, volume.gain) {
        Err(reason) => Written::refused(asked, format!("{measured}. {reason}")),
        Ok(command) => {
            let what = match (keyed, command.is_some()) {
                (false, true) => format!("volume set to {set}"),
                (true, true) => format!("volume keys scaled so the loudest is {set}"),
                (false, false) => format!("its volume is {set} already"),
                (true, false) => format!("its loudest volume key is {set} already"),
            };
            Written {
                name: name.clone(),
                command,
                said: format!("{measured}; {what}.{short}"),
                problem: false,
            }
        }
    }
}

/// The clips waiting for their loudness: sent to the lane, or held until
/// the lane has room or a measurement of the same span has answered.
#[derive(Debug, Default)]
pub struct Normalizing {
    sent: Vec<Sent>,
    waiting: VecDeque<(Uuid, Asked)>,
}

impl Normalizing {
    /// Whether clips are held back from the lane.
    pub fn holding(&self) -> bool {
        !self.waiting.is_empty()
    }

    fn count(&self) -> usize {
        self.sent.iter().map(|sent| sent.clips.len()).sum::<usize>() + self.waiting.len()
    }

    fn waits_for(&self, clip: ClipId) -> bool {
        (self.sent.iter().flat_map(|sent| &sent.clips))
            .chain(self.waiting.iter().map(|(_, asked)| asked))
            .any(|asked| asked.clip == clip)
    }

    /// Everything waited for is forgotten: the project is another. The lane
    /// is told by whoever replaced it.
    pub fn clear(&mut self) {
        self.sent.clear();
        self.waiting.clear();
    }

    // A clip asked for again is no longer waited for as it was. A request
    // left with no clip answers nothing here; the lane replaces it if the
    // same span is asked for again.
    fn forget(&mut self, clip: ClipId) {
        for sent in &mut self.sent {
            sent.clips.retain(|asked| asked.clip != clip);
        }
        self.sent.retain(|sent| !sent.clips.is_empty());
        self.waiting.retain(|(_, asked)| asked.clip != clip);
    }

    /// Captures each clip of `clips` that [`refusal`] lets through and has
    /// its sound measured through `lane`, one request a span. Hands back
    /// what to say, or why none was asked for. A clip that is refused is
    /// passed over and its reason said. A clip asked for again is waited
    /// for only as it is now.
    pub fn ask(
        &mut self,
        project: &Project,
        instance: Uuid,
        clips: &[ClipId],
        mut lane: impl FnMut(LoudnessJob) -> Result<u64, MediaError>,
    ) -> Result<String, String> {
        let mut refused: Vec<String> = Vec::new();
        let mut taken: Vec<String> = Vec::new();
        for id in clips {
            if let Some(reason) = refusal(project, *id) {
                refused.push(reason);
                continue;
            }
            let Some(asked) = Asked::of(project, *id) else {
                continue;
            };
            // A clip already waiting takes its own place again; every clip
            // over the bound is passed over and counted.
            if self.count() >= MOST_WAITING && !self.waits_for(*id) {
                refused.push(format!(
                    "At most {MOST_WAITING} clips wait to be measured at a time."
                ));
                continue;
            }
            self.forget(*id);
            match self.send(instance, asked, &mut lane) {
                Ok(name) => taken.push(name),
                Err(reason) => refused.push(reason),
            }
        }
        let passed = match refused.as_slice() {
            [] => String::new(),
            [reason] => format!(" One was passed over: {reason}"),
            [reason, ..] => format!(" {} were passed over: {reason}", refused.len()),
        };
        match taken.as_slice() {
            [] => Err(refused
                .into_iter()
                .next()
                .unwrap_or_else(|| "Select a clip first.".to_owned())),
            [name] => Ok(format!("Measuring the loudness of {name}.{passed}")),
            _ => Ok(format!(
                "Measuring the loudness of {} clips.{passed}",
                taken.len()
            )),
        }
    }

    /// Hands one clip to the lane, or to a measurement of the same span
    /// with the same audio effects, or holds it. `Err` says why it cannot be
    /// measured at all.
    fn send(
        &mut self,
        instance: Uuid,
        asked: Asked,
        lane: &mut impl FnMut(LoudnessJob) -> Result<u64, MediaError>,
    ) -> Result<String, String> {
        let name = asked.name.clone();
        if let Some(sent) = (self.sent.iter_mut()).find(|sent| {
            sent.project == instance && sent.clips.iter().any(|other| other.same(&asked))
        }) {
            sent.clips.push(asked);
            return Ok(name);
        }
        // A job for the same span would stop the one under way: it waits
        // for that one's answer.
        let target = asked.target();
        if (self.sent.iter()).any(|sent| sent.clips.iter().any(|other| other.target() == target)) {
            self.waiting.push_back((instance, asked));
            return Ok(name);
        }
        match lane(asked.job(instance)) {
            Ok(request) => {
                self.sent.push(Sent {
                    request,
                    project: instance,
                    clips: vec![asked],
                });
                Ok(name)
            }
            // Asked again when the lane has room.
            Err(MediaError::Busy) => {
                self.waiting.push_back((instance, asked));
                Ok(name)
            }
            Err(error) => Err(format!("{name} cannot be measured: {error}")),
        }
    }

    /// Hands what is held to the lane, in the order it was asked for, while
    /// the lane has room. Says why any could not be measured at all.
    pub fn pump(
        &mut self,
        mut lane: impl FnMut(LoudnessJob) -> Result<u64, MediaError>,
    ) -> Vec<String> {
        let mut problems = Vec::new();
        // One that finds the lane full is held again, in its place.
        for (instance, asked) in std::mem::take(&mut self.waiting) {
            if let Err(problem) = self.send(instance, asked, &mut lane) {
                problems.push(problem);
            }
        }
        problems
    }

    /// Takes the lane's answer to a measurement. Hands back what it makes of
    /// each clip that waited for it, in the order they were asked for: a
    /// step to take for those still as measured, and words for the rest.
    /// An answer to nothing waited for here, or for another project, is
    /// dropped and makes nothing.
    pub fn answered(
        &mut self,
        project: &Project,
        instance: Uuid,
        event: SoundEvent,
    ) -> Vec<Written> {
        let SoundEvent::Measured {
            request,
            project: asked_by,
            result,
            ..
        } = event
        else {
            return Vec::new();
        };
        let Some(index) = (self.sent.iter()).position(|sent| sent.request == request) else {
            return Vec::new();
        };
        let sent = self.sent.remove(index);
        if sent.project != instance || asked_by != instance {
            return Vec::new();
        }
        (sent.clips.iter())
            .map(|asked| {
                let clip = match asked.still(project) {
                    Ok(clip) => clip,
                    Err(said) => return Written::refused(asked, said),
                };
                let name = &asked.name;
                match &result {
                    Ok(Loudness::Measured {
                        integrated,
                        true_peak,
                    }) => written(asked, clip, *integrated, *true_peak),
                    Ok(Loudness::Silent) => Written::refused(asked, format!("{name}: {SILENT}.")),
                    Ok(Loudness::TooShort) => {
                        Written::refused(asked, format!("{name}: {TOO_SHORT}."))
                    }
                    Err(why) => {
                        // The first line of what FFmpeg said: the rest repeats it.
                        let why = why.lines().next().unwrap_or_default();
                        Written::refused(asked, format!("{name} was not measured: {why}"))
                    }
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules;
    use editor_core::{AnimTrack, Asset, Ease, Editor, EffectKind, MediaKind, TrackKind};

    fn seconds(value: f64) -> Time {
        Time::from_seconds(value)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    fn db(gain: f32) -> f32 {
        20.0 * gain.log10()
    }

    // ---- The rule ---------------------------------------------------------

    #[test]
    fn the_target_is_reached_when_nothing_holds_it() {
        let quiet = volume(-20.0, -10.0).unwrap();
        assert_eq!(quiet.held, None);
        assert!(close(-20.0 + db(quiet.gain), TARGET), "{quiet:?}");
        // A loud source is turned down, by the same rule.
        let loud = volume(-8.0, -2.0).unwrap();
        assert_eq!(loud.held, None);
        assert!(loud.gain < 1.0 && close(-8.0 + db(loud.gain), TARGET));
        // Figures that are not numbers make no volume.
        assert_eq!(volume(f32::NAN, -10.0), None);
        assert_eq!(volume(-20.0, f32::INFINITY), None);
    }

    #[test]
    fn the_peak_ceiling_holds_the_volume_short_of_the_target() {
        // 6 dB up would take a peak of -3 dBFS to +3.
        let held = volume(-20.0, -3.0).unwrap();
        assert_eq!(held.held, Some(Held::Peak));
        assert!(close(-3.0 + db(held.gain), CEILING), "{held:?}");
        assert!(-20.0 + db(held.gain) < TARGET);
    }

    #[test]
    fn the_bound_holds_the_volume_at_eight() {
        // 26 dB up is twenty times; the peak would allow it.
        let held = volume(-40.0, -40.0).unwrap();
        assert_eq!(
            held,
            Volume {
                gain: MOST,
                held: Some(Held::Bound)
            }
        );
        // The ceiling below the bound is the one said.
        let peak = volume(-40.0, -10.0).unwrap();
        assert_eq!(peak.held, Some(Held::Peak));
        assert!(peak.gain < MOST);
    }

    /// A four second clip of a sound on an audio track, its volume keyed
    /// with these values at these seconds.
    fn keyed(keys: &[(f64, f32)]) -> (Editor, ClipId) {
        let (mut editor, [first, ..]) = project();
        let keys = (keys.iter())
            .map(|(at, value)| Keyframe {
                at: seconds(*at),
                value: *value,
                ease: Ease::Linear,
            })
            .collect();
        editor
            .dispatch(Command::SetKeyframes {
                clip_id: first,
                target: None,
                key: ParamKey::Gain,
                keys,
            })
            .unwrap();
        (editor, first)
    }

    fn key_values(editor: &Editor, clip: ClipId) -> Vec<f32> {
        let clip = editor.project().clip(clip).unwrap().1;
        volume_track(clip)
            .unwrap()
            .keys
            .iter()
            .map(|key| key.value)
            .collect()
    }

    #[test]
    fn a_clip_without_keys_has_its_volume_set_in_one_step_that_undoes() {
        let (mut editor, [first, ..]) = project();
        let clip = editor.project().clip(first).unwrap().1.clone();
        let made = command(&clip, 2.5).unwrap().unwrap();
        assert_eq!(made.label(), LABEL);
        let steps = editor.history().count();
        editor.dispatch(made).unwrap();
        assert_eq!(editor.history().count(), steps + 1);
        assert_eq!(editor.undo_label(), Some(LABEL));
        assert_eq!(editor.project().clip(first).unwrap().1.gain, 2.5);
        assert!(editor.undo());
        assert_eq!(editor.project().clip(first).unwrap().1.gain, 1.0);
        // The volume it has already writes nothing.
        assert!(command(&clip, 1.0).unwrap().is_none());
    }

    #[test]
    fn a_keyed_clip_has_its_loudest_key_set_and_its_ratios_kept() {
        let (mut editor, clip) = keyed(&[(0.0, 1.0), (1.0, 0.25), (3.0, 0.5)]);
        let stored = editor.project().clip(clip).unwrap().1.clone();
        let made = command(&stored, 1.6).unwrap().unwrap();
        editor.dispatch(made).unwrap();
        let values = key_values(&editor, clip);
        assert_eq!(values[0], 1.6, "the highest is at the volume");
        assert!(close(values[1], 0.4) && close(values[2], 0.8), "{values:?}");
        assert!(close(values[0] / values[1], 4.0));
        // The stored volume, which is not what plays, is left.
        assert_eq!(editor.project().clip(clip).unwrap().1.gain, 1.0);
        assert!(editor.undo());
        assert_eq!(key_values(&editor, clip), [1.0, 0.25, 0.5]);
    }

    #[test]
    fn a_ducked_clip_keeps_its_dips_at_the_same_depth_under_the_new_base() {
        // What `duck_keys` writes for one span from 1 s to 2 s, base 0.5
        // and depth 0.25, at the duck's attack and release.
        let ducked = sound::duck_keys(
            &[(seconds(1.0), seconds(2.0))],
            seconds(4.0),
            0.5,
            0.25,
            crate::duck::ATTACK,
            crate::duck::RELEASE,
        );
        let keys: Vec<(f64, f32)> = (ducked.iter())
            .map(|key| (key.at.as_seconds(), key.value))
            .collect();
        assert_eq!(keys, [(0.85, 0.5), (1.0, 0.125), (2.0, 0.125), (2.4, 0.5)]);
        let (mut editor, clip) = keyed(&keys);
        let stored = editor.project().clip(clip).unwrap().1.clone();
        let base = volume(-23.0, -12.0).unwrap().gain;
        editor
            .dispatch(command(&stored, base).unwrap().unwrap())
            .unwrap();
        let values = key_values(&editor, clip);
        assert_eq!((values[0], values[3]), (base, base));
        for dip in &values[1..3] {
            assert!(close(*dip, base * 0.25), "{values:?}");
        }
    }

    #[test]
    fn keys_that_cannot_be_scaled_are_refused_and_nothing_is_written() {
        let (editor, clip) = keyed(&[(0.0, 1.0), (2.0, 0.5)]);
        let stored = editor.project().clip(clip).unwrap().1.clone();
        let before = editor.project().clone();
        // The rule never hands over more than 8; a volume that would take a
        // key past it is refused all the same.
        let past = command(&stored, 9.0).unwrap_err();
        assert_eq!(
            past,
            "A volume key would pass 800%, the most a clip's volume is."
        );
        let (silent, clip) = keyed(&[(0.0, 0.0), (2.0, 0.0)]);
        let stored = silent.project().clip(clip).unwrap().1.clone();
        let none = command(&stored, 2.0).unwrap_err();
        assert!(none.starts_with("Every volume key of this clip is at 0%"));
        assert_eq!(*editor.project(), before);
    }

    #[test]
    fn at_the_bound_no_key_is_scaled_past_it() {
        let (mut editor, clip) = keyed(&[(0.0, 0.3), (1.0, 0.299_999_98), (2.0, 0.1)]);
        let stored = editor.project().clip(clip).unwrap().1.clone();
        editor
            .dispatch(command(&stored, MOST).unwrap().unwrap())
            .unwrap();
        let values = key_values(&editor, clip);
        assert!(values.iter().all(|value| *value <= MOST), "{values:?}");
        assert_eq!(values[0], MOST);
    }

    // ---- Asking and answering ---------------------------------------------

    /// `voice.wav`, ten seconds of sound, with two four second clips of it
    /// on the first audio track: the first from its start at 0, the second
    /// from 5 s of it at 5 s. And `shot.mp4`, a video with sound, from 0 on
    /// the first video track.
    fn project() -> (Editor, [ClipId; 3]) {
        let mut editor = Editor::default();
        let mut voice = Asset::new("/media/voice.wav".into(), MediaKind::Audio, seconds(10.0));
        voice.name = "voice.wav".to_owned();
        voice.has_audio = true;
        voice.file_size = 4321;
        let mut shot = Asset::new("/media/shot.mp4".into(), MediaKind::Video, seconds(10.0));
        shot.name = "shot.mp4".to_owned();
        shot.has_audio = true;
        let clip = |asset: &Asset, start: f64| {
            let mut clip = Clip::from_asset(asset, seconds(start));
            clip.source_in = seconds(start);
            clip.duration = seconds(4.0);
            clip
        };
        let clips = [clip(&voice, 0.0), clip(&voice, 5.0), clip(&shot, 0.0)];
        let ids = [clips[0].id, clips[1].id, clips[2].id];
        editor
            .dispatch(Command::AddAssets {
                assets: vec![voice, shot],
            })
            .unwrap();
        for (clip, kind) in
            clips
                .into_iter()
                .zip([TrackKind::Audio, TrackKind::Audio, TrackKind::Video])
        {
            let track_id = track(&editor, kind);
            editor
                .dispatch(Command::AddClip { track_id, clip })
                .unwrap();
        }
        (editor, ids)
    }

    fn track(editor: &Editor, kind: TrackKind) -> editor_core::TrackId {
        (editor.project().tracks.iter())
            .find(|track| track.kind == kind)
            .unwrap()
            .id
    }

    /// The sound lane, standing in: it takes every job and names it by
    /// count, or is full.
    #[derive(Default)]
    struct Lane {
        jobs: Vec<LoudnessJob>,
        full: bool,
    }

    impl Lane {
        fn take(&mut self) -> impl FnMut(LoudnessJob) -> Result<u64, MediaError> + '_ {
            |job| {
                if self.full {
                    return Err(MediaError::Busy);
                }
                self.jobs.push(job);
                Ok(self.jobs.len() as u64)
            }
        }
    }

    /// What the lane says of the job it named `request`.
    fn answer(lane: &Lane, request: u64, result: Result<Loudness, String>) -> SoundEvent {
        let job = &lane.jobs[request as usize - 1];
        SoundEvent::Measured {
            request,
            project: job.project,
            asset_id: job.asset.id,
            source: SourceIdentity::of(&job.asset),
            result,
        }
    }

    fn measured(integrated: f32, true_peak: f32) -> Result<Loudness, String> {
        Ok(Loudness::Measured {
            integrated,
            true_peak,
        })
    }

    /// The application's end of an answer: each step taken, each word said.
    fn take(
        editor: &mut Editor,
        waiting: &mut Normalizing,
        event: SoundEvent,
    ) -> Vec<(String, bool)> {
        let written = waiting.answered(editor.project(), editor.instance_id(), event);
        (written.into_iter())
            .map(|written| {
                if let Some(command) = written.command {
                    let steps = editor.history().count();
                    editor.dispatch(command).unwrap();
                    assert_eq!(editor.history().count(), steps + 1, "one step a clip");
                    assert_eq!(editor.undo_label(), Some(LABEL));
                }
                (written.said, written.problem)
            })
            .collect()
    }

    fn gain(editor: &Editor, clip: ClipId) -> f32 {
        editor.project().clip(clip).unwrap().1.gain
    }

    #[test]
    fn two_clips_of_one_media_are_both_measured_and_each_written_as_it_answers() {
        let (mut editor, [first, second, _]) = project();
        let here = editor.instance_id();
        let (mut lane, mut waiting) = (Lane::default(), Normalizing::default());
        let said = waiting.ask(editor.project(), here, &[first, second], lane.take());
        assert_eq!(said.unwrap(), "Measuring the loudness of 2 clips.");
        let spans: Vec<_> = lane.jobs.iter().map(|job| (job.from, job.span)).collect();
        assert_eq!(
            spans,
            [(Time::ZERO, seconds(4.0)), (seconds(5.0), seconds(4.0))]
        );
        assert!(lane.jobs.iter().all(|job| job.project == here));

        // The second answers first, and is written by itself.
        let steps = editor.history().count();
        let said = take(
            &mut editor,
            &mut waiting,
            answer(&lane, 2, measured(-20.0, -10.0)),
        );
        assert_eq!(
            said,
            [(
                "voice.wav measured -20.0 LUFS, peak -10.0 dBFS; volume set to 200%.".to_owned(),
                false
            )]
        );
        assert!(close(
            gain(&editor, second),
            volume(-20.0, -10.0).unwrap().gain
        ));
        assert_eq!(gain(&editor, first), 1.0);
        let said = take(
            &mut editor,
            &mut waiting,
            answer(&lane, 1, measured(-20.0, -4.0)),
        );
        assert_eq!(said.len(), 1);
        assert!(
            said[0].0.ends_with("so that its peak stays at -1 dBTP."),
            "{said:?}"
        );
        assert!(close(db(gain(&editor, first)), 3.0));
        assert_eq!(editor.history().count(), steps + 2);
        // Each undoes by itself, and an answer again is to nothing.
        assert!(editor.undo());
        assert_eq!(gain(&editor, first), 1.0);
        assert!(gain(&editor, second) > 1.0);
        let again = answer(&lane, 1, measured(-20.0, -4.0));
        assert!(waiting.answered(editor.project(), here, again).is_empty());
    }

    #[test]
    fn a_stale_answer_writes_nothing_and_says_so() {
        type Change = fn(ClipId) -> Command;
        let changes: [(&str, Change); 3] = [
            ("removed", |clip| Command::DeleteClips {
                clip_ids: vec![clip],
                ripple: false,
            }),
            ("changed", |clip| Command::TrimClip {
                clip_id: clip,
                source_in: Time::ZERO,
                duration: seconds(3.0),
                ripple: false,
            }),
            ("changed", |clip| Command::SetEffects {
                clip_id: clip,
                effects: vec![Effect::new(EffectKind::Compressor)],
            }),
        ];
        for (word, change) in changes {
            let (mut editor, [first, ..]) = project();
            let here = editor.instance_id();
            let (mut lane, mut waiting) = (Lane::default(), Normalizing::default());
            waiting
                .ask(editor.project(), here, &[first], lane.take())
                .unwrap();
            editor.dispatch(change(first)).unwrap();
            let before = editor.project().clone();
            let event = answer(&lane, 1, measured(-20.0, -10.0));
            let said = take(&mut editor, &mut waiting, event);
            assert_eq!(said.len(), 1, "{word}");
            assert!(said[0].1 && said[0].0.contains(word), "{said:?}");
            assert_eq!(*editor.project(), before, "{word}");
        }
        // A picture whose sound a detached clip took over while it was
        // measured, and a clip whose track was muted meanwhile: what refuses
        // them before a request refuses them at the answer.
        let (mut editor, [first, _, shot]) = project();
        let here = editor.instance_id();
        let (mut lane, mut waiting) = (Lane::default(), Normalizing::default());
        waiting
            .ask(editor.project(), here, &[first, shot], lane.take())
            .unwrap();
        let detach = rules::detach_audio(editor.project(), shot).unwrap();
        editor.dispatch(detach).unwrap();
        editor
            .dispatch(Command::SetTrackState {
                track_id: track(&editor, TrackKind::Audio),
                locked: None,
                hidden: None,
                muted: Some(true),
                name: None,
                gain: None,
                solo: None,
            })
            .unwrap();
        let before = editor.project().clone();
        let mut said = take(
            &mut editor,
            &mut waiting,
            answer(&lane, 1, measured(-20.0, -10.0)),
        );
        said.extend(take(
            &mut editor,
            &mut waiting,
            answer(&lane, 2, measured(-20.0, -10.0)),
        ));
        assert_eq!(
            said,
            [
                (
                    "voice.wav: That clip is not heard: its track is muted.".to_owned(),
                    true
                ),
                (
                    "shot.mp4: Sound is on the linked audio clip.".to_owned(),
                    true
                ),
            ]
        );
        assert_eq!(*editor.project(), before);
    }

    #[test]
    fn only_the_audio_effects_it_hears_make_a_clip_changed() {
        let (mut editor, [first, _, shot]) = project();
        let here = editor.instance_id();
        let (mut lane, mut waiting) = (Lane::default(), Normalizing::default());
        let mut off = Effect::new(EffectKind::Compressor);
        off.enabled = false;
        editor
            .dispatch(Command::SetEffects {
                clip_id: first,
                effects: vec![off.clone()],
            })
            .unwrap();
        waiting
            .ask(editor.project(), here, &[first, shot], lane.take())
            .unwrap();
        assert!(lane.jobs.iter().all(|job| job.effects.is_empty()));
        // A picture effect on the video clip, and a switched-off effect's
        // setting, change nothing the measurement hears.
        off.params = vec![editor_core::ParamValue {
            key: ParamKey::Amount,
            value: 10.0,
        }];
        for (clip_id, effects) in [
            (shot, vec![Effect::new(EffectKind::Blur)]),
            (first, vec![off]),
        ] {
            editor
                .dispatch(Command::SetEffects { clip_id, effects })
                .unwrap();
        }
        for request in [1, 2] {
            let event = answer(&lane, request, measured(-20.0, -10.0));
            let said = take(&mut editor, &mut waiting, event);
            assert!(said.len() == 1 && !said[0].1, "{said:?}");
        }
        assert!(gain(&editor, first) > 1.0 && gain(&editor, shot) > 1.0);
    }

    #[test]
    fn every_clip_over_the_bound_is_passed_over_and_counted() {
        let (mut editor, [first, ..]) = project();
        editor
            .dispatch(Command::AddTrack {
                track: Track::new("Audio 2", TrackKind::Audio),
            })
            .unwrap();
        let track_id = (editor.project().tracks.iter())
            .find(|track| track.name == "Audio 2")
            .unwrap()
            .id;
        // Eighths of a second of the voice, each a span of its own.
        let mut ids = Vec::new();
        for index in 0..MOST_WAITING + 2 {
            let mut clip = editor.project().clip(first).unwrap().1.clone();
            clip.id = Uuid::new_v4();
            clip.start = seconds(index as f64 / 8.0);
            clip.source_in = clip.start;
            clip.duration = seconds(1.0 / 8.0);
            ids.push(clip.id);
            editor
                .dispatch(Command::AddClip { track_id, clip })
                .unwrap();
        }
        let here = editor.instance_id();
        let (mut lane, mut waiting) = (Lane::default(), Normalizing::default());
        let said = waiting.ask(editor.project(), here, &ids, lane.take());
        assert_eq!(
            said.unwrap(),
            format!(
                "Measuring the loudness of {MOST_WAITING} clips. 2 were passed over: \
                 At most {MOST_WAITING} clips wait to be measured at a time."
            )
        );
        assert_eq!(lane.jobs.len(), MOST_WAITING);
        // One already waiting is asked for again in its own place.
        let said = waiting.ask(editor.project(), here, &ids[..1], lane.take());
        assert!(said.is_ok(), "{said:?}");
        assert_eq!(lane.jobs.len(), MOST_WAITING + 1);
        assert!(
            waiting
                .ask(editor.project(), here, &ids[MOST_WAITING..], lane.take())
                .is_err()
        );
    }

    #[test]
    fn a_picture_whose_linked_audio_clip_owns_its_sound_is_refused_before_any_request() {
        let (mut editor, [_, _, shot]) = project();
        let detach = rules::detach_audio(editor.project(), shot).unwrap();
        editor.dispatch(detach).unwrap();
        let here = editor.instance_id();
        let (mut lane, mut waiting) = (Lane::default(), Normalizing::default());
        let refused = waiting.ask(editor.project(), here, &[shot], lane.take());
        assert_eq!(refused.unwrap_err(), "Sound is on the linked audio clip.");
        assert_eq!(
            takers(editor.project(), &[shot]).unwrap_err(),
            "Sound is on the linked audio clip."
        );
        assert!(lane.jobs.is_empty() && !waiting.holding());
        // Its sound, on the audio track, is measured; the picture beside it
        // is passed over and said to be.
        let sound = (editor.project().tracks.iter())
            .filter(|track| track.kind == TrackKind::Audio)
            .flat_map(|track| &track.clips)
            .find(|clip| clip.linked_id.is_some())
            .unwrap()
            .id;
        let said = waiting.ask(editor.project(), here, &[shot, sound], lane.take());
        assert_eq!(
            said.unwrap(),
            "Measuring the loudness of shot.mp4. One was passed over: Sound is on the linked audio clip."
        );
        assert_eq!(lane.jobs.len(), 1);
    }

    #[test]
    fn a_clip_that_is_not_heard_or_has_no_sound_is_refused_in_words() {
        let (mut editor, [first, ..]) = project();
        let track_id = track(&editor, TrackKind::Audio);
        editor
            .dispatch(Command::SetTrackState {
                track_id,
                locked: None,
                hidden: None,
                muted: Some(true),
                name: None,
                gain: None,
                solo: None,
            })
            .unwrap();
        assert_eq!(
            refusal(editor.project(), first).unwrap(),
            "That clip is not heard: its track is muted."
        );
        assert!(editor.undo());
        editor
            .dispatch(Command::SetGain {
                clip_id: first,
                gain: 0.0,
            })
            .unwrap();
        assert_eq!(
            refusal(editor.project(), first).unwrap(),
            "That clip is not heard: its volume is 0%."
        );
        let title = Clip::text("Title", Time::ZERO, seconds(2.0));
        let id = title.id;
        let track_id = track(&editor, TrackKind::Text);
        editor
            .dispatch(Command::AddClip {
                track_id,
                clip: title,
            })
            .unwrap();
        assert_eq!(
            refusal(editor.project(), id).unwrap(),
            "That clip has no sound to normalize."
        );
        assert_eq!(
            takers(editor.project(), &[]).unwrap_err(),
            "Select a clip first."
        );
    }

    #[test]
    fn silence_and_a_span_too_short_to_measure_each_say_so_and_change_nothing() {
        for (result, words) in [
            (Ok(Loudness::Silent), "voice.wav: This clip is silent."),
            (
                Ok(Loudness::TooShort),
                "voice.wav: Too short or too quiet to measure (under 0.4 s).",
            ),
            (
                Err("ffmpeg: could not measure the sound: broken\nmore".to_owned()),
                "voice.wav was not measured: ffmpeg: could not measure the sound: broken",
            ),
        ] {
            let (mut editor, [first, ..]) = project();
            let here = editor.instance_id();
            let (mut lane, mut waiting) = (Lane::default(), Normalizing::default());
            waiting
                .ask(editor.project(), here, &[first], lane.take())
                .unwrap();
            let before = editor.project().clone();
            let event = answer(&lane, 1, result);
            let said = take(&mut editor, &mut waiting, event);
            assert_eq!(said, [(words.to_owned(), true)]);
            assert_eq!(*editor.project(), before);
        }
    }

    #[test]
    fn one_span_is_measured_once_and_a_span_under_way_is_not_replaced() {
        let (mut editor, [first, ..]) = project();
        // Two copies of the first clip's span on a track of their own: one
        // as it is, and one through a compressor.
        editor
            .dispatch(Command::AddTrack {
                track: Track::new("Audio 2", TrackKind::Audio),
            })
            .unwrap();
        let copy = |editor: &Editor, start: f64| {
            let mut clip = editor.project().clip(first).unwrap().1.clone();
            clip.id = Uuid::new_v4();
            clip.start = seconds(start);
            clip
        };
        let (same, mut other) = (copy(&editor, 0.0), copy(&editor, 5.0));
        other.effects = vec![Effect::new(EffectKind::Compressor)];
        let ids = [first, same.id, other.id];
        let track_id = (editor.project().tracks.iter())
            .find(|track| track.name == "Audio 2")
            .unwrap()
            .id;
        for clip in [same, other] {
            editor
                .dispatch(Command::AddClip { track_id, clip })
                .unwrap();
        }
        let here = editor.instance_id();
        let (mut lane, mut waiting) = (Lane::default(), Normalizing::default());
        let said = waiting
            .ask(editor.project(), here, &ids, lane.take())
            .unwrap();
        assert_eq!(said, "Measuring the loudness of 3 clips.");
        // One job for the first two; the third waits, as the lane would
        // stop the one under way for a job of the same span.
        assert_eq!(lane.jobs.len(), 1);
        assert!(waiting.holding());
        assert!(waiting.pump(lane.take()).is_empty());
        assert_eq!(lane.jobs.len(), 1, "it still waits");
        let event = answer(&lane, 1, measured(-20.0, -10.0));
        let said = take(&mut editor, &mut waiting, event);
        assert_eq!(said.len(), 2);
        assert_eq!(gain(&editor, ids[0]), gain(&editor, ids[1]));
        assert_eq!(gain(&editor, ids[2]), 1.0);
        assert!(waiting.pump(lane.take()).is_empty());
        assert_eq!(lane.jobs.len(), 2);
        let kinds: Vec<EffectKind> = lane.jobs[1].effects.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            [EffectKind::Compressor],
            "measured through its own effects"
        );
        let event = answer(&lane, 2, measured(-26.0, -20.0));
        take(&mut editor, &mut waiting, event);
        assert!(close(db(gain(&editor, ids[2])), 12.0));
        assert!(!waiting.holding());
    }

    #[test]
    fn a_full_lane_holds_clips_until_it_has_room() {
        let (editor, [first, second, _]) = project();
        let here = editor.instance_id();
        let (mut lane, mut waiting) = (Lane::default(), Normalizing::default());
        lane.full = true;
        let said = waiting.ask(editor.project(), here, &[first, second], lane.take());
        assert_eq!(said.unwrap(), "Measuring the loudness of 2 clips.");
        assert!(lane.jobs.is_empty() && waiting.holding());
        assert!(waiting.pump(lane.take()).is_empty());
        assert!(lane.jobs.is_empty());
        lane.full = false;
        assert!(waiting.pump(lane.take()).is_empty());
        assert_eq!(lane.jobs.len(), 2);
        assert_eq!(lane.jobs[0].from, Time::ZERO, "in the order asked");
        assert!(!waiting.holding());
    }

    #[test]
    fn an_answer_for_another_project_or_after_a_clear_writes_nothing() {
        let (editor, [first, ..]) = project();
        let here = editor.instance_id();
        let (mut lane, mut waiting) = (Lane::default(), Normalizing::default());
        waiting
            .ask(editor.project(), here, &[first], lane.take())
            .unwrap();
        let event = answer(&lane, 1, measured(-20.0, -10.0));
        assert!(
            waiting
                .answered(editor.project(), Uuid::new_v4(), event)
                .is_empty()
        );
        waiting
            .ask(editor.project(), here, &[first], lane.take())
            .unwrap();
        waiting.clear();
        let event = answer(&lane, 2, measured(-20.0, -10.0));
        assert!(waiting.answered(editor.project(), here, event).is_empty());
        // A clip asked for again is waited for once, by its newer request.
        waiting
            .ask(editor.project(), here, &[first], lane.take())
            .unwrap();
        waiting
            .ask(editor.project(), here, &[first], lane.take())
            .unwrap();
        let older = answer(&lane, 3, measured(-20.0, -10.0));
        assert!(waiting.answered(editor.project(), here, older).is_empty());
        let newer = answer(&lane, 4, measured(-20.0, -10.0));
        assert_eq!(waiting.answered(editor.project(), here, newer).len(), 1);
    }

    #[test]
    fn a_normalized_keyed_clip_is_saved_and_read_back_as_written() {
        let (mut editor, clip) = keyed(&[(0.0, 1.0), (1.0, 0.25)]);
        let here = editor.instance_id();
        let (mut lane, mut waiting) = (Lane::default(), Normalizing::default());
        waiting
            .ask(editor.project(), here, &[clip], lane.take())
            .unwrap();
        let event = answer(&lane, 1, measured(-23.0, -12.0));
        let said = take(&mut editor, &mut waiting, event);
        assert!(
            said[0].0.contains("volume keys scaled so the loudest is"),
            "{said:?}"
        );
        let dir = std::env::temp_dir().join(format!("aster-normalize-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Normalized.aster");
        editor_store::save_project(&file, editor.project()).unwrap();
        let loaded = editor_store::load_project(&file);
        std::fs::remove_dir_all(&dir).unwrap();
        let loaded = loaded.unwrap();
        let read: Vec<AnimTrack> = loaded.clip(clip).unwrap().1.animation.clone();
        assert_eq!(read, editor.project().clip(clip).unwrap().1.animation);
        assert_eq!(
            key_values(&editor, clip)[0],
            volume(-23.0, -12.0).unwrap().gain
        );
    }

    // ---- Through the sound lane --------------------------------------------

    /// Three seconds of a 1 kHz tone at a tenth of full scale (-20 dBFS),
    /// mono at 8 kHz, written sample by sample into `dir`.
    fn tone(dir: &std::path::Path) -> Asset {
        let samples: Vec<i16> = (0..24_000)
            .map(|index| {
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
        let path = dir.join("tone.wav");
        std::fs::write(&path, &bytes).expect("FIXTURE: the sound file");
        let mut asset = Asset::new(path, MediaKind::Audio, seconds(3.0));
        asset.name = "tone.wav".to_owned();
        asset.has_audio = true;
        asset.file_size = bytes.len() as u64;
        asset
    }

    fn ffmpeg_available() -> bool {
        let tool = std::env::var_os("ASTER_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
        std::process::Command::new(tool)
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    #[test]
    fn a_tone_measured_by_the_lane_is_set_to_the_target() {
        if !ffmpeg_available() {
            eprintln!("SKIP: FFmpeg is unavailable; nothing was measured through the lane");
            return;
        }
        let dir = std::env::temp_dir().join(format!("aster-normalize-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let asset = tone(&dir);
        let mut editor = Editor::default();
        let mut clip = Clip::from_asset(&asset, Time::ZERO);
        clip.duration = seconds(2.0);
        clip.source_in = seconds(0.5);
        let id = clip.id;
        editor
            .dispatch(Command::AddAssets {
                assets: vec![asset],
            })
            .unwrap();
        let track_id = track(&editor, TrackKind::Audio);
        editor
            .dispatch(Command::AddClip { track_id, clip })
            .unwrap();

        let sound = editor_media::SoundService::new(|| {}).unwrap();
        let mut waiting = Normalizing::default();
        let here = editor.instance_id();
        waiting
            .ask(editor.project(), here, &[id], |job| sound.loudness(job))
            .unwrap();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let said = loop {
            if let Some(event) = sound.poll() {
                let said = take(&mut editor, &mut waiting, event);
                if !said.is_empty() {
                    break said;
                }
            }
            assert!(std::time::Instant::now() < until, "nothing was measured");
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        drop(sound);
        std::fs::remove_dir_all(&dir).unwrap();
        eprintln!("MEASURE: {said:?}");
        assert!(!said[0].1, "{said:?}");
        assert!(said[0].0.starts_with("tone.wav measured -"), "{said:?}");
        // One channel at -20 dBFS is spread over two at -23, as a voice is
        // by `-ac 2`: about -23 LUFS, and so about 9 dB up.
        let set = db(gain(&editor, id));
        assert!((8.5..9.5).contains(&set), "{set} dB from {said:?}");
        assert_eq!(editor.undo_label(), Some(LABEL));
    }
}
