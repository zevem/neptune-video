//! Voiceover: a take recorded while the timeline plays, placed where the
//! words were said.
//!
//! Recording is not a command: the person starts it, and the take enters
//! the project as one ordinary step, "Record voiceover", a `Batch` of
//! `AddAssets`, `AddTrack` when no audio track has room, and `AddClip`. Undo
//! takes the clip and the media out of the project; the file stays.
//!
//! [`Recording`] is the take under way. It keeps its own clock, the frames
//! the service has taken: the project's end stops the transport, never the
//! take, which runs on in an empty project and past the last clip with the
//! playhead held. The take is tied to the playback clock rather than to the
//! press, for the sound device takes a while to start: on the first frame
//! after the device began where the output reports a position, that
//! position and the frames taken so far are kept as the [`Anchor`], and the
//! clip starts there with what came before it trimmed by `source_in` (the
//! pre-roll stays in the file). Where no position comes, because nothing
//! plays or no output answers within a second, the transport's position on
//! that frame is the clock.
//!
//! Nothing here touches a file or waits. The take's path is made on a worker
//! (`editor_store::new_take`), and the device is opened and the file written
//! and closed by the service's own thread; each answer comes through `poll`.
use crate::rules;
use editor_core::{Asset, Clip, Command, MediaKind, Playback, Project, Time, Track, TrackKind};
use editor_media::{MediaError, RecordEvent, RecordService, Take};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use uuid::Uuid;

/// What the history calls the step that places a take.
pub const LABEL: &str = "Record voiceover";
/// The name of the audio track made for a take that finds no room.
pub const TRACK: &str = "Voiceover";
/// What a project without a file is told: a take is written beside it.
pub const UNTITLED: &str = "Save the project to record";
/// What a second take is told while one is under way.
pub const BUSY: &str = "A take is already being recorded";
/// The shortest placed part of a take that becomes a clip: a tenth of a
/// second. A shorter one stays on disk and is not placed.
pub const SHORTEST: Time = Time(100_000);
/// How long a playing timeline waits for the sound device's first position
/// before the transport's own is taken as the clock.
pub const DEVICE_WAIT: Duration = Duration::from_secs(1);
/// How often the window is drawn while a take runs, for its elapsed time and
/// level.
pub const REPAINT: Duration = Duration::from_millis(100);

/// What the desktop needs of the service recording a take. `RecordService`
/// is one; a test's stand-in is another.
pub trait Recorder {
    /// Asks the take to end; `Finished` follows through `poll`.
    fn stop(&self);
    fn poll(&self) -> Option<RecordEvent>;
    /// Frames taken since the device began, which is their place in the file.
    fn frames(&self) -> u64;
    /// How long the take is so far.
    fn elapsed(&self) -> Time;
}

impl Recorder for RecordService {
    fn stop(&self) {
        RecordService::stop(self);
    }
    fn poll(&self) -> Option<RecordEvent> {
        RecordService::poll(self)
    }
    fn frames(&self) -> u64 {
        RecordService::frames(self)
    }
    fn elapsed(&self) -> Time {
        RecordService::elapsed(self)
    }
}

/// Where a take meets the timeline: the frame of the file that was heard
/// with the timeline at `at`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub frames: u64,
    pub at: Time,
}

/// What the viewer's record control shows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum RecordView {
    #[default]
    Idle,
    /// Asked for: the file's place is being made or the microphone opened.
    Starting,
    /// A take is running: how long it is, and its loudest sample since the
    /// last frame (1 is full scale).
    Recording { elapsed: Time, level: f32 },
    /// Asked to stop: the file is being closed.
    Stopping,
}

enum Phase<R> {
    /// The take's path is being made on a worker.
    Path,
    /// The service is opening the device and making the file.
    Opening(R),
    /// The device plays and the file is made.
    Running {
        service: R,
        /// When `Started` was read.
        started: Instant,
        /// `Started` was read in this frame: the transport was only now
        /// asked to play, and no position can be of it yet.
        fresh: bool,
        anchor: Option<Anchor>,
        level: f32,
    },
    /// Asked to stop; the file is being closed.
    Stopping { service: R, anchor: Option<Anchor> },
}

/// A take, from the press to its closed file.
struct Taking<R> {
    /// The editor instance it was started in: only that project is given it.
    project: Uuid,
    phase: Phase<R>,
}

/// The take under way, if any: one at a time.
pub struct Recording<R = RecordService> {
    taking: Option<Taking<R>>,
}

impl<R> Default for Recording<R> {
    fn default() -> Self {
        Self { taking: None }
    }
}

/// A closed take, and what becomes of it.
#[derive(Debug)]
pub struct Landed {
    /// The step that places it, when it is placed.
    pub command: Option<Command>,
    /// What is said of it: where the file is when it is not placed, and
    /// frames that were lost.
    pub said: String,
    pub problem: bool,
    /// The file, which stays whatever is done with it.
    pub path: PathBuf,
}

/// What a frame of the take asks of the application.
#[derive(Debug, Default)]
pub struct Framed {
    /// The transport was started or stopped: its sound has to follow.
    pub transport: bool,
    /// Nothing was recorded, and why.
    pub failed: Option<String>,
    /// The take is closed.
    pub landed: Option<Landed>,
}

impl<R: Recorder> Recording<R> {
    /// Whether a take is asked for, running or being closed.
    pub fn busy(&self) -> bool {
        self.taking.is_some()
    }

    pub fn view(&self) -> RecordView {
        match self.taking.as_ref().map(|taking| &taking.phase) {
            None => RecordView::Idle,
            Some(Phase::Path | Phase::Opening(_)) => RecordView::Starting,
            Some(Phase::Running { service, level, .. }) => RecordView::Recording {
                elapsed: service.elapsed(),
                level: *level,
            },
            Some(Phase::Stopping { .. }) => RecordView::Stopping,
        }
    }

