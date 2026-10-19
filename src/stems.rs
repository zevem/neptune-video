//! Separate stems: the whole sound of a clip's media split into its voice
//! and its instruments by a stem separator the person installed (Demucs),
//! and placed under the clip as one step.
//!
//! The sound lane runs the tool (see [`editor_media::SeparateJob`]) and puts
//! the stems in the project's media folder, `<document stem> media/` beside
//! the saved document, where they are the person's files: the editor never
//! removes them, also not on undo. When they come, and the clip is still
//! there with its media unchanged, one "Separate stems" step adds them to
//! the library, puts each on a new audio track named after it, at the volume
//! and solo of the clip's track, as a clip of
//! the source clip's span, speed, time map, volume, volume keys, fades and
//! audio effects, links them to the source clip and turns the source clip
//! down to 0% without its volume keys, so the stems are heard once and not
//! over the original. The whole file is separated, so the stems line up
//! with their source and survive a later trim.
//!
//! The clip menu and the command panel ask here and are refused in the same
//! words; an agent cannot start a separation (spec principle 8), only write
//! such a step itself. Nothing here reads a file or waits.
use crate::{
    extract::{Card, Said, Shown},
    normalize,
    ui::fades,
};
use editor_core::{
    Asset, AssetId, Clip, ClipId, Command, ParamKey, Project, Time, Track, TrackKind, sound,
};
use editor_media::{Cancel, Extracting, STEMS, SeparateJob, Separator, SoundEvent, SourceIdentity};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// What the rows that ask for it are called, and the step it makes.
pub const LABEL: &str = "Separate stems";
/// Why it cannot be chosen without a separator.
pub const NEEDS_TOOL: &str = "Needs a stem separator; see the documentation";
/// Why it cannot be chosen before the session's probe has answered.
pub const LOOKING: &str = "Looking for a stem separator…";
/// Why a project without a file is refused: its stems have no folder.
pub const UNTITLED: &str = "Save the project to separate stems";

/// Whether this session has a stem separator, as its probe answered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    /// The probe has not answered yet.
    #[default]
    Looking,
    /// No separator was found, or the one found cannot be run.
    Missing,
    Found,
}

impl Tool {
    pub fn of(answer: Option<&Result<Separator, String>>) -> Self {
        match answer {
            None => Self::Looking,
            Some(Err(_)) => Self::Missing,
            Some(Ok(_)) => Self::Found,
        }
    }

    /// Why nothing can be separated, when nothing can.
    pub fn refusal(self) -> Option<&'static str> {
        match self {
            Self::Looking => Some(LOOKING),
            Self::Missing => Some(NEEDS_TOOL),
            Self::Found => None,
        }
    }
}

fn name_of(clip: &Clip, asset: &Asset) -> String {
    match clip.name.trim() {
        "" => asset.name.clone(),
        name => name.to_owned(),
    }
}

/// The clip asked of, with its track and its media, or why its sound is
/// not separated.
fn separable(project: &Project, clip: ClipId) -> Result<(&Track, &Clip, &Asset), String> {
    let (track, clip) = project.clip(clip).ok_or("The clip no longer exists.")?;
    let asset = (clip.asset_id())
        .and_then(|asset| project.assets.get(&asset))
        .filter(|_| fades::has_sound(project, clip))
        .ok_or("That clip has no sound to separate.")?;
    if sound::owned(project, clip) {
        return Err(format!("{}: separate it there.", fades::OWNED));
    }
    if !sound::sounds(project, track, clip) {
        return Err(format!(
            "That clip is not heard: {}.",
            normalize::unheard(project, track)
        ));
    }
    if track.locked {
        return Err("Unlock the clip's track to separate its stems.".to_owned());
    }
    if asset.duration <= Time::ZERO {
        return Err(format!("{} has no length to separate.", asset.name));
    }
    Ok((track, clip, asset))
}

/// Why "Separate stems" would be refused of `clip`, the separator aside,
/// when it would be.
pub fn refusal(project: &Project, clip: ClipId) -> Option<String> {
    separable(project, clip).err()
}

/// What is to be separated, captured when it was asked for: the clip, and
/// its media as it was then.
#[derive(Clone, Debug, PartialEq)]
pub struct Asked {
    /// The project that asked: only it is given the stems.
    pub project: Uuid,
    pub clip: ClipId,
    pub asset: Asset,
    /// What the notices call it: the clip's name, or the media's.
    pub name: String,
}

