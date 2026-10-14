//! Save audio as a file: the sound of a library media file, or of the span
//! of it a clip plays, written to a WAV file the person picks.
//!
//! Nothing is edited. The sound lane writes the file (see
//! [`editor_media::ExtractJob`]): the source's own first sound stream, with
//! no clip volume, fade, effect or speed. Once it is written the person is
//! offered to add it to the project by the ordinary import, which is the
//! only step that enters the history. The library's media menu, the clip
//! menu and the project menu ask here, and are refused in the same words.
//!
//! [`Saving`] holds the files being written and the written ones offered to
//! the project. A file is the person's, asked of no project: a project that
//! is replaced while one is written leaves it writing, and its answer still
//! says where the file is, but offers nothing to the project that came
//! after. Nothing here reads a file or waits, except [`target`], which is
//! run where the dialog is awaited.
use editor_core::{Asset, AssetId, ClipId, MediaKind, Project, Time};
use editor_media::{Cancel, ExtractJob, Extracting, SoundEvent};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// What the rows that ask for it are called.
pub const LABEL: &str = "Save audio as…";
/// The extension the file is written with.
pub const EXTENSION: &str = "wav";
/// How long a written file is offered to the project, in seconds.
pub const OFFER_SECONDS: f64 = 12.0;

/// Whose sound is saved, captured when the row was chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The whole of a library media file's sound.
    Asset(AssetId),
    /// The span of its media's sound a clip plays, from the lowest source
    /// time it reaches to the highest.
    Clip(ClipId),
}

/// What is to be written, captured when the dialog is shown: the media as
/// it was then, and the span of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Asked {
    /// The project that asked: only it is offered the file.
    pub project: Uuid,
    pub asset: Asset,
    pub from: Time,
    pub span: Time,
    /// What the notices call it: the clip's name, or the media's.
    pub name: String,
}

fn has_sound(asset: &Asset) -> bool {
    asset.has_audio || asset.kind == MediaKind::Audio
}

impl Asked {
    /// What `source` asks to be written, or why it has nothing to write.
    pub fn of(project: &Project, instance: Uuid, source: Source) -> Result<Self, String> {
        let (asset, from, span, name) = match source {
            Source::Asset(id) => {
                let asset =
                    (project.assets.get(&id)).ok_or("That media is no longer in the library.")?;
                if !has_sound(asset) {
                    return Err("That media has no sound to save.".to_owned());
                }
                (asset, Time::ZERO, asset.duration, asset.name.clone())
            }
            Source::Clip(id) => {
                let (_, clip) = project.clip(id).ok_or("The clip no longer exists.")?;
                let asset = (clip.asset_id())
                    .and_then(|asset| project.assets.get(&asset))
                    .filter(|asset| has_sound(asset))
                    .ok_or("That clip has no sound to save.")?;
                let (first, last) = clip.source_span();
                let (from, until) = (first.min(last), first.max(last));
                let name = match clip.name.trim() {
                    "" => asset.name.clone(),
                    name => name.to_owned(),
                };
                (asset, from, Time(until.0 - from.0), name)
            }
        };
        if asset.duration <= Time::ZERO {
            return Err(format!("{} has no length to save.", asset.name));
        }
        if span <= Time::ZERO {
            return Err("That clip plays no stretch of its sound.".to_owned());
        }
        if from >= asset.duration {
            return Err("That clip plays past the end of its media.".to_owned());
        }
        Ok(Self {
            project: instance,
            asset: asset.clone(),
            from,
            span,
            name,
        })
    }

    /// The name the dialog offers: the media's, with `.wav` in place of its
    /// extension.
    pub fn file_name(&self) -> String {
        let stem = Path::new(&self.asset.name).file_stem().map_or_else(
            || self.asset.name.clone(),
            |stem| stem.to_string_lossy().into_owned(),
        );
        let stem = stem.replace(['/', '\\'], "-");
        let stem = match stem.trim() {
            "" => "Audio",
            stem => stem,
        };
        format!("{stem}.{EXTENSION}")
    }

    /// The lane's job that writes it to `to`, which the lane refuses when
    /// it is any of `project`'s media by any name.
    pub fn job(&self, project: &Project, to: PathBuf) -> ExtractJob {
        ExtractJob {
            project: self.project,
            asset: self.asset.clone(),
            from: self.from,
            span: self.span,
            to,
            keep: (project.assets.values())
                .map(|asset| asset.path.clone())
                .collect(),
        }
    }
}