    /// How soon the window has to be drawn again for the take: ten times a
    /// second while it runs, and never otherwise. Its other moments come
    /// with a wake of their own.
    pub fn repaint_after(&self) -> Option<Duration> {
        matches!(
            self.taking.as_ref().map(|taking| &taking.phase),
            Some(Phase::Running { .. })
        )
        .then_some(REPAINT)
    }

    /// Asks for a take in the editor instance `project`, whose document is
    /// at `document`. Refused in words while a take is under way and for a
    /// project with no file, before anything is opened. A timeline that
    /// plays is paused, so the playhead holds until `Started` rolls it at
    /// 1×. `Ok` is where the take's path is to be made beside, off the frame
    /// loop; its answer goes to [`Recording::path_answered`].
    pub fn ask(
        &mut self,
        project: Uuid,
        document: Option<&Path>,
        playback: &mut Playback,
    ) -> Result<PathBuf, String> {
        if self.busy() {
            return Err(BUSY.to_owned());
        }
        let document = document.ok_or(UNTITLED)?;
        playback.pause();
        self.taking = Some(Taking {
            project,
            phase: Phase::Path,
        });
        Ok(document.to_path_buf())
    }

    /// The path asked for in `project` was made, or not. The service is
    /// started only for the take that waits for it: an answer for a project
    /// that was replaced, or a take that was stopped meanwhile, opens no
    /// device. `start` opens it; `Err` says in words why nothing records.
    pub fn path_answered(
        &mut self,
        project: Uuid,
        answer: Result<PathBuf, String>,
        start: impl FnOnce(PathBuf) -> Result<R, MediaError>,
    ) -> Result<(), String> {
        let waits = matches!(
            &self.taking,
            Some(Taking { project: asked, phase: Phase::Path }) if *asked == project
        );
        if !waits {
            return Ok(());
        }
        let started = answer
            .map_err(|error| format!("Cannot record: {error}"))
            .and_then(|path| start(path).map_err(|error| error.to_string()));
        match started {
            Ok(service) => {
                if let Some(taking) = &mut self.taking {
                    taking.phase = Phase::Opening(service);
                }
                Ok(())
            }
            Err(error) => {
                self.taking = None;
                Err(error)
            }
        }
    }

    /// Ends the take: the transport stops where it is, and the service is
    /// asked to close the file. A take whose anchor has not come is anchored
    /// here, to the transport's position. One whose path is still being made
    /// is given up, and nothing opens. `true` when the transport stopped.
    pub fn stop(&mut self, playback: &mut Playback) -> bool {
        let Some(taking) = self.taking.take() else {
            return false;
        };
        // In every phase: whatever plays under a take ends with it.
        let stopped = playback.playing;
        playback.pause();
        let phase = match taking.phase {
            Phase::Path => return stopped,
            Phase::Opening(service) => {
                service.stop();
                Phase::Stopping {
                    service,
                    anchor: None,
                }
            }
            Phase::Running {
                service, anchor, ..
            } => {
                let anchor = anchor.or(Some(Anchor {
                    frames: service.frames(),
                    at: playback.position,
                }));
                service.stop();
                Phase::Stopping { service, anchor }
            }
            stopping @ Phase::Stopping { .. } => stopping,
        };
        self.taking = Some(Taking {
            project: taking.project,
            phase,
        });
        stopped
    }

    /// The project the take was started in is no longer the document. The
    /// take is stopped and its file kept; once closed it is said where.
    pub fn project_replaced(&mut self) {
        let mut idle = Playback::default();
        self.stop(&mut idle);
    }

    /// Closes the take at once, as the window goes: its file is kept and
    /// nothing is placed. Waits for the service's thread to close the file.
    pub fn close(&mut self) {
        let Some(taking) = self.taking.take() else {
            return;
        };
        if let Phase::Opening(service)
        | Phase::Running { service, .. }
        | Phase::Stopping { service, .. } = &taking.phase
        {
            service.stop();
        }
    }

    /// The take's anchor, once it has come.
    #[cfg(test)]
    fn anchor(&self) -> Option<Anchor> {
        match self.taking.as_ref().map(|taking| &taking.phase) {
            Some(Phase::Running { anchor, .. } | Phase::Stopping { anchor, .. }) => *anchor,
            _ => None,
        }
    }