impl Asked {
    /// What separating `clip` asks for, or why it is refused.
    pub fn of(project: &Project, instance: Uuid, clip: ClipId) -> Result<Self, String> {
        let (_, held, asset) = separable(project, clip)?;
        Ok(Self {
            project: instance,
            clip,
            asset: asset.clone(),
            name: name_of(held, asset),
        })
    }

    /// The lane's job that separates the whole media into `to`, the
    /// project's media folder.
    pub fn job(&self, separator: Separator, to: PathBuf) -> SeparateJob {
        SeparateJob {
            project: self.project,
            asset: self.asset.clone(),
            to,
            separator,
        }
    }
}

/// A stem's clip of `source`'s span, speed, time map, volume, volume keys,
/// fades and audio effects, held to what the stem has: a stem a little
/// shorter than its source, as a video's sound may be, cuts a clip of one
/// speed short. Its link is made with the others.
fn stem_clip(source: &Clip, stem: &Asset, label: &str, name: &str) -> Result<Clip, String> {
    let mut clip = Clip::from_asset(stem, source.start);
    clip.name = format!("{name} ({label})");
    clip.source_in = source.source_in;
    clip.duration = source.duration;
    clip.playback_rate = source.playback_rate;
    clip.retime = source.retime.clone();
    clip.gain = source.gain;
    clip.group_id = source.group_id;
    clip.fade_in = source.fade_in;
    clip.fade_out = source.fade_out;
    // An audio clip animates its volume and nothing else, and takes the
    // audio effects alone: a picture clip's are of its picture.
    clip.animation = (source.animation.iter())
        .filter(|track| track.target.is_none() && track.key == ParamKey::Gain)
        .cloned()
        .collect();
    clip.effects = (source.effects.iter())
        .filter(|effect| effect.kind.is_audio())
        .cloned()
        .collect();
    if clip.source_span().1 <= stem.duration + Time(2) {
        return Ok(clip);
    }
    let short = || {
        format!(
            "{} is shorter than the part of its media {name} plays",
            stem.name
        )
    };
    if clip.retime.is_some() || clip.source_in >= stem.duration {
        return Err(short());
    }
    let left = (stem.duration - clip.source_in).as_seconds() / clip.playback_rate;
    clip.duration = Time::from_seconds(left).min(clip.duration);
    if clip.duration <= Time::ZERO {
        return Err(short());
    }
    clip.fade_in = clip.fade_in.min(clip.duration);
    clip.fade_out = clip.fade_out.min(clip.duration - clip.fade_in);
    Ok(clip)
}

/// "Separate stems", as one step: the stems added to the library, each on a
/// new audio track named after it under `clip`'s span, linked to `clip`,
/// and `clip` at 0% without volume keys. A stem's track takes the volume
/// and solo of `clip`'s, so the stems are heard where and as loud as it
/// was. A clip that is no longer the one heard, as when its sound was
/// detached while it was separated, is refused: turning it down would leave
/// what is heard on top of the stems. `stems` are in
/// [`editor_media::STEMS`]' order.
pub fn place(project: &Project, clip: ClipId, stems: &[Asset]) -> Result<Command, String> {
    let (track, source) = project.clip(clip).ok_or("The clip no longer exists.")?;
    if track.locked {
        return Err("its track is locked".to_owned());
    }
    if sound::owned(project, source) {
        return Err("its sound is now on the linked audio clip".to_owned());
    }
    if !sound::sounds(project, track, source) {
        return Err(format!(
            "it is no longer heard: {}",
            normalize::unheard(project, track)
        ));
    }
    if stems.len() != STEMS.len() {
        return Err(format!(
            "the separator gave {} stems, not {}",
            stems.len(),
            STEMS.len()
        ));
    }
    let asset = (source.asset_id())
        .and_then(|asset| project.assets.get(&asset))
        .ok_or("its media is no longer in the library")?;
    let name = name_of(source, asset);
    let mut commands = vec![Command::AddAssets {
        assets: stems.to_vec(),
    }];
    let mut linked = vec![source.id];
    for (stem, (_, label)) in stems.iter().zip(STEMS) {
        let made = stem_clip(source, stem, label, &name)?;
        let mut held = Track::new(label, TrackKind::Audio);
        held.gain = track.gain;
        held.solo = track.solo;
        let track_id = held.id;
        linked.push(made.id);
        commands.push(Command::AddTrack { track: held });
        commands.push(Command::AddClip {
            track_id,
            clip: made,
        });
    }
    commands.push(Command::LinkClips { clip_ids: linked });
    commands.push(Command::SetGain {
        clip_id: source.id,
        gain: 0.0,
    });
    let keyed = (source.animation.iter())
        .any(|track| track.target.is_none() && track.key == ParamKey::Gain);
    if keyed {
        commands.push(Command::SetKeyframes {
            clip_id: source.id,
            target: None,
            key: ParamKey::Gain,
            keys: Vec::new(),
        });
    }
    Ok(Command::Batch {
        label: LABEL.to_owned(),
        commands,
    })
}