/// Why `source` has no sound to save, when it has none: for a row shown
/// without being able to be chosen.
pub fn refusal(project: &Project, source: Source) -> Option<String> {
    Asked::of(project, Uuid::nil(), source).err()
}

/// The file a path picked in the dialog names: `.wav` is added when it has
/// no extension, and a name that comes of that and is taken is refused, for
/// the dialog asked about no such file. Another extension is refused. Run
/// where the dialog is awaited, never on the window's thread: it asks
/// whether a file is there.
pub fn target(picked: PathBuf, exists: impl Fn(&Path) -> bool) -> Result<PathBuf, String> {
    match picked.extension() {
        Some(extension) if extension.eq_ignore_ascii_case(EXTENSION) => Ok(picked),
        Some(_) => Err(format!(
            "Audio is saved as WAV: give the file a name that ends in .{EXTENSION}."
        )),
        None => {
            let mut named = picked.into_os_string();
            named.push(format!(".{EXTENSION}"));
            let named = PathBuf::from(named);
            if exists(&named) {
                return Err(format!(
                    "{} is already there: choose it in the dialog to replace it.",
                    shown(&named)
                ));
            }
            Ok(named)
        }
    }
}

/// Refuses a file that is media of the project, by its path as the library
/// has it, with the media's name. The lane refuses the project's media by
/// any other name: see [`Asked::job`].
pub fn guard(project: &Project, to: &Path) -> Result<(), String> {
    match (project.assets.values()).find(|asset| asset.path == to) {
        Some(asset) => Err(format!(
            "{} is media of this project: its sound is not saved over it.",
            asset.name
        )),
        None => Ok(()),
    }
}

/// A file's name as the notices give it.
fn shown(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

fn length(time: Time) -> String {
    if time < Time::from_seconds(60.0) {
        format!("{:.1} s", time.as_seconds())
    } else {
        crate::timefmt::short(time)
    }
}

/// A file the lane is writing.
#[derive(Clone, Debug)]
struct Writing {
    request: u64,
    name: String,
    to: PathBuf,
    done: f32,
}

/// A written file offered to the project that asked for it, until `until`.
#[derive(Clone, Debug)]
struct Offer {
    request: u64,
    to: PathBuf,
    said: String,
    until: f64,
}

/// What a card in the notice area shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Shown {
    /// Being written, `0..=1` of the way: with Cancel.
    Writing(f32),
    /// Written: with Add to project.
    Written,
    /// Stems being separated (see [`crate::stems`]), as far as the tool
    /// says, or not yet said: with Cancel.
    Separating(Option<f32>),
}

/// One card in the notice area, named by its request.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub request: u64,
    pub text: String,
    pub shown: Shown,
}

/// What an answer says, and whether it is a failure.
#[derive(Clone, Debug, PartialEq)]
pub struct Said {
    pub text: String,
    pub problem: bool,
}

/// The files being written and those offered to the project.
#[derive(Default)]
pub struct Saving {
    writing: Vec<Writing>,
    offers: Vec<Offer>,
}

impl Saving {
    /// A file the lane took. The files of the same span of the same media
    /// that the lane gave up for it go, and what to say of them is handed
    /// back; one the lane was too late to give up keeps its card, and its
    /// answer comes.
    pub fn sent(&mut self, asked: &Asked, to: PathBuf, sent: &Extracting) -> Option<String> {
        let replaced = (self.writing)
            .extract_if(.., |writing| sent.replaced.contains(&writing.request))
            .last();
        self.writing.push(Writing {
            request: sent.request,
            name: asked.name.clone(),
            to: to.clone(),
            done: 0.0,
        });
        replaced.map(|old| {
            format!(
                "{} is written to {} in place of {}.",
                old.name,
                shown(&to),
                shown(&old.to)
            )
        })
    }

    /// How far a file is written; `false` when `request` is no file's.
    pub fn progress(&mut self, request: u64, done: f32) -> bool {
        match self
            .writing
            .iter_mut()
            .find(|writing| writing.request == request)
        {
            Some(writing) => {
                writing.done = done.clamp(0.0, 1.0);
                true
            }
            None => false,
        }
    }