    /// One frame: what the service said is taken, and the anchor when it
    /// is due. `project` and `instance` are the document now; `heard` is the
    /// sound device's position this frame, read once with the transport's.
    pub fn frame(
        &mut self,
        project: &Project,
        instance: Uuid,
        playback: &mut Playback,
        heard: Option<Time>,
        now: Instant,
    ) -> Framed {
        let mut framed = Framed::default();
        loop {
            let Some(taking) = &mut self.taking else {
                return framed;
            };
            let service = match &taking.phase {
                Phase::Path => return framed,
                Phase::Opening(service)
                | Phase::Running { service, .. }
                | Phase::Stopping { service, .. } => service,
            };
            let Some(event) = service.poll() else { break };
            match event {
                RecordEvent::Started { .. } => {
                    let phase = std::mem::replace(&mut taking.phase, Phase::Path);
                    taking.phase = match phase {
                        Phase::Opening(service) => {
                            // Only the project it was asked in is played.
                            if taking.project == instance && roll(playback, project.duration()) {
                                framed.transport = true;
                            }
                            Phase::Running {
                                service,
                                started: now,
                                fresh: true,
                                anchor: None,
                                level: 0.0,
                            }
                        }
                        other => other,
                    };
                }
                RecordEvent::Level(level) => {
                    if let Phase::Running { level: shown, .. } = &mut taking.phase {
                        *shown = level;
                    }
                }
                RecordEvent::Failed(error) => {
                    // Given up while opening, it was asked for and is no
                    // problem.
                    let asked = matches!(taking.phase, Phase::Stopping { .. });
                    self.taking = None;
                    if !(asked && matches!(error, MediaError::Cancelled)) {
                        framed.failed = Some(error.to_string());
                    }
                    return framed;
                }
                RecordEvent::Finished(take) => {
                    let Some(Taking {
                        project: asked,
                        phase,
                    }) = self.taking.take()
                    else {
                        return framed;
                    };
                    let anchor = match phase {
                        Phase::Running {
                            service, anchor, ..
                        } => {
                            // It ended by itself: the transport stops too.
                            framed.transport |= playback.playing;
                            playback.pause();
                            anchor.unwrap_or(Anchor {
                                frames: service.frames(),
                                at: playback.position,
                            })
                        }
                        Phase::Stopping { anchor, .. } => anchor.unwrap_or(Anchor {
                            frames: take.frames,
                            at: playback.position,
                        }),
                        Phase::Path | Phase::Opening(_) => Anchor {
                            frames: take.frames,
                            at: playback.position,
                        },
                    };
                    framed.landed = Some(land(project, instance == asked, &take, anchor));
                    return framed;
                }
            }
        }
        if let Some(Taking {
            phase:
                Phase::Running {
                    service,
                    started,
                    fresh,
                    anchor,
                    ..
                },
            ..
        }) = &mut self.taking
        {
            if anchor.is_none() && !*fresh {
                let waited = now.saturating_duration_since(*started) >= DEVICE_WAIT;
                let at = match heard {
                    Some(position) if playback.playing => Some(position),
                    // Nothing plays, or the device has not answered in time:
                    // the transport is the clock.
                    _ if !playback.playing || waited => Some(playback.position),
                    _ => None,
                };
                *anchor = at.map(|at| Anchor {
                    frames: service.frames(),
                    at,
                });
            }
            *fresh = false;
        }
        framed
    }
}

/// Starts the timeline from its playhead at normal speed, when anything of
/// it lies ahead: a take is placed as long as it was recorded. `true` when
/// the transport changed; `false` when it already plays at that speed, or
/// nothing plays, in an empty project or with the playhead at its end.
fn roll(playback: &mut Playback, end: Time) -> bool {
    // Already heard at the take's pace: nothing to restart.
    if playback.playing && playback.speed == 1.0 {
        return false;
    }
    if playback.position >= end {
        return false;
    }
    let _ = playback.set_speed(1.0);
    playback.play();
    true
}

/// `frames` at `rate` as a time, rounded down.
fn at_rate(frames: u64, rate: u32) -> Time {
    if rate == 0 {
        return Time::ZERO;
    }
    Time((u128::from(frames) * 1_000_000 / u128::from(rate)) as i64)
}

/// The step that places `take` from `anchor` in `project`, or why it is not
/// placed. The media is made from the take itself, without a probe: an
/// audio file as long as the frames the writer counted. The clip starts at
/// the anchor's position and plays the take from the anchor's frame to its
/// end, on the first unlocked audio track with room over that span, or on a
/// new "Voiceover" track.
pub fn placement(project: &Project, take: &Take, anchor: Anchor) -> Result<Command, String> {
    let length = at_rate(take.frames, take.rate);
    let skipped = at_rate(anchor.frames.min(take.frames), take.rate);
    let placed = length - skipped;
    if placed < SHORTEST {
        return Err(format!(
            "The take was too short to place. It is kept in {}",
            take.path.display()
        ));
    }
    let mut asset = Asset::new(take.path.clone(), MediaKind::Audio, length);
    asset.has_audio = true;
    asset.file_size = take.bytes;
    let mut clip = Clip::from_asset(&asset, anchor.at.max(Time::ZERO));
    clip.source_in = skipped;
    clip.duration = placed;
    let free = (project.tracks.iter()).find(|track| {
        track.kind == TrackKind::Audio
            && !track.locked
            && rules::free_start(track, clip.start, clip.duration, &[]) == clip.start
    });
    let mut commands = vec![Command::AddAssets {
        assets: vec![asset],
    }];
    let track_id = match free {
        Some(track) => track.id,
        None => {
            let track = Track::new(TRACK, TrackKind::Audio);
            let id = track.id;
            commands.push(Command::AddTrack { track });
            id
        }
    };
    commands.push(Command::AddClip { track_id, clip });
    Ok(Command::Batch {
        label: LABEL.to_owned(),
        commands,
    })
}

/// What becomes of a closed take in `project`, which is the one it was
/// recorded for when `same` holds.
fn land(project: &Project, same: bool, take: &Take, anchor: Anchor) -> Landed {
    let mut tail = String::new();
    if take.dropped > 0 {
        let lost = at_rate(take.dropped, take.rate);
        let amount = match lost < Time(100_000) {
            true => "Less than a tenth of a second".to_owned(),
            false => length(lost),
        };
        tail.push_str(&format!(
            ". {amount} of sound was lost: the recorder fell behind"
        ));
    }
    if let Some(ended) = &take.ended {
        tail.push_str(&format!(". {ended}"));
    }
    let lost = take.dropped > 0 || take.ended.is_some();
    let (command, said, problem) = if !same {
        (
            None,
            format!(
                "The take is kept in {}: the project it was recorded for was closed, so nothing was added",
                take.path.display()
            ),
            true,
        )
    } else {
        match placement(project, take, anchor) {
            Ok(command) => (
                Some(command),
                format!(
                    "Recorded {} of voiceover",
                    length(at_rate(
                        take.frames.saturating_sub(anchor.frames),
                        take.rate
                    ))
                ),
                lost,
            ),
            Err(said) => (None, said, true),
        }
    };
    Landed {
        command,
        said: format!("{said}{tail}"),
        problem,
        path: take.path.clone(),
    }
}

/// A take's length as a notice gives it.
fn length(time: Time) -> String {
    if time < Time::from_seconds(60.0) {
        format!("{:.1} s", time.as_seconds())
    } else {
        crate::timefmt::short(time)
    }
}