/// A separation the lane is running.
#[derive(Clone, Debug)]
struct Running {
    request: u64,
    clip: ClipId,
    asset: AssetId,
    name: String,
    /// How far the tool says it is, once it has said.
    done: Option<f32>,
}

/// What an answer brings: the step to make, if any, and what to say.
#[derive(Clone, Debug)]
pub struct Answer {
    pub command: Option<Command>,
    pub said: Said,
    /// The stems' files, for a step that fails to name.
    pub files: Vec<PathBuf>,
}

/// The files' whole paths, as a notice names them.
pub fn listed(files: &[PathBuf]) -> String {
    let files: Vec<String> = (files.iter())
        .map(|path| path.display().to_string())
        .collect();
    files.join(" and ")
}

/// The separations under way.
#[derive(Default)]
pub struct Separating {
    running: Vec<Running>,
}

impl Separating {
    /// A separation the lane took. Those of the same media it gave up for
    /// it go, and what to say of them is handed back; one the lane was too
    /// late to give up keeps its card, and its answer comes.
    pub fn sent(&mut self, asked: &Asked, sent: &Extracting) -> Option<String> {
        let replaced = (self.running)
            .extract_if(.., |running| sent.replaced.contains(&running.request))
            .last();
        self.running.push(Running {
            request: sent.request,
            clip: asked.clip,
            asset: asked.asset.id,
            name: asked.name.clone(),
            done: None,
        });
        replaced.map(|old| {
            format!(
                "Separating {} in place of {}: the same media is separated once.",
                asked.name, old.name
            )
        })
    }

    /// How far a separation is; `false` when `request` is none's.
    pub fn progress(&mut self, request: u64, done: f32) -> bool {
        match (self.running.iter_mut()).find(|running| running.request == request) {
            Some(running) => {
                running.done = Some(done.clamp(0.0, 1.0));
                true
            }
            None => false,
        }
    }

    /// Takes the answer for a separation. Stems for `instance` whose clip is
    /// still there, of the media as it was, are placed by the step handed
    /// back; any others are only said, with where their files are. An event
    /// that is no separation's answer says nothing and changes nothing.
    pub fn answered(
        &mut self,
        project: &Project,
        instance: Uuid,
        event: SoundEvent,
    ) -> Option<Answer> {
        let SoundEvent::Separated {
            request,
            project: asked_by,
            asset_id,
            source,
            result,
        } = event
        else {
            return None;
        };
        let running = (self.running.iter())
            .position(|running| running.request == request)
            .map(|at| self.running.remove(at));
        let name = (running.as_ref()).map_or_else(
            || {
                (project.assets.get(&asset_id))
                    .map_or_else(|| "the media".to_owned(), |asset| asset.name.clone())
            },
            |running| running.name.clone(),
        );
        let stems = match result {
            Ok(stems) => stems,
            Err(why) => {
                return Some(Answer {
                    command: None,
                    said: Said {
                        text: format!(
                            "The stems of {name} were not made: {}",
                            why.lines().next().unwrap_or_default()
                        ),
                        problem: true,
                    },
                    files: Vec::new(),
                });
            }
        };
        let files: Vec<PathBuf> = stems.iter().map(|stem| stem.path.clone()).collect();
        let placed = match &running {
            None => Err("nothing waited for them".to_owned()),
            Some(_) if asked_by != instance => {
                Err("the project they were made for is no longer open".to_owned())
            }
            Some(running) => self::still(project, running, &source)
                .and_then(|()| place(project, running.clip, &stems)),
        };
        Some(match placed {
            Ok(command) => Answer {
                command: Some(command),
                said: Said {
                    text: format!(
                        "Separated {name} into {}; the original clip is at 0%.",
                        (STEMS.iter())
                            .map(|(_, label)| *label)
                            .collect::<Vec<_>>()
                            .join(" and ")
                    ),
                    problem: false,
                },
                files,
            },
            Err(why) => Answer {
                command: None,
                said: Said {
                    text: format!(
                        "The stems of {name} were not added, as {why}. They are kept in {}",
                        listed(&files)
                    ),
                    problem: true,
                },
                files,
            },
        })
    }