    /// Takes the answer for a file, and what to say of it besides its
    /// card. A file written for `instance` is offered to it until `now` and
    /// [`OFFER_SECONDS`], and its card says so; one written for a project
    /// that was replaced is only said, with where it is. An event that is
    /// no file's answer says nothing and changes nothing.
    pub fn answered(&mut self, instance: Uuid, now: f64, event: SoundEvent) -> Option<Said> {
        let SoundEvent::Extracted {
            request,
            project,
            to,
            result,
            ..
        } = event
        else {
            return None;
        };
        let writing = (self.writing.iter())
            .position(|writing| writing.request == request)
            .map(|at| self.writing.remove(at));
        let name = writing.map_or_else(|| shown(&to), |writing| writing.name);
        match result {
            Ok(written) if project == instance => {
                let said = format!("Saved {} of {name} as {}.", length(written), shown(&to));
                self.offers.push(Offer {
                    request,
                    to,
                    said,
                    until: now + OFFER_SECONDS,
                });
                None
            }
            // The whole path: the project that would have shown it is gone.
            Ok(written) => Some(Said {
                text: format!("Saved {} of {name} as {}.", length(written), to.display()),
                problem: false,
            }),
            Err(why) => Some(Said {
                // The first line of what FFmpeg said: the rest repeats it.
                text: format!(
                    "The sound of {name} was not saved: {}",
                    why.lines().next().unwrap_or_default()
                ),
                problem: true,
            }),
        }
    }

    /// What came of Cancel on a file's card, as the lane said. One given
    /// up is forgotten; one the lane is putting in place, or wrote, keeps
    /// its card until its answer.
    pub fn cancelled(&mut self, request: u64, outcome: Cancel) -> Option<String> {
        let at = (self.writing.iter()).position(|writing| writing.request == request)?;
        let writing = &self.writing[at];
        let (name, to) = (writing.name.clone(), shown(&writing.to));
        Some(match outcome {
            Cancel::GivenUp => {
                self.writing.remove(at);
                format!("Saving {name} was stopped; {to} was not written.")
            }
            Cancel::Finishing => format!("{to} is being put in place; what came of it follows."),
            Cancel::Written => format!("{to} is written already."),
        })
    }

    /// The written file offered as `request`, which is no longer offered.
    pub fn take_offer(&mut self, request: u64) -> Option<PathBuf> {
        let at = (self.offers.iter()).position(|offer| offer.request == request)?;
        Some(self.offers.remove(at).to)
    }

    /// The project was replaced: what was offered to it is not offered to
    /// the next. Files being written go on.
    pub fn project_replaced(&mut self) {
        self.offers.clear();
    }

    /// Lets offers whose time is up go, and says when the next one does.
    pub fn expire(&mut self, now: f64) -> Option<f64> {
        self.offers.retain(|offer| offer.until > now);
        (self.offers.iter())
            .map(|offer| offer.until)
            .min_by(f64::total_cmp)
    }