/// How long a take is so far, as the record control reads it: whole
/// seconds, never ahead of the take.
pub fn elapsed_text(elapsed: Time) -> String {
    let whole = Time(elapsed.0.max(0) / 1_000_000 * 1_000_000);
    crate::timefmt::short(whole)
}

/// A take's loudest sample as the share of the level bar it fills: -60 dBFS
/// and under is empty, full scale is full.
pub fn level_fill(level: f32) -> f32 {
    if level.is_nan() || level <= 0.0 {
        return 0.0;
    }
    ((20.0 * level.log10() + 60.0) / 60.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::{Editor, TrackId};
    use std::{
        cell::{Cell, RefCell},
        collections::VecDeque,
        rc::Rc,
        time::SystemTime,
    };

    const RATE: u32 = 48_000;

    fn seconds(value: f64) -> Time {
        Time::from_seconds(value)
    }

    /// What the stand-in for `RecordService` says and counts.
    #[derive(Default)]
    struct Device {
        frames: Cell<u64>,
        said: RefCell<VecDeque<RecordEvent>>,
        stopped: Cell<bool>,
    }

    /// The service as the recording holds it: the test keeps the other end.
    struct Fake(Rc<Device>);

    impl Recorder for Fake {
        fn stop(&self) {
            self.0.stopped.set(true);
        }
        fn poll(&self) -> Option<RecordEvent> {
            self.0.said.borrow_mut().pop_front()
        }
        fn frames(&self) -> u64 {
            self.0.frames.get()
        }
        fn elapsed(&self) -> Time {
            at_rate(self.0.frames.get(), RATE)
        }
    }

    /// The application's part of a take, as `App` does it, with a fake
    /// service and a fake clock.
    struct Desk {
        editor: Editor,
        playback: Playback,
        recording: Recording<Fake>,
        device: Rc<Device>,
        now: Instant,
        dir: PathBuf,
        document: PathBuf,
        /// Where the take is written, once the device was opened there.
        path: Option<PathBuf>,
        opened: usize,
        said: Vec<(String, bool)>,
    }

    impl Drop for Desk {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    impl Desk {
        fn new(editor: Editor) -> Self {
            let dir = std::env::temp_dir().join(format!("neptune-video-record-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Self {
                editor,
                playback: Playback::default(),
                recording: Recording::default(),
                device: Rc::default(),
                now: Instant::now(),
                document: dir.join("Film.nvp"),
                dir,
                path: None,
                opened: 0,
                said: Vec::new(),
            }
        }

        fn empty() -> Self {
            Self::new(Editor::new(Project::new("Film")).unwrap())
        }

        /// A project with one picture clip of `length` from 0.
        fn lasting(length: f64) -> Self {
            let mut editor = Editor::new(Project::new("Film")).unwrap();
            let mut asset = Asset::new("/media/shot.mp4".into(), MediaKind::Video, seconds(length));
            asset.width = 1920;
            asset.height = 1080;
            let clip = Clip::from_asset(&asset, Time::ZERO);
            let video = track(&editor, TrackKind::Video);
            editor
                .dispatch(Command::AddAssets {
                    assets: vec![asset],
                })
                .unwrap();
            editor
                .dispatch(Command::AddClip {
                    track_id: video,
                    clip,
                })
                .unwrap();
            Self::new(editor)
        }

        /// Record pressed, and the take's path made beside the document.
        fn press(&mut self) -> Result<(), String> {
            let instance = self.editor.instance_id();
            let beside = self
                .recording
                .ask(instance, Some(&self.document), &mut self.playback)?;
            let path = editor_store::new_take(&beside, SystemTime::now())
                .map_err(|error| error.to_string());
            self.answer(instance, path)
        }

        /// The worker's answer for `project` comes back.
        fn answer(&mut self, project: Uuid, path: Result<PathBuf, String>) -> Result<(), String> {
            let (device, opened, at) = (&self.device, &mut self.opened, &mut self.path);
            self.recording.path_answered(project, path, |path| {
                *opened += 1;
                *at = Some(path);
                Ok(Fake(Rc::clone(device)))
            })
        }

        /// The device plays and the file is made, `frames` already taken.
        fn started(&mut self, frames: u64) {
            let path = self.path.clone().expect("FIXTURE: no device was opened");
            std::fs::write(&path, [0; 44]).unwrap();
            self.device.frames.set(frames);
            self.device
                .said
                .borrow_mut()
                .push_back(RecordEvent::Started {
                    rate: RATE,
                    channels: 1,
                });
        }

        /// The file is closed with `frames` in it.
        fn finished(&mut self, frames: u64, dropped: u64, ended: Option<&str>) {
            self.device.frames.set(frames);
            let take = Take {
                path: self.path.clone().unwrap(),
                frames,
                rate: RATE,
                channels: 1,
                bytes: 44 + frames * 4,
                dropped,
                ended: ended.map(str::to_owned),
            };
            self.device
                .said
                .borrow_mut()
                .push_back(RecordEvent::Finished(take));
        }

        /// A frame `ms` after the last, with the take at `frames` and the
        /// sound device at `heard`, read as `App::logic` reads them.
        fn frame(&mut self, ms: u64, frames: u64, heard: Option<Time>) {
            self.now += Duration::from_millis(ms);
            self.device.frames.set(frames);
            if self.playback.playing {
                let duration = self.editor.project().duration();
                self.playback.tick_with_audio(self.now, duration, heard);
            }
            let framed = self.recording.frame(
                self.editor.project(),
                self.editor.instance_id(),
                &mut self.playback,
                heard,
                self.now,
            );
            if let Some(failed) = framed.failed {
                self.said.push((failed, true));
            }
            if let Some(landed) = framed.landed {
                if let Some(command) = landed.command {
                    self.editor.dispatch(command).unwrap();
                }
                self.said.push((landed.said, landed.problem));
            }
        }

        fn stop(&mut self) {
            self.recording.stop(&mut self.playback);
        }

        /// The clip made of the take, with its track and media.
        fn placed(&self) -> Option<(Track, Clip, Asset)> {
            let path = self.path.as_ref()?;
            let project = self.editor.project();
            let asset = project.assets.values().find(|asset| &asset.path == path)?;
            project.tracks.iter().find_map(|track| {
                let clip = track
                    .clips
                    .iter()
                    .find(|clip| clip.asset_id() == Some(asset.id))?;
                Some(((**track).clone(), clip.clone(), asset.clone()))
            })
        }

        fn labels(&self) -> Vec<String> {
            self.editor
                .history()
                .map(|entry| entry.label.clone())
                .collect()
        }
    }

    fn track(editor: &Editor, kind: TrackKind) -> TrackId {
        let project = editor.project();
        project.tracks.iter().find(|t| t.kind == kind).unwrap().id
    }

    #[test]
    fn an_empty_project_takes_a_five_second_take_at_zero_whole() {
        let mut desk = Desk::empty();
        desk.press().unwrap();
        assert_eq!(desk.recording.view(), RecordView::Starting);
        desk.started(0);
        desk.frame(16, 0, None);
        // Nothing to play: the playhead stays, and is the clock.
        assert!(!desk.playback.playing);
        desk.frame(16, 0, None);
        desk.frame(5_000, 5 * u64::from(RATE), None);
        assert_eq!(
            desk.recording.view(),
            RecordView::Recording {
                elapsed: seconds(5.0),
                level: 0.0
            }
        );
        desk.stop();
        assert!(desk.device.stopped.get());
        assert_eq!(desk.recording.view(), RecordView::Stopping);
        desk.finished(5 * u64::from(RATE), 0, None);
        desk.frame(16, 5 * u64::from(RATE), None);
        let (track, clip, asset) = desk.placed().expect("the take was not placed");
        assert_eq!(
            (clip.start, clip.source_in, clip.duration),
            (Time::ZERO, Time::ZERO, seconds(5.0))
        );
        // The free audio track the project has, not a new one.
        assert_eq!(track.name, "Audio 1");
        assert_eq!(asset.kind, MediaKind::Audio);
        assert!(asset.has_audio);
        assert_eq!(asset.duration, seconds(5.0));
        assert_eq!(asset.file_size, 44 + 5 * u64::from(RATE) * 4);
        assert_eq!(desk.labels(), [LABEL]);
        assert_eq!(
            desk.said,
            [("Recorded 5.0 s of voiceover".to_owned(), false)]
        );
        assert!(!desk.recording.busy());
        assert_eq!(desk.playback.position, Time::ZERO);
    }

    #[test]
    fn a_take_outlasts_the_project_and_the_playhead_holds_at_its_end() {
        let mut desk = Desk::lasting(2.0);
        desk.press().unwrap();
        desk.started(0);
        desk.frame(16, 0, None);
        assert!(
            desk.playback.playing,
            "the timeline plays when the device does"
        );
        desk.frame(16, 0, Some(Time::ZERO));
        // Ten seconds of frames; the sound device stops at the project's end.
        let mut frames = 0;
        for tenth in 1..=100 {
            frames += u64::from(RATE) / 10;
            let at = seconds(f64::from(tenth) / 10.0);
            let heard = desk.playback.playing.then_some(at.min(seconds(2.0)));
            desk.frame(100, frames, heard);
            if tenth == 30 {
                assert!(!desk.playback.playing, "the transport stopped at the end");
                assert!(
                    matches!(desk.recording.view(), RecordView::Recording { elapsed, .. } if elapsed == seconds(3.0)),
                    "the end of the project ended the take"
                );
            }
        }
        assert_eq!(desk.playback.position, seconds(2.0));
        desk.stop();
        desk.finished(frames, 0, None);
        desk.frame(16, frames, None);
        let (_, clip, _) = desk.placed().unwrap();
        assert_eq!(
            (clip.start, clip.source_in, clip.duration),
            (Time::ZERO, Time::ZERO, seconds(10.0))
        );
        assert_eq!(desk.playback.position, seconds(2.0));
    }

    #[test]
    fn the_device_clock_answering_after_300_ms_is_where_the_clip_starts() {
        let mut desk = Desk::lasting(10.0);
        desk.playback.seek(seconds(1.0));
        desk.press().unwrap();
        desk.started(0);
        desk.frame(0, 0, None);
        assert!(desk.playback.playing);
        // The wall clock moves the playhead while the device starts up.
        desk.frame(100, 4_800, None);
        desk.frame(100, 9_600, None);
        assert!(desk.playback.position > seconds(1.0));
        // Its first position: the playhead the sound began from.
        desk.frame(100, 14_400, Some(seconds(1.0)));
        assert_eq!(
            desk.recording.anchor(),
            Some(Anchor {
                frames: 14_400,
                at: seconds(1.0)
            })
        );
        desk.frame(100, 4 * u64::from(RATE), Some(seconds(4.7)));
        desk.stop();
        desk.finished(4 * u64::from(RATE), 0, None);
        desk.frame(16, 4 * u64::from(RATE), None);
        let (_, clip, asset) = desk.placed().unwrap();
        assert_eq!(clip.start, seconds(1.0));
        assert_eq!(clip.source_in, seconds(0.3));
        assert_eq!(clip.duration, seconds(3.7));
        // The pre-roll stays in the media, to be trimmed back in.
        assert_eq!(asset.duration, seconds(4.0));
    }

    #[test]
    fn without_a_device_position_in_a_second_the_transport_is_the_clock() {
        let mut desk = Desk::lasting(10.0);
        desk.press().unwrap();
        desk.started(0);
        desk.frame(0, 0, None);
        assert!(desk.playback.playing);
        let mut frames = 0;
        for _ in 0..9 {
            frames += 4_800;
            desk.frame(100, frames, None);
            assert_eq!(desk.recording.anchor(), None);
        }
        frames += 4_800;
        desk.frame(100, frames, None);
        let at = desk.playback.position;
        assert!(at > Time::ZERO);
        assert_eq!(desk.recording.anchor(), Some(Anchor { frames, at }));
        desk.frame(1_000, frames + u64::from(RATE), None);
        desk.stop();
        desk.finished(frames + u64::from(RATE), 0, None);
        desk.frame(16, frames + u64::from(RATE), None);
        let (_, clip, _) = desk.placed().unwrap();
        assert_eq!(
            (clip.start, clip.source_in, clip.duration),
            (at, seconds(1.0), seconds(1.0))
        );
    }

    #[test]
    fn an_import_in_flight_keeps_its_own_step_beside_the_take() {
        let mut desk = Desk::empty();
        desk.press().unwrap();
        desk.started(0);
        desk.frame(16, 0, None);
        desk.frame(16, 0, None);
        desk.frame(2_000, 2 * u64::from(RATE), None);
        desk.stop();
        // The import's answer is committed as `App::take_media` commits it,
        // before the take's file is closed.
        let other = Asset::new("/media/music.wav".into(), MediaKind::Audio, seconds(30.0));
        let other_id = other.id;
        desk.editor
            .dispatch(Command::Batch {
                label: "Import media".into(),
                commands: vec![Command::AddAssets {
                    assets: vec![other],
                }],
            })
            .unwrap();
        desk.finished(2 * u64::from(RATE), 0, None);
        desk.frame(16, 2 * u64::from(RATE), None);
        assert_eq!(desk.labels(), ["Import media", LABEL]);
        assert_eq!(desk.editor.project().assets.len(), 2);
        // Each is its own step.
        assert!(desk.editor.undo());
        assert!(desk.placed().is_none());
        assert!(desk.editor.project().assets.contains_key(&other_id));
        assert!(desk.editor.undo());
        assert!(desk.editor.project().assets.is_empty());
    }

    #[test]
    fn a_track_filled_or_removed_during_the_take_gives_a_new_track() {
        for removed in [false, true] {
            let mut desk = Desk::empty();
            let audio = track(&desk.editor, TrackKind::Audio);
            desk.press().unwrap();
            desk.started(0);
            desk.frame(16, 0, None);
            desk.frame(16, 0, None);
            let command = if removed {
                Command::RemoveTrack { track_id: audio }
            } else {
                let music = Asset::new("/media/music.wav".into(), MediaKind::Audio, seconds(9.0));
                let clip = Clip::from_asset(&music, Time::ZERO);
                desk.editor
                    .dispatch(Command::AddAssets {
                        assets: vec![music],
                    })
                    .unwrap();
                Command::AddClip {
                    track_id: audio,
                    clip,
                }
            };
            desk.editor.dispatch(command).unwrap();
            desk.frame(3_000, 3 * u64::from(RATE), None);
            desk.stop();
            desk.finished(3 * u64::from(RATE), 0, None);
            desk.frame(16, 3 * u64::from(RATE), None);
            let (track, clip, _) = desk.placed().expect("the take was not placed");
            assert_eq!((track.name.as_str(), track.kind), (TRACK, TrackKind::Audio));
            assert_ne!(track.id, audio);
            assert_eq!(clip.duration, seconds(3.0));
            // One step makes the track and places the clip.
            assert!(desk.editor.undo());
            assert!(
                !(desk.editor.project().tracks.iter()).any(|track| track.name == TRACK),
                "removed {removed}: undo left the track"
            );
        }
    }

    #[test]
    fn a_locked_audio_track_is_passed_over() {
        let mut desk = Desk::empty();
        let audio = track(&desk.editor, TrackKind::Audio);
        desk.editor
            .dispatch(Command::SetTrackState {
                track_id: audio,
                locked: Some(true),
                hidden: None,
                muted: None,
                name: None,
                gain: None,
                solo: None,
            })
            .unwrap();
        desk.press().unwrap();
        desk.started(0);
        desk.frame(16, 0, None);
        desk.frame(16, 0, None);
        desk.stop();
        desk.finished(u64::from(RATE), 0, None);
        desk.frame(16, u64::from(RATE), None);
        assert_eq!(desk.placed().unwrap().0.name, TRACK);
    }

    #[test]
    fn a_project_replaced_during_the_take_adds_nothing_and_says_where_it_is() {
        let mut desk = Desk::empty();
        desk.press().unwrap();
        desk.started(0);
        desk.frame(16, 0, None);
        desk.frame(16, 0, None);
        desk.frame(1_000, u64::from(RATE), None);
        // Another project is opened, as `App::replace_editor` does it.
        desk.editor = Editor::new(Project::new("Other")).unwrap();
        desk.playback = Playback::default();
        desk.recording.project_replaced();
        assert!(
            desk.device.stopped.get(),
            "the take went on in another project"
        );
        desk.finished(u64::from(RATE), 0, None);
        desk.frame(16, u64::from(RATE), None);
        assert!(desk.editor.project().assets.is_empty());
        assert_eq!(desk.labels(), Vec::<String>::new());
        let path = desk.path.clone().unwrap();
        assert!(path.exists());
        let [(said, true)] = desk.said.as_slice() else {
            panic!("said {:?}", desk.said);
        };
        assert!(said.contains(&path.display().to_string()), "{said}");
        assert!(!desk.recording.busy());
    }

    #[test]
    fn an_untitled_project_is_refused_before_any_device_is_opened() {
        let mut desk = Desk::empty();
        let instance = desk.editor.instance_id();
        assert_eq!(
            desk.recording.ask(instance, None, &mut desk.playback),
            Err(UNTITLED.to_owned())
        );
        assert!(!desk.recording.busy());
        assert_eq!(desk.recording.view(), RecordView::Idle);
        // Whatever answers later opens nothing.
        let path = desk.dir.join("stray.wav");
        desk.answer(instance, Ok(path)).unwrap();
        assert_eq!(desk.opened, 0);
    }

    #[test]
    fn a_path_answered_for_a_replaced_project_opens_no_device() {
        let mut desk = Desk::empty();
        let first = desk.editor.instance_id();
        desk.recording
            .ask(first, Some(&desk.document), &mut desk.playback)
            .unwrap();
        desk.recording.project_replaced();
        desk.answer(first, Ok(desk.dir.join("a.wav"))).unwrap();
        assert_eq!(desk.opened, 0);
        assert!(!desk.recording.busy());
        // Nor does one for the old project once the new one asks.
        desk.editor = Editor::new(Project::new("Other")).unwrap();
        let second = desk.editor.instance_id();
        desk.recording
            .ask(second, Some(&desk.document), &mut desk.playback)
            .unwrap();
        desk.answer(first, Ok(desk.dir.join("b.wav"))).unwrap();
        assert_eq!(desk.opened, 0);
        desk.answer(second, Ok(desk.dir.join("c.wav"))).unwrap();
        assert_eq!(desk.opened, 1);
        // A path that could not be made is said, and nothing records.
        let mut desk = Desk::empty();
        let instance = desk.editor.instance_id();
        desk.recording
            .ask(instance, Some(&desk.document), &mut desk.playback)
            .unwrap();
        let refused = desk.answer(
            instance,
            Err("Recordings go in X, which is not a folder".into()),
        );
        assert_eq!(
            refused,
            Err("Cannot record: Recordings go in X, which is not a folder".to_owned())
        );
        assert_eq!(desk.opened, 0);
        assert!(!desk.recording.busy());
    }

    #[test]
    fn a_second_take_is_refused_while_one_is_under_way() {
        let mut desk = Desk::empty();
        desk.press().unwrap();
        assert_eq!(desk.press(), Err(BUSY.to_owned()));
        assert_eq!(desk.opened, 1);
    }

    #[test]
    fn undo_takes_the_take_out_of_the_project_and_leaves_its_file() {
        let mut desk = Desk::empty();
        desk.press().unwrap();
        desk.started(0);
        desk.frame(16, 0, None);
        desk.frame(16, 0, None);
        desk.stop();
        desk.finished(2 * u64::from(RATE), 0, None);
        desk.frame(16, 2 * u64::from(RATE), None);
        let path = desk.path.clone().unwrap();
        // Written beside the document, in its media folder.
        assert_eq!(path.parent(), Some(desk.dir.join("Film media").as_path()));
        assert!(desk.placed().is_some());
        assert!(desk.editor.undo());
        assert!(desk.placed().is_none());
        assert!(desk.editor.project().assets.is_empty());
        assert!(path.exists(), "undo removed the take's file");
        assert!(desk.editor.redo());
        assert!(desk.placed().is_some());
    }

    #[test]
    fn a_placed_take_is_saved_and_read_back_as_placed() {
        let mut desk = Desk::lasting(10.0);
        desk.press().unwrap();
        desk.started(0);
        desk.frame(0, 0, None);
        desk.frame(100, 4_800, Some(seconds(0.5)));
        desk.stop();
        desk.finished(3 * u64::from(RATE), 0, None);
        desk.frame(16, 3 * u64::from(RATE), None);
        let (_, clip, asset) = desk.placed().unwrap();
        editor_store::save_project(&desk.document, desk.editor.project()).unwrap();
        let loaded = editor_store::load_project(&desk.document).unwrap();
        assert_eq!(loaded.assets.get(&asset.id), Some(&asset));
        assert_eq!(
            loaded.clip(clip.id).map(|(_, read)| read.clone()),
            Some(clip)
        );
    }

    #[test]
    fn a_short_take_is_kept_and_not_placed_and_lost_frames_are_said() {
        let mut desk = Desk::empty();
        desk.press().unwrap();
        desk.started(0);
        desk.frame(16, 0, None);
        desk.frame(16, 0, None);
        desk.stop();
        // 83 ms, of which 50 were lost to an overrun.
        desk.finished(4_000, 2_400, None);
        desk.frame(16, 4_000, None);
        assert!(desk.placed().is_none());
        assert!(desk.labels().is_empty());
        let path = desk.path.clone().unwrap();
        assert!(path.exists());
        let [(said, true)] = desk.said.as_slice() else {
            panic!("said {:?}", desk.said);
        };
        assert!(
            said.starts_with("The take was too short to place"),
            "{said}"
        );
        assert!(said.contains(&path.display().to_string()), "{said}");
        assert!(
            said.ends_with(
                ". Less than a tenth of a second of sound was lost: the recorder fell behind"
            ),
            "{said}"
        );
    }

    #[test]
    fn lost_sound_is_said_to_the_tenth_of_a_second() {
        let project = Project::new("Film");
        let lost = |dropped: u64| {
            let take = Take {
                path: "/takes/Take.wav".into(),
                frames: 10 * u64::from(RATE),
                rate: RATE,
                channels: 1,
                bytes: 44,
                dropped,
                ended: None,
            };
            let anchor = Anchor {
                frames: 0,
                at: Time::ZERO,
            };
            land(&project, false, &take, anchor).said
        };
        let said = |amount: &str| {
            format!(
                "The take is kept in /takes/Take.wav: the project it was recorded for was \
                 closed, so nothing was added. {amount} of sound was lost: the recorder fell behind"
            )
        };
        // 1 ms is not rounded up to a second, nor 1.5 s to two.
        assert_eq!(lost(48), said("Less than a tenth of a second"));
        assert_eq!(lost(4_800), said("0.1 s"));
        assert_eq!(lost(72_000), said("1.5 s"));
        assert_eq!(lost(90 * u64::from(RATE)), said("1:30"));
    }

    #[test]
    fn record_pressed_while_playing_holds_the_playhead_until_the_take_runs() {
        let mut desk = Desk::lasting(10.0);
        desk.playback.set_speed(2.0).unwrap();
        desk.playback.play();
        desk.frame(16, 0, None);
        desk.frame(500, 0, None);
        let pressed_at = desk.playback.position;
        assert!(pressed_at > Time::ZERO);
        desk.press().unwrap();
        assert!(
            !desk.playback.playing,
            "the timeline played on while starting"
        );
        // The microphone takes its time to open; the playhead holds.
        desk.frame(500, 0, None);
        assert_eq!(desk.playback.position, pressed_at);
        desk.started(0);
        desk.frame(16, 0, None);
        assert!(desk.playback.playing);
        assert_eq!(desk.playback.speed, 1.0);
        assert_eq!(desk.playback.position, pressed_at);

        // Ended before the device answered, nothing plays and nothing lands.
        let mut desk = Desk::lasting(10.0);
        desk.playback.set_speed(2.0).unwrap();
        desk.playback.play();
        desk.press().unwrap();
        desk.stop();
        assert!(!desk.playback.playing);
        (desk.device.said.borrow_mut()).push_back(RecordEvent::Failed(MediaError::Cancelled));
        desk.frame(16, 0, None);
        assert!(!desk.playback.playing);
        assert!(!desk.recording.busy());
        assert!(desk.placed().is_none());
        assert!(desk.said.is_empty(), "{:?}", desk.said);
    }

    #[test]
    fn a_take_stopped_in_any_phase_stops_the_transport_under_it() {
        let mut desk = Desk::lasting(10.0);
        let instance = desk.editor.instance_id();
        let document = desk.document.clone();
        // Whatever else set it playing while the path was made.
        desk.recording
            .ask(instance, Some(&document), &mut desk.playback)
            .unwrap();
        desk.playback.play();
        assert!(desk.recording.stop(&mut desk.playback));
        assert!(!desk.playback.playing);
        assert!(!desk.recording.busy());
    }

    #[test]
    fn a_take_that_ends_by_itself_stops_the_transport_and_says_why() {
        let mut desk = Desk::lasting(30.0);
        desk.press().unwrap();
        desk.started(0);
        desk.frame(0, 0, None);
        desk.frame(16, 0, Some(Time::ZERO));
        desk.frame(2_000, 2 * u64::from(RATE), Some(seconds(2.0)));
        assert!(desk.playback.playing);
        desk.finished(
            2 * u64::from(RATE),
            0,
            Some("Cannot write the take: disk full"),
        );
        desk.frame(16, 2 * u64::from(RATE), Some(seconds(2.0)));
        assert!(
            !desk.playback.playing,
            "the transport played on after the take"
        );
        assert!(desk.placed().is_some(), "what was written is placed");
        let [(said, true)] = desk.said.as_slice() else {
            panic!("said {:?}", desk.said);
        };
        assert!(said.ends_with("Cannot write the take: disk full"), "{said}");
    }

    #[test]
    fn no_microphone_is_said_and_nothing_plays() {
        let mut desk = Desk::lasting(10.0);
        desk.press().unwrap();
        (desk.device.said.borrow_mut()).push_back(RecordEvent::Failed(MediaError::NoMicrophone));
        desk.frame(16, 0, None);
        assert_eq!(desk.said, [("No microphone".to_owned(), true)]);
        assert!(!desk.playback.playing);
        assert!(!desk.recording.busy());
        assert_eq!(desk.recording.view(), RecordView::Idle);
    }

    #[test]
    fn a_take_given_up_while_starting_says_nothing() {
        let mut desk = Desk::lasting(10.0);
        desk.press().unwrap();
        desk.stop();
        assert!(desk.device.stopped.get());
        (desk.device.said.borrow_mut()).push_back(RecordEvent::Failed(MediaError::Cancelled));
        desk.frame(16, 0, None);
        assert!(desk.said.is_empty(), "{:?}", desk.said);
        assert!(!desk.playback.playing);
        assert!(!desk.recording.busy());
        // Given up before its path came, nothing opens at all.
        let mut desk = Desk::empty();
        let instance = desk.editor.instance_id();
        desk.recording
            .ask(instance, Some(&desk.document), &mut desk.playback)
            .unwrap();
        desk.stop();
        desk.answer(instance, Ok(desk.dir.join("late.wav")))
            .unwrap();
        assert_eq!(desk.opened, 0);
    }

    #[test]
    fn no_repaint_is_asked_for_once_the_take_is_placed() {
        let mut desk = Desk::empty();
        assert_eq!(desk.recording.repaint_after(), None);
        desk.press().unwrap();
        // Starting and closing are woken by the service itself.
        assert_eq!(desk.recording.repaint_after(), None);
        desk.started(0);
        desk.frame(16, 0, None);
        assert_eq!(desk.recording.repaint_after(), Some(REPAINT));
        desk.frame(16, 0, None);
        desk.stop();
        assert_eq!(desk.recording.repaint_after(), None);
        desk.finished(u64::from(RATE), 0, None);
        desk.frame(16, u64::from(RATE), None);
        assert!(desk.placed().is_some());
        assert_eq!(desk.recording.repaint_after(), None);
        assert_eq!(desk.recording.view(), RecordView::Idle);
    }

    #[test]
    fn the_control_reads_whole_seconds_and_the_level_in_decibels() {
        assert_eq!(elapsed_text(seconds(0.99)), "0:00");
        assert_eq!(elapsed_text(seconds(61.5)), "1:01");
        assert_eq!(level_fill(0.0), 0.0);
        assert_eq!(level_fill(f32::NAN), 0.0);
        assert_eq!(level_fill(0.001), 0.0);
        assert_eq!(level_fill(1.0), 1.0);
        assert!((level_fill(0.1) - 2.0 / 3.0).abs() < 1e-4);
    }
}