    /// What came of Cancel on a separation's card, as the lane said. One
    /// given up is forgotten; one the lane is putting in place, or put,
    /// keeps its card until its answer.
    pub fn cancelled(&mut self, request: u64, outcome: Cancel) -> Option<String> {
        let at = (self.running.iter()).position(|running| running.request == request)?;
        let name = self.running[at].name.clone();
        Some(match outcome {
            Cancel::GivenUp => {
                self.running.remove(at);
                format!("Separating {name} was stopped; no stem was kept.")
            }
            Cancel::Finishing | Cancel::Written => {
                format!("The stems of {name} are being put in place; what came of it follows.")
            }
        })
    }

    /// The cards to show, one a separation.
    pub fn cards(&self) -> Vec<Card> {
        (self.running.iter())
            .map(|running| Card {
                request: running.request,
                text: format!("Separating the stems of {}", running.name),
                shown: Shown::Separating(running.done),
            })
            .collect()
    }
}

/// Whether the clip a separation was asked of is still there, of the media
/// it was, as that media was.
fn still(project: &Project, running: &Running, source: &SourceIdentity) -> Result<(), String> {
    let (_, clip) = project
        .clip(running.clip)
        .ok_or("the clip no longer exists")?;
    let asset = (clip.asset_id())
        .filter(|asset| *asset == running.asset)
        .and_then(|asset| project.assets.get(&asset))
        .ok_or("the clip plays other media now")?;
    if SourceIdentity::of(asset) != *source {
        return Err("its media changed while it was separated".to_owned());
    }
    Ok(())
}