    /// The cards to show, files being written first.
    pub fn cards(&self) -> Vec<Card> {
        let writing = self.writing.iter().map(|writing| Card {
            request: writing.request,
            text: format!("Saving {} as {}", writing.name, shown(&writing.to)),
            shown: Shown::Writing(writing.done),
        });
        let offers = self.offers.iter().map(|offer| Card {
            request: offer.request,
            text: offer.said.clone(),
            shown: Shown::Written,
        });
        writing.chain(offers).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::{Clip, Command, Ease, Editor, Retime, TimeKey, Track, TrackKind};
    use editor_media::SourceIdentity;

    fn seconds(value: f64) -> Time {
        Time::from_seconds(value)
    }

    /// A project with a video with sound, a silent video and a title, the
    /// video with sound on the timeline.
    struct Bench {
        editor: Editor,
        talk: Asset,
        silent: Asset,
        clip: ClipId,
        title: ClipId,
    }

    fn bench() -> Bench {
        let mut editor = Editor::default();
        let mut talk = Asset::new("/media/talk.mp4".into(), MediaKind::Video, seconds(10.0));
        talk.has_audio = true;
        let silent = Asset::new("/media/silent.mp4".into(), MediaKind::Video, seconds(4.0));
        editor
            .dispatch(Command::AddAssets {
                assets: vec![talk.clone(), silent.clone()],
            })
            .unwrap();
        let track = Track::new("Video 1", TrackKind::Video);
        let track_id = track.id;
        editor.dispatch(Command::AddTrack { track }).unwrap();
        let mut clip = Clip::from_asset(&talk, Time::ZERO);
        clip.source_in = seconds(2.0);
        clip.duration = seconds(3.0);
        clip.name = "Answer".to_owned();
        let id = clip.id;
        editor
            .dispatch(Command::AddClip { track_id, clip })
            .unwrap();
        let text = Track::new("Text 1", TrackKind::Text);
        let text_id = text.id;
        editor.dispatch(Command::AddTrack { track: text }).unwrap();
        let title = Clip::text("Hello", Time::ZERO, seconds(2.0));
        let title_id = title.id;
        editor
            .dispatch(Command::AddClip {
                track_id: text_id,
                clip: title,
            })
            .unwrap();
        Bench {
            editor,
            talk,
            silent,
            clip: id,
            title: title_id,
        }
    }

    #[test]
    fn a_media_file_gives_its_whole_sound_and_a_clip_the_span_it_plays() {
        let bench = bench();
        let project = bench.editor.project();
        let here = Uuid::from_u128(9);
        let whole = Asked::of(project, here, Source::Asset(bench.talk.id)).unwrap();
        assert_eq!(
            (whole.project, whole.asset.id, whole.from, whole.span),
            (here, bench.talk.id, Time::ZERO, seconds(10.0))
        );
        assert_eq!(whole.name, "talk.mp4");
        assert_eq!(whole.file_name(), "talk.wav");
        let job = whole.job(project, "/out/talk.wav".into());
        assert_eq!(
            (job.from, job.span, job.project),
            (Time::ZERO, seconds(10.0), here)
        );
        // The lane is handed every media file of the project to refuse.
        let mut keep = job.keep.clone();
        keep.sort();
        assert_eq!(
            keep,
            vec![bench.silent.path.clone(), bench.talk.path.clone()]
        );

        // A clip: its own span, named for the clip; the file for its media.
        let span = Asked::of(project, here, Source::Clip(bench.clip)).unwrap();
        assert_eq!((span.from, span.span), (seconds(2.0), seconds(3.0)));
        assert_eq!(
            (span.name.as_str(), span.file_name()),
            ("Answer", "talk.wav".to_owned())
        );

        // At twice the speed it is half as long and plays the same source:
        // the span is of the source, and its speed no part of the file.
        let mut editor = bench.editor;
        editor
            .dispatch(Command::SetPlaybackRate {
                clip_id: bench.clip,
                rate: 2.0,
            })
            .unwrap();
        let (_, clip) = editor.project().clip(bench.clip).unwrap();
        assert_eq!(clip.duration, seconds(1.5));
        let fast = Asked::of(editor.project(), here, Source::Clip(bench.clip)).unwrap();
        assert_eq!((fast.from, fast.span), (seconds(2.0), seconds(3.0)));
        // Trimmed to its second second, it plays that one.
        editor
            .dispatch(Command::TrimClip {
                clip_id: bench.clip,
                source_in: seconds(3.0),
                duration: seconds(0.5),
                ripple: false,
            })
            .unwrap();
        let trimmed = Asked::of(editor.project(), here, Source::Clip(bench.clip)).unwrap();
        assert_eq!((trimmed.from, trimmed.span), (seconds(3.0), seconds(1.0)));
    }

    #[test]
    fn a_clip_run_backwards_gives_the_span_from_its_lowest_source() {
        let bench = bench();
        let mut editor = bench.editor;
        let (_, clip) = editor.project().clip(bench.clip).unwrap();
        // From 3 s into its source back to its start.
        let retime = Retime {
            keys: vec![
                TimeKey {
                    at: Time::ZERO,
                    source: seconds(3.0),
                    ease: Ease::Linear,
                },
                TimeKey {
                    at: clip.duration,
                    source: Time::ZERO,
                    ease: Ease::Linear,
                },
            ],
        };
        let commands = crate::speed::retime_commands(
            editor.project().clip(bench.clip).unwrap().0,
            clip,
            &retime,
            clip.duration,
        );
        for command in commands {
            editor.dispatch(command).unwrap();
        }
        let asked = Asked::of(editor.project(), Uuid::nil(), Source::Clip(bench.clip)).unwrap();
        assert_eq!((asked.from, asked.span), (seconds(2.0), seconds(3.0)));
    }

    #[test]
    fn what_has_no_sound_is_refused_in_words() {
        let bench = bench();
        let project = bench.editor.project();
        let refused = |source| refusal(project, source).unwrap();
        assert_eq!(
            refused(Source::Asset(bench.silent.id)),
            "That media has no sound to save."
        );
        assert_eq!(
            refused(Source::Asset(Uuid::new_v4())),
            "That media is no longer in the library."
        );
        assert_eq!(
            refused(Source::Clip(bench.title)),
            "That clip has no sound to save."
        );
        assert_eq!(
            refused(Source::Clip(Uuid::new_v4())),
            "The clip no longer exists."
        );
        assert_eq!(refusal(project, Source::Clip(bench.clip)), None);
        assert_eq!(refusal(project, Source::Asset(bench.talk.id)), None);
    }

    #[test]
    fn the_picked_file_is_a_wav_and_no_media_of_the_project() {
        let none = |_: &Path| false;
        let taken = |_: &Path| true;
        assert_eq!(
            target("/out/take.wav".into(), taken),
            Ok(PathBuf::from("/out/take.wav"))
        );
        assert_eq!(
            target("/out/take.WAV".into(), taken),
            Ok(PathBuf::from("/out/take.WAV"))
        );
        // No extension: one is added, unless that names a file the dialog
        // did not ask about.
        assert_eq!(
            target("/out/take".into(), none),
            Ok(PathBuf::from("/out/take.wav"))
        );
        assert_eq!(
            target("/out/take".into(), taken),
            Err("take.wav is already there: choose it in the dialog to replace it.".to_owned())
        );
        assert_eq!(
            target("/out/film.aster".into(), none),
            Err("Audio is saved as WAV: give the file a name that ends in .wav.".to_owned())
        );

        let bench = bench();
        let project = bench.editor.project();
        assert_eq!(
            guard(project, Path::new("/media/talk.mp4")),
            Err("talk.mp4 is media of this project: its sound is not saved over it.".to_owned())
        );
        assert_eq!(guard(project, Path::new("/out/talk.wav")), Ok(()));
    }

    /// What the lane says it took, and gave up for it.
    fn took(request: u64, replaced: &[u64]) -> Extracting {
        Extracting {
            request,
            replaced: replaced.to_vec(),
        }
    }

    fn answer(request: u64, project: Uuid, to: &str, result: Result<Time, String>) -> SoundEvent {
        let asset = Asset::new("/media/talk.mp4".into(), MediaKind::Video, seconds(10.0));
        SoundEvent::Extracted {
            request,
            project,
            asset_id: asset.id,
            source: SourceIdentity::of(&asset),
            to: to.into(),
            result,
        }
    }

    #[test]
    fn a_written_file_is_offered_to_the_project_that_asked_and_to_no_other() {
        let bench = bench();
        let here = Uuid::from_u128(1);
        let asked = Asked::of(bench.editor.project(), here, Source::Clip(bench.clip)).unwrap();
        let mut saving = Saving::default();
        assert_eq!(
            saving.sent(&asked, "/out/answer.wav".into(), &took(7, &[])),
            None
        );
        assert!(saving.progress(7, 0.4));
        assert!(!saving.progress(8, 0.4));
        assert_eq!(
            saving.cards(),
            vec![Card {
                request: 7,
                text: "Saving Answer as answer.wav".to_owned(),
                shown: Shown::Writing(0.4),
            }]
        );
        // Another event is not a file's.
        let other = SoundEvent::Progress {
            request: 3,
            done: 0.5,
        };
        assert_eq!(saving.answered(here, 0.0, other), None);
        assert_eq!(saving.cards().len(), 1);

        // Written for this project: offered, and said on the card.
        let said = saving.answered(
            here,
            100.0,
            answer(7, here, "/out/answer.wav", Ok(seconds(3.0))),
        );
        assert_eq!(said, None);
        assert_eq!(
            saving.cards(),
            vec![Card {
                request: 7,
                text: "Saved 3.0 s of Answer as answer.wav.".to_owned(),
                shown: Shown::Written,
            }]
        );
        assert_eq!(
            saving.expire(100.0 + OFFER_SECONDS - 1.0),
            Some(100.0 + OFFER_SECONDS)
        );
        assert_eq!(saving.take_offer(7), Some(PathBuf::from("/out/answer.wav")));
        assert_eq!(saving.take_offer(7), None);
        assert!(saving.cards().is_empty());

        // An offer leaves when its time is up, or with its project.
        saving.sent(&asked, "/out/again.wav".into(), &took(8, &[]));
        let _ = saving.answered(
            here,
            0.0,
            answer(8, here, "/out/again.wav", Ok(seconds(3.0))),
        );
        assert_eq!(saving.expire(OFFER_SECONDS), None);
        assert!(saving.cards().is_empty());
        saving.sent(&asked, "/out/again.wav".into(), &took(9, &[]));
        let _ = saving.answered(
            here,
            0.0,
            answer(9, here, "/out/again.wav", Ok(seconds(3.0))),
        );
        saving.project_replaced();
        assert!(saving.cards().is_empty());

        // Written after the project was replaced: said, with where it is,
        // and offered to nobody.
        saving.sent(&asked, "/out/late.wav".into(), &took(10, &[]));
        saving.project_replaced();
        assert_eq!(saving.cards().len(), 1, "a file being written goes on");
        let next = Uuid::from_u128(2);
        let said = saving
            .answered(
                next,
                0.0,
                answer(10, here, "/out/late.wav", Ok(seconds(65.0))),
            )
            .unwrap();
        assert_eq!(
            said,
            Said {
                text: format!(
                    "Saved 1:05 of Answer as {}.",
                    Path::new("/out/late.wav").display()
                ),
                problem: false,
            }
        );
        assert!(saving.cards().is_empty());
        assert_eq!(saving.take_offer(10), None);
    }

    #[test]
    fn a_file_not_written_is_said_and_a_replaced_or_stopped_one_too() {
        let bench = bench();
        let here = Uuid::from_u128(1);
        let project = bench.editor.project();
        let asked = Asked::of(project, here, Source::Clip(bench.clip)).unwrap();
        let mut saving = Saving::default();
        saving.sent(&asked, "/out/one.wav".into(), &took(1, &[]));
        // The same span again: the lane gave the first up.
        assert_eq!(
            saving.sent(&asked, "/out/two.wav".into(), &took(2, &[1])),
            Some("Answer is written to two.wav in place of one.wav.".to_owned())
        );
        // Another span of the same media is another file.
        let whole = Asked::of(project, here, Source::Asset(bench.talk.id)).unwrap();
        assert_eq!(
            saving.sent(&whole, "/out/all.wav".into(), &took(3, &[])),
            None
        );
        assert_eq!(saving.cards().len(), 2);

        let failed = answer(
            2,
            here,
            "/out/two.wav",
            Err("ffmpeg: could not write the sound: No space left\nmore".to_owned()),
        );
        assert_eq!(
            saving.answered(here, 0.0, failed).unwrap(),
            Said {
                text: "The sound of Answer was not saved: ffmpeg: could not write the sound: No space left"
                    .to_owned(),
                problem: true,
            }
        );
        assert_eq!(
            saving.cancelled(3, Cancel::GivenUp),
            Some("Saving talk.mp4 was stopped; all.wav was not written.".to_owned())
        );
        assert_eq!(saving.cancelled(3, Cancel::GivenUp), None);
        assert!(saving.cards().is_empty());
        // An answer of a file this window forgot still says where it is.
        let said = saving.answered(
            here,
            0.0,
            answer(4, here, "/out/lost.wav", Ok(seconds(1.0))),
        );
        assert_eq!(said, None);
        assert_eq!(
            saving.cards()[0].text,
            "Saved 1.0 s of lost.wav as lost.wav."
        );
    }

    #[test]
    fn a_file_the_lane_was_too_late_to_give_up_keeps_its_card_and_its_name() {
        let bench = bench();
        let here = Uuid::from_u128(1);
        let asked = Asked::of(bench.editor.project(), here, Source::Clip(bench.clip)).unwrap();
        let mut saving = Saving::default();
        saving.sent(&asked, "/out/one.wav".into(), &took(1, &[]));
        // The same span again, while the first is put in place: the lane
        // gave nothing up, so nothing is said to be in its place.
        assert_eq!(
            saving.sent(&asked, "/out/two.wav".into(), &took(2, &[])),
            None
        );
        assert_eq!(saving.cards().len(), 2);
        // Cancel, too late: the card stays and says what follows.
        assert_eq!(
            saving.cancelled(1, Cancel::Finishing),
            Some("one.wav is being put in place; what came of it follows.".to_owned())
        );
        assert_eq!(saving.cards().len(), 2);
        // The first answers under its own name, and is offered.
        assert_eq!(
            saving.answered(here, 0.0, answer(1, here, "/out/one.wav", Ok(seconds(3.0)))),
            None
        );
        let cards = saving.cards();
        assert_eq!(cards[1].text, "Saved 3.0 s of Answer as one.wav.");
        assert_eq!(cards[1].shown, Shown::Written);
        // Written, its answer unread: said so, and the card stays for it.
        assert_eq!(
            saving.cancelled(2, Cancel::Written),
            Some("two.wav is written already.".to_owned())
        );
        assert_eq!(saving.cards()[0].shown, Shown::Writing(0.0));
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

    /// Two seconds of a 16-bit mono 8 kHz WAV, written here.
    fn tone(dir: &Path) -> Asset {
        let samples: Vec<i16> = (0..16_000)
            .map(|at| ((at as f64 * 0.785).sin() * 8_000.0) as i16)
            .collect();
        let data = (samples.len() * 2) as u32;
        let mut bytes = Vec::new();
        bytes.extend(b"RIFF");
        bytes.extend((36 + data).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
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
        let path = dir.join("voice.wav");
        std::fs::write(&path, &bytes).expect("FIXTURE: the sound file");
        let mut asset = Asset::new(path, MediaKind::Audio, seconds(2.0));
        asset.file_size = bytes.len() as u64;
        asset
    }

    /// Polls the lane until it answers for a file, handing progress to
    /// `saving` on the way.
    fn await_answer(
        sound: &editor_media::SoundService,
        saving: &mut Saving,
        instance: Uuid,
    ) -> Option<Said> {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            if let Some(event) = sound.poll() {
                if let SoundEvent::Progress { request, done } = event {
                    assert!(saving.progress(request, done));
                    continue;
                }
                assert!(matches!(event, SoundEvent::Extracted { .. }), "{event:?}");
                return saving.answered(instance, 0.0, event);
            }
            assert!(std::time::Instant::now() < until, "nothing was written");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn a_span_written_by_the_lane_is_offered_and_after_a_replace_only_said() {
        if !ffmpeg_available() {
            eprintln!("SKIP: FFmpeg is unavailable; nothing was written through the lane");
            return;
        }
        let dir = std::env::temp_dir().join(format!("aster-extract-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let asset = tone(&dir);
        let mut editor = Editor::default();
        editor
            .dispatch(Command::AddAssets {
                assets: vec![asset.clone()],
            })
            .unwrap();
        let here = editor.instance_id();
        let asked = Asked::of(editor.project(), here, Source::Asset(asset.id)).unwrap();
        let sound = editor_media::SoundService::new(|| {}).unwrap();
        let mut saving = Saving::default();

        let to = dir.join(asked.file_name());
        assert_eq!(to, dir.join("voice.wav"));
        // The dialog's name is the source itself here: refused by the lane.
        let request = sound.extract(asked.job(editor.project(), to.clone()));
        assert!(request.is_err(), "{request:?}");
        let to = target(dir.join("voice copy"), |path| path.exists()).unwrap();
        guard(editor.project(), &to).unwrap();
        let sent = sound
            .extract(asked.job(editor.project(), to.clone()))
            .unwrap();
        let request = sent.request;
        saving.sent(&asked, to.clone(), &sent);
        assert_eq!(await_answer(&sound, &mut saving, here), None);
        let cards = saving.cards();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].text, "Saved 2.0 s of voice.wav as voice copy.wav.");
        assert!(to.exists() && !editor_media::part_path(&to).exists());
        // Taken up, it is that file, for the ordinary import.
        assert_eq!(saving.take_offer(request), Some(to.clone()));

        // Asked, and the project replaced before it is written: the lane
        // writes it all the same, and it is said with where it is.
        let late = dir.join("late.wav");
        let sent = sound
            .extract(asked.job(editor.project(), late.clone()))
            .unwrap();
        saving.sent(&asked, late.clone(), &sent);
        sound.cancel_all();
        saving.project_replaced();
        let said = await_answer(&sound, &mut saving, Uuid::new_v4()).unwrap();
        assert_eq!(
            said,
            Said {
                text: format!("Saved 2.0 s of voice.wav as {}.", late.display()),
                problem: false,
            }
        );
        assert!(saving.cards().is_empty());
        assert!(late.exists());
        drop(sound);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