/// The folder a document's stems go in, or why there is none: a project
/// without a file has no folder.
pub fn folder(document: Option<&Path>) -> Result<PathBuf, String> {
    let document = document.ok_or(UNTITLED)?;
    editor_store::media_folder(document).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::{
        AnimTrack, Ease, Editor, Effect, EffectKind, Keyframe, MediaKind, Retime, TimeKey,
    };

    fn seconds(value: f64) -> Time {
        Time::from_seconds(value)
    }

    const HERE: Uuid = Uuid::from_u128(9);

    /// A song on an audio track, from 1 s, playing 3 s of it from 2 s at
    /// 80% with a volume key, faded in and denoised; a video with sound on
    /// a video track with a glow; and a title.
    struct Bench {
        editor: Editor,
        song: Asset,
        clip: ClipId,
        shot: ClipId,
        title: ClipId,
    }

    fn sound(path: &str, kind: MediaKind, length: f64) -> Asset {
        let mut asset = Asset::new(path.into(), kind, seconds(length));
        asset.has_audio = true;
        asset.file_size = 1_000;
        asset
    }

    fn bench() -> Bench {
        let mut editor = Editor::default();
        let song = sound("/media/song.mp3", MediaKind::Audio, 10.0);
        let shot = sound("/media/talk.mp4", MediaKind::Video, 6.0);
        editor
            .dispatch(Command::AddAssets {
                assets: vec![song.clone(), shot.clone()],
            })
            .unwrap();
        let mut add = |kind: TrackKind, name: &str, clip: Clip| {
            let track = Track::new(name, kind);
            let track_id = track.id;
            editor.dispatch(Command::AddTrack { track }).unwrap();
            let id = clip.id;
            editor
                .dispatch(Command::AddClip { track_id, clip })
                .unwrap();
            id
        };
        let mut clip = Clip::from_asset(&song, seconds(1.0));
        clip.name = "Chorus".to_owned();
        clip.source_in = seconds(2.0);
        clip.duration = seconds(3.0);
        clip.gain = 0.8;
        clip.fade_in = seconds(0.5);
        clip.effects.push(Effect::new(EffectKind::Denoise));
        clip.animation.push(AnimTrack {
            target: None,
            key: ParamKey::Gain,
            keys: vec![Keyframe {
                at: seconds(1.0),
                value: 0.5,
                ease: Ease::Linear,
            }],
        });
        let clip = add(TrackKind::Audio, "Audio 1", clip);
        let mut picture = Clip::from_asset(&shot, Time::ZERO);
        picture.duration = seconds(4.0);
        picture.effects.push(Effect::new(EffectKind::Glow));
        let shot = add(TrackKind::Video, "Video 1", picture);
        let title = add(
            TrackKind::Text,
            "Text 1",
            Clip::text("Hello", Time::ZERO, seconds(2.0)),
        );
        Bench {
            editor,
            song,
            clip,
            shot,
            title,
        }
    }

    /// The two stems a separator makes of `source`, as their probe reads
    /// them: 44.1 kHz WAVs as long as the source.
    fn stems_of(source: &Asset, length: f64) -> Vec<Asset> {
        (STEMS.iter())
            .map(|(_, label)| {
                let mut stem = sound(
                    &format!("/media/Film media/song ({label}).wav"),
                    MediaKind::Audio,
                    length,
                );
                stem.folder_id = source.folder_id;
                stem
            })
            .collect()
    }

    /// The answer of the lane for `request`, made for `project`.
    fn answer(asked: &Asked, request: u64, project: Uuid, stems: Vec<Asset>) -> SoundEvent {
        SoundEvent::Separated {
            request,
            project,
            asset_id: asked.asset.id,
            source: SourceIdentity::of(&asked.asset),
            result: Ok(stems),
        }
    }

    /// `asked` sent to the lane as `request`.
    fn sent(separating: &mut Separating, asked: &Asked, request: u64) {
        let said = separating.sent(
            asked,
            &Extracting {
                request,
                replaced: Vec::new(),
            },
        );
        assert_eq!(said, None);
    }

    #[test]
    fn a_clip_that_is_heard_is_asked_and_the_rest_refused_in_words() {
        let bench = bench();
        let project = bench.editor.project();
        let asked = Asked::of(project, HERE, bench.clip).unwrap();
        assert_eq!((asked.project, asked.clip), (HERE, bench.clip));
        assert_eq!(
            (asked.asset.id, asked.name.as_str()),
            (bench.song.id, "Chorus")
        );
        assert_eq!(refusal(project, bench.shot), None);
        assert_eq!(
            refusal(project, bench.title).as_deref(),
            Some("That clip has no sound to separate.")
        );
        assert_eq!(
            refusal(project, ClipId::new_v4()).as_deref(),
            Some("The clip no longer exists.")
        );
        let mut editor = Editor::new(project.clone()).unwrap();
        let picture = crate::rules::detach_audio(editor.project(), bench.shot).unwrap();
        editor.dispatch(picture).unwrap();
        assert_eq!(
            refusal(editor.project(), bench.shot).as_deref(),
            Some("Sound is on the linked audio clip: separate it there.")
        );
        let track_of = |editor: &Editor, clip| editor.project().clip(clip).unwrap().0.id;
        let switch = |track_id, muted, locked| Command::SetTrackState {
            track_id,
            locked,
            hidden: None,
            muted,
            name: None,
            gain: None,
            solo: None,
        };
        let audio = track_of(&editor, bench.clip);
        editor.dispatch(switch(audio, Some(true), None)).unwrap();
        assert_eq!(
            refusal(editor.project(), bench.clip).as_deref(),
            Some("That clip is not heard: its track is muted.")
        );
        editor
            .dispatch(switch(audio, Some(false), Some(true)))
            .unwrap();
        assert_eq!(
            refusal(editor.project(), bench.clip).as_deref(),
            Some("Unlock the clip's track to separate its stems.")
        );

        // The separator, as the session's probe answered.
        assert_eq!(Tool::of(None).refusal(), Some(LOOKING));
        assert_eq!(
            Tool::of(Some(&Err("none".into()))).refusal(),
            Some(NEEDS_TOOL)
        );
        // The folder is beside the saved document; without one, none.
        assert_eq!(folder(None), Err(UNTITLED.to_owned()));
        assert_eq!(
            folder(Some(Path::new("/work/Film.nvp"))),
            Ok(PathBuf::from("/work/Film media"))
        );
    }

    #[test]
    fn stems_are_placed_under_the_clip_as_one_step_and_undone_whole() {
        let mut bench = bench();
        let before = bench.editor.project().clone();
        let asked = Asked::of(&before, HERE, bench.clip).unwrap();
        let mut separating = Separating::default();
        sent(&mut separating, &asked, 5);
        assert!(separating.progress(5, 0.4));
        assert!(!separating.progress(6, 0.4));
        assert_eq!(
            separating.cards(),
            [Card {
                request: 5,
                text: "Separating the stems of Chorus".to_owned(),
                shown: Shown::Separating(Some(0.4)),
            }]
        );
        let stems = stems_of(&bench.song, 10.0);
        let answer = separating
            .answered(&before, HERE, answer(&asked, 5, HERE, stems.clone()))
            .unwrap();
        assert!(separating.cards().is_empty());
        assert!(!answer.said.problem, "{}", answer.said.text);
        assert_eq!(
            answer.said.text,
            "Separated Chorus into Vocals and Instruments; the original clip is at 0%."
        );
        let steps = bench.editor.history().count();
        bench.editor.dispatch(answer.command.unwrap()).unwrap();
        assert_eq!(bench.editor.history().count(), steps + 1, "one step");
        assert_eq!(bench.editor.undo_label(), Some(LABEL));

        let project = bench.editor.project();
        let (_, source) = project.clip(bench.clip).unwrap();
        // The original is silent and keeps no volume key; its fades and
        // effects stay with it.
        assert_eq!(source.gain, 0.0);
        assert!(source.animation.is_empty());
        let link = source.linked_id.expect("the stems are linked to it");
        for (stem, (_, label)) in stems.iter().zip(STEMS) {
            assert!(project.assets.contains_key(&stem.id));
            let track = (project.tracks.iter())
                .find(|track| track.name == label)
                .expect("a track named after the stem");
            assert_eq!(track.kind, TrackKind::Audio);
            let [placed] = track.clips.as_slice() else {
                panic!("one clip on {label}");
            };
            assert_eq!(placed.asset_id(), Some(stem.id));
            assert_eq!(placed.name, format!("Chorus ({label})"));
            // The source's span, speed, volume, keys, fades and effects.
            assert_eq!(
                (
                    placed.start,
                    placed.source_in,
                    placed.duration,
                    placed.playback_rate
                ),
                (
                    source.start,
                    source.source_in,
                    source.duration,
                    source.playback_rate
                )
            );
            assert_eq!(placed.gain, 0.8);
            assert_eq!(
                placed.animation,
                before.clip(bench.clip).unwrap().1.animation
            );
            assert_eq!(
                (placed.fade_in, placed.fade_out),
                (source.fade_in, source.fade_out)
            );
            assert_eq!(placed.effects, source.effects);
            assert_eq!(placed.linked_id, Some(link));
        }
        project.validate().unwrap();

        // Undo brings back the volume and its key, and takes the stems out
        // of the library and the timeline: the project is as it was.
        assert!(bench.editor.undo());
        let after = bench.editor.project();
        assert_eq!(after.tracks, before.tracks);
        assert_eq!(after.assets, before.assets);
    }

    #[test]
    fn stems_are_heard_as_loud_as_a_soloed_clip_at_half_volume() {
        let mut bench = bench();
        let track_id = bench.editor.project().clip(bench.clip).unwrap().0.id;
        bench
            .editor
            .dispatch(Command::SetTrackState {
                track_id,
                locked: None,
                hidden: None,
                muted: None,
                name: None,
                gain: Some(0.5),
                solo: Some(true),
            })
            .unwrap();
        let project = bench.editor.project().clone();
        let command = place(&project, bench.clip, &stems_of(&bench.song, 10.0)).unwrap();
        bench.editor.dispatch(command).unwrap();
        let project = bench.editor.project();
        let (track, source) = project.clip(bench.clip).unwrap();
        assert!(!sound::sounds(project, track, source));
        let stems: Vec<&Track> = (project.tracks.iter().map(AsRef::as_ref))
            .filter(|track| STEMS.iter().any(|(_, label)| track.name == *label))
            .collect();
        assert_eq!(stems.len(), 2);
        for track in stems {
            // Its own solo keeps the other tracks out, not the stems.
            assert_eq!((track.gain, track.solo), (0.5, true));
            assert!(sound::sounds(project, track, &track.clips[0]));
        }
    }

    #[test]
    fn a_picture_with_sound_gives_its_sound_and_not_its_look() {
        let mut bench = bench();
        let project = bench.editor.project().clone();
        assert!(Asked::of(&project, HERE, bench.shot).is_ok());
        let shot = project.clip(bench.shot).unwrap().1.clone();
        let source = project.assets[&shot.asset_id().unwrap()].clone();
        let command = place(&project, bench.shot, &stems_of(&source, 6.0)).unwrap();
        bench.editor.dispatch(command).unwrap();
        let project = bench.editor.project();
        let (track, picture) = project.clip(bench.shot).unwrap();
        assert_eq!(track.kind, TrackKind::Video);
        assert_eq!(picture.gain, 0.0);
        // Its glow stays on the picture; the stems take no picture effect.
        assert_eq!(picture.effects.len(), 1);
        let stems: Vec<&Clip> = (project.tracks.iter())
            .filter(|track| STEMS.iter().any(|(_, label)| track.name == *label))
            .flat_map(|track| &track.clips)
            .collect();
        assert_eq!(stems.len(), 2);
        assert!(stems.iter().all(|stem| stem.effects.is_empty()
            && stem.linked_id == picture.linked_id
            && stem.duration == shot.duration));
    }

    #[test]
    fn a_stem_shorter_than_its_source_cuts_a_clip_of_one_speed_and_refuses_a_time_map() {
        let bench = bench();
        let project = bench.editor.project();
        // The clip plays 2 s to 5 s of the song; the stems end at 4.5 s.
        let short = stems_of(&bench.song, 4.5);
        let Command::Batch { commands, .. } = place(project, bench.clip, &short).unwrap() else {
            panic!("a batch");
        };
        let clip = (commands.iter())
            .find_map(|command| match command {
                Command::AddClip { clip, .. } => Some(clip),
                _ => None,
            })
            .unwrap();
        assert_eq!(clip.duration, seconds(2.5));
        let mut editor = Editor::new(project.clone()).unwrap();
        editor
            .dispatch(Command::Batch {
                label: LABEL.to_owned(),
                commands,
            })
            .unwrap();

        // A time map is not cut: the stems are refused in words.
        let mut editor = Editor::new(project.clone()).unwrap();
        let (_, held) = project.clip(bench.clip).unwrap();
        editor
            .dispatch(Command::SetRetime {
                clip_id: bench.clip,
                retime: Some(Retime {
                    keys: vec![
                        TimeKey {
                            at: Time::ZERO,
                            source: held.source_in,
                            ease: Ease::Linear,
                        },
                        TimeKey {
                            at: held.duration,
                            source: held.source_in + held.duration,
                            ease: Ease::Linear,
                        },
                    ],
                }),
                duration: held.duration,
            })
            .unwrap();
        let refused = place(editor.project(), bench.clip, &short).unwrap_err();
        assert_eq!(
            refused,
            "song (Vocals).wav is shorter than the part of its media Chorus plays"
        );
    }

    #[test]
    fn a_stale_answer_adds_nothing_and_names_the_files() {
        let bench = bench();
        let project = bench.editor.project().clone();
        let asked = Asked::of(&project, HERE, bench.clip).unwrap();
        let stems = stems_of(&bench.song, 10.0);
        let paths =
            "/media/Film media/song (Vocals).wav and /media/Film media/song (Instruments).wav";
        let stale = |project: &Project, made_for: Uuid, event: Option<SoundEvent>| {
            let mut separating = Separating::default();
            sent(&mut separating, &asked, 5);
            let event = event.unwrap_or_else(|| answer(&asked, 5, made_for, stems.clone()));
            let answer = separating.answered(project, HERE, event).unwrap();
            assert!(answer.command.is_none());
            assert!(answer.said.problem);
            assert!(separating.cards().is_empty());
            answer.said.text
        };

        // Another project is open now.
        let said = stale(&project, Uuid::from_u128(1), None);
        assert_eq!(
            said,
            format!(
                "The stems of Chorus were not added, as the project they were made for is no longer open. They are kept in {paths}"
            )
        );
        // The clip was deleted.
        let mut editor = Editor::new(project.clone()).unwrap();
        editor
            .dispatch(Command::DeleteClips {
                clip_ids: vec![bench.clip],
                ripple: false,
            })
            .unwrap();
        let said = stale(editor.project(), HERE, None);
        assert!(said.contains("as the clip no longer exists"), "{said}");
        assert!(said.ends_with(paths), "{said}");
        // Its media changed on disk meanwhile.
        let mut changed = asked.asset.clone();
        changed.file_size += 1;
        let event = SoundEvent::Separated {
            request: 5,
            project: HERE,
            asset_id: asked.asset.id,
            source: SourceIdentity::of(&changed),
            result: Ok(stems.clone()),
        };
        let said = stale(&project, HERE, Some(event));
        assert!(said.contains("its media changed"), "{said}");
        // Its track was muted meanwhile: the stems would sound where it
        // did not.
        let mut editor = Editor::new(project.clone()).unwrap();
        let track_id = project.clip(bench.clip).unwrap().0.id;
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
        let said = stale(editor.project(), HERE, None);
        assert!(
            said.contains("as it is no longer heard: its track is muted."),
            "{said}"
        );
        // A picture whose sound was detached meanwhile: turning it down
        // would leave the audio clip playing over the stems.
        let picture = Asked::of(&project, HERE, bench.shot).unwrap();
        let mut editor = Editor::new(project.clone()).unwrap();
        let detach = crate::rules::detach_audio(editor.project(), bench.shot).unwrap();
        editor.dispatch(detach).unwrap();
        let mut separating = Separating::default();
        sent(&mut separating, &picture, 8);
        let shot = &picture.asset;
        let refused = separating
            .answered(
                editor.project(),
                HERE,
                answer(&picture, 8, HERE, stems_of(shot, 6.0)),
            )
            .unwrap();
        assert!(refused.command.is_none());
        assert!(
            (refused.said.text)
                .contains("as its sound is now on the linked audio clip. They are kept in"),
            "{}",
            refused.said.text
        );
        // The tool failed: its first line, and no files.
        let event = SoundEvent::Separated {
            request: 5,
            project: HERE,
            asset_id: asked.asset.id,
            source: SourceIdentity::of(&asked.asset),
            result: Err("demucs could not separate the sound: out of memory\nmore".into()),
        };
        let said = stale(&project, HERE, Some(event));
        assert_eq!(
            said,
            "The stems of Chorus were not made: demucs could not separate the sound: out of memory"
        );
        // Not a separation's answer: nothing.
        let mut separating = Separating::default();
        let other = SoundEvent::Progress {
            request: 5,
            done: 0.5,
        };
        assert!(separating.answered(&project, HERE, other).is_none());
    }

    #[test]
    fn a_separation_stopped_is_forgotten_and_one_finishing_keeps_its_card() {
        let bench = bench();
        let project = bench.editor.project();
        let asked = Asked::of(project, HERE, bench.clip).unwrap();
        let mut separating = Separating::default();
        sent(&mut separating, &asked, 5);
        sent(&mut separating, &asked, 6);
        assert_eq!(separating.cards()[0].shown, Shown::Separating(None));
        assert_eq!(
            separating.cancelled(5, Cancel::GivenUp).as_deref(),
            Some("Separating Chorus was stopped; no stem was kept.")
        );
        assert_eq!(
            separating.cancelled(6, Cancel::Finishing).as_deref(),
            Some("The stems of Chorus are being put in place; what came of it follows.")
        );
        assert_eq!(separating.cards().len(), 1);
        assert_eq!(separating.cancelled(5, Cancel::GivenUp), None);
        // A newer separation of the same media, for which the lane gave up
        // the older, takes its card.
        let said = separating.sent(
            &asked,
            &Extracting {
                request: 7,
                replaced: vec![6],
            },
        );
        assert_eq!(
            said.as_deref(),
            Some("Separating Chorus in place of Chorus: the same media is separated once.")
        );
        assert_eq!(
            separating
                .cards()
                .iter()
                .map(|card| card.request)
                .collect::<Vec<_>>(),
            [7]
        );
    }
}
