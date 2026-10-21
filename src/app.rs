//! The application: one frame loop that joins the document, its workers and
//! the panels.
//!
//! `logic` takes what the workers finished (never waiting on one), advances
//! the playback clock and decides when the next frame is due. `ui` draws the
//! panels from the document and runs the actions they emitted. A paused,
//! settled window schedules nothing and sleeps until an event or a worker
//! wakes it.
use crate::{
    agent,
    ai::{self, Planner},
    clipboard,
    duck::{self, Ducking, Stepped},
    extract::{self, Saving},
    file_drop::FileDrop,
    icons::Icon,
    normalize::{self, Normalizing},
    pacing,
    peaks::HeldPeaks,
    presets::{self, Presets},
    record::Recording,
    rules,
    settings::{self, INTERFACE_SCALE, Scale, Settings},
    speech::{self, Inventory, ModelChoice, SpeechRequest, SpeechView},
    speed, stems,
    textures::{self, Textures, Viewing},
    theme::{self, Appearance, Palette, metrics},
    titles::Titles,
    ui::{
        self, Action, ClipDraft, DraftHold, PresetChange, Selection, TrackProgress, TrackRequest,
        Transport,
        agent::{
            AgentInput, AgentState, Entry, PlanCard, PlanState, Step, StepState, WaitingInput,
        },
        captions_tab,
        inspector::{InspectorInput, InspectorState, ProjectDraft},
        kit,
        library::{LibraryInput, LibraryState},
        sheets::{
            self, Answer, Checkpoint, CheckpointAnswer, CheckpointsState, Conflict, ConflictAnswer,
            DuckQuestion, Notice, Overlay, Pending,
        },
        text_tab,
        timeline::{self, Reach, TimelineInput, TimelineState},
        toolbar::ToolbarInput,
        transitions,
        viewer::{SourceInput, ViewerInput, ViewerState},
        widgets,
    },
};
use editor_agent::{AgentHost, AgentOperation};
use editor_core::{
    Asset, AssetId, Clip, ClipId, Clipboard, Command, Editor, EffectKind, MediaKind, ParamKey,
    Playback, Project, ProjectQuery, Time, Track, Tracker,
    captions::{Cue, SubtitleFormat},
    look,
};
use editor_media::{
    AnalysisEvent, AnalysisService, AudioService, MediaEvent, MediaService, PeakCache,
    RecordService, Region, SoundEvent, SoundService, SourceIdentity, SteadyCache, SteadyEvent,
    SteadyJob, TrackJob,
    speech::{DETECTOR, ModelShape, SpeechEvent, SpeechJob, SpeechService, Transcription},
    text::TextService,
};
use editor_store::{DocumentSession, RecoveredSession, RecoveryLease, SaveWorker, StoreError};
use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, Rect, pos2, vec2};
use std::{
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    time::{Duration, Instant, SystemTime},
};

const RECOVERY_INTERVAL: Duration = Duration::from_secs(15);
/// How long a closing window waits for the save worker.
const EXIT_WAIT: Duration = Duration::from_secs(2);
/// How long it waits for a recovery lease a worker was asked for.
const LEASE_WAIT: Duration = Duration::from_secs(1);
/// How long information stays in its toast, unless the pointer holds it.
const NOTICE_SECONDS: f64 = 4.0;
/// A problem stays until it is dismissed.
const PROBLEM_SECONDS: f64 = f64::INFINITY;
const PROJECT_EXTENSION: &str = "nvp";
/// The narrowest content the library is shown beside the right panel in:
/// a window of 820, Terminal's narrowest window with its sidebar.
const LIBRARY_FITS: f32 = metrics::SIDEBAR_MIN_WINDOW - 2.0 * metrics::GUTTER;
/// The narrowest a side panel is drawn, however little room the window has:
/// the least it may be preferred at.
const LIBRARY_LEAST: f32 = *settings::LIBRARY_WIDTHS.start();
const INSPECTOR_LEAST: f32 = *settings::INSPECTOR_WIDTHS.start();
/// The viewer the side panels shrink to keep, and the least it is left with
/// once they are as narrow as they go.
const VIEWER_COMFORT: f32 = 480.0;
const VIEWER_LEAST: f32 = 360.0;
const RECOVERY_SUFFIX: &str = ".recovery";

/// When the recovery copy is written. What was asked for and what is known
/// to be on disk are kept apart: a write that failed is asked for again after
/// the interval, whether or not anything was edited since.
struct Autosave {
    /// When a copy was last asked for, or last failed.
    at: Instant,
    /// The revision of the newest copy known to be written.
    kept: u64,
    /// A write was handed to the worker and has not been answered.
    pending: bool,
}

impl Autosave {
    fn new(revision: u64, now: Instant) -> Self {
        Self {
            at: now,
            kept: revision,
            pending: false,
        }
    }

    fn due(&self, revision: u64, now: Instant) -> bool {
        !self.pending
            && revision != self.kept
            && now.saturating_duration_since(self.at) >= RECOVERY_INTERVAL
    }

    fn submitted(&mut self, now: Instant) {
        self.at = now;
        self.pending = true;
    }

    /// The worker answered for the copy taken at `revision`.
    fn answered(&mut self, revision: u64, written: bool, now: Instant) {
        self.pending = false;
        if written {
            self.kept = revision;
        } else {
            self.at = now;
        }
    }

    /// The copy could not even be asked for.
    fn failed(&mut self, now: Instant) {
        self.pending = false;
        self.at = now;
    }
}

/// The source monitor: one library asset played from its file in the
/// viewer's place. Its document is made for the purpose and thrown away; the
/// project, its history and the timeline's playhead are not touched.
struct Source {
    asset: Asset,
    project: Arc<Project>,
    playback: Playback,
    looping: bool,
}

impl Source {
    fn open(asset: &Asset) -> Result<Self, String> {
        let project = Project::asset_preview(asset).map_err(|error| error.to_string())?;
        Ok(Self {
            asset: asset.clone(),
            project: Arc::new(project),
            playback: Playback::default(),
            looping: false,
        })
    }

    fn duration(&self) -> Time {
        self.asset.duration.max(Time::ZERO)
    }

    /// Whether there is anything to hear, and so a device clock to follow.
    fn audible(&self) -> bool {
        self.asset.has_audio || self.asset.kind == MediaKind::Audio
    }

    /// Applies one transport request, kept inside the source. `true` when
    /// the sound has to be brought in line.
    fn apply(&mut self, transport: Transport) -> Result<bool, String> {
        let rate = self.project.settings.frame_rate;
        let end = self.duration();
        let last = ui::viewer::source_frame_time(&self.asset, rate, end);
        let was_playing = self.playback.playing;
        match transport {
            Transport::TogglePlay => {
                if self.asset.kind == MediaKind::Image {
                    return Err("A still image has nothing to play.".to_owned());
                }
                if was_playing {
                    self.playback.pause();
                } else {
                    if self.playback.position >= last {
                        self.playback.seek(Time::ZERO);
                    }
                    self.playback.play();
                }
                Ok(true)
            }
            Transport::Seek(time) => {
                self.playback.seek(time.min(end));
                Ok(was_playing)
            }
            Transport::Step(frames) => {
                self.playback.step(frames, rate);
                self.playback.seek(self.playback.position.min(last));
                Ok(was_playing)
            }
            Transport::JumpStart => {
                self.playback.seek(Time::ZERO);
                Ok(was_playing)
            }
            Transport::JumpEnd => {
                self.playback.pause();
                self.playback.seek(last);
                Ok(was_playing)
            }
            Transport::SetSpeed(speed) => {
                self.playback
                    .set_speed(speed)
                    .map_err(|error| error.to_string())?;
                Ok(was_playing)
            }
            Transport::ToggleLoop => {
                self.looping = !self.looping;
                Ok(false)
            }
        }
    }

    /// Advances the clock. `true` when the source stopped or started over
    /// by itself and the sound has to follow.
    fn tick(&mut self, now: Instant, heard: Option<Time>) -> bool {
        if !self.playback.playing {
            return false;
        }
        let duration = self.duration();
        self.playback.tick_with_audio(now, duration, heard);
        if self.playback.playing {
            return false;
        }
        if self.looping && duration > Time::ZERO {
            self.playback.seek(Time::ZERO);
            self.playback.play();
        }
        true
    }
}

/// A recovery copy that can be restored: the path it was written for, and
/// the project file it belongs to when there is one.
#[derive(Clone)]
struct Offer {
    base: PathBuf,
    project: Option<PathBuf>,
}

/// What the command line asked for at launch.
pub struct Launch {
    pub project: Option<PathBuf>,
    pub import: Vec<PathBuf>,
    pub agent_session: Option<PathBuf>,
    pub settings: Option<PathBuf>,
    /// Capture the window to this file after a few seconds and exit.
    pub screenshot: Option<PathBuf>,
    pub screenshot_delay: f64,
}

impl Default for Launch {
    fn default() -> Self {
        Self {
            project: None,
            import: Vec::new(),
            agent_session: None,
            settings: None,
            screenshot: None,
            screenshot_delay: 2.5,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RightTab {
    Inspector,
    Agent,
}

enum Pick {
    Open,
    SaveAs,
    Duplicate,
    ImportFiles,
    ImportFolder,
    ImportFonts,
    ImportCaptions,
    ExportCaptions(editor_core::TrackId, SubtitleFormat),
    SpeechModel,
    /// The folder the export sheet writes into.
    ExportFolder,
    /// A folder the agent may use.
    AgentFolder,
}

/// A subtitle file's cues and the language it names, or why it was not
/// read.
type SubtitleRead = Result<(Vec<Cue>, Option<String>), String>;

/// Results that workers started by the application hand back.
enum Job {
    Picked(Pick, Vec<PathBuf>),
    /// What is in the models folder.
    Models(Inventory),
    /// A model file chosen by hand was read, or why it was not.
    ModelChecked {
        path: PathBuf,
        result: Result<ModelShape, String>,
    },
    Loaded {
        epoch: u64,
        /// The document's revision when the load began.
        revision: u64,
        path: PathBuf,
        /// The project with the session that remembers the bytes it was
        /// read from.
        result: Result<(Project, Arc<DocumentSession>), String>,
        /// A recovery copy sits beside the project.
        recovery: bool,
    },
    /// The destination of a Save As was looked at, and what was there noted.
    Prepared {
        epoch: u64,
        context: uuid::Uuid,
        path: PathBuf,
        /// What the document was when the destination was chosen.
        origin: Origin,
        result: Result<Arc<DocumentSession>, String>,
    },
    Recovered {
        epoch: u64,
        revision: u64,
        offer: Offer,
        /// The copy, with the file it was written beside as that file was
        /// when the copy was made, and the ownership taken to read it.
        result: Result<(RecoveredSession, RecoveryLease), String>,
    },
    /// Ownership of the recovery copy written for `base` was asked for.
    Leased {
        base: PathBuf,
        /// Typed: only a copy another window owns sends this one elsewhere.
        result: Result<RecoveryLease, StoreError>,
    },
    /// The newest recovery copy of a project that never had a file.
    Orphan(PathBuf),
    /// The checkpoints beside `path`, after one was written when asked.
    Checkpoints {
        epoch: u64,
        path: PathBuf,
        saved: Option<Result<String, String>>,
        list: Result<Vec<Checkpoint>, String>,
    },
    CheckpointLoaded {
        epoch: u64,
        label: String,
        result: Result<Project, String>,
    },
    /// A copy of the project under an identity of its own was written.
    Duplicated {
        epoch: u64,
        context: uuid::Uuid,
        /// The document's revision when the copy was taken.
        revision: u64,
        project: Box<Project>,
        /// The session of the file that was made for the copy.
        result: Result<Arc<DocumentSession>, String>,
    },
    /// Where the sound of a media file, or of a clip's span of it, is to
    /// be written: `None` when the dialog was left.
    AudioPicked {
        asked: Box<extract::Asked>,
        to: Option<Result<PathBuf, String>>,
    },
    /// The path of a voiceover take asked for in the editor instance
    /// `project` was made beside its document, or not.
    TakePath {
        project: uuid::Uuid,
        result: Result<PathBuf, String>,
    },
    /// Subtitle files were read, each whole or with why it was not.
    CaptionsRead {
        epoch: u64,
        files: Vec<(PathBuf, SubtitleRead)>,
    },
    Background(Result<String, String>),
}

pub struct App {
    editor: Editor,
    playback: Playback,
    looping: bool,
    media: Option<MediaService>,
    media_problem: Option<String>,
    audio: Option<AudioService>,
    saver: SaveWorker,
    host: Option<AgentHost>,
    session_path: Option<PathBuf>,
    session_problem: Option<String>,

    path: Option<PathBuf>,
    /// The project's file as this window last read or wrote it. Every save
    /// goes through it, so a file that something else wrote since is not
    /// replaced. It is renewed only by opening a file or saving to another;
    /// a checkpoint restore, an undo and a refused save all keep it.
    document: Option<Arc<DocumentSession>>,
    /// Whether the document is restored work that came back with no file.
    /// Such work is saved only to a file that is not there yet. Set by the
    /// restore alone; a new or opened project and a save that landed end it.
    origin: Origin,
    /// Counts replaced projects, so that a late answer about an earlier one
    /// is recognised and dropped.
    epoch: u64,
    saving: bool,
    /// The session the save in flight writes through, and the revision the
    /// newest such save was asked for at.
    save_session: Option<(Arc<DocumentSession>, u64)>,
    /// The destination of a Save As is being looked at.
    preparing: bool,
    /// The last save was refused because the open file changed on disk.
    conflicted: bool,
    /// The save in flight was asked for by a connected agent: a conflict
    /// is told in a notice, not a sheet over the person's work.
    agent_saving: bool,
    /// Counts finished saves, and how the last one ended, for `document`.
    save_count: u64,
    last_save: Option<editor_agent::SaveOutcome>,
    /// The model the agent panel asks.
    ai: Planner,
    /// The agent that edits this window's project as it works.
    live: ai::Agent,
    /// What this window's agent begins its request ids with.
    agent_tag: String,
    /// What the agent's folders were last worked out from.
    agent_roots_basis: Option<(u64, Option<PathBuf>, Vec<PathBuf>)>,
    /// The document's revision when the open dialog was shown.
    open_revision: u64,
    recovery_dir: Option<PathBuf>,
    /// Where a project without a file keeps its recovery copy, when that is
    /// not the place derived from the editor itself.
    unsaved_base: Option<PathBuf>,
    /// Ownership of the recovery copy this window writes, and the path it is
    /// for. Another window does not offer or replace a copy that is owned.
    lease: Option<(PathBuf, Arc<RecoveryLease>)>,
    leasing: bool,
    /// The copy beside the project's file is another window's, so this
    /// window keeps its own under the settings folder. Where the project is
    /// saved is not changed by it.
    private_recovery: bool,
    /// Remove the recovery copy once the write in flight has landed.
    recovery_stale: bool,
    /// Recovery copies of work that was given up; a late write is removed.
    discarded: Vec<(uuid::Uuid, PathBuf)>,
    discard_on_exit: bool,
    after_save: Option<Pending>,
    autosave: Autosave,
    /// A project whose recovery copy is being offered.
    recover_offer: Option<Offer>,

    settings: Settings,
    settings_saved: Settings,
    settings_path: Option<PathBuf>,
    /// The person's kept looks and brand colours, and as last written.
    presets: Arc<Presets>,
    presets_saved: Arc<Presets>,
    presets_path: Option<PathBuf>,
    /// The file there could not be read: it is moved aside before the
    /// first write, never written over.
    presets_unread: bool,
    palette: Palette,
    textures: Textures,
    query: Option<ProjectQuery>,
    /// The library the textures and the media worker were last in line with.
    assets_seen: Arc<BTreeMap<AssetId, Asset>>,
    source: Option<Source>,
    /// Said at the next frame: found where no notice could be shown.
    problems: Vec<String>,

    selection: Selection,
    draft: Option<ClipDraft>,
    project_draft: ProjectDraft,
    draft_hold: DraftHold,
    clipboard: Option<Clipboard>,
    timeline: TimelineState,
    viewer: ViewerState,
    inspector: InspectorState,
    library: LibraryState,
    agent: AgentState,
    right_tab: RightTab,
    /// In a window too narrow for both side panels, the library was the one
    /// asked for last and takes the inspector's place.
    library_first: bool,
    overlay: Overlay,
    checkpoints: CheckpointsState,
    /// A duplicate is being written; one at a time.
    duplicating: bool,
    notices: Vec<Notice>,
    /// Import batches the media worker has not reported yet, oldest first.
    imports: VecDeque<Vec<PathBuf>>,
    /// The analysis worker, started when a region is first tracked or a
    /// camera path first asked about.
    analysis: Option<AnalysisService>,
    /// Where camera paths are kept between sessions.
    steady_cache: SteadyCache,
    /// The smaller copies of videos the viewer reads in their place.
    proxies: crate::proxies::Proxies,
    /// The sound lane, started when a tile first wants its media's sound.
    sound: Option<SoundService>,
    /// Where peaks are kept between sessions.
    peak_cache: PeakCache,
    /// The peaks held for the waveforms in view.
    peaks: HeldPeaks,
    /// The clips waiting for their loudness, to be normalized.
    normalizing: Normalizing,
    /// The duck under way, waiting for the other clips' peaks.
    ducking: Ducking,
    /// The files the sound lane is writing, and those written and offered
    /// to the project. A replaced project leaves them writing.
    saved_audio: Saving,
    /// The separations the sound lane is running. A replaced project leaves
    /// them running; their stems are then only said.
    separating: stems::Separating,
    /// Whether this session's probe for a stem separator was started, and
    /// whether its answer was taken.
    separator_asked: bool,
    separator_taken: bool,
    /// The voiceover take under way: one at a time. A replaced project
    /// stops it and keeps its file.
    recording: Recording,
    /// The region being tracked there: one at a time.
    tracking: Option<Tracking>,
    /// The speech worker, started when captions are first made from speech.
    speech: Option<SpeechService>,
    /// The captions being made: one request at a time.
    speaking: Option<Speaking>,
    /// Captions that came and wait for the person.
    heard: Option<Heard>,
    /// Where downloaded models are kept, and what a worker last saw there.
    models_dir: Option<PathBuf>,
    models: Option<Inventory>,
    /// What the media worker published besides frames while the frames of a
    /// frame's end were taken, oldest first: the start of the next frame
    /// takes these before anything it publishes after them.
    media_held: VecDeque<MediaEvent>,
    file_drop: FileDrop,
    /// Files from another application are held over the window.
    files_held: bool,
    /// Media named at launch beside a project, imported once that project
    /// is open: an import begun earlier would be cancelled with the blank
    /// document it was replacing.
    import_after_load: Vec<PathBuf>,

    jobs: mpsc::Receiver<Job>,
    job_sender: mpsc::Sender<Job>,
    dialog: rfd::AsyncFileDialog,
    exit_approved: bool,
    screenshot: Option<(PathBuf, Instant, bool)>,

    /// Exports waiting, rendering and finished, one at a time on a thread
    /// of their own.
    renders: crate::export::queue::RenderQueue,
    /// The export sheet's state while it is open, or while a question is
    /// asked over it, and as it was left when closed unfinished: opened
    /// again for the same project, it is shown as it was.
    export: Option<crate::ui::export_sheet::ExportSheetState>,
    /// The export sheet's picture of the output at the playhead, drawn on a
    /// thread of its own while the sheet is open.
    poster: crate::export::poster::PosterService,
    /// An export that waits for the answer to "Replace …?": its settings,
    /// destination, whether it starts now, and the name of a second file
    /// it replaces (its caption file, when the destination is there too).
    export_replace: Option<(
        editor_core::export::ExportSettings,
        PathBuf,
        bool,
        Option<String>,
    )>,
    /// Why the render queue refused what was last asked of it from its own
    /// sheet, said in the sheet's footer while it stays open.
    queue_refusal: Option<String>,
    /// The person's saved export presets and the file they are kept in.
    export_presets: crate::export::presets::ExportPresets,
    export_presets_path: Option<PathBuf>,
    /// The newest write of the presets asked for, and the writes made: a
    /// write that lands after a newer one leaves the file alone, and a file
    /// that could not be read is moved aside before the first write that
    /// succeeds.
    export_presets_asked: u64,
    export_presets_written: Arc<std::sync::Mutex<crate::export::presets::Writes>>,
    /// Stopping the exports was agreed to for the quit being asked about.
    stop_renders: bool,
}

/// A region the analysis worker is following.
#[derive(Clone, Debug, PartialEq)]
struct Tracking {
    request: u64,
    /// The clip the region was drawn on and the media it shows.
    clip: ClipId,
    asset: AssetId,
    /// The tracker the path becomes, and its name.
    tracker: uuid::Uuid,
    name: String,
    done: f32,
    /// The path, once it has come and was not added by itself.
    ready: Option<Ready>,
}

/// Captions being made from the timeline's sound.
struct Speaking {
    request: u64,
    ask: SpeechRequest,
    /// The editor and its revision when the timeline was taken to be
    /// heard; `None` while the model downloads.
    heard_at: Option<(uuid::Uuid, u64)>,
    /// A cancel was asked for.
    stopping: bool,
}

/// Captions that came while adding them would have cost something, and
/// wait for the person to add or give them up.
struct Heard {
    command: Command,
    track: editor_core::TrackId,
    count: usize,
    language: String,
    why: &'static str,
}

/// Why captions that came are not added by themselves, if they are not:
/// the timeline they were heard from has changed, or adding them gives up
/// what was undone or makes a plan under review out of date.
fn heard_waits(changed: bool, can_redo: bool, plan_open: bool) -> Option<&'static str> {
    if changed {
        Some("The timeline changed while they were made, so some may be off.")
    } else if can_redo {
        Some("Adding them gives up what was undone. Redo first to keep both.")
    } else if plan_open {
        Some(
            "Adding them puts the plan under review out of date. Settle the plan first to keep it.",
        )
    } else {
        None
    }
}

/// A tracked path that waits to be added: adding it is an edit nobody
/// pressed for, and it came while such an edit would have taken something
/// from the person.
#[derive(Clone, Debug, PartialEq)]
struct Ready {
    tracker: Tracker,
    /// What is said of it once it is added.
    said: String,
}

/// Why a tracked path that has come is not added by itself, if it is not:
/// an edit gives up what was undone, and makes a plan under review one for
/// a project that is no longer there. The words are the Motion section's.
fn tracker_waits(can_redo: bool, plan_open: bool) -> Option<&'static str> {
    if can_redo {
        Some("Adding it gives up what was undone. Redo first to keep both.")
    } else if plan_open {
        Some("Adding it puts the plan under review out of date. Settle the plan first to keep it.")
    } else {
        None
    }
}

/// The one command a tracked path enters the project as, and what is said
/// of it. `Err(None)` when the answer is about another project or request
/// and is dropped without a word; `Err(Some)` when it is this project's and
/// makes no tracker, with why.
///
/// `instance` is the editor's identity now and `asset` the tracked media as
/// the library has it now; `project`, `target` and `source` are what the
/// answer says it was asked by, for and about.
fn tracker_command(
    instance: uuid::Uuid,
    asset: Option<&Asset>,
    tracking: &Tracking,
    project: uuid::Uuid,
    target: uuid::Uuid,
    source: &SourceIdentity,
    result: Result<editor_media::Tracked, String>,
) -> Result<(Command, String), Option<String>> {
    if project != instance || target != tracking.tracker {
        return Err(None);
    }
    let name = &tracking.name;
    // Media that left the library, or is now of another file, has no path.
    let Some(asset) = asset.filter(|asset| SourceIdentity::of(asset) == *source) else {
        return Err(Some(format!(
            "{name} was not added: its media changed while it was tracked."
        )));
    };
    let tracked = result.map_err(|error| Some(format!("Tracking failed: {error}")))?;
    let (Some(first), Some(last)) = (tracked.keys.first(), tracked.keys.last()) else {
        return Err(Some("Tracking failed: no frame was read.".to_owned()));
    };
    let length = (last.source - first.source).as_seconds();
    if tracked.keys.len() < 2 {
        return Err(Some(if tracked.lost_at.is_some() {
            format!("{name} was not added: the region was lost in the next frame.")
        } else {
            format!("{name} was not added: there was one frame to track.")
        }));
    }
    let said = match tracked.lost_at {
        Some(_) => format!("{name} follows the region for {length:.2} s, then lost it."),
        None => format!("{name} follows the region for {length:.2} s."),
    };
    Ok((
        Command::SetTracker {
            tracker: Tracker {
                id: target,
                name: name.clone(),
                asset_id: asset.id,
                keys: tracked.keys,
            },
        },
        said,
    ))
}

/// Why the speech worker did not take a request, in the person's words.
fn speech_refused(error: editor_media::MediaError) -> String {
    match error {
        editor_media::MediaError::Busy => {
            "The last captions are still stopping; try again in a moment.".to_owned()
        }
        error => format!("Captions cannot be made: {error}"),
    }
}

/// An empty project with nothing to save yet. The core's own default counts
/// as unsaved work, which an untouched window is not.
fn blank() -> Editor {
    Editor::new(Project::default()).unwrap_or_default()
}

use rules::kind_name;

fn with_extension(mut path: PathBuf) -> PathBuf {
    if path.extension().is_none() {
        path.set_extension(PROJECT_EXTENSION);
    }
    path
}

/// When a checkpoint was made, from the milliseconds its file name starts
/// with.
fn checkpoint_time(path: &Path) -> Option<u64> {
    let stem = path.file_stem()?.to_str()?;
    let millis: u64 = stem.split('-').next()?.parse().ok()?;
    Some(millis / 1000)
}

fn list_checkpoints(path: &Path) -> Result<Vec<Checkpoint>, String> {
    let list = editor_store::list_checkpoints(path)
        .map_err(|error| format!("The checkpoints could not be read: {error}"))?;
    Ok(list
        .into_iter()
        .map(|info| Checkpoint {
            made: checkpoint_time(&info.path),
            path: info.path,
            label: info.label,
        })
        .collect())
}

fn same_file(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

/// The path a window's recovery copy is written for: beside the project's
/// file, or a place of this window's own for a project that has no file, or
/// whose copy beside the file belongs to another window (`private`).
fn recovery_home(
    path: Option<&Path>,
    unsaved: Option<&Path>,
    dir: Option<&Path>,
    instance: uuid::Uuid,
    private: bool,
) -> Option<PathBuf> {
    let own = || dir.map(|dir| dir.join(format!("{instance}.{PROJECT_EXTENSION}")));
    if private && let Some(own) = own() {
        return Some(own);
    }
    path.or(unsaved).map(Path::to_path_buf).or_else(own)
}

/// Removes the recovery copy for `base` while it is owned: under `held`, or
/// under ownership taken here. A copy another window has since taken up is
/// that window's work and is left where it is.
fn remove_own_recovery(base: &Path, held: Option<Arc<RecoveryLease>>) -> Result<(), StoreError> {
    let _owned = match held {
        Some(lease) => lease,
        // No lock is made for a copy that is not there.
        None if !editor_store::recovery_path(base).exists() => return Ok(()),
        None => match RecoveryLease::acquire(base) {
            Ok(lease) => Arc::new(lease),
            Err(StoreError::RecoveryInUse(_)) => return Ok(()),
            Err(error) => return Err(error),
        },
    };
    editor_store::remove_recovery(base)
}

/// The file a recovery copy written for `base` speaks for: the open document,
/// when the copy sits beside that very file. A copy kept anywhere else, for a
/// project without a file or beside one another window owns, names no file
/// and comes back as unsaved work.
fn bound_document(
    base: &Path,
    document: Option<&Arc<DocumentSession>>,
) -> Option<Arc<DocumentSession>> {
    document.filter(|document| document.path() == base).cloned()
}

/// Where restored work is saved to, and where its recovery copy stays when
/// that is nowhere: the file the copy was bound to as `document`, or no file
/// at all. The path the copy was offered under gives no right to a file.
fn restored_home(
    base: &Path,
    document: Option<&Arc<DocumentSession>>,
) -> (Option<PathBuf>, Option<PathBuf>) {
    match document {
        Some(document) => (Some(document.path().to_path_buf()), None),
        None => (None, Some(base.to_path_buf())),
    }
}

/// Whether a refused save was refused for another writer's sake, and if so
/// whether that writer was only busy. The edits stay unsaved either way.
fn other_writer(error: &StoreError) -> Option<bool> {
    match error {
        StoreError::DocumentConflict(_) => Some(false),
        StoreError::DocumentBusy(_) => Some(true),
        _ => None,
    }
}

/// What the open document is, as far as choosing where to save it goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Origin {
    /// Opened, made here, or already saved to a file of its own. Save As may
    /// replace a project file the person chose.
    #[default]
    Ordinary,
    /// Restored from a recovery copy that recorded no file: an old copy, a
    /// window's private one, or one found away from its file. Nothing says
    /// which file this work was made from, so it may only make a new one.
    Restored,
}

impl Origin {
    /// The origin of work restored with `document` as its file.
    fn restored(document: Option<&Arc<DocumentSession>>) -> Self {
        match document {
            Some(_) => Self::Ordinary,
            None => Self::Restored,
        }
    }

    /// Looks at `path` as the destination of a Save As. Ordinary work notes
    /// what is there and may replace it; restored work without a file is
    /// refused a path that is taken, and a file that arrives after this
    /// look is refused by the save itself.
    fn prepare(self, path: &Path) -> Result<Arc<DocumentSession>, String> {
        match self {
            Self::Ordinary => DocumentSession::prepare(path).map_err(|error| error.to_string()),
            Self::Restored => DocumentSession::create(path).map_err(|error| match error {
                StoreError::DocumentConflict(_) => "it already exists, and restored work is \
                     kept in a new file so that nothing is replaced"
                    .to_owned(),
                other => other.to_string(),
            }),
        }
    }
}

/// The session a Save As writes through. A destination that is the file
/// already open keeps the session it was opened with: choosing one's own
/// file again must not make a changed file look unchanged.
fn destination(
    open: Option<&Arc<DocumentSession>>,
    prepared: Arc<DocumentSession>,
) -> Arc<DocumentSession> {
    match open {
        Some(open) if open.path() == prepared.path() => open.clone(),
        _ => prepared,
    }
}

/// Writes `project` to a file of its own at `path`. A file that is already
/// there is left as it is, whether it was there before or arrived meanwhile.
fn create_document(path: &Path, project: &Project) -> Result<Arc<DocumentSession>, String> {
    let taken = |error: StoreError| match error {
        StoreError::DocumentConflict(path) => format!(
            "{} already exists, and a duplicate needs a file of its own",
            path.display()
        ),
        other => other.to_string(),
    };
    let session = DocumentSession::create(path).map_err(taken)?;
    editor_store::save_project_with_session(&session, project).map_err(taken)?;
    // A recovery copy beside a file that was only now made describes
    // something that is no longer there.
    editor_store::remove_recovery(session.path()).map_err(|error| error.to_string())?;
    Ok(session)
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, launch: Launch) -> Self {
        let ctx = &cc.egui_ctx;
        // A window told where its settings are keeps what it derives beside
        // them, and not with the user's own.
        let steady_dir = (launch.settings.as_deref())
            .and_then(Path::parent)
            .map_or_else(crate::steady::cache_dir, |beside| {
                beside.join("stabilization")
            });
        let steady_cache = SteadyCache::at(steady_dir.clone());
        let peak_cache = PeakCache::at(
            (launch.settings.as_deref())
                .and_then(Path::parent)
                .map_or_else(crate::peaks::cache_dir, |beside| beside.join("peaks")),
        );
        let told_settings = launch.settings.clone();
        let settings_path = launch.settings.or_else(Settings::default_path);
        let settings = settings_path
            .as_deref()
            .map(Settings::load)
            .unwrap_or_default();
        let palette = Palette::new(settings.appearance, settings.accent);
        theme::install_fonts(ctx);
        theme::apply(ctx, &palette);
        // The interface's scale is the person's and is kept with their
        // settings: egui's own chords would neither keep it nor say it.
        ctx.options_mut(|options| options.zoom_with_keyboard = false);
        ctx.set_zoom_factor(settings.interface_scale.min(ui::largest_scale(ctx)));

        let wake = |ctx: &egui::Context| {
            let ctx = ctx.clone();
            move || ctx.request_repaint()
        };
        let mut notices = Vec::new();
        let mut problem = |text: String| {
            notices.push(Notice {
                text,
                problem: true,
                until: PROBLEM_SECONDS,
            });
        };
        let (media, media_problem) = match MediaService::new(wake(ctx)) {
            Ok(media) => (Some(media), None),
            Err(error) => (None, Some(error.to_string())),
        };
        let titles = match TextService::new(crate::titles::bundled_fonts(), wake(ctx)) {
            Ok(titles) => Some(titles),
            Err(error) => {
                problem(format!("Titles cannot be drawn: {error}"));
                None
            }
        };
        // The person's presets live beside their settings.
        let presets_path = settings_path.as_deref().and_then(Presets::path_beside);
        let (presets, presets_unread) = match presets_path.as_deref().map(Presets::read) {
            Some(presets::Read::Presets(presets)) => (presets, false),
            Some(presets::Read::Unreadable(why)) => {
                problem(format!(
                    "Your presets were not read: {why}. The file is kept as it is."
                ));
                (Presets::default(), true)
            }
            _ => (Presets::default(), false),
        };
        let presets = Arc::new(presets);
        // The saved export presets live beside them, in a file of their own.
        let export_presets_path =
            (settings_path.as_deref()).and_then(crate::export::presets::ExportPresets::path_beside);
        let (export_presets, export_presets_unread) = match export_presets_path
            .as_deref()
            .map(crate::export::presets::ExportPresets::read)
        {
            Some(crate::export::presets::Read::Presets(presets)) => (presets, false),
            Some(crate::export::presets::Read::Unreadable(why)) => {
                problem(format!(
                    "Your export presets were not read: {why}. The file is kept as it is."
                ));
                (crate::export::presets::ExportPresets::default(), true)
            }
            _ => (crate::export::presets::ExportPresets::default(), false),
        };
        // Exports, and the export sheet's poster, draw on the window's
        // device, and keep to the camera paths the viewer keeps. Without
        // FFmpeg the queue says so when asked.
        let poster = crate::export::poster::PosterService::new(
            (cc.wgpu_render_state.as_ref())
                .map(|state| (state.device.clone(), state.queue.clone())),
            Some(steady_dir.clone()),
        );
        let renders = crate::export::queue::RenderQueue::new(
            wake(ctx),
            (cc.wgpu_render_state.as_ref())
                .map(|state| (state.device.clone(), state.queue.clone())),
            Some(steady_dir),
        );
        let audio = match AudioService::new(wake(ctx)) {
            Ok(audio) => Some(audio),
            Err(error) => {
                problem(format!("Sound is unavailable: {error}"));
                None
            }
        };
        // Without proxies the preview reads every original, as it can: said
        // quietly.
        let proxies = crate::proxies::Proxies::start(
            wake(ctx),
            crate::proxies::Proxies::dir(told_settings.as_deref()),
            &settings.preview,
        );
        if let Some(text) = proxies.problem() {
            notices.push(Notice {
                text: text.to_owned(),
                problem: false,
                until: NOTICE_SECONDS,
            });
        }
        // The live session an external agent connects to.
        let (host, session_path, session_problem) = match AgentHost::new(wake(ctx)) {
            Ok(mut host) => {
                let path = launch.agent_session.or_else(|| {
                    settings_path.as_deref().and_then(Path::parent).map(|dir| {
                        dir.join("sessions")
                            .join(format!("{}.json", host.descriptor().instance_id))
                    })
                });
                match path {
                    Some(path) => match host.publish(&path) {
                        Ok(()) => (Some(host), Some(path), None),
                        Err(error) => (Some(host), None, Some(error.to_string())),
                    },
                    None => (
                        Some(host),
                        None,
                        Some("There is no folder to publish the session in.".to_owned()),
                    ),
                }
            }
            Err(error) => (None, None, Some(error.to_string())),
        };

        let agent_tag = host.as_ref().map_or_else(
            || ai::TAG.to_owned(),
            |host| ai::window_tag(host.descriptor().instance_id),
        );
        let right_tab = if settings.agent_panel {
            RightTab::Agent
        } else {
            RightTab::Inspector
        };
        let (job_sender, jobs) = mpsc::channel();
        let mut app = Self {
            editor: blank(),
            playback: Playback::default(),
            looping: false,
            media,
            media_problem,
            audio,
            saver: SaveWorker::new(wake(ctx)),
            host,
            session_path,
            session_problem,
            agent_tag,
            path: None,
            document: None,
            origin: Origin::Ordinary,
            epoch: 0,
            saving: false,
            save_session: None,
            preparing: false,
            conflicted: false,
            ai: Planner::new(wake(ctx)),
            live: ai::Agent::new(wake(ctx)),
            agent_roots_basis: None,
            open_revision: 0,
            recovery_dir: None,
            unsaved_base: None,
            lease: None,
            leasing: false,
            private_recovery: false,
            recovery_stale: false,
            discarded: Vec::new(),
            discard_on_exit: false,
            after_save: None,
            autosave: Autosave::new(0, Instant::now()),
            recover_offer: None,
            settings_saved: settings.clone(),
            settings,
            settings_path,
            presets_saved: Arc::clone(&presets),
            presets,
            presets_path,
            presets_unread,
            palette,
            textures: Textures::default(),
            query: None,
            assets_seen: Arc::default(),
            source: None,
            problems: Vec::new(),
            selection: Selection::default(),
            draft: None,
            project_draft: ProjectDraft::default(),
            agent_saving: false,
            save_count: 0,
            last_save: None,
            draft_hold: DraftHold::default(),
            clipboard: None,
            timeline: TimelineState::default(),
            viewer: ViewerState::with_device(cc.wgpu_render_state.clone())
                .with_titles(Titles::new(titles)),
            inspector: InspectorState::default(),
            library: LibraryState::default(),
            agent: AgentState::default(),
            right_tab,
            library_first: false,
            overlay: Overlay::None,
            checkpoints: CheckpointsState::default(),
            duplicating: false,
            notices,
            imports: VecDeque::new(),
            analysis: None,
            steady_cache,
            proxies,
            sound: None,
            peak_cache,
            peaks: HeldPeaks::default(),
            normalizing: Normalizing::default(),
            ducking: Ducking::default(),
            saved_audio: Saving::default(),
            separating: stems::Separating::default(),
            separator_asked: false,
            separator_taken: false,
            recording: Recording::default(),
            tracking: None,
            speech: None,
            speaking: None,
            heard: None,
            models_dir: speech::models_dir(told_settings.as_deref()),
            models: None,
            media_held: VecDeque::new(),
            file_drop: FileDrop::default(),
            files_held: false,
            import_after_load: Vec::new(),
            jobs,
            job_sender,
            dialog: rfd::AsyncFileDialog::new().set_parent(cc),
            exit_approved: false,
            screenshot: launch.screenshot.map(|path| {
                (
                    path,
                    Instant::now() + Duration::from_secs_f64(launch.screenshot_delay),
                    false,
                )
            }),
            renders,
            export: None,
            poster,
            export_replace: None,
            queue_refusal: None,
            export_presets,
            export_presets_path,
            export_presets_asked: 0,
            export_presets_written: Arc::new(std::sync::Mutex::new(
                crate::export::presets::Writes::new(export_presets_unread),
            )),
            stop_renders: false,
        };
        app.recovery_dir = app
            .settings_path
            .as_deref()
            .and_then(Path::parent)
            .map(|dir| dir.join("recovery"));
        match launch.project {
            Some(path) => {
                app.import_after_load = launch.import;
                app.load(ctx, path);
            }
            None => {
                app.find_orphan(ctx);
                if let Some(text) = app.import(launch.import) {
                    app.notify(ctx, text, true);
                }
            }
        }
        app
    }

    // ---- Notices ----------------------------------------------------------

    fn notify(&mut self, ctx: &egui::Context, text: impl Into<String>, problem: bool) {
        let now = ctx.input(|i| i.time);
        let until = now
            + if problem {
                PROBLEM_SECONDS
            } else {
                NOTICE_SECONDS
            };
        sheets::tell(
            &mut self.notices,
            Notice {
                text: text.into(),
                problem,
                until,
            },
        );
    }

    // ---- Document ---------------------------------------------------------

    /// Everything that depends on the document is brought back in line with
    /// it. Called after any change, whoever made it.
    fn document_changed(&mut self) {
        let project = self.editor.project();
        self.selection
            .clips
            .retain(|id| project.clip(*id).is_some());
        if self
            .selection
            .asset
            .is_some_and(|id| !project.assets.contains_key(&id))
        {
            self.selection.asset = None;
        }
        if self
            .draft
            .as_ref()
            .is_some_and(|draft| project.clip(draft.clip).is_none())
        {
            self.draft = None;
        }
        self.media_changed();
        // The clips under the playhead may show other frames than they did.
        self.textures.jumped();
        if self.playback.playing {
            self.sync_audio();
        }
    }

    /// Brings the textures, the media worker and the source monitor in line
    /// with the library. Only an asset that now decodes from something else
    /// costs anything: a name, a folder or a clip edit changes nothing here.
    fn media_changed(&mut self) {
        let assets = self.editor.project().assets.clone();
        if Arc::ptr_eq(&assets, &self.assets_seen) {
            return;
        }
        let stale = textures::stale(&self.assets_seen, &assets);
        for id in stale.changed.iter().chain(&stale.removed) {
            self.textures.forget_asset(*id);
            self.viewer.forget_lut(*id);
            // Peaks read of the old file are not this media's sound.
            self.peaks.media_replaced(*id, self.sound.as_ref());
        }
        if !stale.changed.is_empty() {
            // Frames of the old file may be queued or on their way. They
            // carry only the asset's identifier, so the worker's generation
            // is what tells them apart.
            self.restart_media();
        }
        self.assets_seen = assets.clone();
        // Media that came, or is of another file now, may want a proxy.
        let project = self.editor.project();
        (self.proxies).library_changed(project, &self.settings.preview, &mut self.textures);
        let Some(source) = &mut self.source else {
            return;
        };
        match assets.get(&source.asset.id) {
            // A new name shows in the monitor's header.
            Some(asset) if textures::same_source(asset, &source.asset) => {
                source.asset = asset.clone();
            }
            // The monitor starts over on what the asset now is.
            Some(asset) => match Source::open(asset) {
                Ok(fresh) => {
                    *source = fresh;
                    self.sync_source_audio();
                }
                Err(_) => self.close_source(),
            },
            None => self.close_source(),
        }
    }

    /// Drops everything the media worker has queued or finished, and asks
    /// again for what is still wanted. Imports that were waiting are handed
    /// back to it: they are not about any asset that changed.
    fn restart_media(&mut self) {
        let Some(media) = &self.media else { return };
        media.cancel();
        self.textures.ask_again();
        let (waiting, refused) = hand_back(std::mem::take(&mut self.imports), |paths| {
            media.import(paths).map(drop)
        });
        self.imports = waiting;
        self.problems.extend(
            refused
                .into_iter()
                .map(|error| format!("An import was interrupted: {error}")),
        );
    }

    // ---- Source monitor ---------------------------------------------------

    /// Shows `asset` in the source monitor. The timeline stops where it is.
    /// Whether the source monitor has anything to show of an asset. One
    /// that has left the library is left for the monitor to say so.
    fn previewable(&self, id: AssetId) -> bool {
        (self.editor.project().assets.get(&id)).is_none_or(rules::placeable)
    }

    fn open_source(&mut self, ctx: &egui::Context, id: AssetId) -> Result<(), String> {
        self.commit_drafts(ctx, true);
        let asset = self
            .editor
            .project()
            .assets
            .get(&id)
            .ok_or("That media is no longer in the library.")?;
        if !rules::placeable(asset) {
            return Err(format!(
                "{} is a LUT: it has no picture to preview.",
                asset.name
            ));
        }
        let source = Source::open(asset)
            .map_err(|error| format!("{} cannot be previewed: {error}", asset.name))?;
        self.playback.pause();
        self.viewer.cancel();
        self.selection.clips.clear();
        self.selection.asset = Some(id);
        start_source(&mut self.source, source, &mut self.textures);
        // Whatever was sounding, the timeline's or another source's, stops.
        self.sync_source_audio();
        Ok(())
    }

    /// Leaves the source monitor. The timeline is where it was, paused.
    fn close_source(&mut self) {
        if self.source.take().is_some() {
            self.playback.pause();
            self.sync_audio();
        }
    }

    fn sync_source_audio(&mut self) {
        let (Some(audio), Some(source)) = (&self.audio, &self.source) else {
            return;
        };
        if source.audible() {
            audio.sync(
                source.project.clone(),
                source.playback.position,
                source.playback.playing,
                source.playback.speed,
            );
        } else {
            // Nothing to hear: the wall clock keeps the time.
            audio.stop();
        }
    }

    fn sync_audio(&mut self) {
        if let Some(audio) = &self.audio {
            audio.sync(
                self.editor.snapshot(),
                self.playback.position,
                self.playback.playing,
                self.playback.speed,
            );
        }
    }

    /// Puts the cues read from `path` on a caption track of their own, in
    /// the language the file names, else in that of the track in view.
    fn import_captions(&mut self, path: &Path, read: SubtitleRead) -> Result<String, String> {
        let (cues, named) = read?;
        let project = self.editor.project();
        let viewed = captions_tab::in_view(project, self.library.captions.track, &self.selection)
            .and_then(|id| project.tracks.iter().find(|track| track.id == id))
            .and_then(|track| track.language.clone());
        let guessed = named.is_none();
        let language = named.or(viewed).unwrap_or_else(|| "en".to_owned());
        let (command, track, count) = captions_tab::imported(project, &cues, &language)?;
        self.edit(command)?;
        self.library.captions.track = Some(track);
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let captions = if count == 1 {
            "1 caption"
        } else {
            &format!("{count} captions")
        };
        let language = crate::languages::label(&language);
        Ok(if guessed {
            format!(
                "Imported {captions} from {name} as {language}: change the language in the Captions tab if it is another."
            )
        } else {
            format!("Imported {captions} from {name} ({language})")
        })
    }

    /// Applies one command. `Ok` carries what was done, in the words the
    /// history uses.
    fn edit(&mut self, command: Command) -> Result<String, String> {
        match self.editor.dispatch(command) {
            Ok(outcome) => {
                if outcome.changed {
                    self.document_changed();
                }
                Ok(outcome.label)
            }
            Err(error) => Err(error.to_string()),
        }
    }

    /// Puts `editor` in place of the document. What the old one had not
    /// saved is given up here, and its recovery copy with it, unless the new
    /// document is that very copy.
    fn replace_editor(
        &mut self,
        ctx: &egui::Context,
        editor: Editor,
        path: Option<PathBuf>,
        document: Option<Arc<DocumentSession>>,
        unsaved_base: Option<PathBuf>,
    ) {
        let old = self.recovery_base();
        let old_context = self.editor.instance_id();
        self.editor = editor;
        self.path = path;
        self.private_recovery = false;
        self.document = document;
        self.unsaved_base = unsaved_base;
        self.origin = Origin::Ordinary;
        self.conflicted = false;
        // What the model was asked, or proposed, was about the old document.
        if self.live.talking() || self.live.turn().is_some() {
            self.live.end();
            self.agent.push(Entry::Note(
                "Another project is open, so the agent's conversation ended.".to_owned(),
            ));
        }
        if let Some(open) = self.ai.forget() {
            self.agent.settle(
                open,
                PlanState::Closed("The project it was planned for was closed.".to_owned()),
            );
        }
        if let Some(old) = old
            && Some(&old) != self.recovery_base().as_ref()
        {
            // Only a copy this window wrote is its own to remove.
            if self.owns(&old) {
                self.discard_recovery(ctx, old_context, old);
            }
            self.lease = None;
        }
        self.epoch += 1;
        if let Some(media) = &self.media {
            media.cancel();
        }
        if let Some(audio) = &self.audio {
            audio.stop();
        }
        // A region being tracked was the old project's.
        if let Some(analysis) = &self.analysis {
            analysis.cancel_all();
        }
        self.tracking = None;
        self.inspector.end_region();
        self.inspector.set_tracking(None);
        // And so were captions being made or waiting.
        if let (Some(service), Some(speaking)) = (&self.speech, self.speaking.take()) {
            service.cancel(speaking.request);
        }
        self.heard = None;
        // And so was a camera being measured, and every path held.
        self.viewer.steady.clear();
        self.inspector.set_steady(None);
        // And the sound being read for its waveforms, and every one held,
        // and the clips waiting for their loudness.
        self.peaks.project_replaced(self.sound.as_ref());
        self.normalizing.clear();
        self.ducking.clear();
        self.inspector.set_ducking(None);
        // A file being written is the person's and goes on; one written is
        // no longer offered to the project that asked for it.
        self.saved_audio.project_replaced();
        // A take is stopped and kept; once closed, where it is is said.
        self.recording.project_replaced();
        self.textures.clear();
        self.viewer.release_canvases();
        self.viewer.titles.clear();
        self.assets_seen = self.editor.project().assets.clone();
        // Proxies asked for the old project are given up, and the new one's
        // asked for.
        let project = self.editor.project();
        (self.proxies).replaced(project, &self.settings.preview, &mut self.textures);
        self.source = None;
        self.selection.clear();
        self.draft = None;
        self.project_draft = ProjectDraft::default();
        self.clipboard = None;
        self.playback = Playback::default();
        self.imports.clear();
        self.saving = false;
        self.save_session = None;
        self.after_save = None;
        self.recovery_stale = false;
        self.autosave = Autosave::new(self.editor.revision(), Instant::now());
        // The part of the library in view is the person's, not the project's.
        let (tab, kind) = (self.library.tab, self.library.kind);
        self.library = LibraryState::default();
        (self.library.tab, self.library.kind) = (tab, kind);
        // A worker still busy with the old project's checkpoints stays counted.
        self.checkpoints.list = None;
        self.checkpoints.name.clear();
        // A duck waiting on its question was of the old project's clips.
        if matches!(
            self.overlay,
            Overlay::Checkpoints | Overlay::Conflict(_) | Overlay::Duck(_)
        ) {
            self.overlay = Overlay::None;
        }
        self.timeline.cancel_drag();
        self.timeline.fit = true;
        self.viewer.cancel();
    }

    // ---- Recovery ---------------------------------------------------------

    /// The path the recovery copy is written for: the project's file, or a
    /// place of its own for a project that has none yet.
    fn recovery_base(&self) -> Option<PathBuf> {
        recovery_home(
            self.path.as_deref(),
            self.unsaved_base.as_deref(),
            self.recovery_dir.as_deref(),
            self.editor.instance_id(),
            self.private_recovery,
        )
    }

    fn owns(&self, base: &Path) -> bool {
        self.lease.as_ref().is_some_and(|(held, _)| held == base)
    }

    /// Ownership of the copy for `base` as the window closes: the lease that
    /// is held, one a worker was already asked for, or a new one. `None`
    /// when another window owns the copy.
    fn exit_lease(&mut self, base: &Path) -> Option<Arc<RecoveryLease>> {
        if let Some((held, lease)) = &self.lease
            && held == base
        {
            return Some(lease.clone());
        }
        // A lease that is on its way would refuse a second one.
        let wait = if self.leasing {
            LEASE_WAIT
        } else {
            Duration::ZERO
        };
        let until = Instant::now() + wait;
        while let Ok(job) = self
            .jobs
            .recv_timeout(until.saturating_duration_since(Instant::now()))
        {
            if let Job::Leased {
                base: granted,
                result,
            } = job
                && granted == base
            {
                self.leasing = false;
                if let Ok(lease) = result {
                    return Some(Arc::new(lease));
                }
                break;
            }
        }
        RecoveryLease::acquire(base).ok().map(Arc::new)
    }

    fn write_recovery(&mut self, ctx: &egui::Context) {
        let Some(base) = self.recovery_base() else {
            return;
        };
        let lease = match &self.lease {
            Some((held, lease)) if *held == base => lease.clone(),
            _ => {
                // Ownership comes first, on a worker; the copy is written
                // when it is granted.
                self.lease = None;
                if !self.leasing {
                    self.leasing = self.spawn(ctx, "neptune-video-recovery", move || Job::Leased {
                        result: RecoveryLease::acquire(&base),
                        base,
                    });
                    if !self.leasing {
                        self.autosave.failed(Instant::now());
                    }
                }
                return;
            }
        };
        // The copy carries what the file held when this window last read or
        // wrote it, so that restoring it later cannot pass over another
        // writer's save.
        let document = bound_document(&base, self.document.as_ref());
        match self
            .saver
            .request_recovery(base, &self.editor, lease, document)
        {
            Ok(()) => {
                // Only the worker's answer says the copy exists.
                self.autosave.submitted(Instant::now());
                self.recovery_stale = false;
            }
            Err(error) => {
                self.autosave.failed(Instant::now());
                self.notify(
                    ctx,
                    format!("The recovery copy was not written: {error}"),
                    true,
                );
            }
        }
    }

    fn remove_recovery(&self, ctx: &egui::Context, base: PathBuf) {
        // The ownership this window holds goes with the job, so that the
        // copy is still its own when the job gets to it.
        let held = self
            .lease
            .as_ref()
            .filter(|(owned, _)| *owned == base)
            .map(|(_, lease)| lease.clone());
        self.spawn(ctx, "neptune-video-recovery", move || {
            Job::Background(
                remove_own_recovery(&base, held)
                    .map(|()| String::new())
                    .map_err(|error| format!("The recovery copy was not removed: {error}")),
            )
        });
    }

    /// Removes a recovery copy for good: a write of it that is still under
    /// way is removed when it lands.
    fn discard_recovery(&mut self, ctx: &egui::Context, context: uuid::Uuid, base: PathBuf) {
        self.discarded.push((context, base.clone()));
        if self.discarded.len() > 32 {
            self.discarded.remove(0);
        }
        self.remove_recovery(ctx, base);
    }

    /// Looks for the recovery copy of a project that was never saved.
    fn find_orphan(&self, ctx: &egui::Context) {
        let Some(dir) = self.recovery_dir.clone() else {
            return;
        };
        self.spawn(ctx, "neptune-video-recovery", move || {
            let newest = std::fs::read_dir(&dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|entry| {
                    let name = entry.file_name().into_string().ok()?;
                    let base = name.strip_suffix(RECOVERY_SUFFIX)?;
                    let modified = entry.metadata().ok()?.modified().ok()?;
                    Some((modified, dir.join(base)))
                })
                // A copy another window is writing is that window's work.
                .filter(|(_, base)| !RecoveryLease::is_active(base).unwrap_or(true))
                .max_by_key(|(modified, _)| *modified);
            match newest {
                Some((_, base)) => Job::Orphan(base),
                None => Job::Background(Ok(String::new())),
            }
        });
    }

    // ---- Files ------------------------------------------------------------

    /// Runs `work` on a thread of its own. `false` when it could not start.
    fn spawn(
        &self,
        ctx: &egui::Context,
        name: &str,
        work: impl FnOnce() -> Job + Send + 'static,
    ) -> bool {
        let sender = self.job_sender.clone();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                let _ = sender.send(work());
                ctx.request_repaint();
            });
        if let Err(error) = &spawned {
            let _ = self.job_sender.send(Job::Background(Err(format!(
                "Cannot start a worker: {error}"
            ))));
        }
        spawned.is_ok()
    }

    // ---- Checkpoints and duplicates ---------------------------------------

    /// Lists the project's checkpoints on a worker, after writing one named
    /// `label` when there is a label. One such worker runs at a time.
    fn checkpoint_job(&mut self, ctx: &egui::Context, label: Option<String>) {
        let Some(path) = self.path.clone() else {
            return;
        };
        if self.checkpoints.busy {
            return;
        }
        if label.is_some() {
            self.commit_drafts(ctx, true);
        }
        let epoch = self.epoch;
        let project = self.editor.snapshot();
        self.checkpoints.busy = self.spawn(ctx, "neptune-video-checkpoint", move || {
            let saved = label.map(|label| {
                editor_store::checkpoint(&path, &project, &label)
                    .map(|_| label)
                    .map_err(|error| format!("The checkpoint was not saved: {error}"))
            });
            let list = list_checkpoints(&path);
            Job::Checkpoints {
                epoch,
                path,
                saved,
                list,
            }
        });
    }

    /// Restores the newest checkpoint called `name`.
    fn restore_named(&mut self, ctx: &egui::Context, name: String) -> Result<(), String> {
        let path = self.checkpoint_home()?;
        let epoch = self.epoch;
        self.checkpoints.busy = self.spawn(ctx, "neptune-video-checkpoint", move || {
            let wanted = name.to_lowercase();
            let result = list_checkpoints(&path).and_then(|list| {
                // The list is newest first.
                let found = list
                    .into_iter()
                    .find(|checkpoint| checkpoint.label.to_lowercase() == wanted)
                    .ok_or("there is no checkpoint with that name")?;
                editor_store::load_project(&found.path).map_err(|error| error.to_string())
            });
            Job::CheckpointLoaded {
                epoch,
                label: name,
                result,
            }
        });
        Ok(())
    }

    /// The file checkpoints are kept beside, when a worker is free for them.
    fn checkpoint_home(&self) -> Result<PathBuf, String> {
        if self.checkpoints.busy {
            return Err("A checkpoint is still being written or read.".to_owned());
        }
        self.path.clone().ok_or_else(|| {
            "Save the project first: checkpoints are kept beside its file.".to_owned()
        })
    }

    fn restore_checkpoint(&mut self, ctx: &egui::Context, checkpoint: Checkpoint) {
        if self.checkpoints.busy {
            return;
        }
        let epoch = self.epoch;
        self.checkpoints.busy = self.spawn(ctx, "neptune-video-checkpoint", move || {
            Job::CheckpointLoaded {
                epoch,
                result: editor_store::load_project(&checkpoint.path)
                    .map_err(|error| error.to_string()),
                label: checkpoint.label,
            }
        });
    }

    /// Writes the project to `path` under an identity of its own. The file
    /// it came from is not written.
    fn duplicate_to(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.commit_drafts(ctx, true);
        let source = self.editor.project();
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().trim().to_owned())
            .filter(|stem| !stem.is_empty())
            .unwrap_or_else(|| format!("{} copy", source.name));
        let project = Box::new(editor_store::duplicate_project(source, name));
        let own = self.path.clone();
        let epoch = self.epoch;
        let context = self.editor.instance_id();
        let revision = self.editor.revision();
        self.duplicating = self.spawn(ctx, "neptune-video-duplicate", move || {
            let result = if own.is_some_and(|own| same_file(&own, &path)) {
                Err("a duplicate needs a file of its own".to_owned())
            } else {
                create_document(&path, &project)
            };
            Job::Duplicated {
                epoch,
                context,
                revision,
                project,
                result,
            }
        });
    }

    fn pick(&mut self, ctx: &egui::Context, pick: Pick) {
        let dialog = self.dialog.clone();
        // The dialog is made on the main thread, as macOS requires, and
        // awaited on a worker while the window keeps drawing.
        match pick {
            Pick::Open => {
                let choice = dialog
                    .set_title("Open project")
                    .add_filter("Neptune Video project", &[PROJECT_EXTENSION])
                    .pick_file();
                self.spawn(ctx, "neptune-video-dialog", move || {
                    let path = pollster::block_on(choice).map(|file| file.path().to_path_buf());
                    Job::Picked(Pick::Open, path.into_iter().collect())
                });
            }
            Pick::SaveAs => {
                let name = format!("{}.{PROJECT_EXTENSION}", self.editor.project().name);
                let choice = dialog
                    .set_title("Save project")
                    .add_filter("Neptune Video project", &[PROJECT_EXTENSION])
                    .set_file_name(name)
                    .save_file();
                self.spawn(ctx, "neptune-video-dialog", move || {
                    let path = pollster::block_on(choice).map(|file| file.path().to_path_buf());
                    Job::Picked(Pick::SaveAs, path.into_iter().collect())
                });
            }
            Pick::Duplicate => {
                let name = format!("{} copy.{PROJECT_EXTENSION}", self.editor.project().name);
                let choice = dialog
                    .set_title("Duplicate project")
                    .add_filter("Neptune Video project", &[PROJECT_EXTENSION])
                    .set_file_name(name)
                    .save_file();
                self.spawn(ctx, "neptune-video-dialog", move || {
                    let path = pollster::block_on(choice).map(|file| file.path().to_path_buf());
                    Job::Picked(Pick::Duplicate, path.into_iter().collect())
                });
            }
            Pick::ImportFiles => {
                let choice = dialog
                    .set_title("Import media")
                    .add_filter(
                        "Media",
                        &[
                            "mp4", "mov", "mkv", "webm", "avi", "m4v", "mp3", "wav", "aac", "m4a",
                            "flac", "ogg", "opus", "png", "jpg", "jpeg", "webp", "bmp", "gif",
                            "cube", "ttf", "otf", "ttc", "otc",
                        ],
                    )
                    .add_filter("All files", &["*"])
                    .pick_files();
                self.spawn(ctx, "neptune-video-dialog", move || {
                    let files = pollster::block_on(choice).unwrap_or_default();
                    Job::Picked(
                        Pick::ImportFiles,
                        files.iter().map(|file| file.path().to_path_buf()).collect(),
                    )
                });
            }
            Pick::ImportFonts => {
                let choice = dialog
                    .set_title("Import fonts")
                    .add_filter("Fonts", &["ttf", "otf", "ttc", "otc"])
                    .pick_files();
                self.spawn(ctx, "neptune-video-dialog", move || {
                    let files = pollster::block_on(choice).unwrap_or_default();
                    Job::Picked(
                        Pick::ImportFonts,
                        files.iter().map(|file| file.path().to_path_buf()).collect(),
                    )
                });
            }
            Pick::SpeechModel => {
                let choice = dialog
                    .set_title("Choose a Whisper model")
                    .add_filter("Whisper models", &["bin"])
                    .pick_file();
                self.spawn(ctx, "neptune-video-dialog", move || {
                    let file = pollster::block_on(choice);
                    Job::Picked(
                        Pick::SpeechModel,
                        file.iter().map(|file| file.path().to_path_buf()).collect(),
                    )
                });
            }
            Pick::ImportCaptions => {
                let choice = dialog
                    .set_title("Import captions")
                    .add_filter("Subtitles", &["srt", "vtt"])
                    .pick_files();
                self.spawn(ctx, "neptune-video-dialog", move || {
                    let files = pollster::block_on(choice).unwrap_or_default();
                    Job::Picked(
                        Pick::ImportCaptions,
                        files.iter().map(|file| file.path().to_path_buf()).collect(),
                    )
                });
            }
            Pick::ExportCaptions(track, format) => {
                let name = captions_tab::export_name(self.editor.project(), track, format);
                let (filter, extension) = match format {
                    SubtitleFormat::Srt => ("SubRip subtitles", "srt"),
                    SubtitleFormat::Vtt => ("WebVTT subtitles", "vtt"),
                };
                let choice = dialog
                    .set_title("Export captions")
                    .add_filter(filter, &[extension])
                    .set_file_name(name)
                    .save_file();
                self.spawn(ctx, "neptune-video-dialog", move || {
                    let path = pollster::block_on(choice).map(|file| file.path().to_path_buf());
                    Job::Picked(
                        Pick::ExportCaptions(track, format),
                        path.into_iter().collect(),
                    )
                });
            }
            Pick::ImportFolder => {
                let choice = dialog.set_title("Import folder").pick_folder();
                self.spawn(ctx, "neptune-video-dialog", move || {
                    let path = pollster::block_on(choice).map(|file| file.path().to_path_buf());
                    Job::Picked(Pick::ImportFolder, path.into_iter().collect())
                });
            }
            Pick::AgentFolder => {
                let choice = dialog.set_title("A folder the agent may use").pick_folder();
                self.spawn(ctx, "neptune-video-dialog", move || {
                    let path = pollster::block_on(choice).map(|file| file.path().to_path_buf());
                    Job::Picked(Pick::AgentFolder, path.into_iter().collect())
                });
            }
            Pick::ExportFolder => {
                let mut dialog = dialog.set_title("Export to folder");
                if let Some(folder) = self.export.as_ref().and_then(|state| state.folder()) {
                    dialog = dialog.set_directory(folder);
                }
                let choice = dialog.pick_folder();
                self.spawn(ctx, "neptune-video-dialog", move || {
                    let path = pollster::block_on(choice).map(|file| file.path().to_path_buf());
                    Job::Picked(Pick::ExportFolder, path.into_iter().collect())
                });
            }
        }
    }

    /// Asks where the sound `source` names is to be saved. What is saved is
    /// captured now; the file's name is settled where the dialog is awaited,
    /// which may look at the disk.
    fn pick_audio_file(
        &mut self,
        ctx: &egui::Context,
        source: extract::Source,
    ) -> Result<(), String> {
        let asked = extract::Asked::of(self.editor.project(), self.editor.instance_id(), source)?;
        let choice = self
            .dialog
            .clone()
            .set_title("Save audio")
            .add_filter("WAV audio", &[extract::EXTENSION])
            .set_file_name(asked.file_name())
            .save_file();
        self.spawn(ctx, "neptune-video-dialog", move || {
            let picked = pollster::block_on(choice).map(|file| file.path().to_path_buf());
            Job::AudioPicked {
                to: picked.map(|picked| extract::target(picked, Path::exists)),
                asked: Box::new(asked),
            }
        });
        Ok(())
    }

    fn import(&mut self, paths: Vec<PathBuf>) -> Option<String> {
        if paths.is_empty() {
            return None;
        }
        let Some(media) = &self.media else {
            return Some(
                self.media_problem
                    .clone()
                    .unwrap_or_else(|| "Media cannot be imported.".to_owned()),
            );
        };
        match media.import(paths.clone()) {
            Ok(_) => {
                self.imports.push_back(paths);
                None
            }
            Err(error) => Some(error.to_string()),
        }
    }

    fn load(&mut self, ctx: &egui::Context, path: PathBuf) {
        let epoch = self.epoch;
        let revision = self.editor.revision();
        self.spawn(ctx, "neptune-video-open", move || {
            let result =
                editor_store::load_project_with_session(&path).map_err(|error| error.to_string());
            // The file is known from here on by the one path its session
            // has for it, whichever name it was opened under.
            let path = match &result {
                Ok((_, session)) => session.path().to_path_buf(),
                Err(_) => path,
            };
            // A copy another window is writing is not offered here.
            let recovery = editor_store::recovery_path(&path).exists()
                && !RecoveryLease::is_active(&path).unwrap_or(true);
            Job::Loaded {
                epoch,
                revision,
                path,
                result,
                recovery,
            }
        });
    }

    /// Saves the document through `session`. The worker replaces the file
    /// only while it is still what the session last saw there; otherwise the
    /// edits stay unsaved and the person is asked.
    fn save_through(&mut self, ctx: &egui::Context, session: Arc<DocumentSession>) {
        self.commit_drafts(ctx, true);
        // Registered first, so that the acknowledgement marks exactly the
        // document that was written and later edits stay unsaved.
        let revision = self.editor.register_save();
        match self.saver.request_document(session.clone(), &self.editor) {
            Ok(()) => {
                self.saving = true;
                self.save_session = Some((session, revision));
            }
            Err(error) => {
                self.after_save = None;
                if std::mem::take(&mut self.agent_saving) {
                    self.finished_save(session.path(), Err(error.to_string()));
                }
                self.notify(ctx, format!("The project was not saved: {error}"), true);
            }
        }
    }

    /// Looks at the destination of a Save As on a worker, then saves there.
    /// What is at the destination now is what the save may replace, unless
    /// the document is restored work without a file: that takes a new file.
    fn prepare_save(&mut self, ctx: &egui::Context, path: PathBuf, create_only: bool) {
        if self.preparing {
            self.notify(ctx, "A save is still being prepared", true);
            return;
        }
        let epoch = self.epoch;
        let context = self.editor.instance_id();
        let origin = self.origin;
        self.preparing = self.spawn(ctx, "neptune-video-save-as", move || Job::Prepared {
            epoch,
            context,
            origin,
            // An agent's Save As makes a new file only, as restored work does.
            result: if create_only {
                Origin::Restored.prepare(&path)
            } else {
                origin.prepare(&path)
            },
            path,
        });
        if !self.preparing {
            self.after_save = None;
        }
    }

    /// A save was refused because another writer has the file. The edits are
    /// still in the window and still unsaved; nothing is retried or written
    /// elsewhere without being asked for.
    fn save_refused(&mut self, path: PathBuf, busy: bool) {
        let open = self
            .document
            .as_ref()
            .is_some_and(|document| document.path() == path);
        if open && !busy {
            self.conflicted = true;
        }
        self.overlay = Overlay::Conflict(Conflict { path, busy, open });
    }

    /// Runs `pending` now, or asks about unsaved work first.
    fn guard(&mut self, ctx: &egui::Context, pending: Pending) {
        self.commit_drafts(ctx, true);
        if self.editor.is_dirty() {
            self.overlay = Overlay::Unsaved(pending);
        } else {
            self.proceed(ctx, pending);
        }
    }

    fn proceed(&mut self, ctx: &egui::Context, pending: Pending) {
        match pending {
            Pending::New => self.replace_editor(ctx, blank(), None, None, None),
            Pending::Open(None) => {
                self.open_revision = self.editor.revision();
                self.pick(ctx, Pick::Open);
            }
            Pending::Open(Some(path)) => self.load(ctx, path),
            // Exports that started while unsaved work was asked about are
            // asked about too.
            Pending::Quit if self.renders.busy() && !self.stop_renders => {
                self.overlay = Overlay::StopRendering;
            }
            Pending::Quit => {
                // Quitting with unsaved work was chosen: its copy goes too.
                self.discard_on_exit = self.editor.is_dirty();
                self.exit_approved = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    // ---- Workers ----------------------------------------------------------

    fn take_jobs(&mut self, ctx: &egui::Context) {
        while let Ok(job) = self.jobs.try_recv() {
            match job {
                Job::Picked(Pick::Open, paths) => {
                    let Some(path) = paths.into_iter().next() else {
                        continue;
                    };
                    // Work done while the dialog was up is asked about.
                    if self.editor.revision() != self.open_revision && self.editor.is_dirty() {
                        self.overlay = Overlay::Unsaved(Pending::Open(Some(path)));
                    } else {
                        self.load(ctx, path);
                    }
                }
                Job::Picked(Pick::SaveAs, paths) => match paths.into_iter().next() {
                    Some(path) => self.prepare_save(ctx, with_extension(path), false),
                    // Leaving the dialog also abandons what waited on it.
                    None => self.after_save = None,
                },
                Job::Picked(Pick::Duplicate, paths) => {
                    if let Some(path) = paths.into_iter().next() {
                        if self.duplicating {
                            self.notify(ctx, "A duplicate is still being written", true);
                        } else {
                            self.duplicate_to(ctx, with_extension(path));
                        }
                    }
                }
                // The sheet that asked must still be open; otherwise the
                // answer is dropped.
                Job::Picked(Pick::AgentFolder, paths) => {
                    for path in paths {
                        // The same rule as the server's: a home, drive or
                        // system folder is too wide to hand an agent.
                        if crate::mcp::usable_roots(vec![path.clone()]).is_empty() {
                            self.notify(
                                ctx,
                                format!(
                                    "{} is too wide for the agent; choose a folder inside it",
                                    path.display()
                                ),
                                true,
                            );
                        } else if !self.settings.ai.folders.contains(&path) {
                            self.settings.ai.folders.push(path);
                        }
                    }
                }
                Job::Picked(Pick::ExportFolder, paths) => {
                    if let Some(state) = self.export.as_mut() {
                        state.folder_chosen(paths.into_iter().next());
                    }
                }
                Job::Picked(Pick::ImportFiles | Pick::ImportFolder | Pick::ImportFonts, paths) => {
                    if let Some(problem) = self.import(paths) {
                        self.notify(ctx, problem, true);
                    }
                }
                Job::AudioPicked { asked, to } => match to {
                    None => {}
                    Some(Err(problem)) => self.notify(ctx, problem, true),
                    Some(Ok(to)) => {
                        if let Err(problem) = self.save_audio(ctx, &asked, to) {
                            self.notify(ctx, problem, true);
                        }
                    }
                },
                Job::Picked(Pick::ImportCaptions, paths) => {
                    if paths.is_empty() {
                        continue;
                    }
                    let epoch = self.epoch;
                    self.spawn(ctx, "neptune-video-captions", move || Job::CaptionsRead {
                        epoch,
                        files: (paths.into_iter())
                            .map(|path| {
                                let read = captions_tab::read_file(&path);
                                (path, read)
                            })
                            .collect(),
                    });
                }
                Job::Picked(Pick::ExportCaptions(track, format), paths) => {
                    let Some(path) = paths.into_iter().next() else {
                        continue;
                    };
                    match captions_tab::exported(self.editor.project(), track, format) {
                        Ok((text, count)) => {
                            self.spawn(ctx, "neptune-video-captions", move || {
                                Job::Background(captions_tab::write_file(&path, &text).map(|()| {
                                    let name = path.file_name().map_or_else(
                                        || path.display().to_string(),
                                        |name| name.to_string_lossy().into_owned(),
                                    );
                                    match count {
                                        1 => format!("Saved 1 caption to {name}"),
                                        _ => format!("Saved {count} captions to {name}"),
                                    }
                                }))
                            });
                        }
                        Err(why) => self.notify(ctx, why, true),
                    }
                }
                Job::Picked(Pick::SpeechModel, paths) => {
                    let Some(path) = paths.into_iter().next() else {
                        continue;
                    };
                    self.spawn(ctx, "neptune-video-models", move || {
                        let result = ModelShape::read(&path).map_err(|error| error.to_string());
                        Job::ModelChecked { path, result }
                    });
                }
                Job::ModelChecked { path, result } => match result {
                    Ok(shape) => {
                        let name = path.file_name().map_or_else(
                            || path.display().to_string(),
                            |name| name.to_string_lossy().into_owned(),
                        );
                        self.settings.speech.file = Some(path);
                        let english = if shape.multilingual() {
                            ""
                        } else {
                            " It hears English only."
                        };
                        self.notify(
                            ctx,
                            format!("Captions will be made with {name}.{english}"),
                            false,
                        );
                    }
                    Err(why) => self.notify(ctx, format!("That model was not chosen: {why}"), true),
                },
                Job::Models(inventory) => self.models = Some(inventory),
                Job::CaptionsRead { epoch, files } => {
                    if epoch != self.epoch {
                        continue;
                    }
                    for (path, read) in files {
                        match self.import_captions(&path, read) {
                            Ok(text) => self.notify(ctx, text, false),
                            Err(why) => self.notify(ctx, why, true),
                        }
                    }
                }
                Job::Loaded {
                    epoch,
                    revision,
                    path,
                    result,
                    recovery,
                } => {
                    if epoch != self.epoch {
                        continue;
                    }
                    let wanted = std::mem::take(&mut self.import_after_load);
                    // Edits made while the file was being read are not
                    // replaced without asking.
                    if self.editor.revision() != revision && self.editor.is_dirty() {
                        self.overlay = Overlay::Unsaved(Pending::Open(Some(path)));
                        continue;
                    }
                    let opened = result.and_then(|(project, session)| {
                        Ok((Editor::new(project).map_err(|e| e.to_string())?, session))
                    });
                    match opened {
                        Ok((editor, session)) => {
                            self.settings.remember(&path);
                            // A copy this window kept for this same path is
                            // of the work that was just given up.
                            let given_up = self.owns(&path).then(|| self.editor.instance_id());
                            self.replace_editor(
                                ctx,
                                editor,
                                Some(path.clone()),
                                Some(session),
                                None,
                            );
                            if let Some(context) = given_up {
                                self.discard_recovery(ctx, context, path.clone());
                            } else if recovery {
                                self.recover_offer = Some(Offer {
                                    base: path.clone(),
                                    project: Some(path),
                                });
                            }
                        }
                        Err(error) => {
                            self.settings.forget(&path);
                            self.notify(
                                ctx,
                                format!("{} could not be opened: {error}", path.display()),
                                true,
                            );
                        }
                    }
                    if let Some(problem) = self.import(wanted) {
                        self.notify(ctx, problem, true);
                    }
                }
                Job::Prepared {
                    epoch,
                    context,
                    path,
                    origin,
                    result,
                } => {
                    self.preparing = false;
                    // The destination was chosen for another document, or
                    // looked at under a rule that is no longer this one's.
                    if epoch != self.epoch
                        || context != self.editor.instance_id()
                        || origin != self.origin
                    {
                        continue;
                    }
                    match result {
                        Ok(prepared) => {
                            let session = destination(self.document.as_ref(), prepared);
                            self.save_through(ctx, session);
                        }
                        Err(error) => {
                            self.after_save = None;
                            if std::mem::take(&mut self.agent_saving) {
                                self.finished_save(&path, Err(error.clone()));
                            }
                            self.notify(
                                ctx,
                                format!("{} cannot be saved to: {error}", path.display()),
                                true,
                            );
                        }
                    }
                }
                Job::Recovered {
                    epoch,
                    revision,
                    offer,
                    result,
                } => {
                    if epoch != self.epoch {
                        continue;
                    }
                    if self.editor.revision() != revision && self.editor.is_dirty() {
                        // The document moved on; the copy stays for later.
                        self.notify(
                            ctx,
                            "The recovery copy was kept: this project changed meanwhile",
                            true,
                        );
                        continue;
                    }
                    match result {
                        Ok((restored, lease)) => {
                            // The session is the one the copy recorded, with
                            // the file as it was then: a later save is held
                            // against that and not against the file as it is
                            // today. A copy that recorded none has no file.
                            let document = bound_document(&offer.base, restored.document.as_ref());
                            let changed = document.is_some() && restored.document_changed;
                            let detached = document.is_none() && offer.project.is_some();
                            let (path, unsaved) = restored_home(&offer.base, document.as_ref());
                            let origin = Origin::restored(document.as_ref());
                            self.replace_editor(ctx, restored.editor, path, document, unsaved);
                            self.origin = origin;
                            self.lease = Some((offer.base, Arc::new(lease)));
                            self.conflicted = changed;
                            if changed {
                                self.notify(
                                    ctx,
                                    "Unsaved changes were restored. The file changed on \
                                     disk since they were made and will not be replaced",
                                    true,
                                );
                            } else if detached {
                                self.notify(
                                    ctx,
                                    "Unsaved changes were restored as a project without \
                                     a file. Saving asks where to keep it",
                                    true,
                                );
                            } else {
                                self.notify(ctx, "Unsaved changes were restored", false);
                            }
                        }
                        Err(error) => self.notify(
                            ctx,
                            format!("The recovery copy could not be read: {error}"),
                            true,
                        ),
                    }
                }
                Job::Leased { base, result } => {
                    self.leasing = false;
                    if self.recovery_base().as_ref() != Some(&base) {
                        // The document moved elsewhere; the ownership lapses.
                        continue;
                    }
                    match result {
                        Ok(lease) => {
                            self.lease = Some((base, Arc::new(lease)));
                            if self.editor.is_dirty() {
                                self.write_recovery(ctx);
                            }
                        }
                        // Another window of this project owns the copy beside
                        // its file. This one keeps its own, which that window
                        // never writes or removes, and saves where it did.
                        Err(StoreError::RecoveryInUse(_))
                            if !self.private_recovery
                                && self.path.as_ref() == Some(&base)
                                && self.recovery_dir.is_some() =>
                        {
                            self.private_recovery = true;
                            if self.editor.is_dirty() {
                                self.write_recovery(ctx);
                            }
                        }
                        Err(error) => {
                            // Asked again after the interval: the work is
                            // still unprotected.
                            self.autosave.failed(Instant::now());
                            self.notify(
                                ctx,
                                format!("No recovery copy is being kept: {error}"),
                                true,
                            );
                        }
                    }
                }
                Job::Orphan(base) => {
                    // Offered only over an untouched window.
                    if self.path.is_none() && !self.editor.is_dirty() {
                        self.recover_offer = Some(Offer {
                            base,
                            project: None,
                        });
                    }
                }
                Job::Checkpoints {
                    epoch,
                    path,
                    saved,
                    list,
                } => {
                    self.checkpoints.busy = false;
                    match saved {
                        Some(Ok(label)) => {
                            self.notify(ctx, format!("Checkpoint “{label}” saved"), false);
                        }
                        Some(Err(problem)) => self.notify(ctx, problem, true),
                        None => {}
                    }
                    if epoch == self.epoch && self.path.as_ref() == Some(&path) {
                        self.checkpoints.list = Some(list);
                    } else if self.overlay == Overlay::Checkpoints
                        && self.checkpoints.list.is_none()
                    {
                        // The sheet is waiting for another project's list.
                        self.checkpoint_job(ctx, None);
                    }
                }
                Job::CheckpointLoaded {
                    epoch,
                    label,
                    result,
                } => {
                    self.checkpoints.busy = false;
                    if epoch != self.epoch {
                        continue;
                    }
                    // One undoable step: the file, the history and the
                    // editing context stay those of this project.
                    self.commit_drafts(ctx, true);
                    let restored = result.and_then(|project| {
                        self.edit(Command::RestoreProject {
                            project: Box::new(project),
                        })
                    });
                    match restored {
                        Ok(_) => {
                            let end = self.editor.project().duration();
                            if self.playback.position > end {
                                self.playback.seek(end);
                                self.sync_audio();
                            }
                            self.timeline.fit = true;
                            self.notify(ctx, format!("Restored “{label}”"), false);
                        }
                        Err(error) => {
                            self.notify(ctx, format!("“{label}” was not restored: {error}"), true)
                        }
                    }
                }
                Job::Duplicated {
                    epoch,
                    context,
                    revision,
                    project,
                    result,
                } => {
                    self.duplicating = false;
                    let session = match result {
                        Ok(session) => session,
                        Err(error) => {
                            self.notify(ctx, format!("The duplicate was not saved: {error}"), true);
                            continue;
                        }
                    };
                    let path = session.path().to_path_buf();
                    self.settings.remember(&path);
                    let name = project.name.clone();
                    // The copy takes over only when it is the document as it
                    // stands; later edits stay with the project they were
                    // made in.
                    self.commit_drafts(ctx, true);
                    let current = epoch == self.epoch
                        && context == self.editor.instance_id()
                        && revision == self.editor.revision();
                    match Editor::new(*project) {
                        Ok(editor) if current => {
                            let at = self.playback.position;
                            self.replace_editor(ctx, editor, Some(path), Some(session), None);
                            self.playback.seek(at);
                            self.notify(ctx, format!("Now editing “{name}”"), false);
                        }
                        _ => self.notify(ctx, format!("Duplicate saved as “{name}”"), false),
                    }
                }
                Job::TakePath { project, result } => {
                    // Only the take that waits for this path opens a device.
                    let wake = ctx.clone();
                    let started = self.recording.path_answered(project, result, |path| {
                        RecordService::start(path, move || wake.request_repaint())
                    });
                    if let Err(problem) = started {
                        self.notify(ctx, problem, true);
                    }
                }
                Job::Background(Ok(text)) => {
                    if !text.is_empty() {
                        self.notify(ctx, text, false);
                    }
                }
                Job::Background(Err(text)) => self.notify(ctx, text, true),
            }
        }
    }

    // ---- Voiceover ---------------------------------------------------------

    /// Asks for a take: refused in words for a project without a file and
    /// while one is under way, before anything is opened. The take's path
    /// is made beside the document on a worker, and the microphone opened
    /// when it answers; the timeline plays once the device does. See
    /// [`crate::record`].
    fn record(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let document = self.document.as_ref().map(|session| session.path());
        let instance = self.editor.instance_id();
        let playing = self.playback.playing;
        // A playing timeline is paused: it rolls from the playhead at 1× once
        // the device runs.
        let beside = self.recording.ask(instance, document, &mut self.playback)?;
        if playing {
            self.sync_audio();
        }
        // The timeline is what records, from its playhead.
        self.close_source();
        let spawned = self.spawn(ctx, "neptune-video-take", move || Job::TakePath {
            project: instance,
            result: editor_store::new_take(&beside, SystemTime::now())
                .map_err(|error| error.to_string()),
        });
        if !spawned {
            self.recording.stop(&mut self.playback);
        }
        Ok(())
    }

    /// Ends the take under way, and the playback it was recorded over.
    fn stop_recording(&mut self) {
        if self.recording.stop(&mut self.playback) {
            self.sync_audio();
        }
    }

    /// Why a transport request is refused while a take is under way: the
    /// take is placed where it was heard, so the playhead stays with it.
    fn held_by_take(&self, to: &str) -> Result<(), String> {
        match self.recording.busy() {
            true => Err(format!("Stop recording to {to}.")),
            false => Ok(()),
        }
    }

    /// One frame of the take: what its service said, and its anchor when
    /// due. `heard` is the sound device's position, as the transport read
    /// it this frame.
    fn step_recording(&mut self, ctx: &egui::Context, heard: Option<Time>) {
        if !self.recording.busy() {
            return;
        }
        let instance = self.editor.instance_id();
        let framed = self.recording.frame(
            self.editor.project(),
            instance,
            &mut self.playback,
            heard,
            Instant::now(),
        );
        if framed.transport {
            self.sync_audio();
        }
        if let Some(problem) = framed.failed {
            self.notify(ctx, problem, true);
        }
        if let Some(landed) = framed.landed {
            if let Some(command) = landed.command
                && let Err(error) = self.edit(command)
            {
                let path = landed.path.display();
                self.notify(
                    ctx,
                    format!("The take could not be placed: {error}. It is kept in {path}"),
                    true,
                );
                return;
            }
            self.notify(ctx, landed.said, landed.problem);
        }
    }

    // ---- Motion tracking -------------------------------------------------

    /// Starts the analysis worker, when it is first needed.
    fn start_analysis(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.analysis.is_none() {
            let ctx = ctx.clone();
            let service = AnalysisService::new(move || ctx.request_repaint())
                .map_err(|error| error.to_string())?;
            self.analysis = Some(service);
        }
        Ok(())
    }

    // ---- Stabilization ---------------------------------------------------

    /// Has the analysis worker measure the camera's path through a video.
    /// The one it was measuring is given up: one video at a time.
    fn start_steady(&mut self, ctx: &egui::Context, asset: AssetId) -> Result<(), String> {
        let asset = (self.editor.project().assets.get(&asset))
            .ok_or("That media is no longer in the library.")?
            .clone();
        let job = SteadyJob {
            project: self.editor.instance_id(),
            asset: asset.clone(),
            cache: self.steady_cache.clone(),
        };
        let old = self.viewer.steady.cancel();
        self.start_analysis(ctx)
            .map_err(|error| format!("Stabilization is unavailable: {error}"))?;
        let Some(analysis) = &self.analysis else {
            return Err("Stabilization is unavailable.".to_owned());
        };
        if let Some(old) = old {
            analysis.cancel(old);
        }
        let asked = analysis
            .steady(job)
            .map_err(|error| format!("{} was not analysed: {error}", asset.name))?;
        self.viewer.steady.begin(&asset, asked);
        Ok(())
    }

    fn cancel_steady(&mut self) {
        if let (Some(analysis), Some(request)) = (&self.analysis, self.viewer.steady.cancel()) {
            analysis.cancel(request);
        }
    }

    /// Asks the analysis worker about the camera paths this frame wanted
    /// and did not have. Asking reads no file here and returns at once.
    fn ask_steadies(&mut self, ctx: &egui::Context) {
        let wants = self.viewer.steady.take_wants();
        if wants.is_empty() {
            return;
        }
        let project = self.editor.snapshot();
        let instance = self.editor.instance_id();
        let started = self.start_analysis(ctx).is_ok();
        for id in wants {
            let asked = (project.assets.get(&id))
                .filter(|_| started)
                .zip(self.analysis.as_ref())
                .ok_or(())
                .and_then(|(asset, analysis)| {
                    let job = SteadyJob {
                        project: instance,
                        asset: asset.clone(),
                        cache: self.steady_cache.clone(),
                    };
                    analysis.load_steady(job).map_err(|_| ())
                });
            match asked {
                Ok(request) => self.viewer.steady.asked(id, request),
                // Asked again by whoever next wants it.
                Err(()) => self.viewer.steady.unasked(id),
            }
        }
    }

    /// Tells the inspector what is known of the selected clip's camera
    /// path, and wants that path while the clip is steadied or its Motion
    /// section is shown.
    fn show_steady(&mut self, project: &Project) {
        let shown = (self.selection.single())
            .and_then(|id| project.clip(id))
            .and_then(|(_, clip)| {
                let asset = project.assets.get(&clip.asset_id()?)?;
                if asset.kind != MediaKind::Video {
                    return None;
                }
                let effect = (clip.effects.iter()).find(|e| e.kind == EffectKind::Stabilize);
                let steady = &mut self.viewer.steady;
                if effect.is_none()
                    && self.inspector.section != ui::inspector::Section::Motion
                    && steady.measuring().is_none_or(|m| m.asset != asset.id)
                {
                    return None;
                }
                steady.want(asset);
                let smoothness = (look::spec(EffectKind::Stabilize).iter())
                    .find(|spec| spec.key == ParamKey::Smoothness)
                    .map(|spec| {
                        (effect.and_then(|effect| effect.param(spec.key))).unwrap_or(spec.default)
                    })
                    .unwrap_or(0.5);
                let (from, until) = clip.source_span();
                Some(ui::SteadyShown {
                    clip: clip.id,
                    asset: asset.id,
                    status: steady.status(asset),
                    suggested: steady.suggested(
                        asset,
                        from.min(until),
                        from.max(until),
                        smoothness,
                    ),
                })
            });
        self.inspector.set_steady(shown);
    }

    /// Takes what the analysis worker said of camera paths. A path is held
    /// only when it is this project's, of the media as it is now and the
    /// answer to what was asked: see [`crate::steady::Steadies::take`].
    fn take_steadies(&mut self, ctx: &egui::Context) {
        use crate::steady::Taken;
        while let Some(event) = (self.analysis.as_ref()).and_then(AnalysisService::poll_steady) {
            match event {
                SteadyEvent::Progress { request, done } => {
                    self.viewer.steady.progress(request, done);
                }
                SteadyEvent::Done {
                    request,
                    project,
                    asset_id,
                    source,
                    measured,
                    result,
                    unkept,
                } => {
                    let now = self.editor.project().assets.get(&asset_id);
                    let name = now.map_or_else(|| "The video".to_owned(), |a| a.name.clone());
                    let taken = self.viewer.steady.take(
                        self.editor.instance_id(),
                        now,
                        request,
                        project,
                        asset_id,
                        &source,
                        measured,
                        result,
                    );
                    match taken {
                        Taken::Kept if measured => match unkept {
                            None => self.notify(ctx, format!("{name} is analysed."), false),
                            Some(why) => self.notify(
                                ctx,
                                format!(
                                    "{name} is analysed, but this could not be kept for another session: {why}"
                                ),
                                true,
                            ),
                        },
                        Taken::Failed(why) if measured => {
                            self.notify(ctx, format!("{name} was not analysed: {why}"), true);
                        }
                        Taken::Changed => self.notify(
                            ctx,
                            format!("{name} changed while it was analysed. Analyse it again."),
                            true,
                        ),
                        _ => {}
                    }
                }
            }
        }
    }

    // ---- Waveforms ------------------------------------------------------

    /// Starts the sound lane, when it is first needed.
    fn start_sound(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.sound.is_none() {
            let ctx = ctx.clone();
            let service = SoundService::new(move || ctx.request_repaint())
                .map_err(|error| error.to_string())?;
            self.sound = Some(service);
        }
        Ok(())
    }

    /// Asks the sound lane about the peaks the tiles of this frame wanted
    /// and did not have: see [`HeldPeaks::ask`].
    fn ask_peaks(&mut self, ctx: &egui::Context) {
        if !self.peaks.wanting() {
            // Nothing to ask: sound read for tiles no longer drawn is given
            // up all the same.
            if let Some(sound) = &self.sound {
                self.peaks.give_up_unwanted(sound);
            }
            return;
        }
        if let Err(error) = self.start_sound(ctx) {
            // Said once, and not tried again for the media that asked.
            for (id, _) in self.peaks.take_wants() {
                self.peaks.refuse(id);
            }
            self.peaks.beats.give_up_wanted();
            self.notify(ctx, format!("Waveforms are unavailable: {error}"), true);
            return;
        }
        let Some(sound) = &self.sound else {
            return;
        };
        let refused = self.peaks.ask(
            sound,
            &self.editor.snapshot(),
            self.editor.instance_id(),
            &self.peak_cache,
        );
        for text in refused {
            self.notify(ctx, text, true);
        }
    }

    /// Shows or hides the beats of these media for the session, and says
    /// so. They are found when a tile of each is next drawn with its peaks
    /// held: see [`crate::peaks::HeldBeats`].
    fn show_beats(&mut self, assets: &[AssetId], shown: bool) -> String {
        for asset in assets {
            self.peaks.beats.show(*asset, shown);
        }
        let project = self.editor.project();
        let names: Vec<&str> = (assets.iter())
            .filter_map(|id| project.assets.get(id))
            .map(|asset| asset.name.as_str())
            .collect();
        let names = names.join(", ");
        if shown {
            format!("Beats are shown on {names}")
        } else {
            format!("Beats are hidden on {names}")
        }
    }

    /// Takes what the sound lane said. Peaks are held only when they are
    /// this project's, of the media as it is now and the answer to what was
    /// asked: see [`HeldPeaks::take`].
    fn take_sound(&mut self, ctx: &egui::Context) {
        while let Some(event) = self.sound.as_ref().and_then(SoundService::poll) {
            let instance = self.editor.instance_id();
            if let SoundEvent::Progress { request, done } = event
                && (self.saved_audio.progress(request, done)
                    || self.separating.progress(request, done))
            {
                continue;
            }
            if let SoundEvent::Separated { .. } = event {
                let answer = (self.separating).answered(self.editor.project(), instance, event);
                if let Some(answer) = answer {
                    self.stems_answered(ctx, answer);
                }
                continue;
            }
            if let SoundEvent::Extracted { .. } = event {
                let now = ctx.input(|i| i.time);
                if let Some(said) = self.saved_audio.answered(instance, now, event) {
                    self.notify(ctx, said.text, said.problem);
                }
                continue;
            }
            if let SoundEvent::Measured { .. } = event {
                let written = (self.normalizing).answered(self.editor.project(), instance, event);
                for written in written {
                    self.write_normalized(ctx, written);
                }
                continue;
            }
            let failed = (self.peaks).answered(instance, self.editor.project(), event);
            if let Some(text) = failed {
                self.notify(ctx, text, true);
            }
        }
        // Clips held for room in the lane, or for an answer about the same
        // span, are asked for now.
        if self.normalizing.holding()
            && let Some(sound) = &self.sound
        {
            let problems = self.normalizing.pump(|job| sound.loudness(job));
            for problem in problems {
                self.notify(ctx, problem, true);
            }
        }
    }

    // ---- Saved audio --------------------------------------------------------

    /// Has the sound lane write what `asked` names to `to`: see
    /// [`extract`]. Nothing is edited.
    fn save_audio(
        &mut self,
        ctx: &egui::Context,
        asked: &extract::Asked,
        to: PathBuf,
    ) -> Result<(), String> {
        extract::guard(self.editor.project(), &to)?;
        self.start_sound(ctx)
            .map_err(|error| format!("The sound cannot be saved: {error}"))?;
        let Some(sound) = &self.sound else {
            return Err("The sound cannot be saved.".to_owned());
        };
        let sent = sound
            .extract(asked.job(self.editor.project(), to.clone()))
            .map_err(|error| format!("The sound of {} was not saved: {error}", asked.name))?;
        if let Some(said) = self.saved_audio.sent(asked, to, &sent) {
            self.notify(ctx, said, false);
        }
        ctx.request_repaint();
        Ok(())
    }

    /// Gives up the file being written as `request`, and says what came of
    /// it. One the lane is already putting in place, or wrote, is not given
    /// up, and its answer comes.
    fn cancel_saved_audio(&mut self, ctx: &egui::Context, request: u64) {
        let outcome = (self.sound.as_ref())
            .map_or(editor_media::Cancel::GivenUp, |sound| sound.cancel(request));
        if let Some(said) = self.saved_audio.cancelled(request, outcome) {
            self.notify(ctx, said, false);
        }
    }

    // ---- Stems ---------------------------------------------------------------

    /// Has the stem separator split the whole sound of `clip`'s media into
    /// the project's media folder: see [`stems`]. Nothing is edited until
    /// the stems come. Refused in words without a separator, without a
    /// saved document, and for a clip that is not heard.
    fn separate_stems(&mut self, ctx: &egui::Context, clip: ClipId) -> Result<String, String> {
        let separator = match editor_media::stem_separator() {
            Some(Ok(separator)) => separator,
            Some(Err(why)) => return Err(format!("{}. {why}", stems::NEEDS_TOOL)),
            None => return Err(stems::LOOKING.to_owned()),
        };
        let asked = stems::Asked::of(self.editor.project(), self.editor.instance_id(), clip)?;
        let to = stems::folder(self.document.as_ref().map(|session| session.path()))?;
        self.start_sound(ctx)
            .map_err(|error| format!("Stems cannot be separated: {error}"))?;
        let Some(sound) = &self.sound else {
            return Err("Stems cannot be separated.".to_owned());
        };
        let sent = sound
            .separate(asked.job(separator, to.clone()))
            .map_err(|error| format!("The stems of {} were not made: {error}", asked.name))?;
        let replaced = self.separating.sent(&asked, &sent);
        ctx.request_repaint();
        Ok(replaced.unwrap_or_else(|| {
            format!(
                "Separating {} into {}. This can take minutes; waveforms wait behind it.",
                asked.name,
                to.display()
            )
        }))
    }

    /// Places stems that came, as one step, or says why they were not and
    /// where their files are.
    fn stems_answered(&mut self, ctx: &egui::Context, answer: stems::Answer) {
        if let Some(command) = answer.command
            && let Err(error) = self.edit(command)
        {
            self.notify(
                ctx,
                format!(
                    "The stems could not be placed: {error}. They are kept in {}",
                    stems::listed(&answer.files)
                ),
                true,
            );
            return;
        }
        self.notify(ctx, answer.said.text, answer.said.problem);
    }

    /// Gives up the separation under way as `request`, and says what came
    /// of it. One the lane is already putting in place is not given up, and
    /// its answer comes.
    fn cancel_separate(&mut self, ctx: &egui::Context, request: u64) {
        let outcome = (self.sound.as_ref())
            .map_or(editor_media::Cancel::GivenUp, |sound| sound.cancel(request));
        if let Some(said) = self.separating.cancelled(request, outcome) {
            self.notify(ctx, said, false);
        }
    }

    // ---- Loudness ---------------------------------------------------------

    /// Has the sound lane measure each of `clips` that is normalized: see
    /// [`Normalizing::ask`]. Nothing is edited until the figures come.
    fn normalize(&mut self, ctx: &egui::Context, clips: &[ClipId]) -> Result<String, String> {
        self.start_sound(ctx)
            .map_err(|error| format!("Loudness cannot be measured: {error}"))?;
        let Some(sound) = &self.sound else {
            return Err("Loudness cannot be measured.".to_owned());
        };
        let instance = self.editor.instance_id();
        let said = (self.normalizing).ask(self.editor.project(), instance, clips, |job| {
            sound.loudness(job)
        })?;
        self.notify(ctx, said.clone(), false);
        Ok(said)
    }

    /// Takes the step one answer made for a clip, and says what it did.
    fn write_normalized(&mut self, ctx: &egui::Context, written: normalize::Written) {
        let normalize::Written {
            name,
            command,
            said,
            problem,
            ..
        } = written;
        if let Some(command) = command
            && let Err(error) = self.edit(command)
        {
            self.notify(ctx, format!("{name} was not normalized: {error}"), true);
            return;
        }
        self.notify(ctx, said, problem);
    }

    // ---- Ducking -----------------------------------------------------------

    /// Ducks `clips` by `depth` under the other clips: see [`Ducking`]. A
    /// clip that does not sound is refused in words before anything is
    /// asked, and the person is asked first when volume keys would be
    /// replaced.
    fn duck(
        &mut self,
        ctx: &egui::Context,
        clips: Vec<ClipId>,
        depth: f32,
    ) -> Result<Option<String>, String> {
        let project = self.editor.project();
        let keyed = duck::keyed(project, &duck::takers(project, &clips)?);
        if keyed > 0 {
            self.overlay = Overlay::Duck(DuckQuestion {
                clips,
                depth,
                keyed,
            });
            return Ok(None);
        }
        self.start_ducking(ctx, &clips, depth).map(Some)
    }

    /// Starts the duck in place of one under way, and says so.
    fn start_ducking(
        &mut self,
        ctx: &egui::Context,
        clips: &[ClipId],
        depth: f32,
    ) -> Result<String, String> {
        if let Some(Some(request)) = self.ducking.give_up(&mut self.peaks)
            && let Some(sound) = &self.sound
        {
            sound.cancel(request);
        }
        let started = self.ducking.start(
            self.editor.project(),
            self.editor.instance_id(),
            self.editor.revision(),
            clips,
            depth,
        );
        self.inspector
            .set_ducking(self.ducking.progress(self.editor.project()));
        let said = started?;
        self.notify(ctx, said.clone(), false);
        ctx.request_repaint();
        Ok(said)
    }

    /// Gives up the duck under way, if there is one, and says so.
    fn stop_ducking(&mut self, ctx: &egui::Context) {
        let Some(cancel) = self.ducking.give_up(&mut self.peaks) else {
            return;
        };
        if let (Some(request), Some(sound)) = (cancel, &self.sound) {
            sound.cancel(request);
        }
        self.inspector.set_ducking(None);
        self.notify(
            ctx,
            format!("{} was stopped; nothing was changed.", duck::LABEL),
            false,
        );
    }

    /// Takes the next voice's peaks when they are held, or wants them of
    /// the held peaks and asks the lane for them here; and writes the duck
    /// once every voice is taken.
    fn step_ducking(&mut self, ctx: &egui::Context) {
        if !self.ducking.busy() {
            return;
        }
        let stepped = self.ducking.step(
            self.editor.project(),
            self.editor.instance_id(),
            self.editor.revision(),
            &mut self.peaks,
        );
        match stepped {
            // Asked now, not by the frame that draws the tiles: this runs
            // alone while the window is hidden, and the duck reads on.
            None if self.peaks.wanting() => self.ask_peaks(ctx),
            None => {}
            // The next voice is wanted in a frame of its own.
            Some(Stepped::Took) => ctx.request_repaint(),
            Some(Stepped::Done(Ok((command, said)))) => match self.edit(*command) {
                Ok(_) => self.notify(ctx, said, false),
                Err(error) => self.notify(ctx, format!("Nothing was ducked: {error}"), true),
            },
            Some(Stepped::Done(Err(said))) => self.notify(ctx, said, true),
            Some(Stepped::GivenUp { said, cancel }) => {
                if let (Some(request), Some(sound)) = (cancel, &self.sound) {
                    sound.cancel(request);
                }
                self.notify(ctx, said, true);
            }
        }
        self.inspector
            .set_ducking(self.ducking.progress(self.editor.project()));
    }

    /// Hands a region to the analysis worker. The one it was on is given
    /// up: one region is tracked at a time.
    fn start_tracking(&mut self, ctx: &egui::Context, request: TrackRequest) -> Result<(), String> {
        let project = self.editor.project();
        let region = request.region;
        let asset = (project.assets.get(&region.asset))
            .ok_or("That media is no longer in the library.")?
            .clone();
        if project.trackers.len() >= look::MAX_TRACKERS {
            return Err("A project holds 64 trackers at most.".to_owned());
        }
        self.start_analysis(ctx)
            .map_err(|error| format!("Tracking is unavailable: {error}"))?;
        let Some(analysis) = &self.analysis else {
            return Err("Tracking is unavailable.".to_owned());
        };
        if let Some(old) = self.tracking.take() {
            analysis.cancel(old.request);
        }
        let tracker = uuid::Uuid::new_v4();
        let asked = analysis
            .track(TrackJob {
                project: self.editor.instance_id(),
                target: tracker,
                asset,
                region: Region {
                    center: region.center,
                    size: region.size,
                },
                from: request.from,
                until: request.until,
            })
            .map_err(|error| format!("The region was not tracked: {error}"))?;
        self.tracking = Some(Tracking {
            request: asked,
            clip: region.clip,
            asset: region.asset,
            tracker,
            name: request.name,
            done: 0.0,
            ready: None,
        });
        self.show_tracking();
        Ok(())
    }

    fn cancel_tracking(&mut self) {
        if let (Some(analysis), Some(tracking)) = (&self.analysis, self.tracking.take()) {
            analysis.cancel(tracking.request);
        }
        self.show_tracking();
    }

    /// Tells the inspector what is being tracked.
    fn show_tracking(&mut self) {
        let shown = self.tracking.as_ref().map(|tracking| TrackProgress {
            clip: tracking.clip,
            asset: tracking.asset,
            name: tracking.name.clone(),
            done: tracking.done,
            waits: (tracking.ready.as_ref())
                .map(|_| self.tracker_waits().unwrap_or_default().to_owned()),
        });
        self.inspector.set_tracking(shown);
    }

    /// Takes what the analysis worker published. A path enters the project
    /// as one `SetTracker`, when it is still about this project's media.
    fn take_analysis(&mut self, ctx: &egui::Context) {
        while let Some(event) = self.analysis.as_ref().and_then(AnalysisService::poll) {
            match event {
                AnalysisEvent::Progress { request, done } => {
                    if let Some(tracking) =
                        (self.tracking.as_mut()).filter(|tracking| tracking.request == request)
                    {
                        tracking.done = done;
                    }
                }
                AnalysisEvent::Tracked {
                    request,
                    project,
                    target,
                    asset_id,
                    source,
                    result,
                } => {
                    // An answer to a request this window no longer waits
                    // for, or to another project's, is nobody's.
                    let Some(tracking) = self
                        .tracking
                        .take_if(|tracking| tracking.request == request)
                    else {
                        continue;
                    };
                    let now = self.editor.project().assets.get(&asset_id);
                    match tracker_command(
                        self.editor.instance_id(),
                        now,
                        &tracking,
                        project,
                        target,
                        &source,
                        result,
                    ) {
                        Ok((command, said)) => {
                            let tracker = match command {
                                Command::SetTracker { tracker }
                                    if self.tracker_waits().is_some() =>
                                {
                                    tracker
                                }
                                command => {
                                    self.add_tracker(ctx, command, said);
                                    continue;
                                }
                            };
                            let ready = Ready { tracker, said };
                            // Nobody pressed for this edit: it waits rather
                            // than take the redo or the plan with it.
                            self.notify(
                                ctx,
                                format!(
                                    "{} is ready. Add it in the Motion section.",
                                    tracking.name
                                ),
                                false,
                            );
                            self.tracking = Some(Tracking {
                                done: 1.0,
                                ready: Some(ready),
                                ..tracking
                            });
                        }
                        Err(Some(why)) => self.notify(ctx, why, true),
                        Err(None) => {}
                    }
                }
            }
        }
        // A path that waited is added once nothing is lost by it.
        if self.tracker_waits().is_none() {
            self.add_ready_tracker(ctx);
        }
        self.show_tracking();
    }

    /// Why a path that has come would wait: see [`tracker_waits`].
    fn tracker_waits(&self) -> Option<&'static str> {
        tracker_waits(self.editor.can_redo(), self.ai.open().is_some())
    }

    /// Adds the path that waits, if one does: the person asked for it, or
    /// nothing is lost by it any more.
    fn add_ready_tracker(&mut self, ctx: &egui::Context) {
        let Some(tracking) = (self.tracking).take_if(|tracking| tracking.ready.is_some()) else {
            return;
        };
        if let Some(Ready { tracker, said }) = tracking.ready {
            self.add_tracker(ctx, Command::SetTracker { tracker }, said);
        }
        self.show_tracking();
    }

    /// The one edit a tracked path is. The project refuses a path whose
    /// media has left the library since it came.
    fn add_tracker(&mut self, ctx: &egui::Context, command: Command, said: String) {
        self.commit_drafts(ctx, true);
        match self.edit(command) {
            Ok(_) => self.notify(ctx, said, false),
            Err(error) => {
                self.notify(ctx, format!("The tracker was not added: {error}"), true);
            }
        }
    }

    // ---- Captions from speech ---------------------------------------------

    /// Hands a request to the speech worker: the model's download first, a
    /// step that is over at once when the model is there, then the
    /// timeline as it is now.
    fn generate_captions(
        &mut self,
        ctx: &egui::Context,
        request: SpeechRequest,
    ) -> Result<(), String> {
        if self.speaking.is_some() {
            return Err("Captions are already being made.".to_owned());
        }
        if self.heard.is_some() {
            return Err("Add or discard the captions that are ready first.".to_owned());
        }
        if self.speech.is_none() {
            let ctx = ctx.clone();
            let service = SpeechService::new(move || ctx.request_repaint())
                .map_err(|error| format!("Captions cannot be made: {error}"))?;
            self.speech = Some(service);
        }
        let speaking = Speaking {
            request: 0,
            ask: request,
            heard_at: None,
            stopping: false,
        };
        let ModelChoice::Catalog(model) = speaking.ask.model else {
            return self.transcribe(speaking);
        };
        let dir = (self.models_dir.clone())
            .ok_or("There is no folder to keep models in. Choose a model file instead.")?;
        let Some(service) = &self.speech else {
            return Err("Captions cannot be made.".to_owned());
        };
        let request = service
            .start(SpeechJob::Download { model, dir })
            .map_err(speech_refused)?;
        self.speaking = Some(Speaking {
            request,
            ..speaking
        });
        Ok(())
    }

    /// Hands the timeline as it is now to be heard.
    fn transcribe(&mut self, speaking: Speaking) -> Result<(), String> {
        let model = match &speaking.ask.model {
            ModelChoice::Catalog(model) => (self.models_dir.as_deref())
                .map(|dir| model.path(dir))
                .ok_or("There is no folder to keep models in.")?,
            ModelChoice::File(path) => path.clone(),
        };
        let Some(service) = &self.speech else {
            return Err("Captions cannot be made.".to_owned());
        };
        let request = service
            .start(SpeechJob::Transcribe(Transcription {
                project: self.editor.snapshot(),
                start: speaking.ask.start,
                end: speaking.ask.end,
                model,
                detector: self.models_dir.as_deref().map(|dir| DETECTOR.path(dir)),
                language: speaking.ask.whisper_language(),
                translate: speaking.ask.translate,
            }))
            .map_err(speech_refused)?;
        self.speaking = Some(Speaking {
            request,
            heard_at: Some((self.editor.instance_id(), self.editor.revision())),
            ..speaking
        });
        Ok(())
    }

    fn cancel_speech(&mut self) {
        if let (Some(service), Some(speaking)) = (&self.speech, &mut self.speaking) {
            service.cancel(speaking.request);
            speaking.stopping = true;
        }
    }

    /// Looks at which models are downloaded, on a worker.
    fn look_at_models(&mut self, ctx: &egui::Context) {
        if let Some(dir) = self.models_dir.clone() {
            self.spawn(ctx, "neptune-video-models", move || {
                Job::Models(Inventory::look(&dir))
            });
        }
    }

    /// What the speech worker is doing, for the Captions tab.
    fn speech_view(&self) -> SpeechView {
        if let Some(heard) = &self.heard {
            return SpeechView::Ready {
                count: heard.count,
                language: heard.language.clone(),
                why: heard.why.to_owned(),
            };
        }
        let Some(speaking) = &self.speaking else {
            return SpeechView::Idle;
        };
        let progress = (self.speech.as_ref())
            .and_then(SpeechService::progress)
            .filter(|progress| progress.request == speaking.request);
        SpeechView::Working {
            stage: progress.map(|progress| progress.stage),
            done: progress.map_or(0.0, |progress| progress.done),
            stopping: speaking.stopping,
        }
    }

    /// Takes what the speech worker published. A model that came is heard
    /// with; words that came become one step, or wait.
    fn take_speech(&mut self, ctx: &egui::Context) {
        while let Some(event) = self.speech.as_ref().and_then(SpeechService::poll) {
            // An answer to a request this window no longer waits for is
            // nobody's.
            let Some(speaking) =
                (self.speaking).take_if(|speaking| speaking.request == event.request())
            else {
                continue;
            };
            match event {
                SpeechEvent::Downloaded { .. } => {
                    self.look_at_models(ctx);
                    if speaking.stopping {
                        self.notify(ctx, "Captions were not made.", false);
                    } else if let Err(why) = self.transcribe(speaking) {
                        self.notify(ctx, format!("Captions were not made: {why}"), true);
                    }
                }
                SpeechEvent::Transcribed {
                    words, language, ..
                } => self.take_words(ctx, &speaking, &words, language.as_deref()),
                SpeechEvent::Failed { message, .. } => {
                    self.notify(ctx, format!("Captions were not made: {message}"), true);
                }
                SpeechEvent::Cancelled { .. } => {
                    self.notify(ctx, "Captions were not made.", false);
                }
            }
        }
    }

    /// The words heard as one step: added now, or held while adding them
    /// would cost something.
    fn take_words(
        &mut self,
        ctx: &egui::Context,
        speaking: &Speaking,
        words: &[editor_core::captions::SpokenWord],
        heard: Option<&str>,
    ) {
        let Some((instance, revision)) = speaking.heard_at else {
            return;
        };
        if instance != self.editor.instance_id() {
            return;
        }
        let language = speech::caption_language(
            speaking.ask.language.as_deref(),
            speaking.ask.translate,
            heard,
        );
        let (command, track, count) =
            match speech::generated(self.editor.project(), words, &language, speaking.ask.length) {
                Ok(made) => made,
                Err(why) => {
                    self.notify(ctx, why, true);
                    return;
                }
            };
        let heard = Heard {
            command,
            track,
            count,
            language,
            why: "",
        };
        match heard_waits(
            self.editor.revision() != revision,
            self.editor.can_redo(),
            self.ai.open().is_some(),
        ) {
            Some(why) => {
                self.heard = Some(Heard { why, ..heard });
                self.notify(
                    ctx,
                    "Captions are ready. Add them in the Captions tab.",
                    false,
                );
            }
            None => self.add_heard(ctx, heard),
        }
    }

    /// The one edit captions from speech are.
    fn add_heard(&mut self, ctx: &egui::Context, heard: Heard) {
        self.commit_drafts(ctx, true);
        match self.edit(heard.command) {
            Ok(_) => {
                self.library.captions.track = Some(heard.track);
                let captions = match heard.count {
                    1 => "1 caption".to_owned(),
                    count => format!("{count} captions"),
                };
                let language = crate::languages::label(&heard.language);
                self.notify(ctx, format!("Added {captions} in {language}."), false);
            }
            Err(error) => {
                self.notify(ctx, format!("The captions were not added: {error}"), true);
            }
        }
    }

    fn take_media(&mut self, ctx: &egui::Context) {
        loop {
            let Some(media) = &self.media else {
                self.media_held.clear();
                return;
            };
            let generation = media.generation();
            // What the end of the last frame left comes first, as published.
            let Some(event) = self.media_held.pop_front().or_else(|| media.poll()) else {
                return;
            };
            match event {
                MediaEvent::Imported {
                    generation: from,
                    assets,
                    folders,
                    errors,
                } => {
                    if from != generation {
                        continue;
                    }
                    self.imports.pop_front();
                    // A file that is already in the library is not added twice.
                    let project = self.editor.project();
                    let offered = assets.len();
                    let assets: Vec<_> = assets
                        .into_iter()
                        .filter(|asset| {
                            !project.assets.values().any(|have| have.path == asset.path)
                        })
                        .collect();
                    let skipped = offered - assets.len();
                    let added = assets.len();
                    if added > 0 {
                        let mut commands: Vec<Command> = folders
                            .into_iter()
                            .map(|folder| Command::AddFolder { folder })
                            .collect();
                        commands.push(Command::AddAssets { assets });
                        if let Err(error) = self.edit(Command::Batch {
                            label: "Import media".into(),
                            commands,
                        }) {
                            self.notify(ctx, format!("The import failed: {error}"), true);
                        }
                    } else if skipped > 0 && errors.is_empty() {
                        self.notify(ctx, "That media is already in the library", false);
                    }
                    for error in errors.iter().take(3) {
                        let name = error.path.file_name().map_or_else(
                            || error.path.display().to_string(),
                            |name| name.to_string_lossy().into_owned(),
                        );
                        self.notify(ctx, format!("{name}: {}", error.message), true);
                    }
                    if errors.len() > 3 {
                        self.notify(
                            ctx,
                            format!("{} more files could not be imported", errors.len() - 3),
                            true,
                        );
                    }
                }
                MediaEvent::FrameReady {
                    generation: from,
                    asset_id,
                    time,
                    size,
                    consumer,
                    frame,
                } => {
                    // A frame of media that has left the library is dropped,
                    // and so is one whose consumer no longer draws that
                    // asset at that size.
                    if from == generation
                        && let Some(asset) = self.editor.project().assets.get(&asset_id)
                    {
                        self.textures.accept(
                            ctx,
                            Some(asset),
                            consumer,
                            asset_id,
                            time,
                            size,
                            &frame,
                        );
                    }
                }
                MediaEvent::LutLoaded {
                    generation: from,
                    asset_id,
                    path,
                    file_size,
                    table,
                } => {
                    // A table read before the worker started over, for an
                    // asset that has left the library or for the file it
                    // used to be of, is dropped. The frame drawn after this
                    // is drawn with the one that is kept.
                    if from != generation {
                        continue;
                    }
                    let asset = self.editor.project().assets.get(&asset_id);
                    let taken =
                        (self.viewer).take_lut(asset, asset_id, from, &path, file_size, table);
                    if let crate::compose::lut::Taken::Failed(why) = taken {
                        self.notify(ctx, why, true);
                    }
                }
                MediaEvent::Error {
                    generation: from,
                    asset_id,
                    consumer,
                    size,
                    message,
                } => {
                    if from != generation {
                        continue;
                    }
                    match asset_id {
                        // A failure for a layer that is no longer drawn, or
                        // no longer asks for that, is nobody's to hear of.
                        Some(asset) => {
                            if !self.textures.failed(consumer, asset, size) {
                                continue;
                            }
                        }
                        None => {
                            self.imports.pop_front();
                        }
                    }
                    self.notify(ctx, message, true);
                }
            }
        }
    }

    /// Takes what the proxy lane published, gives the viewer the person's
    /// preview choices, and tells the lane which proxies the viewer has open.
    fn take_proxies(&mut self, ctx: &egui::Context) {
        let project = self.editor.project();
        let taken = (self.proxies).take(project, &self.settings.preview, &mut self.textures);
        if taken.ready {
            ctx.request_repaint();
        }
        for failure in taken.failures {
            self.notify(ctx, failure, true);
        }
    }

    /// Hands the layers the frames the worker published while this frame was
    /// composed: see [`take_published_frames`]. An import or a failure it
    /// published is not this moment's, and waits for [`Self::take_media`].
    fn take_frames(&mut self, ctx: &egui::Context) {
        let Some(media) = &self.media else { return };
        let assets = &self.editor.project().assets;
        take_published_frames(
            ctx,
            media,
            &mut self.textures,
            &mut self.media_held,
            |asset| assets.get(&asset),
        );
    }

    fn take_saves(&mut self, ctx: &egui::Context) {
        while let Some(result) = self.saver.poll() {
            // A load starts the revisions again, so the path alone does not
            // say which document an acknowledgement is about.
            let mine = result.context_id == self.editor.instance_id()
                && result.project_id == self.editor.project().id;
            if result.recovery {
                let key = (result.context_id, result.path);
                if self.discarded.contains(&key) {
                    self.remove_recovery(ctx, key.1);
                } else if mine {
                    self.autosave
                        .answered(result.revision, result.result.is_ok(), Instant::now());
                    if let Err(error) = result.result {
                        let text = format!("The recovery copy was not written: {error}");
                        self.notify(ctx, text, true);
                    } else if std::mem::take(&mut self.recovery_stale) && !self.editor.is_dirty() {
                        self.remove_recovery(ctx, key.1);
                    }
                }
                continue;
            }
            let Some((session, asked)) = self
                .save_session
                .clone()
                .filter(|(session, _)| mine && session.path() == result.path)
            else {
                continue;
            };
            // An earlier save of the same file may answer while a later one
            // is still queued behind it.
            let agent = if result.revision >= asked {
                self.saving = false;
                self.save_session = None;
                let outcome = result
                    .result
                    .as_ref()
                    .map(|_| ())
                    .map_err(ToString::to_string);
                self.finished_save(&result.path, outcome);
                std::mem::take(&mut self.agent_saving)
            } else {
                false
            };
            if let Err(error) = result.result {
                self.after_save = None;
                match other_writer(&error) {
                    // An agent's save is told in a notice; the agent reads
                    // `conflicted` from `document`.
                    Some(busy) if agent => {
                        let open = (self.document.as_ref())
                            .is_some_and(|document| document.path() == result.path);
                        if open && !busy {
                            self.conflicted = true;
                        }
                        self.notify(ctx, format!("The agent's save was refused: {error}"), true);
                    }
                    // Reviewed in a sheet: the edits stay, unsaved.
                    Some(busy) => self.save_refused(result.path, busy),
                    None => self.notify(ctx, format!("The project was not saved: {error}"), true),
                }
                continue;
            }
            self.editor.mark_saved(result.revision);
            // The project may have just gained a file, or moved to another.
            let old = self.recovery_base();
            self.settings.remember(&result.path);
            if self.path.as_ref() != Some(&result.path) {
                // Another file: its copy is asked for beside it first.
                self.private_recovery = false;
            }
            self.path = Some(result.path.clone());
            self.document = Some(session);
            // The work has a file of its own now, saved like any other.
            self.origin = Origin::Ordinary;
            self.conflicted = false;
            self.unsaved_base = None;
            let home = self.recovery_base();
            if let Some(old) = old.filter(|old| Some(old) != home.as_ref()) {
                if self.owns(&old) {
                    self.discard_recovery(ctx, result.context_id, old);
                }
                self.lease = None;
            }
            self.notify(ctx, "Saved", false);
            if self.editor.is_dirty() {
                // Edits made while the file was being written are not in it.
                // They are kept in a fresh recovery copy, and whatever waited
                // on the save asks about them again.
                self.write_recovery(ctx);
                if let Some(pending) = self.after_save.take() {
                    self.overlay = Overlay::Unsaved(pending);
                }
                continue;
            }
            // The recovery copy describes work that is now in the file. One
            // still being written is removed when it lands.
            if self.autosave.pending {
                self.recovery_stale = true;
            } else if let Some(home) = home.filter(|home| self.owns(home)) {
                self.remove_recovery(ctx, home);
            }
            if let Some(pending) = self.after_save.take() {
                self.proceed(ctx, pending);
            }
        }
    }

    /// Requests from connected agents, applied on this thread through the
    /// same reducer as every other edit.
    fn take_agent_calls(&mut self, ctx: &egui::Context) {
        loop {
            let Some(call) = self.host.as_ref().and_then(AgentHost::poll) else {
                self.publish_document();
                return;
            };
            let what = match &call.request.operation {
                AgentOperation::Inspect => None,
                AgentOperation::Capabilities => None,
                AgentOperation::History | AgentOperation::ChangesSince { .. } => None,
                AgentOperation::Document | AgentOperation::Save { .. } => {
                    self.document_operation(ctx, call);
                    continue;
                }
                AgentOperation::Apply { command } => Some(command.label().to_owned()),
                AgentOperation::Undo => Some("Undo".to_owned()),
                AgentOperation::Redo => Some("Redo".to_owned()),
                AgentOperation::ExportCapabilities
                | AgentOperation::QueueExport { .. }
                | AgentOperation::RenderJobs
                | AgentOperation::CancelRender { .. }
                | AgentOperation::PauseRender { .. }
                | AgentOperation::ResumeRender { .. }
                | AgentOperation::RetryRender { .. }
                | AgentOperation::RemoveRender { .. } => {
                    self.render_operation(call);
                    continue;
                }
            };
            // This window's own agent shows its steps itself.
            let own = ai::own_request(&self.agent_tag, &call.request.request_id);
            // Only changes wait for the person; reading the project never does.
            if let Some(what) = &what
                && let Some((code, message)) = self.agent_refusal()
            {
                let request_id = call.request.request_id.clone();
                let revision = self.editor.revision();
                call.reply(editor_agent::AgentResponse::error(
                    request_id, revision, code, message,
                ));
                if !own {
                    self.agent.push(Entry::External {
                        ok: false,
                        text: format!("Agent asked for “{what}”; held back while you edit"),
                    });
                }
                continue;
            }
            let before = self.editor.revision();
            call.execute(&mut self.editor);
            let changed = self.editor.revision() != before;
            if changed {
                self.document_changed();
                if own {
                    self.live.count_edit();
                }
            }
            if own {
                continue;
            }
            if let Some(what) = what {
                self.agent.push(Entry::External {
                    ok: changed,
                    text: if changed {
                        format!("Agent: {what}")
                    } else {
                        format!("Agent asked for “{what}”; nothing changed")
                    },
                });
            }
        }
    }

    /// The document as a connected agent sees it. The host answers `document`
    /// from what is published here, so an agent watching for changes never
    /// wakes the window.
    fn document_info(&self) -> editor_agent::DocumentInfo {
        let path = (self.document.as_ref())
            .map(|document| document.path().to_path_buf())
            .or_else(|| self.path.clone());
        let mut info = editor_agent::DocumentInfo::of(&self.editor, path);
        info.saving = self.saving || self.preparing;
        info.conflicted = self.conflicted;
        info.playhead = Some(self.playback.position);
        info.selection = self.selection.clips.clone();
        info.file_version = self
            .document
            .as_ref()
            .and_then(|document| document.version());
        info.editing = self.editing();
        info.asking = self.asking();
        info.save_count = self.save_count;
        info.last_save = self.last_save.clone();
        info
    }

    /// The person holds a change that is not an edit yet.
    fn editing(&self) -> bool {
        self.draft.is_some() || self.project_draft != ProjectDraft::default()
    }

    /// The window waits on the person's answer about this document.
    fn asking(&self) -> bool {
        matches!(
            self.overlay,
            Overlay::Unsaved(_) | Overlay::Conflict(_) | Overlay::StopRendering
        )
    }

    /// Why an agent's edit or save must wait, if it must: the person's
    /// gesture would overwrite it, their answer could discard it, or a save
    /// is writing the document.
    fn agent_refusal(&self) -> Option<(&'static str, &'static str)> {
        use editor_agent::codes;
        if self.editing() {
            Some((
                codes::PERSON_EDITING,
                "The person is changing something in the window; retry when they finish",
            ))
        } else if self.asking() {
            Some((
                codes::DESKTOP_BUSY,
                "The window is asking the person about this document; retry later",
            ))
        } else if self.saving || self.preparing {
            Some((
                codes::DESKTOP_BUSY,
                "The window is saving the document; retry once it is saved",
            ))
        } else {
            None
        }
    }

    fn finished_save(&mut self, path: &Path, outcome: Result<(), String>) {
        self.save_count += 1;
        self.last_save = Some(editor_agent::SaveOutcome {
            ok: outcome.is_ok(),
            path: path.to_path_buf(),
            message: outcome.err().unwrap_or_default(),
        });
    }

    fn publish_document(&self) {
        if let Some(host) = &self.host {
            host.publish_document(self.editor.revision(), self.document_info());
        }
    }

    /// `document` and `save` from a connected agent, answered at once while
    /// the requester waits. A save starts as the person's does, drafts
    /// committed and the file written by the save worker; the answer says
    /// `saving`, and a later `document` says whether it is still dirty or in
    /// conflict. A destination is a new file only: an agent never replaces
    /// a file with Save As.
    fn document_operation(&mut self, ctx: &egui::Context, call: editor_agent::AgentCall) {
        let Some(call) = call.claim() else {
            return;
        };
        let started = match &call.request.operation {
            AgentOperation::Save { destination } => {
                let started = self.agent_save(ctx, &call.request, destination.clone());
                self.agent.push(Entry::External {
                    ok: started.is_ok(),
                    text: match &started {
                        Ok(()) => "Agent: Save".to_owned(),
                        Err(_) => "Agent asked to save; nothing was saved".to_owned(),
                    },
                });
                started
            }
            _ => Ok(()),
        };
        let result = match started {
            Ok(()) => editor_agent::AgentResult::Document {
                document: Box::new(self.document_info()),
            },
            Err(refused) => refused,
        };
        let request_id = call.request.request_id.clone();
        call.reply(editor_agent::AgentResponse {
            request_id,
            revision: self.editor.revision(),
            result,
        });
        self.publish_document();
    }

    fn agent_save(
        &mut self,
        ctx: &egui::Context,
        request: &editor_agent::AgentRequest,
        destination: Option<PathBuf>,
    ) -> Result<(), editor_agent::AgentResult> {
        use editor_agent::{AgentResult, codes};
        editor_agent::check_identity(&self.editor, request)?;
        if self.saving || self.preparing {
            return Err(AgentResult::error(
                codes::SAVE_BUSY,
                "A save is already under way; ask for the document again",
            ));
        }
        if let Some((code, message)) = self.agent_refusal() {
            return Err(AgentResult::error(code, message));
        }
        let Some(path) = destination else {
            return match self.document.clone() {
                Some(session) => {
                    self.agent_saving = true;
                    self.save_through(ctx, session);
                    Ok(())
                }
                None => Err(AgentResult::error(
                    codes::NO_DESTINATION,
                    "This project has no file yet; send a destination for a new one",
                )),
            };
        };
        if !path.is_absolute() || path.extension().is_none_or(|extension| extension != "nvp") {
            return Err(AgentResult::error(
                codes::INVALID_EDIT,
                format!("{} must be an absolute path ending in .nvp", path.display()),
            ));
        }
        if std::fs::symlink_metadata(&path).is_ok() {
            return Err(AgentResult::error(
                codes::DESTINATION_EXISTS,
                format!(
                    "{} exists; an agent saves only to a new file",
                    path.display()
                ),
            ));
        }
        self.agent_saving = true;
        self.prepare_save(ctx, path, true);
        Ok(())
    }

    /// Render operations are answered from the render queue, never through
    /// the reducer, and only while their requester still waits: each is
    /// claimed, answered at once from the queue's state and replied to
    /// exactly once. A queued export is of the project as the agent
    /// inspected it, drafts not committed, and is said in the agent panel.
    fn render_operation(&mut self, call: editor_agent::AgentCall) {
        let Some(call) = call.claim() else {
            return;
        };
        let presets = &self.export_presets;
        let result = crate::export::wiring::render_result(
            &self.renders,
            &self.editor,
            &call.request,
            || crate::export::wiring::offered_presets(presets),
            self.path.clone(),
        );
        if let (AgentOperation::QueueExport { .. }, editor_agent::AgentResult::RenderJob { job }) =
            (&call.request.operation, &result)
            && !ai::own_request(&self.agent_tag, &call.request.request_id)
        {
            self.agent.push(Entry::External {
                ok: true,
                text: format!("Agent: Export {}", job.name),
            });
        }
        let request_id = call.request.request_id.clone();
        call.reply(editor_agent::AgentResponse {
            request_id,
            revision: self.editor.revision(),
            result,
        });
    }

    // ---- Export -----------------------------------------------------------

    /// Opens the export sheet over nothing but the render queue, drafts
    /// committed first. A sheet closed unfinished for this project comes
    /// back as it was left. Otherwise it starts from the settings and preset
    /// last used, over the whole timeline, writing a file named after the
    /// project into the folder the last export went to, the project's,
    /// Videos or home.
    fn open_export(&mut self, ctx: &egui::Context) {
        if !matches!(self.overlay, Overlay::None | Overlay::RenderQueue) {
            return;
        }
        self.commit_drafts(ctx, true);
        let context = self.editor.instance_id();
        self.export_replace = None;
        if let Some(mut draft) = (self.export.take()).filter(|draft| draft.owner() == Some(context))
        {
            draft.reopened();
            self.export = Some(draft);
            self.overlay = Overlay::Export;
            return;
        }
        let settings = crate::export::wiring::last_settings(self.settings.export_last.as_ref());
        let name = crate::export::wiring::file_stem(&self.editor.project().name);
        let user = directories::UserDirs::new();
        let videos = (user.as_ref())
            .and_then(directories::UserDirs::video_dir)
            .map(Path::to_path_buf);
        let home = user.as_ref().map(|user| user.home_dir().to_path_buf());
        let folder = crate::export::wiring::initial_folder(
            self.settings.export_folder.as_deref(),
            self.path.as_deref(),
            videos,
            home,
            Path::is_dir,
        );
        self.export = Some(
            ui::export_sheet::ExportSheetState::new(settings, name, folder)
                .with_owner(context)
                .with_preset(self.settings.export_preset.clone()),
        );
        self.overlay = Overlay::Export;
    }

    /// Draws the export sheet from the queue's capabilities, the presets,
    /// the selection and the playhead, with the poster of the output there,
    /// and does what it answers; under a question asked over it
    /// (`answering` false) it is only drawn.
    fn export_sheet(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        project: &Arc<Project>,
        answering: bool,
    ) {
        use crate::export::queue::CapabilityState;
        use ui::export_sheet::{CapabilityView, ExportSheetInput};
        let Some(mut state) = self.export.take() else {
            if answering {
                self.overlay = Overlay::None;
            }
            return;
        };
        let poster = self.poster.show(
            ctx,
            project,
            (self.editor.instance_id(), self.editor.revision()),
            state.settings(),
            self.playback.position,
        );
        let caps = self.renders.capabilities();
        let view = match &caps {
            CapabilityState::Probing => CapabilityView::Probing,
            CapabilityState::Ready(caps) => CapabilityView::Ready(caps),
            CapabilityState::Failed(why) => CapabilityView::Failed(why),
        };
        let presets = crate::export::wiring::offered_presets(&self.export_presets);
        // What is known without reading anything: camera paths not measured.
        let steady = &self.viewer.steady;
        let warnings = crate::export::wiring::warnings(project, state.settings(), |asset| {
            matches!(
                steady.status(asset),
                crate::steady::Status::NotAnalysed
                    | crate::steady::Status::Stale
                    | crate::steady::Status::Failed(_)
            )
        });
        let input = ExportSheetInput {
            project,
            caps: view,
            presets: &presets,
            selection: crate::export::wiring::selection_span(project, &self.selection.clips),
            playhead: self.playback.position,
            poster,
            poster_problem: self.poster.problem(),
            warnings: &warnings,
            queue_len: self.renders.jobs().len(),
        };
        let answer = ui::export_sheet::show(ctx, p, &mut state, &input);
        self.export = Some(state);
        if let Some(answer) = answer.filter(|_| answering) {
            self.answer_export(ctx, answer);
        }
    }

    fn answer_export(&mut self, ctx: &egui::Context, answer: ui::export_sheet::ExportAnswer) {
        use ui::export_sheet::ExportAnswer;
        match answer {
            // What was set is kept for the sheet to be opened again.
            ExportAnswer::Close => self.overlay = Overlay::None,
            ExportAnswer::Queue {
                settings,
                destination,
                start_now,
            } => {
                // A file there, the export or its caption file, is replaced
                // only when the person says so.
                let there = crate::export::wiring::replaced_files(
                    self.editor.project(),
                    &settings,
                    &destination,
                    crate::export::wiring::file_there,
                );
                let mut names = there.iter().map(|path| {
                    path.file_name().map_or_else(
                        || path.display().to_string(),
                        |name| name.to_string_lossy().into_owned(),
                    )
                });
                if let Some(file) = names.next() {
                    self.export_replace = Some((settings, destination, start_now, names.next()));
                    self.overlay = Overlay::ReplaceExport(file);
                } else {
                    self.queue_export(ctx, settings, destination, start_now);
                }
            }
            ExportAnswer::ChooseFolder => self.pick(ctx, Pick::ExportFolder),
            ExportAnswer::SavePreset { name, settings } => {
                match self.export_presets.keep(&name, &settings) {
                    Ok(_) => self.save_export_presets(ctx),
                    Err(why) => self.refuse_in_export(ctx, why),
                }
            }
            ExportAnswer::DeletePreset(id) => {
                if self.export_presets.remove(&id).is_some() {
                    self.save_export_presets(ctx);
                }
            }
            ExportAnswer::Reprobe => self.renders.reprobe(),
            ExportAnswer::OpenQueue => self.overlay = Overlay::RenderQueue,
        }
    }

    /// Queues an export of the project as it is now, drafts committed. The
    /// sheet closes and its choices are remembered once the queue takes it;
    /// a refusal is said and the sheet stays open as it was.
    fn queue_export(
        &mut self,
        ctx: &egui::Context,
        settings: editor_core::export::ExportSettings,
        destination: PathBuf,
        start_now: bool,
    ) {
        self.commit_drafts(ctx, true);
        let job = crate::export::wiring::new_job(
            self.editor.snapshot(),
            self.editor.instance_id(),
            self.editor.revision(),
            settings.clone(),
            destination.clone(),
            start_now,
            self.path.clone(),
        );
        match self.renders.enqueue(job) {
            Ok(_) => {
                self.settings.export_last = crate::export::wiring::remembered(&settings);
                self.settings.export_folder = destination.parent().map(Path::to_path_buf);
                self.settings.export_preset =
                    (self.export.take()).and_then(|state| state.preset().map(str::to_owned));
                if start_now {
                    self.overlay = Overlay::RenderQueue;
                } else {
                    self.overlay = Overlay::None;
                    let file = destination
                        .file_name()
                        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
                    self.notify(
                        ctx,
                        format!("{file} is held in the render queue; start it there"),
                        false,
                    );
                }
            }
            Err(error) => {
                self.overlay = Overlay::Export;
                self.refuse_in_export(ctx, error.to_string());
            }
        }
    }

    /// Says why the export sheet's request was refused inside the sheet,
    /// where the person is looking: a toast waits under an open sheet that
    /// leaves it no room, as in a short window. With no sheet open it is a
    /// problem notice.
    fn refuse_in_export(&mut self, ctx: &egui::Context, why: String) {
        match self.export.as_mut() {
            Some(state) if matches!(self.overlay, Overlay::Export | Overlay::ReplaceExport(_)) => {
                state.refuse(why);
            }
            _ => self.notify(ctx, why, true),
        }
    }

    fn answer_queue(
        &mut self,
        ctx: &egui::Context,
        answer: ui::render_queue::QueueAnswer,
        jobs: &[editor_core::export::RenderJobInfo],
    ) {
        use ui::render_queue::QueueAnswer;
        // A refusal answers what was last asked; anything asked next
        // replaces it.
        self.queue_refusal = None;
        let renders = &self.renders;
        // A job whose state changed since the panel was drawn refuses the
        // operation; the panel shows the new state.
        match answer {
            QueueAnswer::Close => self.overlay = Overlay::None,
            QueueAnswer::Pause(id) => _ = renders.pause(id),
            QueueAnswer::Resume(id) => _ = renders.resume(id),
            QueueAnswer::Cancel(id) => _ = renders.cancel(id),
            QueueAnswer::Retry(id) => _ = renders.retry(id),
            QueueAnswer::Remove(id) => _ = renders.remove(id),
            QueueAnswer::MoveUp(id) => _ = renders.move_by(id, -1),
            QueueAnswer::MoveDown(id) => _ = renders.move_by(id, 1),
            QueueAnswer::ClearFinished => renders.clear_finished(),
            QueueAnswer::StartAll => _ = renders.resume_all(),
            QueueAnswer::Reveal(id) => {
                let file = jobs.iter().find(|job| job.id == id);
                if let Some(file) = file
                    && let Err(why) = crate::export::wiring::reveal(&file.destination)
                {
                    // Said in the queue, which is open: a toast would wait
                    // under it in a short window.
                    if self.overlay == Overlay::RenderQueue {
                        self.queue_refusal = Some(why);
                    } else {
                        self.notify(ctx, why, true);
                    }
                }
            }
            QueueAnswer::NewExport => self.open_export(ctx),
        }
    }

    /// Tells of exports that finished: a short notice for a file made, a
    /// problem notice for a failure, which also stays in the queue. While
    /// the queue is open its rows tell it.
    fn take_renders(&mut self, ctx: &egui::Context) {
        let open = self.overlay == Overlay::RenderQueue;
        while let Some(event) = self.renders.poll() {
            if let Some((text, problem)) = crate::export::wiring::finished_notice(&event, open) {
                self.notify(ctx, text, problem);
            }
        }
    }

    /// Asks what quitting asks next, or quits.
    fn ask_to_quit(&mut self, ctx: &egui::Context) {
        use crate::export::wiring::{CloseQuestion, close_question};
        self.commit_drafts(ctx, true);
        match close_question(
            self.renders.busy(),
            self.stop_renders,
            self.editor.is_dirty(),
        ) {
            CloseQuestion::Rendering => self.overlay = Overlay::StopRendering,
            CloseQuestion::Unsaved => self.overlay = Overlay::Unsaved(Pending::Quit),
            CloseQuestion::None => self.proceed(ctx, Pending::Quit),
        }
    }

    /// Writes the saved export presets on a worker. Writes land in the
    /// order they were asked for: one overtaken by a newer write is not
    /// made.
    fn save_export_presets(&mut self, ctx: &egui::Context) {
        let Some(path) = self.export_presets_path.clone() else {
            return;
        };
        self.export_presets_asked += 1;
        let asked = self.export_presets_asked;
        let presets = self.export_presets.clone();
        let written = Arc::clone(&self.export_presets_written);
        self.spawn(ctx, "neptune-video-export-presets", move || {
            let mut writes = written
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            Job::Background(match writes.save(&presets, &path, asked) {
                None | Some(Ok(None)) => Ok(String::new()),
                Some(Ok(Some(kept))) => Ok(format!(
                    "The export presets file that could not be read was kept as {}.",
                    kept.display()
                )),
                Some(Err(error)) => Err(format!("Export presets were not saved: {error}")),
            })
        });
    }

    // ---- Drafts -----------------------------------------------------------

    /// Turns held values into commands once the interaction that made them
    /// has ended, so that a drag or a typed value is one undo step.
    fn commit_drafts(&mut self, ctx: &egui::Context, force: bool) {
        let held = (self.draft_hold).holds(ctx, &self.draft, &self.project_draft);
        if self.draft.is_none() && self.project_draft == ProjectDraft::default() {
            return;
        }
        let stale = self
            .draft
            .as_ref()
            .is_some_and(|draft| self.selection.single() != Some(draft.clip));
        if held && !force && !stale {
            // The event that ends the interaction (a button let go, a Return,
            // a field left) draws the frame that looks again.
            return;
        }
        self.draft_hold.forget();

        if let Some(draft) = self.draft.take() {
            let result = match draft.command(self.editor.project()) {
                None => Ok(String::new()),
                Some(command) => self.edit(command),
            };
            if let Err(error) = result {
                self.notify(ctx, format!("That change was not applied: {error}"), true);
            }
        }
        let project_draft = std::mem::take(&mut self.project_draft);
        if let Some(name) = project_draft.name {
            let name = name.trim().to_owned();
            if !name.is_empty()
                && name != self.editor.project().name
                && let Err(error) = self.edit(Command::RenameProject { name })
            {
                self.notify(ctx, error, true);
            }
        }
        if let Some(settings) = project_draft.canvas
            && settings != self.editor.project().settings
        {
            if !settings.validate() {
                self.notify(ctx, "That canvas size or frame rate cannot be used", true);
            } else if let Err(error) = self.edit(Command::SetSettings { settings }) {
                self.notify(ctx, error, true);
            }
        }
    }

    // ---- Actions ----------------------------------------------------------

    fn run(&mut self, ctx: &egui::Context, action: Action) {
        if let Err(problem) = self.perform(ctx, action) {
            self.notify(ctx, problem, true);
        }
    }

    /// Carries out one action. `Ok` says what was done when there is
    /// something to say; `Err` says why it was not.
    fn perform(&mut self, ctx: &egui::Context, action: Action) -> Result<Option<String>, String> {
        let rate = self.editor.project().settings.frame_rate;
        match action {
            Action::NewProject => self.guard(ctx, Pending::New),
            Action::OpenProject => self.guard(ctx, Pending::Open(None)),
            Action::OpenRecent(path) => self.guard(ctx, Pending::Open(Some(path))),
            // A project whose file was never read here has nothing to
            // compare that file with, and is asked where to go.
            Action::Save => match self.document.clone() {
                Some(session) => self.save_through(ctx, session),
                None => self.pick(ctx, Pick::SaveAs),
            },
            Action::SaveAs => self.pick(ctx, Pick::SaveAs),
            Action::DuplicateProject => {
                if self.duplicating {
                    return Err("A duplicate is still being written.".to_owned());
                }
                self.pick(ctx, Pick::Duplicate);
            }
            Action::DuplicateTo(path) => {
                if self.duplicating {
                    return Err("A duplicate is still being written.".to_owned());
                }
                self.duplicate_to(ctx, with_extension(path));
            }
            Action::SaveCheckpoint(name) => {
                self.checkpoint_home()?;
                self.checkpoint_job(ctx, Some(name));
            }
            Action::RestoreCheckpoint(name) => self.restore_named(ctx, name)?,
            Action::OpenCheckpoints => {
                self.overlay = Overlay::Checkpoints;
                self.checkpoints.list = None;
                self.checkpoints.focus = true;
                self.checkpoint_job(ctx, None);
            }
            Action::ImportFiles => self.pick(ctx, Pick::ImportFiles),
            Action::ImportFolder => self.pick(ctx, Pick::ImportFolder),
            Action::ImportFonts => self.pick(ctx, Pick::ImportFonts),
            Action::ImportCaptions => self.pick(ctx, Pick::ImportCaptions),
            Action::ExportCaptions { track, format } => {
                self.commit_drafts(ctx, true);
                captions_tab::exported(self.editor.project(), track, format)?;
                self.pick(ctx, Pick::ExportCaptions(track, format));
            }
            Action::OpenSettings => self.overlay = Overlay::Settings,
            Action::OpenPreferences(pane) => {
                pane.show(ctx);
                self.overlay = Overlay::Settings;
            }
            // The command line, or the request to the model, as Ctrl+K and
            // Shift+Ctrl+K open them.
            Action::OpenAgent { ask } => {
                self.settings.show_inspector = true;
                self.library_first = false;
                self.settings.agent_ask = ask;
                self.right_tab = RightTab::Agent;
                self.agent.focus = true;
            }
            Action::ResetLayout => reset_layout(&mut self.settings),
            Action::SetReduceMotion(on) => {
                self.settings.reduce_motion = on;
                kit::set_reduced_motion(ctx, on);
                self.restyle(ctx);
            }
            Action::DismissNotice(id) => self.notices.retain(|notice| notice.id() != id),
            Action::OpenExport => self.open_export(ctx),
            // The export sheet's state is kept for it to be opened again.
            Action::OpenRenderQueue => {
                if matches!(self.overlay, Overlay::None | Overlay::Export) {
                    self.overlay = Overlay::RenderQueue;
                }
            }

            Action::Edit(command) => {
                self.commit_drafts(ctx, true);
                return self.edit(command).map(Some);
            }
            Action::Undo => {
                self.commit_drafts(ctx, true);
                let label = self.editor.undo_label().map(str::to_owned);
                if !self.editor.undo() {
                    return Err("There is nothing to undo.".to_owned());
                }
                self.document_changed();
                return Ok(label.map(|label| format!("Undid “{label}”")));
            }
            Action::Redo => {
                self.commit_drafts(ctx, true);
                let label = self.editor.redo_label().map(str::to_owned);
                if !self.editor.redo() {
                    return Err("There is nothing to redo.".to_owned());
                }
                self.document_changed();
                return Ok(label.map(|label| format!("Redid “{label}”")));
            }
            Action::Copy => {
                self.copy(ctx)?;
                // The next paste is of these clips, not of keyframes.
                self.timeline.keys.forget_copied();
                return Ok(Some("Copied".to_owned()));
            }
            Action::Cut => {
                self.copy(ctx)?;
                self.timeline.keys.forget_copied();
                let clip_ids = std::mem::take(&mut self.selection.clips);
                return self
                    .edit(Command::DeleteClips {
                        clip_ids,
                        ripple: false,
                    })
                    .map(|_| Some("Cut".to_owned()));
            }
            Action::Paste => {
                let clipboard = self.clipboard.clone().ok_or("Nothing has been copied.")?;
                return self.paste(&clipboard, self.playback.position).map(Some);
            }
            Action::PasteClips(clipboard) => {
                let done = self.paste(&clipboard, self.playback.position);
                // The menu pastes the same clips from here on.
                self.clipboard = Some(clipboard);
                return done.map(Some);
            }
            Action::Duplicate => {
                if self.selection.clips.is_empty() {
                    return Err("Select the clips to duplicate first.".to_owned());
                }
                let clipboard = self
                    .editor
                    .copy(&self.selection.clips)
                    .map_err(|error| error.to_string())?;
                // The copy goes straight after what it was made from.
                let after = clipboard
                    .clips
                    .iter()
                    .map(|item| item.clip.end())
                    .max()
                    .unwrap_or(Time::ZERO);
                return self
                    .paste(&clipboard, after)
                    .map(|_| Some("Duplicated".to_owned()));
            }
            Action::Delete { ripple } => {
                if self.selection.clips.is_empty() {
                    return Err("Select the clips to delete first.".to_owned());
                }
                let clip_ids = std::mem::take(&mut self.selection.clips);
                return self
                    .edit(Command::DeleteClips { clip_ids, ripple })
                    .map(Some);
            }
            Action::SplitAtPlayhead => return self.split().map(Some),
            Action::DetachAudio(clip) => {
                // A volume still held in a field is the one the sound takes.
                self.commit_drafts(ctx, true);
                return self.detach_audio(clip).map(Some);
            }
            Action::Normalize(clips) => {
                // Effects held in a field are the ones the sound is measured
                // through.
                self.commit_drafts(ctx, true);
                return self.normalize(ctx, &clips).map(Some);
            }
            Action::Duck { clips, depth } => {
                // A volume held in a field is the one the dips are under.
                self.commit_drafts(ctx, true);
                return self.duck(ctx, clips, depth);
            }
            Action::CancelDuck => self.stop_ducking(ctx),
            Action::ShowBeats { assets, shown } => {
                // The tiles that show them are drawn again, and want them.
                ctx.request_repaint();
                return Ok(Some(self.show_beats(&assets, shown)));
            }
            Action::MarkersFromBeats(clip) => {
                let command = self.peaks.beats.markers(self.editor.project(), clip)?;
                return self.edit(command).map(Some);
            }
            Action::SaveAudio(source) => self.pick_audio_file(ctx, source)?,
            Action::CancelSaveAudio(request) => self.cancel_saved_audio(ctx, request),
            Action::AddSavedAudio(request) => {
                // The file is imported as any other: one "Import media" step.
                if let Some(to) = self.saved_audio.take_offer(request)
                    && let Some(problem) = self.import(vec![to])
                {
                    return Err(problem);
                }
            }
            Action::DismissSavedAudio(request) => {
                self.saved_audio.take_offer(request);
            }
            Action::SeparateStems(clip) => {
                // A volume still held in a field is the one the stems take.
                self.commit_drafts(ctx, true);
                return self.separate_stems(ctx, clip).map(Some);
            }
            Action::CancelSeparate(request) => self.cancel_separate(ctx, request),
            Action::AddTransitions { places, kind } => {
                self.commit_drafts(ctx, true);
                let saved = &self.settings.transitions;
                match transitions::asked(self.editor.project(), saved, &places, kind)? {
                    // Asked for where there is one already, the one that is
                    // there is shown.
                    transitions::Asked::Show(clip, edge) => {
                        self.timeline.mark(&mut self.selection, clip, edge);
                    }
                    transitions::Asked::Make(command, made) => {
                        let done = self.edit(*command)?;
                        // The one that was made is the one the inspector shows.
                        if let Some((clip, edge)) = made {
                            self.timeline.mark(&mut self.selection, clip, edge);
                        }
                        return Ok(Some(done));
                    }
                }
            }
            Action::SaveTransitionDefault(transition) => {
                let text = format!(
                    "New {} transitions start like this one",
                    transitions::name(transition.kind).to_lowercase()
                );
                self.settings.transitions =
                    transitions::saved_with(&self.settings.transitions, transition);
                self.notify(ctx, text, false);
            }
            Action::SelectAll => {
                self.selection.asset = None;
                self.selection.clips = self
                    .editor
                    .project()
                    .tracks
                    .iter()
                    .flat_map(|track| track.clips.iter().map(|clip| clip.id))
                    .collect();
            }
            Action::AddMarker => {
                return self
                    .edit(timeline::marker_at(self.playback.position))
                    .map(Some);
            }
            Action::AddTrack(kind) => {
                let count = self
                    .editor
                    .project()
                    .tracks
                    .iter()
                    .filter(|track| track.kind == kind)
                    .count();
                let track = Track::new(format!("{} {}", kind_name(kind), count + 1), kind);
                return self.edit(Command::AddTrack { track }).map(Some);
            }
            Action::AddText => return self.add_look(text_tab::Look::typed(), None, None).map(Some),
            Action::AddLook { look, track, at } => {
                self.commit_drafts(ctx, true);
                return self.add_look(&look, track, at).map(Some);
            }
            Action::Presets(change) => {
                if let Some(said) = self.change_presets(change)? {
                    self.notify(ctx, said, false);
                }
            }
            Action::InsertAsset { asset, place } => {
                return self.insert_asset(asset, place).map(Some);
            }
            Action::AddBackgroundMusic(asset) => {
                // A length still held in a field is part of what the bed
                // has to cover.
                self.commit_drafts(ctx, true);
                return self.background_music(asset).map(Some);
            }
            Action::RemoveAsset(asset) => {
                // The textures follow in `media_changed`.
                let (command, said) = remove_asset_command(self.editor.project(), asset);
                let done = self.edit(command)?;
                if let Some(said) = said {
                    self.notify(ctx, said, false);
                }
                return Ok(Some(done));
            }
            Action::PreviewAsset(asset) => {
                self.held_by_take("preview media")?;
                self.open_source(ctx, asset)?;
            }
            Action::CloseSource => self.close_source(),
            Action::Source(transport) => {
                let source = self.source.as_mut().ok_or("No media is being previewed.")?;
                let at = source.playback.position;
                let sync = source.apply(transport)?;
                // The transport moved the playhead, not the clock.
                if source.playback.position != at {
                    self.textures.jumped();
                }
                if sync {
                    self.sync_source_audio();
                }
            }

            // The timeline's transport. Asking it for anything turns the
            // viewer back to the timeline.
            Action::TogglePlay => {
                // Space ends a take as `Action::StopRecord`; the play button,
                // K, L, a double-click and the panel do not.
                self.held_by_take("play or pause")?;
                self.close_source();
                if self.playback.playing {
                    self.playback.pause();
                } else {
                    let duration = self.editor.project().duration();
                    if duration == Time::ZERO {
                        return Err("There is nothing on the timeline to play.".to_owned());
                    }
                    if self.playback.position >= duration {
                        self.playback.seek(Time::ZERO);
                        self.textures.jumped();
                    }
                    self.playback.play();
                }
                self.sync_audio();
            }
            Action::Seek(time) => {
                self.held_by_take("move the playhead")?;
                self.close_source();
                let at = self.playback.position;
                self.playback.seek(time);
                // Held on the ruler, the same seek comes every frame: only
                // one that moved the playhead gives up what was asked for.
                if self.playback.position != at {
                    self.textures.jumped();
                }
                if self.playback.playing {
                    self.sync_audio();
                }
            }
            Action::Step(frames) => {
                self.held_by_take("move the playhead")?;
                self.close_source();
                let was_playing = self.playback.playing;
                let at = self.playback.position;
                self.playback.step(frames, rate);
                if self.playback.position != at {
                    self.textures.jumped();
                }
                if was_playing {
                    self.sync_audio();
                }
                self.timeline.follow(self.playback.position);
            }
            Action::JumpStart => {
                self.held_by_take("move the playhead")?;
                self.close_source();
                let at = self.playback.position;
                self.playback.seek(Time::ZERO);
                if self.playback.position != at {
                    self.textures.jumped();
                }
                self.timeline.follow(Time::ZERO);
                if self.playback.playing {
                    self.sync_audio();
                }
            }
            Action::JumpEnd => {
                self.held_by_take("move the playhead")?;
                self.close_source();
                let end = self.editor.project().duration();
                let at = self.playback.position;
                self.playback.pause();
                self.playback.seek(end);
                if self.playback.position != at {
                    self.textures.jumped();
                }
                self.timeline.follow(end);
                self.sync_audio();
            }
            Action::SetSpeed(speed) => {
                self.held_by_take("change the speed")?;
                self.playback
                    .set_speed(speed)
                    .map_err(|error| error.to_string())?;
                if self.playback.playing {
                    self.sync_audio();
                }
            }
            Action::ToggleLoop => self.looping = !self.looping,
            Action::Record => {
                self.commit_drafts(ctx, true);
                self.record(ctx)?;
            }
            Action::StopRecord => self.stop_recording(),

            Action::ZoomTimeline(factor) => self.timeline.zoom_centered(factor),
            Action::ScaleInterface(asked) => {
                let asked = asked.clamp(*INTERFACE_SCALE.start(), *INTERFACE_SCALE.end());
                // No larger than leaves the editor the least room it is
                // laid out for in this window.
                let room = ui::largest_scale(ctx);
                let scale = asked.min(room);
                self.settings.interface_scale = scale;
                ctx.set_zoom_factor(scale);
                // One capsule says the size, however many steps were taken.
                ui::window::say_interface_size(ctx, scale, asked > room);
            }
            Action::FitTimeline => self.timeline.fit = true,
            Action::ToggleSnapping => self.settings.snapping = !self.settings.snapping,
            // A panel that is on but has given its room to the other one in
            // a narrow window is brought forward, not turned off unseen.
            Action::ToggleLibrary => {
                let width = ctx.content_rect().width() - 2.0 * metrics::GUTTER;
                let (shown, _) = self.side_panels(width);
                self.settings.show_library = !shown;
                self.library_first = !shown;
            }
            Action::ToggleInspector => {
                let width = ctx.content_rect().width() - 2.0 * metrics::GUTTER;
                let (_, shown) = self.side_panels(width);
                self.settings.show_inspector = !shown;
                self.library_first &= shown;
            }
            Action::ShowLibrary(tab) => {
                self.settings.show_library = true;
                self.library_first = true;
                self.library.tab = tab;
            }
            Action::ToggleKeyLane => {
                let project = self.editor.project();
                if !self.timeline.keys.can_toggle_lane(project, &self.selection) {
                    return Err(
                        "Select a clip that has keyframes or sound to see its keyframes."
                            .to_owned(),
                    );
                }
                self.timeline.keys.toggle_lane(project, &self.selection);
            }
            Action::ToggleGraph => self.timeline.keys.graph = !self.timeline.keys.graph,
            Action::SetAppearance(appearance) => {
                self.settings.appearance = appearance;
                self.restyle(ctx);
            }
            Action::SetAccent(accent) => {
                self.settings.accent = accent;
                self.restyle(ctx);
            }
            Action::SetAutosave(on) => self.settings.autosave = on,
            Action::SetPreviewQuality(quality) => self.settings.preview.set_quality(quality),
            Action::SetProxyMode(mode) => {
                self.settings.preview.set_proxies(mode);
                let project = self.editor.project();
                (self.proxies).mode_changed(project, &self.settings.preview, &mut self.textures);
            }
            Action::SetProxyBudget(gib) => {
                self.settings.preview.set_proxy_budget_gib(gib);
                self.proxies.set_budget(&self.settings.preview);
            }
            Action::CreateProxy(asset) => {
                if let Some(asset) = self.editor.project().assets.get(&asset) {
                    self.proxies.create(asset, &mut self.textures)?;
                }
            }
            Action::RemoveProxy(asset) => {
                if let Some(asset) = self.editor.project().assets.get(&asset) {
                    self.proxies.remove(asset, &mut self.textures);
                }
            }
            Action::ClearProxies => {
                let project = self.editor.project();
                self.proxies.clear(project, &mut self.textures);
            }

            Action::AgentSubmit(line) => self.agent_submit(ctx, line),
            Action::AiSubmit(prompt) => self.ai_submit(ctx, prompt),
            Action::AiCancel => {
                if self.live.turn().is_some() {
                    self.live.stop();
                } else {
                    self.ai.cancel();
                }
            }
            Action::SetAiReview(review) => self.settings.ai.review = review,
            Action::AddAgentFolder => self.pick(ctx, Pick::AgentFolder),
            Action::RemoveAgentFolder(index) => {
                if index < self.settings.ai.folders.len() {
                    self.settings.ai.folders.remove(index);
                }
            }
            Action::AiNewConversation => {
                self.live.end();
                self.agent.push(Entry::Note(
                    "New conversation. The agent remembers nothing of the last one.".to_owned(),
                ));
            }
            Action::AiApply(id) => return self.ai_apply(ctx, id).map(Some),
            Action::AiDiscard(id) => {
                if self.ai.discard(id) {
                    self.agent.settle(id, PlanState::Discarded);
                }
            }
            Action::SetAgentAsk(ask) => {
                self.settings.agent_ask = ask;
                self.agent.focus = true;
            }
            Action::SetAiProvider(provider) => self.settings.ai.provider = provider,
            Action::SetAiEffort(effort) => self.settings.ai.effort = effort,
            Action::SetAiModel(model) => self.settings.ai.set_model(&model),
            Action::Notify(text) => self.notify(ctx, text, false),
            Action::Track(request) => self.start_tracking(ctx, request)?,
            Action::GenerateCaptions(request) => self.generate_captions(ctx, request)?,
            Action::CancelSpeech => self.cancel_speech(),
            Action::AddHeardCaptions => {
                if let Some(heard) = self.heard.take() {
                    self.add_heard(ctx, heard);
                }
            }
            Action::DiscardHeardCaptions => self.heard = None,
            Action::SetSpeech(settings) => self.settings.speech = settings,
            Action::PickSpeechModel => self.pick(ctx, Pick::SpeechModel),
            Action::LookAtModels => self.look_at_models(ctx),
            Action::CaptionsFromSpeech => {
                let (start, end) = captions_tab::Stretch::Timeline
                    .times(self.editor.project(), &self.selection)
                    .ok_or("The timeline is empty.")?;
                let settings = &self.settings.speech;
                let request = SpeechRequest {
                    start,
                    end,
                    model: settings.choice(),
                    language: settings.language.clone(),
                    translate: settings.translate,
                    length: settings.length,
                };
                self.generate_captions(ctx, request)?;
            }
            Action::TranslateCaptions { track_id, language } => {
                self.translate_captions(ctx, track_id, &language)?;
            }
            Action::CancelTracking => self.cancel_tracking(),
            Action::AddTracked => self.add_ready_tracker(ctx),
            Action::Stabilize(asset) => self.start_steady(ctx, asset)?,
            Action::CancelStabilize => self.cancel_steady(),
        }
        Ok(None)
    }

    /// Styles the window for the appearance and accent chosen. System is
    /// the appearance the platform reports, and dark where it reports none.
    fn restyle(&mut self, ctx: &egui::Context) {
        let appearance = self.settings.appearance.resolve(ctx.system_theme());
        self.palette = Palette::new(appearance, self.settings.accent);
        theme::apply(ctx, &self.palette);
    }

    fn copy(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.selection.clips.is_empty() {
            return Err("Select the clips to copy first.".to_owned());
        }
        let clipboard = self
            .editor
            .copy(&self.selection.clips)
            .map_err(|error| error.to_string())?;
        // The system clipboard gets them too: the window hears of a paste
        // only when there is text there, and another window can take them.
        if let Some(text) = clipboard::encode(&clipboard) {
            ctx.copy_text(text);
        }
        self.clipboard = Some(clipboard);
        Ok(())
    }

    /// Pastes at `at`, or after the clips already there when the copies
    /// would land on them.
    fn paste(&mut self, clipboard: &Clipboard, at: Time) -> Result<String, String> {
        let project = self.editor.project();
        // Clips copied in another project may use media this one lacks.
        if clipboard::lacks_media(clipboard, &project.assets) {
            return Err("The copied clips use media that is not in this project.".to_owned());
        }
        let before: Vec<ClipId> = project
            .tracks
            .iter()
            .flat_map(|track| track.clips.iter().map(|clip| clip.id))
            .collect();
        let tracks: Vec<&Track> = clipboard
            .clips
            .iter()
            .filter_map(|item| project.tracks.get(item.track_index))
            .map(AsRef::as_ref)
            .collect();
        let collides = clipboard.clips.iter().any(|item| {
            let start = at + (item.clip.start - clipboard.origin);
            let end = start + item.clip.duration;
            project.tracks.get(item.track_index).is_some_and(|track| {
                track
                    .clips
                    .iter()
                    .any(|clip| clip.start < end && start < clip.end())
            })
        });
        let at = if collides {
            tracks
                .iter()
                .flat_map(|track| track.clips.iter().map(Clip::end))
                .max()
                .unwrap_or(at)
        } else {
            at
        };
        let outcome = self
            .editor
            .paste(clipboard, at, None)
            .map_err(|error| error.to_string())?;
        if outcome.changed {
            self.document_changed();
            // What was pasted becomes the selection.
            self.selection.clips = self
                .editor
                .project()
                .tracks
                .iter()
                .flat_map(|track| track.clips.iter().map(|clip| clip.id))
                .filter(|id| !before.contains(id))
                .collect();
            self.selection.asset = None;
        }
        Ok(outcome.label)
    }

    /// Splits the selected clips under the playhead, or every unlocked clip
    /// under it when none of the selection is.
    fn split(&mut self) -> Result<String, String> {
        let project = self.editor.project();
        let at = rules::quantize(self.playback.position, project.settings.frame_rate);
        let selected = self.selection.clips.to_vec();
        let command = rules::split_command(project, at, &selected)?;
        self.edit(command)
    }

    /// Adds a clip of `look` and selects it: on `track` from `at`, or at
    /// `at` (the playhead when not given) on a text track with room there.
    fn add_look(
        &mut self,
        look: &text_tab::Look,
        track: Option<editor_core::TrackId>,
        at: Option<Time>,
    ) -> Result<String, String> {
        let at = at.unwrap_or(self.playback.position);
        let (command, clip) = text_tab::adding(self.editor.project(), look, track, at);
        let done = self.edit(command)?;
        self.selection.only(clip);
        Ok(done)
    }

    /// Changes the person's presets; they are written at the end of the
    /// frame. Says what was done when there is something to say.
    fn change_presets(&mut self, change: PresetChange) -> Result<Option<String>, String> {
        let presets = Arc::make_mut(&mut self.presets);
        match change {
            PresetChange::Keep {
                name,
                content,
                position,
            } => {
                let id = presets.keep(&name, *content, position)?;
                let name = (presets.looks.iter())
                    .find(|look| look.id == id)
                    .map_or(name, |look| look.name.clone());
                Ok(Some(format!(
                    "Kept “{name}”: it is under Yours in the library's Text tab."
                )))
            }
            PresetChange::Rename(id, name) => {
                if presets.rename(id, &name) {
                    Ok(None)
                } else {
                    Err("A preset needs a name.".into())
                }
            }
            PresetChange::Remove(id) => Ok(presets
                .remove(id)
                .map(|look| format!("Removed the preset “{}”.", look.name))),
            PresetChange::AddColor(color) => {
                if presets.add_color(color) {
                    Ok(None)
                } else if presets.colors.contains(&color) {
                    Err("That colour is already one of your brand colours.".into())
                } else {
                    Err(format!(
                        "There are already {} brand colours: remove one to add another.",
                        presets::MOST_COLORS
                    ))
                }
            }
            PresetChange::RemoveColor(color) => {
                presets.colors.retain(|kept| *kept != color);
                Ok(None)
            }
        }
    }

    fn insert_asset(
        &mut self,
        asset: editor_core::AssetId,
        place: Option<(editor_core::TrackId, Time)>,
    ) -> Result<String, String> {
        let (command, clip) = rules::place_asset(self.editor.project(), asset, place)?;
        let done = self.edit(command)?;
        self.selection.only(clip);
        Ok(done)
    }

    /// Puts the sound of `clip` on an audio track, linked to it. The picture
    /// clip stays the one selected.
    fn detach_audio(&mut self, clip: ClipId) -> Result<String, String> {
        let command = rules::detach_audio(self.editor.project(), clip)?;
        self.edit(command)
    }

    /// Lays the sound `asset` under the project as it is now, on a track of
    /// its own. The selection stays where it was.
    fn background_music(&mut self, asset: AssetId) -> Result<String, String> {
        let command = rules::background_music(self.editor.project(), asset)?;
        self.edit(command)
    }

    fn agent_submit(&mut self, ctx: &egui::Context, line: String) {
        let line = line.trim().to_owned();
        self.agent.push(Entry::Asked(line.clone()));
        match line.to_lowercase().as_str() {
            "help" | "?" => {
                self.agent.push(Entry::Help);
                return;
            }
            "clear" => {
                self.agent.log.clear();
                return;
            }
            _ => {}
        }
        let project = self.editor.snapshot();
        let planned = agent::interpret(
            &line,
            &agent::Context {
                project: &project,
                selection: &self.selection,
                playhead: self.playback.position,
                readings: self.viewer.readings(),
            },
        );
        let entry = match planned {
            Err(problem) => Entry::Answer {
                ok: false,
                lines: vec![problem],
            },
            Ok(actions) => {
                let mut lines = Vec::new();
                let mut failed = None;
                for action in actions {
                    match self.perform(ctx, action) {
                        Ok(Some(done)) => lines.push(done),
                        Ok(None) => {}
                        Err(problem) => {
                            failed = Some(problem);
                            break;
                        }
                    }
                }
                match failed {
                    Some(problem) => Entry::Answer {
                        ok: false,
                        lines: vec![problem],
                    },
                    None => Entry::Answer {
                        ok: true,
                        lines: if lines.is_empty() {
                            vec!["Done".to_owned()]
                        } else {
                            lines
                        },
                    },
                }
            }
        };
        self.agent.push(entry);
    }

    /// Sends `prompt` to the model with the document as it stands. The
    /// answer arrives as an event; nothing is edited by asking.
    fn ai_submit(&mut self, ctx: &egui::Context, prompt: String) {
        if !self.settings.ai.review {
            self.agent_converse(ctx, prompt);
            return;
        }
        let selected = self.selection.clips.clone();
        self.ai_ask(ctx, prompt.clone(), prompt, &selected);
    }

    /// Sends `prompt` to the agent that edits as it works. It reaches this
    /// window through `neptune-video-mcp`, started in the session the window
    /// publishes, and its edits come back as requests like any connected
    /// agent's. Nothing is waited for here, except a changed folders file
    /// before a conversation starts.
    fn agent_converse(&mut self, ctx: &egui::Context, prompt: String) {
        let prompt = prompt.trim().to_owned();
        if prompt.is_empty() {
            return;
        }
        // A value still held in a field is part of what is being asked about.
        self.commit_drafts(ctx, true);
        self.agent.push(Entry::Prompt(prompt.clone()));
        let asked = self.tool_server().and_then(|server| {
            if let Some(session) = self.session_path.clone() {
                // While it talks the file follows every change already;
                // a server about to start must find it written.
                let wanted = self.agent_roots();
                let starting = !self.live.follows_roots();
                self.live.set_roots(&roots_file(&session), wanted, starting);
            }
            let told = self.window_note(&prompt);
            self.live.ask(&self.settings.ai, server, told)
        });
        if let Err(problem) = asked {
            self.agent.push(Entry::Answer {
                ok: false,
                lines: vec![problem],
            });
        }
    }

    /// The request with what the window shows: the playhead and selection
    /// the person sees, which the agent cannot see for itself.
    fn window_note(&self, prompt: &str) -> String {
        let rate = self.editor.project().settings.frame_rate;
        let seconds = self.playback.position.as_seconds();
        let frame = (seconds * rate.numerator as f64 / rate.denominator.max(1) as f64).round();
        let selected = if self.selection.clips.is_empty() {
            "no clips are selected".to_owned()
        } else {
            format!(
                "the selected clips are {}",
                (self.selection.clips.iter())
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        format!(
            "[In the window, the playhead is at {seconds:.3} s (frame {frame}) and {selected}.]\n\n{prompt}"
        )
    }

    /// `neptune-video-mcp`, started in this window, with the folders it may use.
    fn tool_server(&self) -> Result<editor_agent::ToolServer, String> {
        let session = self.session_path.as_deref().ok_or_else(|| {
            format!(
                "The agent reaches the project through this window's agent session, which \
                 is not open: {}",
                self.session_problem
                    .as_deref()
                    .unwrap_or("it could not be published")
            )
        })?;
        let command = ai::tool_program()?;
        let text = |path: &Path| {
            path.to_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{} cannot be passed to the agent", path.display()))
        };
        // The folders are in a file the server reads again whenever it
        // changes, so a new one does not end the conversation.
        let mut args = vec![
            "--window".to_owned(),
            text(session)?,
            "--tag".to_owned(),
            self.agent_tag.clone(),
            "--roots-file".to_owned(),
            text(&roots_file(session))?,
        ];
        if let Some(settings) = &self.settings_path {
            args.push("--desktop-settings".to_owned());
            args.push(text(settings)?);
        }
        if let Some(models) = &self.models_dir {
            args.push("--models".to_owned());
            args.push(text(models)?);
        }
        Ok(editor_agent::ToolServer { command, args })
    }

    /// The folders the agent may read and write: the project's, its
    /// media's, those the person added, and the person's usual places.
    /// The server leaves out any that is gone, too wide or inside another.
    /// Never the home
    /// folder, a drive or a system folder.
    fn agent_roots(&self) -> Vec<PathBuf> {
        let project = self.editor.project();
        let mut wanted: Vec<PathBuf> = Vec::new();
        if let Some(path) = (self.document.as_ref())
            .map(|document| document.path().to_path_buf())
            .or_else(|| self.path.clone())
            && let Some(parent) = path.parent()
        {
            wanted.push(parent.to_path_buf());
        }
        let mut seen = std::collections::HashSet::new();
        for asset in project.assets.values() {
            if let Some(parent) = asset.path.parent()
                && seen.insert(parent)
            {
                wanted.push(parent.to_path_buf());
            }
        }
        wanted.extend(self.settings.ai.folders.iter().cloned());
        wanted.extend(usual_folders().iter().cloned());
        wanted
    }

    /// Keeps the agent's roots file up to date while it talks: the person
    /// imports from a new folder, adds one in Settings, or saves the
    /// project somewhere else.
    fn follow_agent_roots(&mut self) {
        if !self.live.follows_roots() {
            return;
        }
        let Some(session) = self.session_path.clone() else {
            return;
        };
        // Only an edit, a save elsewhere or Settings changes them.
        let basis = (
            self.editor.revision(),
            self.path.clone(),
            self.settings.ai.folders.clone(),
        );
        if self.agent_roots_basis.as_ref() == Some(&basis) {
            return;
        }
        self.agent_roots_basis = Some(basis);
        let wanted = self.agent_roots();
        self.live.set_roots(&roots_file(&session), wanted, false);
    }

    /// Asks the model `prompt` about `selected`, whose clips are in its
    /// context whole, whatever else is left out. The panel shows `shown`.
    fn ai_ask(&mut self, ctx: &egui::Context, shown: String, prompt: String, selected: &[ClipId]) {
        let prompt = prompt.trim().to_owned();
        if prompt.is_empty() {
            return;
        }
        // A value still held in a field is part of what is being asked about.
        self.commit_drafts(ctx, true);
        self.agent.push(Entry::Prompt(shown.trim().to_owned()));
        // One proposal is open at a time; asking again sets the last aside.
        if let Some(open) = self.ai.open()
            && self.ai.discard(open)
        {
            self.agent.settle(open, PlanState::Discarded);
        }
        let asked = self.ai.ask(
            &self.settings.ai,
            &self.editor,
            selected,
            self.playback.position,
            prompt,
        );
        if let Err(problem) = asked {
            self.agent.push(Entry::Answer {
                ok: false,
                lines: vec![problem],
            });
        }
    }

    /// Asks the model for the captions of `track_id` in `language`. The
    /// request and its proposal show in the Agent tab, where the copy is
    /// reviewed and applied as one step.
    fn translate_captions(
        &mut self,
        ctx: &egui::Context,
        track_id: editor_core::TrackId,
        language: &str,
    ) -> Result<(), String> {
        let project = self.editor.project();
        let track = (project.tracks.iter())
            .find(|track| track.id == track_id)
            .ok_or("That caption track is gone.")?;
        let (prompt, captions) = captions_tab::translation(self.editor.project(), track, language)?;
        let shown = format!(
            "Translate “{}” into {}",
            track.name,
            crate::languages::label(language)
        );
        self.settings.show_inspector = true;
        self.library_first = false;
        self.settings.agent_ask = true;
        self.right_tab = RightTab::Agent;
        self.ai_ask(ctx, shown, prompt, &captions);
        Ok(())
    }

    /// What the model's runtime reported, shown as it was reported.
    fn take_ai(&mut self) {
        self.follow_agent_roots();
        while let Some(heard) = self.live.poll() {
            match heard {
                ai::Heard::Started { restarted } => {
                    if restarted {
                        self.agent.push(Entry::Note(
                            "A new conversation started, because the agent's provider, model or \
                             effort changed. It remembers nothing of the last one."
                                .to_owned(),
                        ));
                    }
                }
                ai::Heard::Said(text) => self.agent.push(Entry::Said(text)),
                ai::Heard::Called { call, what } => self.agent.push(Entry::Step(Step {
                    call,
                    what,
                    state: StepState::Running,
                })),
                ai::Heard::Returned { call, ok, detail } => self.agent.step_ended(
                    &call,
                    if ok {
                        StepState::Done(detail)
                    } else {
                        StepState::Refused(detail)
                    },
                ),
                ai::Heard::Finished {
                    by,
                    edits,
                    elsewhere,
                } => {
                    self.agent.steps_stopped();
                    let mut note = match edits {
                        // A change may be an edit or the agent's own undo,
                        // so both ways back are named.
                        0 if elsewhere > 0 => format!("{by} · no changes here"),
                        0 => format!("{by} · no changes"),
                        1 => format!("{by} · 1 change, which undo or redo takes back"),
                        n => format!("{by} · {n} changes; undo and redo take them back"),
                    };
                    if elsewhere > 0 {
                        note.push_str(&elsewhere_note(elsewhere));
                    }
                    self.agent.push(Entry::Note(note));
                }
                ai::Heard::Failed(message) => {
                    self.agent.steps_stopped();
                    self.agent.push(Entry::Answer {
                        ok: false,
                        lines: vec![message],
                    });
                }
                ai::Heard::Cancelled { edits, elsewhere } => {
                    self.agent.steps_stopped();
                    let mut note = match edits {
                        0 if elsewhere > 0 => "Stopped. Nothing was changed here.".to_owned(),
                        0 => "Stopped. Nothing was changed.".to_owned(),
                        1 => "Stopped. The change already made stays; undo or redo takes it back."
                            .to_owned(),
                        n => format!(
                            "Stopped. The {n} changes already made stay; undo and redo take them back."
                        ),
                    };
                    if elsewhere > 0 {
                        note.push_str(&elsewhere_note(elsewhere));
                    }
                    self.agent.push(Entry::Note(note));
                }
            }
        }
        while let Some(update) = self.ai.poll() {
            self.agent.push(match update {
                ai::Update::Planned(plan) => Entry::Plan(PlanCard {
                    state: if plan.steps.is_empty() {
                        PlanState::Empty
                    } else {
                        PlanState::Open
                    },
                    plan,
                }),
                ai::Update::Failed(message) => Entry::Answer {
                    ok: false,
                    lines: vec![message],
                },
                ai::Update::Cancelled => Entry::Note("Request cancelled".to_owned()),
            });
        }
    }

    /// Applies an open proposal: one step of the history, or nothing.
    fn ai_apply(&mut self, ctx: &egui::Context, id: uuid::Uuid) -> Result<String, String> {
        // A held value is an edit of its own, made first. It moves the
        // document on, and the proposal then refuses, as it should.
        self.commit_drafts(ctx, true);
        match self.ai.apply(id, &mut self.editor) {
            Ok(applied) => {
                if applied == ai::Applied::Changed {
                    self.document_changed();
                    let end = self.editor.project().duration();
                    if self.playback.position > end {
                        self.playback.seek(end);
                        self.sync_audio();
                    }
                }
                let (state, said) = settled(applied);
                self.agent.settle(id, state);
                Ok(said.to_owned())
            }
            Err(problem) => {
                self.agent
                    .settle(id, PlanState::Closed(format!("Not applied: {problem}")));
                Err(format!("The proposal was not applied: {problem}"))
            }
        }
    }

    // ---- Keyboard ---------------------------------------------------------

    fn shortcuts(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        // Whether a choice's cell has the keyboard, followed in every frame.
        let choosing = ui::ChoiceArrows::begin(ctx);
        // Export and the render queue, which open over nothing else.
        if let Some(action) = ctx.input_mut(|i| export_chords(i, &self.overlay)) {
            actions.push(action);
        }
        let typing = ctx.text_edit_focused();
        // Before Cmd+O, which would take Shift+Cmd+O for its own.
        if let Some(action) = ctx.input_mut(|i| panel_chords(i, typing)) {
            actions.push(action);
        }
        for index in ctx.input_mut(|i| chords_struck(i, typing)) {
            let ids = self.selection.clips.clone();
            actions.push(match index {
                0 => Action::Redo,
                1 => Action::Undo,
                2 => Action::SaveAs,
                3 => Action::Save,
                4 => Action::OpenProject,
                5 => Action::NewProject,
                6 => Action::ImportFolder,
                7 => Action::ImportFiles,
                8 => Action::Duplicate,
                9 => Action::SelectAll,
                10 => Action::Edit(Command::UngroupClips { clip_ids: ids }),
                11 => Action::Edit(Command::GroupClips { clip_ids: ids }),
                12 => Action::OpenSettings,
                // The command line, or the request to the model.
                13 | 17 => Action::OpenAgent { ask: index == 17 },
                14 if self.source.is_some() => Action::Source(Transport::ToggleLoop),
                14 => Action::ToggleLoop,
                // The selected clips and the playhead of this press: each
                // clip's edge nearer the playhead.
                18 => Action::AddTransitions {
                    places: transitions::at_playhead(
                        self.editor.project(),
                        &ids,
                        self.playback.position,
                    ),
                    kind: None,
                },
                _ => continue,
            });
        }
        // The interface's scale answers wherever the keyboard is, in a
        // field or over a sheet, as a browser's does.
        let scale = self.settings.interface_scale.min(ui::largest_scale(ctx));
        if let Some(scaled) = ctx.input_mut(|i| scaled_by_keys(i, scale)) {
            actions.push(Action::ScaleInterface(scaled));
        }
        // An Escape in a field gives up what was typed into it and not yet
        // committed, as it gives up a drag below, and does no more than
        // leave the field. The field had the keyboard in the frame before:
        // egui has taken it away by the time this one is drawn.
        let left_field =
            self.inspector.typed(ctx) && ctx.input(|i| struck(&i.events, Key::Escape, Held::Alone));
        if left_field {
            self.inspector.cancel(&mut self.draft);
        }
        if typing || self.overlay != Overlay::None {
            return;
        }

        // Keyframes selected in the timeline take the arrows, Delete and the
        // clipboard chords, and edit themselves when the timeline is drawn.
        let keyed = self.timeline.keys.owns_keys(ctx, &self.selection);
        // The clipboard chords arrive as events rather than keys.
        let mut other_paste = false;
        ctx.input(|i| {
            for event in &i.events {
                match event {
                    egui::Event::Copy if !keyed => actions.push(Action::Copy),
                    egui::Event::Cut if !keyed => actions.push(Action::Cut),
                    // The keyframes last copied here are pasted by the
                    // timeline, while the clipboard still holds them.
                    egui::Event::Paste(text) if self.timeline.keys.pastes(text) => {}
                    egui::Event::Paste(text) => {
                        other_paste = true;
                        // Text that is not a copy of clips is not for the
                        // timeline.
                        if let Some(clips) = clipboard::decode(text) {
                            actions.push(Action::PasteClips(clips));
                        }
                    }
                    _ => {}
                }
            }
        });
        // Something else was copied since the keyframes were, here or in
        // another window: they are not what a later paste is of.
        if other_paste {
            self.timeline.keys.forget_copied();
        }

        // A key is read with the modifiers of its own event. Those the
        // frame ends with may be others: a whole Shift+→ can arrive in one
        // frame, and by its end Shift is up again.
        // The arrows and Delete are a curve point's while one has the
        // keyboard: it moves and is removed with them, and nothing else is.
        // So are Left and Right while a choice of cells has the keyboard.
        let owned = self.inspector.owns_keys(ctx) || keyed;
        let free = move |key| key_is_free(owned, key) && !choice_takes(choosing, key);
        let pressed = |key| free(key) && ctx.input(|i| struck(&i.events, key, Held::Alone));
        let with_shift = |key| free(key) && ctx.input(|i| struck(&i.events, key, Held::Shift));
        // A key that steps the playhead steps it once for every time it went
        // down, however many of those one frame was handed.
        let steps = |key, held, frames: i64| {
            let presses = match free(key) {
                true => ctx.input(|i| strikes(&i.events, key, held)),
                false => 0,
            };
            (presses > 0).then(|| frames.saturating_mul(presses as i64))
        };
        // R asks for a take, or ends the one under way: the timeline is what
        // records, whatever the viewer shows.
        if let Some(action) = record_key(ctx, self.recording.busy()) {
            actions.push(action);
        }
        // While the source monitor is up, the transport keys are its own and
        // the keys that edit at the timeline's playhead are not heard.
        if let Some(source) = &self.source {
            let second = source.project.settings.frame_rate.fps().round().max(1.0) as i64;
            let playing = source.playback.playing;
            let speed = source.playback.speed;
            if pressed(Key::Escape) {
                actions.push(Action::CloseSource);
                return;
            }
            // A still has no transport to drive.
            if source.asset.kind == MediaKind::Image {
                return;
            }
            let mut ask = |transport| actions.push(Action::Source(transport));
            if pressed(Key::Space) {
                ask(Transport::TogglePlay);
            }
            if let Some(frames) = steps(Key::ArrowLeft, Held::Alone, -1) {
                ask(Transport::Step(frames));
            }
            if let Some(frames) = steps(Key::ArrowRight, Held::Alone, 1) {
                ask(Transport::Step(frames));
            }
            if pressed(Key::Home) {
                ask(Transport::JumpStart);
            }
            if pressed(Key::End) {
                ask(Transport::JumpEnd);
            }
            if let Some(frames) = steps(Key::J, Held::Alone, -second) {
                ask(Transport::Step(frames));
            }
            if pressed(Key::K) && playing {
                ask(Transport::TogglePlay);
            }
            if pressed(Key::L) {
                if playing {
                    ask(Transport::SetSpeed((speed * 2.0).min(8.0)));
                } else {
                    ask(Transport::SetSpeed(1.0));
                    ask(Transport::TogglePlay);
                }
            }
            if let Some(frames) = steps(Key::ArrowLeft, Held::Shift, -second) {
                ask(Transport::Step(frames));
            }
            if let Some(frames) = steps(Key::ArrowRight, Held::Shift, second) {
                ask(Transport::Step(frames));
            }
            return;
        }
        let rate = self.editor.project().settings.frame_rate;
        let second = rate.fps().round().max(1.0) as i64;
        if pressed(Key::Enter)
            && let Some(asset) = self.selection.asset
            && self.previewable(asset)
        {
            actions.push(Action::PreviewAsset(asset));
        }
        if pressed(Key::Space) {
            actions.extend(play_key(Key::Space, &self.playback, self.recording.busy()));
        }
        if let Some(frames) = steps(Key::ArrowLeft, Held::Alone, -1) {
            actions.push(Action::Step(frames));
        }
        if let Some(frames) = steps(Key::ArrowRight, Held::Alone, 1) {
            actions.push(Action::Step(frames));
        }
        if pressed(Key::Home) {
            actions.push(Action::JumpStart);
        }
        if pressed(Key::End) {
            actions.push(Action::JumpEnd);
        }
        if pressed(Key::S) {
            actions.push(Action::SplitAtPlayhead);
        }
        // A clip is selected from the keyboard: the one under the playhead,
        // or the next one over from the clip selected last. It is brought
        // into view with the focus ring, and the playhead is put on it when
        // it was elsewhere, so that what is keyed or cut next is this clip.
        let reaches = [
            (Key::C, Held::Alone, Reach::AtPlayhead),
            (Key::ArrowLeft, Held::Alt, Reach::Previous),
            (Key::ArrowRight, Held::Alt, Reach::Next),
            (Key::ArrowUp, Held::Alt, Reach::Above),
            (Key::ArrowDown, Held::Alt, Reach::Below),
        ];
        for (key, held, reach) in reaches {
            // An arrow a curve point or a keyframe has is its own with Alt
            // held too: it moves the point and reaches for no clip.
            if !free(key) || !ctx.input(|i| struck(&i.events, key, held)) {
                continue;
            }
            let project = self.editor.project();
            let playhead = self.playback.position;
            let from =
                (self.selection.clips.last().copied()).filter(|id| project.clip(*id).is_some());
            let found =
                timeline::reach(project, from, playhead, reach).and_then(|id| project.clip(id));
            match found {
                Some((_, clip)) => {
                    let on = clip.start <= playhead && playhead < clip.end();
                    let (id, start) = (clip.id, clip.start);
                    self.selection.only(id);
                    self.timeline.point_at(id);
                    if !on {
                        actions.push(Action::Seek(start));
                    }
                }
                None => actions.push(Action::Notify(
                    match reach {
                        Reach::AtPlayhead => "No clip is under the playhead.",
                        Reach::Previous => "No clip comes before this one on its track.",
                        Reach::Next => "No clip comes after this one on its track.",
                        Reach::Above => "No clip is on a track above.",
                        Reach::Below => "No clip is on a track below.",
                    }
                    .to_owned(),
                )),
            }
        }
        if pressed(Key::F) {
            // The clip and the playhead of this press are the ones frozen.
            let project = self.editor.project();
            let playhead = self.playback.position;
            let frozen = speed::freeze_target(project, self.selection.single(), playhead)
                .ok_or_else(|| "Put the playhead on a video clip to freeze a frame.".to_owned())
                .and_then(|clip| {
                    speed::freeze_command(project, clip, playhead, self.inspector.freeze_hold())
                });
            actions.push(match frozen {
                Ok(command) => Action::Edit(command),
                Err(reason) => Action::Notify(reason),
            });
        }
        if pressed(Key::M) {
            actions.push(Action::AddMarker);
        }
        if pressed(Key::T) {
            actions.push(Action::AddText);
        }
        if pressed(Key::N) {
            actions.push(Action::ToggleSnapping);
        }
        if pressed(Key::G) {
            self.timeline.keys.graph = !self.timeline.keys.graph;
        }
        if pressed(Key::U) {
            let project = self.editor.project();
            self.timeline.keys.toggle_lane(project, &self.selection);
        }
        if let Some(frames) = steps(Key::J, Held::Alone, -second) {
            actions.push(Action::Step(frames));
        }
        for key in [Key::K, Key::L] {
            if pressed(key) {
                actions.extend(play_key(key, &self.playback, self.recording.busy()));
            }
        }
        if pressed(Key::Delete) || pressed(Key::Backspace) {
            // A transition whose marking is selected is what is removed, and
            // its clip stays.
            let marked = (self.timeline).marked(self.editor.project(), &self.selection);
            if let Some((clip, edge)) = marked {
                actions.push(Action::Edit(transitions::set_command(clip, edge, None)));
            } else if !self.selection.clips.is_empty() {
                actions.push(Action::Delete { ripple: false });
            }
        }
        // A menu or a picker that is open is the innermost thing an Escape
        // closes, and it closes itself.
        if pressed(Key::Escape) && !left_field && !egui::Popup::is_any_open(ctx) {
            // A value held by the pointer in the inspector is a drag too.
            let held = InspectorState::holds(ctx, &self.draft);
            // So is a key colour about to be picked off the canvas, and a
            // region being chosen there for a tracker.
            let picking = self.inspector.end_pick();
            let choosing = self.inspector.track_region(&self.selection).is_some()
                && self.inspector.end_region();
            let cancelled =
                self.timeline.cancel_drag() | self.viewer.cancel() | held | picking | choosing;
            if cancelled {
                self.inspector.cancel(&mut self.draft);
            } else if self.recording.busy() {
                // A take is ended before anything else is let go.
                actions.push(Action::StopRecord);
            } else if self.ducking.busy() {
                // A duck reading the others' sound is let go before the
                // selection it was made of.
                actions.push(Action::CancelDuck);
            } else if !self.timeline.unmark() {
                // A transition's marking is let go before its clip is.
                self.selection.clear();
            }
        }
        if let Some(frames) = steps(Key::ArrowLeft, Held::Shift, -second) {
            actions.push(Action::Step(frames));
        }
        if let Some(frames) = steps(Key::ArrowRight, Held::Shift, second) {
            actions.push(Action::Step(frames));
        }
        if with_shift(Key::Z) {
            actions.push(Action::FitTimeline);
        }
        // With Cmd or Ctrl these scale the interface; alone, the timeline.
        if pressed(Key::Equals) || pressed(Key::Plus) {
            actions.push(Action::ZoomTimeline(1.4));
        }
        if pressed(Key::Minus) {
            actions.push(Action::ZoomTimeline(1.0 / 1.4));
        }
        if (with_shift(Key::Delete) || with_shift(Key::Backspace))
            && !self.selection.clips.is_empty()
        {
            actions.push(Action::Delete { ripple: true });
        }
    }

    // ---- Screenshot (verification) ----------------------------------------

    fn capture(&mut self, ctx: &egui::Context) {
        let Some((path, at, asked)) = &mut self.screenshot else {
            return;
        };
        if !*asked {
            let now = Instant::now();
            if now < *at {
                ctx.request_repaint_after(*at - now);
                return;
            }
            *asked = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            return;
        }
        let image = ctx.input(|i| {
            i.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let saved = image::save_buffer(
                &*path,
                image.as_raw(),
                image.width() as u32,
                image.height() as u32,
                image::ColorType::Rgba8,
            );
            if let Err(error) = saved {
                eprintln!("neptune-video: cannot write {}: {error}", path.display());
            }
            self.exit_approved = true;
            self.screenshot = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    // ---- Layout -----------------------------------------------------------

    fn window_ui(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let ctx = ui.ctx().clone();
        let p = self.palette;
        let full = ui.max_rect();
        ui.painter()
            .rect_filled(full, ui::window::corner_radius(&ctx), p.chrome);
        let project = self.editor.snapshot();
        self.query
            .get_or_insert_with(|| ProjectQuery::new(Arc::clone(&project)))
            .update(Arc::clone(&project));
        self.textures.begin_frame();
        self.viewer.begin_frame();
        self.peaks.begin_frame();
        self.viewer.titles.poll(&ctx);
        self.show_steady(&project);
        // Layers given up since the last frame are no longer decoded for.
        self.textures.retire(self.media.as_ref());

        let toolbar = Rect::from_min_size(full.min, vec2(full.width(), metrics::TOOLBAR_HEIGHT));
        let gutter = metrics::GUTTER;
        let (show_library, show_right) = self.side_panels(full.width() - 2.0 * gutter);

        // The timeline spans the window below the viewer and the panels,
        // inside the gutter; the side panels are flush with the window's
        // edges above it.
        let content_height = full.bottom() - gutter - toolbar.bottom();
        let timeline_height = (content_height * self.settings.timeline_share)
            .clamp(150.0, (content_height - 170.0).max(150.0));
        let timeline_rect = Rect::from_min_max(
            pos2(
                full.left() + gutter,
                full.bottom() - gutter - timeline_height,
            ),
            pos2(full.right() - gutter, full.bottom() - gutter),
        );
        let upper = Rect::from_min_max(
            pos2(full.left(), toolbar.bottom()),
            pos2(full.right(), timeline_rect.top() - gutter),
        );

        // Each side panel slides in from its edge and out past it, and the
        // viewer between them follows. A toggle reversed midway turns
        // around from where the panel is.
        let library_reveal = ui::window::slide(&ctx, egui::Id::new("library-reveal"), show_library);
        let inspector_reveal =
            ui::window::slide(&ctx, egui::Id::new("inspector-reveal"), show_right);
        let (library_width, inspector_width) = drawn_widths(
            full.width(),
            (self.settings.library_width, self.settings.inspector_width),
            full.width() - 2.0 * gutter >= LIBRARY_FITS,
            (library_reveal, inspector_reveal),
        );
        let viewer_at = |library: f32, inspector: f32| {
            Rect::from_x_y_ranges(
                upper.left() + egui::lerp(gutter..=library_width, library)
                    ..=upper.right() - egui::lerp(gutter..=inspector_width, inspector),
                upper.y_range(),
            )
        };
        let viewer = viewer_at(library_reveal, inspector_reveal);
        let settled = {
            let (library, inspector) = panel_widths(
                full.width(),
                self.settings.library_width,
                self.settings.inspector_width,
                (show_library, show_right),
            );
            Rect::from_x_y_ranges(
                upper.left() + if show_library { library } else { gutter }
                    ..=upper.right() - if show_right { inspector } else { gutter },
                upper.y_range(),
            )
        };
        // The viewer's pictures are decoded for where it comes to rest.
        ui::Placement {
            drawn: viewer,
            settled,
        }
        .keep_for_viewer(&ctx);
        let library_rect = Rect::from_x_y_ranges(
            viewer.left() - library_width..=viewer.left(),
            upper.y_range(),
        );
        let right_rect = Rect::from_x_y_ranges(
            viewer.right()..=viewer.right() + inspector_width,
            upper.y_range(),
        );
        // A panel that goes away with the keyboard in it gives the keyboard
        // to the timeline, rather than keeping it out of sight. That is
        // decided once, as the panel sets off, by where it was drawn in the
        // last frame: the viewer slides into its place after that.
        let seen_id = egui::Id::new("side-panels-seen");
        let seen: SidePanels = ctx.data(|data| data.get_temp(seen_id)).unwrap_or_default();
        let focused = ctx
            .memory(|memory| memory.focused())
            .and_then(|id| ctx.read_response(id));
        let held_in = |was: Option<Rect>, shown: bool| {
            let (Some(was), Some(focused), false) = (was, &focused, shown) else {
                return false;
            };
            // What floats over the window, a menu or a sheet, is not the
            // panel's.
            focused.layer_id == ui.layer_id() && was.contains(focused.rect.center())
        };
        let handed = (held_in(seen.library, show_library) || held_in(seen.inspector, show_right))
            .then(|| focused.as_ref().map(|focused| focused.id))
            .flatten();
        ctx.data_mut(|data| {
            data.insert_temp(
                seen_id,
                SidePanels {
                    library: (show_library && library_reveal > 0.0)
                        .then(|| library_rect.intersect(upper)),
                    inspector: (show_right && inspector_reveal > 0.0)
                        .then(|| right_rect.intersect(upper)),
                },
            );
        });

        ui::window::drag_region(ui, toolbar);
        ui::window::controls(ui, pos2(full.left() + 16.0, toolbar.center().y), &p);
        ui::toolbar::show(
            ui,
            toolbar,
            &ToolbarInput {
                p: &p,
                name: self.project_draft.name.as_deref().unwrap_or(&project.name),
                dirty: self.editor.is_dirty(),
                saved_once: self.path.is_some(),
                saving: self.saving || self.preparing,
                conflict: self.conflicted,
                undo: self.editor.undo_label(),
                redo: self.editor.redo_label(),
                recent: &self.settings.recent,
                library: show_library,
                library_width,
                library_reveal,
                inspector: show_right,
                viewer: viewer.x_range(),
                settled: settled.x_range(),
                agent_waiting: self.ai.open().is_some(),
                project: &project,
                clips: &self.selection.clips,
                playhead: self.playback.position,
                freeze_hold: self.inspector.freeze_hold(),
                renders: self.renders.summary(),
            },
            actions,
        );

        if let Some(mut pane) = side_pane(ui, "library-pane", library_rect, upper, library_reveal) {
            // The library is made on its pane, which slides and fades.
            let (ui, rect) = (&mut pane, library_rect);
            let speech_view = self.speech_view();
            let speech_settings = self.settings.speech.clone();
            (self.settings.library_grid, self.settings.library_sort) = ui::library::show(
                ui,
                rect,
                &mut self.library,
                &mut self.selection,
                &mut self.textures,
                &mut self.viewer.titles,
                &LibraryInput {
                    p: &p,
                    project: &project,
                    media: self.media.as_ref(),
                    grid: self.settings.library_grid,
                    sort: self.settings.library_sort,
                    importing: self.imports.len(),
                    problem: self.media_problem.as_deref(),
                    previewing: self.source.as_ref().map(|source| source.asset.id),
                    playhead: self.playback.position,
                    presets: &self.presets,
                    speech: captions_tab::SpeechInput {
                        view: &speech_view,
                        settings: &speech_settings,
                        inventory: self.models.as_ref(),
                    },
                },
                actions,
            );
        }
        // What the inspector and the source monitor show of an asset is its
        // poster, wherever the library is scrolled to.
        if let Some(media) = &self.media {
            let inspected = self
                .selection
                .asset
                .filter(|_| show_right && self.right_tab == RightTab::Inspector)
                .and_then(|id| project.assets.get(&id));
            let monitored = self.source.as_ref().map(|source| &source.asset);
            for asset in inspected.into_iter().chain(monitored) {
                self.textures.want_poster(media, asset);
            }
        }

        // The viewer is made before the inspector, as Tab reaches them, and
        // reads what the inspector left it in the last frame. Only the
        // inspector changes that, and only while it is drawn, so only then
        // is it held to compare.
        let for_viewer = |app: &Self| {
            (
                app.draft.clone(),
                app.project_draft.clone(),
                app.inspector.outlined_mask(),
                app.inspector.colour_pick(&app.selection),
                app.inspector.track_region(&app.selection),
                app.inspector.shows_paths(),
            )
        };
        let watched = inspector_reveal > 0.0 && self.right_tab == RightTab::Inspector;
        let before = watched.then(|| for_viewer(self));
        let tab = self.right_tab;
        // Keyframes held in the timeline are drawn as they would be left.
        self.viewer
            .preview_keys(self.timeline.keys.overriding(&project));
        // The mask being edited is outlined while its rows are on screen.
        let masks_shown = show_right && self.right_tab == RightTab::Inspector;
        self.viewer
            .outline_mask(self.inspector.outlined_mask().filter(|_| masks_shown));
        // A key colour is picked off the canvas while its row is on screen.
        let pick = self.inspector.colour_pick(&self.selection);
        self.viewer.pick_colour(pick.filter(|_| masks_shown));
        // So is the region a tracker is to follow, and the trackers' paths.
        let region = self.inspector.track_region(&self.selection);
        self.viewer.choose_region(region.filter(|_| masks_shown));
        self.viewer
            .show_paths(masks_shown && self.inspector.shows_paths());
        match &self.source {
            Some(source) => ui::viewer::show_source(
                ui,
                viewer,
                &mut self.textures,
                &SourceInput {
                    p: &p,
                    asset: &source.asset,
                    rate: source.project.settings.frame_rate,
                    background: source.project.settings.background,
                    playhead: source.playback.position,
                    playing: source.playback.playing,
                    speed: source.playback.speed,
                    looping: source.looping,
                    media: self.media.as_ref(),
                },
                actions,
            ),
            None => ui::viewer::show(
                ui,
                viewer,
                &mut self.viewer,
                &mut self.selection,
                &mut self.draft,
                &mut self.textures,
                &ViewerInput {
                    p: &p,
                    project: &project,
                    query: self.query.as_ref(),
                    playhead: self.playback.position,
                    playing: self.playback.playing,
                    speed: self.playback.speed,
                    looping: self.looping,
                    media: self.media.as_ref(),
                    record: self.recording.view(),
                    library: show_library,
                    quality: self.settings.preview.quality,
                    cards: &(self.saved_audio.card_requests())
                        .chain(self.separating.card_requests())
                        .collect::<Vec<_>>(),
                },
                actions,
            ),
        }
        // The camera paths the inspector and the canvas wanted and did not
        // have are asked for now, in the frame that wanted them.
        self.ask_steadies(&ctx);
        if self.viewer.take_picked() {
            self.inspector.end_pick();
        }
        if let Some(region) = self.viewer.take_region() {
            self.inspector.set_region(region);
        }
        // A clip made to follow a point on a steadied picture keeps its
        // place by how the canvas read that picture.
        self.inspector.set_readings(self.viewer.readings());
        // What is held for a compositor with nothing to draw is given up on
        // the clock, which an editor that has stopped painting still has.
        if let Some(wait) = self.viewer.rest(&mut self.textures, Instant::now()) {
            ctx.request_repaint_after(wait);
        }
        // More pictures drawn together than the viewer can keep one texel
        // for: said when it comes about, not in every frame it lasts.
        if let Some(composed) = self.textures.newly_unplaced() {
            let text = if composed.unplaced == 1 {
                "1 layer has no preview: too many are on screen at once".to_owned()
            } else {
                format!(
                    "{} layers have no preview: too many are on screen at once",
                    composed.unplaced
                )
            };
            self.notify(&ctx, text, true);
        }
        // A layer the media service had no room for asks again before the
        // frame its clock next crosses, which is all that clock is woken for.
        if self.textures.refused() {
            let rate = match &self.source {
                Some(source) if source.playback.playing => Some(source.project.settings.frame_rate),
                _ if self.playback.playing => Some(self.editor.project().settings.frame_rate),
                _ => None,
            };
            if let Some(rate) = rate {
                let predicted_dt = ctx.input(|input| input.predicted_dt);
                ctx.request_repaint_after(pacing::refused_wake(rate, predicted_dt));
            }
        }

        if let Some(mut pane) = side_pane(ui, "right-pane", right_rect, upper, inspector_reveal) {
            self.right_panel(&mut pane, right_rect, &project, actions);
        }
        // What the inspector changed of what the viewer draws, or the tab
        // chosen in its strip, is drawn in a frame of its own.
        if self.right_tab != tab || before.is_some_and(|before| for_viewer(self) != before) {
            ctx.request_repaint();
        }

        ui::timeline::show(
            ui,
            timeline_rect,
            &mut self.timeline,
            &mut self.selection,
            &mut self.textures,
            &mut self.peaks,
            &TimelineInput {
                p: &p,
                project: &project,
                playhead: self.playback.position,
                snapping: self.settings.snapping,
                media: self.media.as_ref(),
                can_paste: self.clipboard.is_some(),
                freeze_hold: self.inspector.freeze_hold(),
                separator: stems::Tool::of(editor_media::stem_separator().as_ref()),
            },
            actions,
        );
        // The keyboard a hidden panel gave up goes to the lanes, if this
        // frame made them.
        hand_off_keyboard(&ctx, handed, self.timeline.keys.graph);
        // The panes' corners are rounded over what was drawn into them, and
        // not over their neighbours.
        kit::corner_caps(&ui.painter_at(viewer), viewer, p.chrome);
        kit::corner_caps(&ui.painter_at(timeline_rect), timeline_rect, p.chrome);
        self.splitters(ui, viewer, timeline_rect, (library_width, inspector_width));
        // The sound the tiles drawn wanted and did not have is asked for
        // now, in the frame that wanted it.
        self.ask_peaks(&ctx);
        // Peaks or beat grids turned away while a tile of the last frame
        // still held their room are asked for again in a frame of their own.
        if self.peaks.room_is_due() {
            ctx.request_repaint();
        }
        // Nothing is drawn after the lanes that shows a poster. One that was
        // turned away while posters of the last frame still held its room is
        // asked for again in a frame of its own.
        if self.textures.poster_room_is_due() {
            ctx.request_repaint();
        }

        // The source monitor follows the library: another asset chosen there
        // takes its place, and turning to a clip gives the viewer back to the
        // timeline.
        if let Some(source) = &self.source {
            if !self.selection.clips.is_empty() {
                actions.push(Action::CloseSource);
            } else if let Some(asset) = self.selection.asset
                && asset != source.asset.id
            {
                // What cannot be monitored closes the monitor, so that it
                // is not asked to open on it frame after frame.
                actions.push(if self.previewable(asset) {
                    Action::PreviewAsset(asset)
                } else {
                    Action::CloseSource
                });
            }
        }

        // The export sheet's poster is drawn only while the sheet is shown.
        if !matches!(self.overlay, Overlay::Export | Overlay::ReplaceExport(_)) {
            self.poster.stop();
        }
        // The queue's refusal is said only while the queue that was asked
        // stays open.
        if self.overlay != Overlay::RenderQueue {
            self.queue_refusal = None;
        }
        // Sheets and what floats over the window.
        match self.overlay.clone() {
            Overlay::None | Overlay::Conflict(_) => {}
            Overlay::Duck(question) => {
                if let Some(replace) = sheets::replace_keys(&ctx, &p, question.keyed) {
                    self.overlay = Overlay::None;
                    if replace
                        && let Err(problem) =
                            self.start_ducking(&ctx, &question.clips, question.depth)
                    {
                        self.notify(&ctx, problem, true);
                    }
                }
            }
            Overlay::Settings => {
                let cache = self.proxies.cache();
                if sheets::settings(&ctx, &p, &self.settings, cache, actions) {
                    self.overlay = Overlay::None;
                }
            }
            Overlay::Checkpoints => {
                let has_file = self.path.is_some();
                match sheets::checkpoints(&ctx, &p, &mut self.checkpoints, has_file) {
                    None => {}
                    Some(CheckpointAnswer::Close) => self.overlay = Overlay::None,
                    Some(CheckpointAnswer::Create(name)) => self.checkpoint_job(&ctx, Some(name)),
                    Some(CheckpointAnswer::Restore(checkpoint)) => {
                        self.overlay = Overlay::None;
                        self.restore_checkpoint(&ctx, checkpoint);
                    }
                    Some(CheckpointAnswer::SaveProject) => {
                        self.overlay = Overlay::None;
                        actions.push(Action::Save);
                    }
                }
            }
            Overlay::Export => self.export_sheet(&ctx, &p, &project, true),
            Overlay::ReplaceExport(file) => {
                // Asked over the sheet, which stays in sight under it.
                self.export_sheet(&ctx, &p, &project, false);
                let folder = (self.export_replace.as_ref())
                    .and_then(|(_, destination, ..)| destination.parent())
                    .and_then(Path::file_name)
                    .map_or_else(String::new, |folder| folder.to_string_lossy().into_owned());
                let also = (self.export_replace.as_ref()).and_then(|(.., also)| also.clone());
                if let Some(replace) =
                    sheets::replace_export(&ctx, &p, &file, also.as_deref(), &folder)
                {
                    // The sheet comes back as it was, unless the export is
                    // queued.
                    self.overlay = Overlay::Export;
                    if let Some((settings, destination, start_now, _)) = self.export_replace.take()
                        && replace
                    {
                        self.queue_export(&ctx, settings, destination, start_now);
                    }
                }
            }
            Overlay::RenderQueue => {
                let jobs = self.renders.jobs();
                let waiting = self.renders.waiting();
                let refusal = self.queue_refusal.clone();
                if let Some(answer) =
                    ui::render_queue::show(&ctx, &p, &jobs, &waiting, refusal.as_deref())
                {
                    self.answer_queue(&ctx, answer, &jobs);
                }
            }
            Overlay::StopRendering => {
                let summary = self.renders.summary();
                let jobs = summary.active + summary.queued + summary.held;
                if let Some(stop) = sheets::stop_rendering(&ctx, &p, jobs) {
                    self.overlay = Overlay::None;
                    if stop {
                        // Stopped now, not once quitting is through: they
                        // stay cancelled should the person stay.
                        crate::export::wiring::stop_all(&self.renders);
                        self.stop_renders = true;
                        self.ask_to_quit(&ctx);
                    }
                }
            }
            Overlay::Unsaved(pending) => {
                if let Some(answer) = sheets::unsaved(&ctx, &p, &project.name, &pending) {
                    self.overlay = Overlay::None;
                    match answer {
                        Answer::Cancel => {}
                        Answer::Discard => self.proceed(&ctx, pending),
                        Answer::Save => {
                            self.after_save = Some(pending);
                            actions.push(Action::Save);
                        }
                    }
                }
            }
        }
        if let Overlay::Conflict(conflict) = self.overlay.clone()
            && let Some(answer) = sheets::conflict(&ctx, &p, &project.name, &conflict)
        {
            self.overlay = Overlay::None;
            match answer {
                ConflictAnswer::Later => {}
                ConflictAnswer::SaveAs => actions.push(Action::SaveAs),
                ConflictAnswer::Retry => actions.push(Action::Save),
                // Chosen in so many words: the edits here are given up and
                // the file is read as the other writer left it.
                ConflictAnswer::Reopen => self.proceed(&ctx, Pending::Open(Some(conflict.path))),
            }
        }
        if self.overlay == Overlay::None
            && let Some(offer) = self.recover_offer.clone()
        {
            self.recovery_sheet(&ctx, &p, offer);
        }
        let over_sheet = self.overlay != Overlay::None || self.recover_offer.is_some();
        // Files being written, and written ones offered, over the viewer;
        // the toasts keep clear of them.
        let mut cards = self.saved_audio.cards();
        cards.extend(self.separating.cards());
        let shown = sheets::saves(&ctx, &p, &cards, viewer, actions);
        sheets::notices(&ctx, &p, &mut self.notices, &shown, over_sheet, actions);
        ui::window::interface_size(&ctx, &p);
        if self.files_held {
            sheets::drop_target(&ctx, &p);
        }
    }

    fn recovery_sheet(&mut self, ctx: &egui::Context, p: &Palette, offer: Offer) {
        let Some(answer) = sheets::recovery(ctx, p, offer.project.is_some()) else {
            return;
        };
        self.recover_offer = None;
        // Leaving the sheet decides nothing: the copy stays for next time.
        if answer == sheets::Recovery::Later {
            return;
        }
        if answer == sheets::Recovery::Restore {
            let epoch = self.epoch;
            let revision = self.editor.revision();
            self.spawn(ctx, "neptune-video-recover", move || Job::Recovered {
                epoch,
                revision,
                // Owned before it is read: the offer was only a glance, and
                // another window may have taken the copy since. The file is
                // not looked at afresh here: what a later save may replace
                // is what the copy recorded, or nothing.
                result: RecoveryLease::acquire(&offer.base)
                    .and_then(|lease| {
                        let restored = editor_store::load_recovery_with_document(&offer.base)?;
                        Ok((restored, lease))
                    })
                    .map_err(|error| error.to_string()),
                offer,
            });
        } else {
            self.spawn(ctx, "neptune-video-recovery", move || {
                // Taken first, so that a copy in use elsewhere is left alone.
                let removed = RecoveryLease::acquire(&offer.base)
                    .and_then(|_lease| editor_store::remove_recovery(&offer.base));
                Job::Background(
                    removed
                        .map(|()| String::new())
                        .map_err(|error| format!("The recovery copy was not removed: {error}")),
                )
            });
        }
    }

    /// The gutters that resize the side panels and the timeline. Made after
    /// the panes, so that the pointer finds them over what the panes made
    /// beside them, at the viewer's edges as drawn; the keyboard does not
    /// stop at them.
    fn splitters(&mut self, ui: &mut egui::Ui, viewer: Rect, timeline: Rect, widths: (f32, f32)) {
        let p = self.palette;
        let gutter = metrics::GUTTER;
        // The timeline is the window's width less its gutters.
        let window = timeline.width() + 2.0 * gutter;
        let (library_shown, right_shown) = self.side_panels(window - 2.0 * gutter);
        let (library, inspector) = widths;
        let preferred = (self.settings.library_width, self.settings.inspector_width);
        let shown = (library_shown, right_shown);
        // A panel dragged wider keeps the viewer at least this wide.
        let other = |width: f32, shown: bool| if shown { width } else { gutter };
        if library_shown {
            let edge = viewer.left();
            let (delta, reset) = widgets::splitter(
                ui,
                &p,
                Rect::from_x_y_ranges(edge - 3.0..=edge + 3.0, viewer.y_range()),
                egui::Id::new("split-library"),
                true,
            );
            let most = (window - other(inspector, right_shown) - VIEWER_LEAST).max(LIBRARY_LEAST);
            if reset {
                self.settings.library_width = settings::LIBRARY_WIDTH;
            } else if delta != 0.0 {
                self.settings.library_width =
                    dragged_width(window, preferred, shown, true, library + delta, most);
            }
        }
        if right_shown {
            let edge = viewer.right();
            let (delta, reset) = widgets::splitter(
                ui,
                &p,
                Rect::from_x_y_ranges(edge - 3.0..=edge + 3.0, viewer.y_range()),
                egui::Id::new("split-inspector"),
                true,
            );
            let most = (window - other(library, library_shown) - VIEWER_LEAST).max(INSPECTOR_LEAST);
            if reset {
                self.settings.inspector_width = settings::INSPECTOR_WIDTH;
            } else if delta != 0.0 {
                self.settings.inspector_width =
                    dragged_width(window, preferred, shown, false, inspector - delta, most);
            }
        }
        // The timeline's gutter, ten points tall to the pointer.
        let middle = timeline.top() - gutter * 0.5;
        let (delta, reset) = widgets::splitter(
            ui,
            &p,
            Rect::from_x_y_ranges(timeline.x_range(), middle - 4.0..=middle + 4.0),
            egui::Id::new("split-timeline"),
            false,
        );
        let content = timeline.bottom() - viewer.top();
        if reset {
            self.settings.timeline_share = settings::TIMELINE_SHARE;
        } else if delta != 0.0 {
            self.settings.timeline_share =
                ((timeline.height() - delta) / content.max(1.0)).clamp(0.2, 0.7);
        }
    }

    /// Whether the library and the right panel are drawn in a window whose
    /// content is `width` wide. A window too narrow for both beside the
    /// viewer shows the one asked for last, so each stays reachable.
    fn side_panels(&self, width: f32) -> (bool, bool) {
        side_panels(
            width,
            self.settings.show_library,
            self.settings.show_inspector,
            self.library_first,
        )
    }

    fn right_panel(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        project: &Project,
        actions: &mut Vec<Action>,
    ) {
        let p = self.palette;
        // Flush with the window's edge: the strip of tabs and the body are
        // inset from the viewer by 6 and from the edge by 8.
        let inner = Rect::from_min_max(
            pos2(rect.left() + 6.0, rect.top()),
            pos2(rect.right() - 8.0, rect.bottom()),
        );
        let strip = Rect::from_min_size(inner.min, vec2(inner.width(), ui::panel::TABS));
        let tabs = [
            ui::panel::Tab {
                label: "Inspector",
                icon: Icon::Adjustments,
                waiting: 0,
            },
            ui::panel::Tab {
                label: "Agent",
                icon: Icon::Sparkles,
                // A plan waiting to be applied or discarded.
                waiting: usize::from(self.ai.open().is_some()),
            },
        ];
        let selected = match self.right_tab {
            RightTab::Inspector => 0,
            RightTab::Agent => 1,
        };
        if let Some(chosen) = tab_strip(ui, strip, p, &tabs, selected) {
            self.right_tab = if chosen == 0 {
                RightTab::Inspector
            } else {
                RightTab::Agent
            };
        }
        let body = Rect::from_min_max(pos2(inner.left(), strip.bottom()), inner.max);
        self.settings.agent_panel = self.right_tab == RightTab::Agent;
        match self.right_tab {
            RightTab::Inspector => ui::inspector::show(
                ui,
                body,
                &mut self.inspector,
                &self.selection,
                &mut self.draft,
                &mut self.project_draft,
                &InspectorInput {
                    p: &p,
                    project,
                    textures: &self.textures,
                    playhead: self.playback.position,
                    transition: (self.timeline.marked(project, &self.selection))
                        .map(|(_, edge)| edge),
                    // Read, never waited for: the audio worker probes.
                    missing: editor_media::missing_audio_filters().as_deref(),
                    families: self.viewer.titles.families(),
                    brand: &self.presets.colors,
                },
                actions,
            ),
            RightTab::Agent => ui::agent::show(
                ui,
                body,
                &mut self.agent,
                &AgentInput {
                    p: &p,
                    session: self.session_path.as_deref(),
                    session_problem: self.session_problem.as_deref(),
                    ask: self.settings.agent_ask,
                    ai: &self.settings.ai,
                    waiting: match (self.live.turn(), self.ai.waiting()) {
                        (Some(turn), _) => Some(WaitingInput {
                            provider: turn.provider.name(),
                            seconds: turn.since.elapsed().as_secs(),
                            started: turn.started,
                            cancelling: turn.cancelling,
                            working: true,
                            edits: turn.edits,
                        }),
                        (None, Some(waiting)) => Some(WaitingInput {
                            provider: waiting.provider.name(),
                            seconds: waiting.since.elapsed().as_secs(),
                            started: waiting.started,
                            cancelling: waiting.cancelling,
                            working: false,
                            edits: 0,
                        }),
                        (None, None) => None,
                    },
                    current: self.ai.open_is_current(&self.editor),
                    talking: self.live.talking(),
                    proposal_waits: self.ai.open().is_some(),
                },
                actions,
            ),
        }
        if self.inspector.take_families_wanted() {
            self.viewer.titles.want_families();
        }
    }
}

impl FrameEnd for App {
    fn take_published(&mut self, ctx: &egui::Context) {
        self.take_frames(ctx);
        // A canvas composited before one of these frames came shows the one
        // before it, and nothing else would paint the frame it has now.
        if self.viewer.canvas_is_stale(&self.textures) {
            ctx.request_repaint();
        }
    }

    fn act(&mut self, ctx: &egui::Context, action: Action) {
        self.run(ctx, action);
    }

    fn settle(&mut self, ctx: &egui::Context) {
        self.commit_drafts(ctx, false);
    }
}

impl eframe::App for App {
    /// The window paints its own rounded material; what lies outside its
    /// corners is left clear.
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::TRANSPARENT.to_array()
    }

    /// Runs before every `ui`, and alone while the window is hidden, so the
    /// workers are drained and playback keeps time either way. A frame that
    /// is drawn takes the media worker's frames once more at its end: see
    /// [`end_frame`].
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.file_drop.attach(frame, ctx);
        if !self.separator_asked {
            // Once a session, on a thread of its own: the clip menu reads
            // the answer and never waits for it.
            self.separator_asked = true;
            let wake = ctx.clone();
            editor_media::look_for_separator(move || wake.request_repaint());
        }
        if !self.separator_taken
            && let Some(found) = editor_media::stem_separator()
        {
            // A separator that is there and cannot be run is said once; most
            // people have none, and the clip menu says so where it is asked.
            self.separator_taken = true;
            if let Err(why) = found
                && !editor_media::separator_absent(&why)
            {
                self.notify(ctx, format!("Separate stems is unavailable: {why}"), true);
            }
        }
        self.take_jobs(ctx);
        self.take_media(ctx);
        self.take_analysis(ctx);
        self.take_speech(ctx);
        self.take_steadies(ctx);
        self.take_sound(ctx);
        self.step_ducking(ctx);
        self.take_saves(ctx);
        self.take_agent_calls(ctx);
        self.take_ai();
        self.take_renders(ctx);
        self.take_proxies(ctx);
        while let Some(problem) = self.audio.as_ref().and_then(AudioService::poll) {
            self.notify(ctx, format!("Sound: {problem}"), true);
        }
        for problem in std::mem::take(&mut self.problems) {
            self.notify(ctx, problem, true);
        }

        // What egui expects a pass to take, which it takes off every wait it is
        // asked for.
        let predicted_dt = ctx.input(|input| input.predicted_dt);

        // The source monitor's clock. The timeline's is paused while it runs.
        if let Some(source) = &mut self.source
            && source.playback.playing
        {
            // The sound device is the master clock here too, once it runs.
            let heard = source
                .audible()
                .then(|| self.audio.as_ref().and_then(AudioService::position))
                .flatten();
            let turned = source.tick(Instant::now(), heard);
            // The pass that shows its next frame, on the grid it is stepped on.
            let wake = pacing::playback_wake(
                &source.playback,
                source.project.settings.frame_rate,
                predicted_dt,
            );
            if turned {
                // It stopped, or started over from the beginning.
                self.textures.jumped();
                self.sync_source_audio();
            }
            if let Some(wake) = wake {
                ctx.request_repaint_after(wake);
            }
        }

        // Playback clock. The sound device is the master clock once it is
        // running; until then, and without sound, the wall clock is. A take
        // is anchored to the same reading.
        let heard = (self.playback.playing || self.recording.busy())
            .then(|| self.audio.as_ref().and_then(AudioService::position))
            .flatten();
        if self.playback.playing {
            let duration = self.editor.project().duration();
            self.playback
                .tick_with_audio(Instant::now(), duration, heard);
            if !self.playback.playing {
                // The clock stopped itself at the end. A take runs on past
                // it, with the playhead held there: nothing loops under it.
                if self.looping && duration > Time::ZERO && !self.recording.busy() {
                    self.playback.seek(Time::ZERO);
                    self.playback.play();
                    self.textures.jumped();
                }
                self.sync_audio();
            }
            self.timeline.follow(self.playback.position);
            let rate = self.editor.project().settings.frame_rate;
            if let Some(wake) = pacing::playback_wake(&self.playback, rate, predicted_dt) {
                ctx.request_repaint_after(wake);
            }
        }
        self.step_recording(ctx, heard);
        // The take's elapsed time and level, and nothing once it is placed.
        if let Some(wait) = self.recording.repaint_after() {
            ctx.request_repaint_after(wait);
        }
        // The export's encoder check waits while the person watches or
        // records, unless they ask for it.
        self.renders.hold_probe(
            self.playback.playing
                || self.recording.busy()
                || (self.source.as_ref()).is_some_and(|source| source.playback.playing),
        );

        // A recovery copy of unsaved work, at most every few seconds. A
        // project without a file has one too, in the application's folder.
        if self.settings.autosave && self.editor.is_dirty() {
            if !self.leasing && self.autosave.due(self.editor.revision(), Instant::now()) {
                self.write_recovery(ctx);
            }
            ctx.request_repaint_after(RECOVERY_INTERVAL);
        }

        // Written files stop being offered, and notices leave, by themselves.
        let now = ctx.input(|i| i.time);
        if let Some(next) = self.saved_audio.expire(now) {
            ctx.request_repaint_after(Duration::from_secs_f64((next - now).max(0.0)));
        }
        if let Some(next) = sheets::expire(ctx, &mut self.notices) {
            // The last moments fade the toast out, unless motion is reduced.
            let fade = if kit::reduced_motion(ctx) {
                0.0
            } else {
                f64::from(theme::motion::TOAST)
            };
            let wait = next - now - fade;
            ctx.request_repaint_after(Duration::from_secs_f64(if wait > 0.0 {
                wait
            } else if fade > 0.0 {
                1.0 / 60.0
            } else {
                (next - now).max(0.0)
            }));
        }

        // Preferences are written when they changed and no drag is resizing.
        // Nor while the model's name is being typed: one write for the name,
        // not one for each key.
        let naming = ctx.text_edit_focused()
            && (self.settings.ai.claude_model != self.settings_saved.ai.claude_model
                || self.settings.ai.codex_model != self.settings_saved.ai.codex_model);
        if self.presets != self.presets_saved && !ctx.egui_is_using_pointer() {
            self.presets_saved = Arc::clone(&self.presets);
            if let Some(path) = self.presets_path.clone() {
                let presets = Arc::clone(&self.presets);
                let aside = std::mem::take(&mut self.presets_unread);
                self.spawn(ctx, "neptune-video-presets", move || {
                    Job::Background(match presets.save(&path, aside) {
                        Ok(Some(kept)) => Ok(format!(
                            "The presets file that could not be read was kept as {}.",
                            kept.display()
                        )),
                        Ok(None) => Ok(String::new()),
                        Err(error) => Err(format!("Presets were not saved: {error}")),
                    })
                });
            }
        }
        if self.settings != self.settings_saved && !ctx.egui_is_using_pointer() && !naming {
            self.settings_saved = self.settings.clone();
            if let Some(path) = self.settings_path.clone() {
                let settings = self.settings.clone();
                self.spawn(ctx, "neptune-video-settings", move || {
                    Job::Background(
                        settings
                            .save(&path)
                            .map(|()| String::new())
                            .map_err(|error| format!("Settings were not saved: {error}")),
                    )
                });
            }
        }

        // Closing asks first while exports render or wait, then about
        // unsaved work.
        if ctx.input(|i| i.viewport().close_requested()) && !self.exit_approved {
            self.stop_renders = false;
            let question = crate::export::wiring::close_question(
                self.renders.busy(),
                self.stop_renders,
                self.editor.is_dirty(),
            );
            if question != crate::export::wiring::CloseQuestion::None {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.ask_to_quit(ctx);
            }
        }
        self.capture(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let mut actions = Vec::new();

        // Files dropped on the window are imported.
        let dropped = self.file_drop.poll(&ctx);
        self.files_held = dropped.hovering;
        if let Some(problem) = dropped.problem {
            self.notify(&ctx, problem, true);
        }
        if let Some(problem) = self.import(dropped.paths) {
            self.notify(&ctx, problem, true);
        }

        // A scale the window has no room for is held down to the one it
        // has, and given back as the window grows.
        ctx.set_zoom_factor(self.settings.interface_scale.min(ui::largest_scale(&ctx)));
        // Reduce motion is read by every animation of the frame, and the
        // system's appearance can change under a window that follows it.
        let motion = kit::reduced_motion(&ctx) != self.settings.reduce_motion;
        kit::set_reduced_motion(&ctx, self.settings.reduce_motion);
        let dark = self.settings.appearance.resolve(ctx.system_theme()) != Appearance::Light;
        if motion || dark != self.palette.dark {
            self.restyle(&ctx);
        }
        self.shortcuts(&ctx, &mut actions);
        let bounds = ui.max_rect();
        self.window_ui(ui, &mut actions);
        ui::ChoiceArrows::end(&ctx);
        ui::window::frame(ui, bounds, &self.palette);
        end_frame(self, &ctx, actions);
    }

    fn on_exit(&mut self) {
        // First, while the window is still there: the listener is on the
        // window's display connection, and nothing below waits on it.
        self.file_drop.stop();
        if let Some(audio) = &self.audio {
            audio.stop();
        }
        // Exports stop with the window: their processes are killed and
        // their unfinished files removed. Quitting asked first.
        self.renders.shutdown();
        self.poster.stop();
        // So does a proxy being written, and its part file.
        self.proxies.shutdown();
        // A take under way is closed and kept; nothing is placed.
        self.recording.close();
        match self.recovery_base() {
            Some(base) => {
                let dirty = self.editor.is_dirty();
                let parting = if self.discard_on_exit || (!dirty && self.owns(&base)) {
                    // Given up, or already in the project's file.
                    Parting::Remove
                } else if dirty && self.settings.autosave {
                    // Closed without an answer: the latest work is kept.
                    Parting::Keep
                } else {
                    Parting::Leave
                };
                // The copy is touched only by the window that owns it. A
                // lock is not made for a copy that is not there.
                let lease = match parting {
                    Parting::Keep => self.exit_lease(&base),
                    Parting::Remove
                        if self.owns(&base) || editor_store::recovery_path(&base).exists() =>
                    {
                        self.exit_lease(&base)
                    }
                    _ => None,
                };
                let document = bound_document(&base, self.document.as_ref());
                let idle = part_with_recovery(
                    &self.saver,
                    &self.editor,
                    &base,
                    lease,
                    document,
                    parting,
                    EXIT_WAIT,
                );
                // Copies of documents given up earlier, whose last write has
                // only now landed, go as they would have in a later frame.
                while idle && let Some(result) = self.saver.poll() {
                    let key = (result.context_id, result.path);
                    if result.recovery
                        && key.1 != base
                        && self.discarded.contains(&key)
                        && let Ok(_lease) = RecoveryLease::acquire(&key.1)
                    {
                        let _ = editor_store::remove_recovery(&key.1);
                    }
                }
            }
            None => {
                let _ = self.saver.flush(EXIT_WAIT);
            }
        }
        if self.settings != self.settings_saved
            && let Some(path) = &self.settings_path
        {
            let _ = self.settings.save(path);
        }
        if self.presets != self.presets_saved
            && let Some(path) = &self.presets_path
        {
            let _ = self.presets.save(path, self.presets_unread);
        }
    }
}

/// What the end of a frame is made of, once its widgets have said what they
/// ask for. [`end_frame`] is the order they are gone through in.
pub(crate) trait FrameEnd {
    /// Hands the layers the frames the media worker has published.
    fn take_published(&mut self, ctx: &egui::Context);
    /// Carries out one action a widget asked for.
    fn act(&mut self, ctx: &egui::Context, action: Action);
    /// What follows the actions: values held while a gesture lasted.
    fn settle(&mut self, ctx: &egui::Context);
}

/// Ends a frame whose widgets asked for `actions`.
///
/// What the media worker has published is handed over first, once. A frame
/// the worker already had is published while the frame that asked for it is
/// drawn, and an action may move the playhead: left to the start of the next
/// frame, that frame would by then be one asked for before a seek, which no
/// layer takes, and a playhead dragged steadily would show nothing until it
/// rested. Taken here it is the frame its layer asks for now. Nothing asked
/// for before an action is taken after it; that is as it was.
///
/// Then the actions are applied in the order they were asked for, and what
/// was held is settled.
pub(crate) fn end_frame(end: &mut impl FrameEnd, ctx: &egui::Context, actions: Vec<Action>) {
    end.take_published(ctx);
    for action in actions {
        end.act(ctx, action);
    }
    end.settle(ctx);
}

/// Puts `fresh` in the source monitor's place, which is what opens it: on
/// another asset, or on the one it shows, started over with its clock where
/// a fresh one stands. `S` is the monitor as the application keeps it.
///
/// The layer kept for the monitor is told in the same moment, as a seek
/// tells it: the next frame takes what the worker published before it
/// composes, and a frame asked for where the monitor stood before is not
/// taken after its clock went back. See [`Textures::started_over`].
pub(crate) fn start_source<S>(source: &mut Option<S>, fresh: S, textures: &mut Textures) {
    *source = Some(fresh);
    textures.started_over(Viewing::Source);
}

/// Hands `textures` every frame `media` has published and not yet handed
/// over, as [`App::take_media`] hands them: only of this generation of the
/// worker, and only of an asset the library has, which `asset` looks up.
/// They are taken as frames of the end of a frame: see
/// [`Textures::take_late`]. Whatever else the worker published goes on to
/// `held`, in the order it was published, and nothing is done about it here.
pub(crate) fn take_published_frames<'a>(
    ctx: &egui::Context,
    media: &MediaService,
    textures: &mut Textures,
    held: &mut VecDeque<MediaEvent>,
    asset: impl Fn(AssetId) -> Option<&'a Asset>,
) {
    textures.take_late(true);
    loop {
        let generation = media.generation();
        let Some(event) = media.poll() else { break };
        match event {
            MediaEvent::FrameReady {
                generation: from,
                asset_id,
                time,
                size,
                consumer,
                frame,
            } => {
                if from == generation
                    && let Some(asset) = asset(asset_id)
                {
                    textures.accept(ctx, Some(asset), consumer, asset_id, time, size, &frame);
                }
            }
            other => held.push_back(other),
        }
    }
    textures.take_late(false);
}

/// Asks again for the import batches a cancelled worker was still to answer,
/// in their order. Returns those it took, which are waited for as before,
/// and why it refused the others.
/// What the card and the status line say once a proposal was applied. Only an
/// edit that moved the document is called applied: nothing else can be undone.
fn settled(applied: ai::Applied) -> (PlanState, &'static str) {
    match applied {
        ai::Applied::Changed => (PlanState::Applied, "Applied"),
        ai::Applied::Unchanged => (PlanState::Unchanged, "No changes"),
    }
}

fn hand_back<E>(
    waiting: VecDeque<Vec<PathBuf>>,
    mut import: impl FnMut(Vec<PathBuf>) -> Result<(), E>,
) -> (VecDeque<Vec<PathBuf>>, Vec<E>) {
    let mut taken = VecDeque::with_capacity(waiting.len());
    let mut refused = Vec::new();
    for paths in waiting {
        match import(paths.clone()) {
            Ok(()) => taken.push_back(paths),
            Err(error) => refused.push(error),
        }
    }
    (taken, refused)
}

/// Whether the library and the right panel are drawn in a window whose
/// content is `width` wide, of the two that are switched on. Both fit a wide
/// window. A narrow one shows one side panel beside the viewer: the library
/// when it was asked for last (`library_first`), and the right panel
/// otherwise, so that neither is switched on and nowhere to be seen.
fn side_panels(width: f32, library: bool, right: bool, library_first: bool) -> (bool, bool) {
    let right = right && width >= 560.0;
    if width >= LIBRARY_FITS || !(library && right) {
        (library, right)
    } else {
        (library_first, !library_first)
    }
}

/// The widths of the side panels drawn in a window `window` wide, of
/// `library` and `inspector` preferred, by which of them are `shown`. Both
/// shrink by as much as each other, each to its least, to keep the viewer
/// 480 wide; then the viewer gives way, to 360 at 820. One alone takes what
/// leaves the viewer 480 beside the gutter, within its least and preferred;
/// so is a panel that is not shown measured.
fn panel_widths(window: f32, library: f32, inspector: f32, shown: (bool, bool)) -> (f32, f32) {
    let library = library.max(LIBRARY_LEAST);
    let inspector = inspector.max(INSPECTOR_LEAST);
    let alone = |preferred: f32, least: f32| {
        (window - metrics::GUTTER - VIEWER_COMFORT)
            .min(preferred)
            .max(least)
    };
    if shown != (true, true) {
        return (
            alone(library, LIBRARY_LEAST),
            alone(inspector, INSPECTOR_LEAST),
        );
    }
    let short = (library + inspector + VIEWER_COMFORT - window).max(0.0);
    let (library_room, inspector_room) = (library - LIBRARY_LEAST, inspector - INSPECTOR_LEAST);
    // Half each, and what one cannot give, the other.
    let library_gives = (short * 0.5).min(library_room);
    let inspector_gives = (short - library_gives).min(inspector_room);
    let library_gives = (short - inspector_gives).min(library_room);
    (library - library_gives, inspector - inspector_gives)
}

/// The widths of the side panels as drawn while `reveal`, the library's and
/// the inspector's, slide them in and out. Where both fit beside the viewer
/// (`fits`), each moves between its width alone and its width beside the
/// other as the other slides, so that it gives way and takes its room back
/// with the other's motion rather than at once. Where they do not, each is
/// as wide as it is alone.
fn drawn_widths(window: f32, preferred: (f32, f32), fits: bool, reveal: (f32, f32)) -> (f32, f32) {
    let alone = panel_widths(window, preferred.0, preferred.1, (true, false));
    if !fits {
        return alone;
    }
    let both = panel_widths(window, preferred.0, preferred.1, (true, true));
    (
        egui::lerp(alone.0..=both.0, reveal.1),
        egui::lerp(alone.1..=both.1, reveal.0),
    )
}

/// The preferred width that draws the dragged side panel, the library if
/// `library` and else the inspector, `target` wide, of `preferred` and with
/// `shown` as they are, at most `most`. The widths are drawn from the ones
/// preferred, so that is the preference that has the panel's edge follow
/// the pointer: half as much again while the panels shrink alike. Where
/// the window lets the panel go no further, a preference that already goes
/// that far is kept.
fn dragged_width(
    window: f32,
    preferred: (f32, f32),
    shown: (bool, bool),
    library: bool,
    target: f32,
    most: f32,
) -> f32 {
    let (range, now) = if library {
        (settings::LIBRARY_WIDTHS, preferred.0)
    } else {
        (settings::INSPECTOR_WIDTHS, preferred.1)
    };
    let drawn = |width: f32| {
        let widths = if library {
            panel_widths(window, width, preferred.1, shown)
        } else {
            panel_widths(window, preferred.0, width, shown)
        };
        if library { widths.0 } else { widths.1 }
    };
    let (mut low, mut high) = (*range.start(), most.min(*range.end()).max(*range.start()));
    let target = target.clamp(drawn(low), drawn(high));
    if (drawn(now) - target).abs() < 0.01 {
        return now;
    }
    // The least preference drawn as wide: wider ones draw no wider once
    // the window holds the panel back.
    for _ in 0..40 {
        let middle = (low + high) * 0.5;
        if drawn(middle) >= target {
            high = middle;
        } else {
            low = middle;
        }
    }
    (high * 100.0).round() / 100.0
}

/// The right panel's strip of tabs, made in every frame with an arrow's
/// choice in it: the keyboard the arrow moved is on one of its cells, which
/// must be there to have it. The tab chosen by an arrow or a click.
fn tab_strip(
    ui: &mut egui::Ui,
    strip: Rect,
    p: Palette,
    tabs: &[ui::panel::Tab],
    selected: usize,
) -> Option<usize> {
    let cells: Vec<egui::Id> = (tabs.iter())
        .map(|tab| ui.id().with(("panel-tab", tab.label)))
        .collect();
    let arrowed = ui::arrow_choice(ui.ctx(), &cells, selected);
    let clicked = ui::panel::tabs(
        ui,
        strip,
        p,
        tabs,
        arrowed.unwrap_or(selected),
        egui::WidgetType::RadioButton,
    );
    arrowed.or(clicked)
}

/// Where the side panels were drawn in the last frame, while shown.
#[derive(Clone, Copy, Debug, Default)]
struct SidePanels {
    library: Option<Rect>,
    inspector: Option<Rect>,
}

/// The surface a side panel is made on, while any of it shows: its
/// rectangle, clipped to the band above the timeline as it slides past the
/// window's edge, and fading with it. The pane is named `name` whatever was
/// made before it, and takes the same one name from the window whether it
/// is made or not, so that what is made in it and after it keeps its names,
/// and the keyboard, as panels come and go.
fn side_pane(
    ui: &mut egui::Ui,
    name: &str,
    rect: Rect,
    band: Rect,
    reveal: f32,
) -> Option<egui::Ui> {
    if reveal <= 0.0 {
        ui.skip_ahead_auto_ids(1);
        return None;
    }
    let mut pane = ui.new_child(
        egui::UiBuilder::new()
            .id(egui::Id::new(name))
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    pane.set_clip_rect(rect.intersect(band).intersect(ui.clip_rect()));
    pane.multiply_opacity(reveal);
    Some(pane)
}

/// Gives the keyboard a hidden panel had, `handed`, to the timeline's lanes,
/// or gives it up while the graph editor (`graph`) has their place and
/// nothing there takes it. Only if it is still where it was: a panel shown
/// in the same frame may have asked for it, and keeps it.
fn hand_off_keyboard(ctx: &egui::Context, handed: Option<egui::Id>, graph: bool) {
    let Some(handed) = handed else {
        return;
    };
    ctx.memory_mut(|memory| {
        if memory.focused() == Some(handed) {
            if graph {
                memory.surrender_focus(handed);
            } else {
                memory.request_focus(egui::Id::new("timeline-lanes"));
            }
        }
    });
}

/// The window laid out as it is for a fresh one (§2.5): both side panels
/// shown at their widths, and the timeline at its share. Nothing in the
/// project changes, so nothing is added to its history.
fn reset_layout(settings: &mut Settings) {
    let defaults = Settings::default();
    settings.library_width = defaults.library_width;
    settings.inspector_width = defaults.inspector_width;
    settings.timeline_share = defaults.timeline_share;
    settings.show_library = true;
    settings.show_inspector = true;
}

/// The modifiers a key went down with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Held {
    Alone,
    Shift,
    Alt,
}

/// How many times `key` went down among `events` held as `held`, by the
/// modifiers its own event carries. A frame can be handed several presses of
/// one key, and a key held down repeats as presses.
fn strikes(events: &[egui::Event], key: Key, held: Held) -> usize {
    (events.iter())
        .filter(|event| {
            matches!(
                event,
                egui::Event::Key { key: down, pressed: true, modifiers, .. }
                    if *down == key
                        && match held {
                            Held::Alone => modifiers.is_none(),
                            Held::Shift => modifiers.shift_only(),
                            Held::Alt => {
                                modifiers.alt && !modifiers.shift && !modifiers.command && !modifiers.ctrl
                            }
                        }
            )
        })
        .count()
}

/// Whether `key` went down among `events` held as `held`.
fn struck(events: &[egui::Event], key: Key, held: Held) -> bool {
    strikes(events, key, held) > 0
}

/// What `R` asks for in this frame: a take, or the end of the one under way
/// when `busy`. Nothing while a field has the keyboard, where it is a letter.
fn record_key(ctx: &egui::Context, busy: bool) -> Option<Action> {
    if ctx.text_edit_focused() || !ctx.input(|i| struck(&i.events, Key::R, Held::Alone)) {
        return None;
    }
    Some(if busy {
        Action::StopRecord
    } else {
        Action::Record
    })
}

/// What Space, K or L asks of the timeline's transport, playing as
/// `playback` is. While a take is under way (`busy`) Space ends it, as R and
/// Escape do; K and L ask once for what they would do, which is refused in
/// words, and never end the take.
fn play_key(key: Key, playback: &Playback, busy: bool) -> Vec<Action> {
    match key {
        Key::Space if busy => vec![Action::StopRecord],
        Key::Space => vec![Action::TogglePlay],
        Key::K if playback.playing => vec![Action::TogglePlay],
        Key::L if playback.playing => {
            vec![Action::SetSpeed((playback.speed * 2.0).min(8.0))]
        }
        Key::L if busy => vec![Action::TogglePlay],
        Key::L => vec![Action::SetSpeed(1.0), Action::TogglePlay],
        _ => Vec::new(),
    }
}

/// The interface scale the Cmd or Ctrl chords with plus, minus and 0 among
/// this frame's keys step `scale` to, taking those chords; `None` without
/// one. Either modifier serves on every system. `=` is plus without Shift,
/// as browsers have it.
fn scaled_by_keys(input: &mut egui::InputState, scale: f32) -> Option<f32> {
    let mut scaled = None;
    for (key, step) in [
        (Key::Equals, Scale::Larger),
        (Key::Plus, Scale::Larger),
        (Key::Minus, Scale::Smaller),
        (Key::Num0, Scale::Actual),
    ] {
        let presses = input.count_and_consume_key(Modifiers::COMMAND, key)
            + input.count_and_consume_key(Modifiers::CTRL, key);
        for _ in 0..presses {
            scaled = Some(step.applied_to(scaled.unwrap_or(scale)));
        }
    }
    scaled
}

/// Whether `key` is the application's to act on. The arrows and the two
/// removing keys are not while a control that moves and removes with them
/// has the keyboard (`owned`), whatever is held with them.
fn key_is_free(owned: bool, key: Key) -> bool {
    const OWNED: [Key; 6] = [
        Key::ArrowLeft,
        Key::ArrowRight,
        Key::ArrowUp,
        Key::ArrowDown,
        Key::Delete,
        Key::Backspace,
    ];
    !(owned && OWNED.contains(&key))
}

/// The command that takes `asset` out of the library, and what is said of
/// it besides. Trackers of media that no clip shows or reads any more have
/// nothing left to be read on, and nowhere a person would look for them:
/// they go with the media, in the same step of the history, and are named.
/// Media a clip still uses is left to the project to refuse.
pub(crate) fn remove_asset_command(project: &Project, asset: AssetId) -> (Command, Option<String>) {
    let plain = Command::RemoveAssets {
        asset_ids: vec![asset],
    };
    let used = (project.tracks.iter())
        .flat_map(|track| &track.clips)
        .any(|clip| {
            clip.asset_id() == Some(asset)
                || clip
                    .effects
                    .iter()
                    .any(|effect| effect.asset_id == Some(asset))
        });
    let trackers: Vec<&Arc<Tracker>> = (project.trackers.values())
        .filter(|tracker| tracker.asset_id == asset)
        .collect();
    if used || trackers.is_empty() {
        return (plain, None);
    }
    let names: Vec<&str> = (trackers.iter())
        .map(|tracker| tracker.name.as_str())
        .collect();
    let media = (project.assets.get(&asset)).map_or("The media", |asset| asset.name.as_str());
    let said = match names.as_slice() {
        [name] => format!("{media} was removed with its tracker {name}."),
        names => format!(
            "{media} was removed with its trackers {}.",
            names.join(", ")
        ),
    };
    let mut commands: Vec<Command> = (trackers.iter())
        .map(|tracker| Command::RemoveTracker {
            tracker_id: tracker.id,
        })
        .collect();
    commands.push(plain);
    (
        Command::Batch {
            label: "Remove media".to_owned(),
            commands,
        },
        Some(said),
    )
}

/// What becomes of the recovery copy when the window closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Parting {
    /// The work in the window is written to it.
    Keep,
    /// It describes work that was given up or is in the project's file.
    Remove,
    Leave,
}

/// Settles the recovery copy for `base` as the window closes, waiting at most
/// `wait` for the save worker.
///
/// The last copy goes through the worker like every one before it, never
/// beside it: the worker writes one copy at a time and keeps only the newest
/// request for a path, so a slow earlier write cannot land after this one and
/// replace it, whether or not `wait` runs out. The request carries `lease`
/// until it is written, and `document` so that the copy records the file it
/// was made beside.
///
/// A copy is removed only once the worker is known to be idle. When it is
/// not, a write is still under way that would bring the copy back, or that a
/// removal would race; the copy is left, and is offered at the next launch.
/// Offering work that was given up costs an answer. Losing work costs more.
///
/// Returns whether the worker was idle when this was done.
fn part_with_recovery(
    saver: &SaveWorker,
    editor: &Editor,
    base: &Path,
    lease: Option<Arc<RecoveryLease>>,
    document: Option<Arc<DocumentSession>>,
    parting: Parting,
    wait: Duration,
) -> bool {
    let Some(lease) = lease else {
        // Nothing here is this window's to write or remove.
        return saver.flush(wait).is_ok();
    };
    let asked = match parting {
        Parting::Keep => {
            saver.request_recovery(base.to_owned(), editor, lease.clone(), document.clone())
        }
        _ => Ok(()),
    };
    let idle = saver.flush(wait).is_ok();
    match parting {
        // A worker that took no request and has nothing left to write
        // cannot race a write made here.
        Parting::Keep if asked.is_err() && idle => {
            let _ = match &document {
                Some(document) => editor_store::save_recovery_with_document(document, editor),
                None => editor_store::save_recovery(base, editor),
            };
        }
        Parting::Remove if idle => {
            let _ = editor_store::remove_recovery(base);
        }
        _ => {}
    }
    idle
}

/// Cmd+E opens the export sheet and Shift+Cmd+E the render queue, the
/// shifted chord read first. Each is taken from the input whatever is
/// open, and acts over nothing or over the other of the two, which it
/// takes the place of; neither opens again over itself nor over another
/// sheet.
fn export_chords(input: &mut egui::InputState, overlay: &Overlay) -> Option<Action> {
    let queue = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::E);
    let export = KeyboardShortcut::new(Modifiers::COMMAND, Key::E);
    let action = if input.consume_shortcut(&queue) {
        Action::OpenRenderQueue
    } else if input.consume_shortcut(&export) {
        Action::OpenExport
    } else {
        return None;
    };
    // Each opens over nothing, and over the other: the export sheet and the
    // render queue are one press apart.
    let over = match action {
        Action::OpenExport => Overlay::RenderQueue,
        _ => Overlay::Export,
    };
    (*overlay == Overlay::None || *overlay == over).then_some(action)
}

#[cfg(test)]
#[path = "app_export_tests.rs"]
mod export_tests;

/// The application's chords struck in this frame, by the index the
/// shortcuts act on, and consumed. While a field has the keyboard, undo and
/// select-all are its own, and so is Cmd+O with Shift or Option, which is
/// the inspector's chord left for the field: egui would otherwise take it
/// for Open, whose chord ignores them.
fn chords_struck(input: &mut egui::InputState, typing: bool) -> Vec<usize> {
    let command = |key| KeyboardShortcut::new(Modifiers::COMMAND, key);
    let shifted = |key| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, key);
    // More specific chords first: a shifted chord is not also its plain one.
    let chords = [
        (shifted(Key::Z), 0),
        (command(Key::Z), 1),
        (command(Key::Y), 0),
        (shifted(Key::S), 2),
        (command(Key::S), 3),
        (command(Key::O), 4),
        (command(Key::N), 5),
        (shifted(Key::I), 6),
        (command(Key::I), 7),
        (command(Key::D), 8),
        (command(Key::A), 9),
        (shifted(Key::G), 10),
        (command(Key::G), 11),
        (command(Key::Comma), 12),
        (shifted(Key::K), 17),
        (command(Key::K), 13),
        (command(Key::L), 14),
        (command(Key::T), 18),
    ];
    let inspector_chord = (input.events.iter()).any(|event| {
        matches!(
            event,
            egui::Event::Key { key: Key::O, pressed: true, modifiers, .. }
                if modifiers.shift || modifiers.alt
        )
    });
    let mut struck = Vec::new();
    for (chord, index) in chords {
        if typing && (matches!(index, 0 | 1 | 9 | 18) || index == 4 && inspector_chord) {
            continue;
        }
        if input.consume_shortcut(&chord) {
            struck.push(index);
        }
    }
    struck
}

/// Whether `key` is a choice's while one of its cells has the keyboard
/// (`choosing`): Left and Right step among the cells, and Delete, Up and
/// Down are still the application's.
fn choice_takes(choosing: bool, key: Key) -> bool {
    choosing && matches!(key, Key::ArrowLeft | Key::ArrowRight)
}

/// The chords that show and hide the side panels: Terminal's sidebar chord
/// for the library, Shift+Ctrl+B or Cmd+B, and Shift+Ctrl+O or Option+Cmd+O
/// for the inspector. A field that has the keyboard keeps them.
fn panel_chords(input: &mut egui::InputState, typing: bool) -> Option<Action> {
    if typing {
        return None;
    }
    let (library, inspector) = if cfg!(target_os = "macos") {
        (
            KeyboardShortcut::new(Modifiers::COMMAND, Key::B),
            KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::ALT, Key::O),
        )
    } else {
        let shifted = Modifiers::COMMAND | Modifiers::SHIFT;
        (
            KeyboardShortcut::new(shifted, Key::B),
            KeyboardShortcut::new(shifted, Key::O),
        )
    };
    if input.consume_shortcut(&library) {
        Some(Action::ToggleLibrary)
    } else if input.consume_shortcut(&inspector) {
        Some(Action::ToggleInspector)
    } else {
        None
    }
}

/// The file naming the folders this window's agent may use, beside the
/// window's agent session.
fn roots_file(session: &Path) -> PathBuf {
    session.with_extension("roots")
}

/// The person's usual places for media and work, looked up once.
fn usual_folders() -> &'static [PathBuf] {
    static FOLDERS: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();
    FOLDERS.get_or_init(|| {
        directories::UserDirs::new()
            .map(|dirs| {
                [
                    dirs.video_dir(),
                    dirs.desktop_dir(),
                    dirs.document_dir(),
                    dirs.download_dir(),
                    dirs.audio_dir(),
                    dirs.picture_dir(),
                ]
                .into_iter()
                .flatten()
                .map(Path::to_path_buf)
                .collect()
            })
            .unwrap_or_default()
    })
}

/// The part of a note about edits the agent made in projects this window
/// does not show, which its undo does not reach.
fn elsewhere_note(elsewhere: usize) -> String {
    match elsewhere {
        1 => " · 1 change in another project".to_owned(),
        n => format!(" · {n} changes in other projects"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(seconds: f64, sound: bool) -> Asset {
        let mut asset = Asset::new(
            "/media/shot.mp4".into(),
            MediaKind::Video,
            Time::from_seconds(seconds),
        );
        asset.width = 1920;
        asset.height = 1080;
        asset.frame_rate = Some(editor_core::FrameRate::default());
        asset.has_audio = sound;
        asset
    }

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("neptune-video-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn only_a_proposal_that_edited_is_called_applied() {
        assert_eq!(
            settled(ai::Applied::Changed),
            (PlanState::Applied, "Applied")
        );
        assert_eq!(
            settled(ai::Applied::Unchanged),
            (PlanState::Unchanged, "No changes")
        );
        // The two say different things, and only one promises an undo.
        assert!(PlanState::Applied.said().contains("Undo"));
        let unchanged = PlanState::Unchanged.said();
        assert!(!unchanged.contains("Undo") && !unchanged.contains("Applied"));
    }

    fn renamed(editor: &mut Editor, name: &str) {
        editor
            .dispatch(Command::RenameProject {
                name: name.to_owned(),
            })
            .unwrap();
    }

    fn kept_name(base: &Path) -> String {
        editor_store::load_recovery(base)
            .unwrap()
            .project()
            .name
            .clone()
    }

    /// A worker that cannot start its next write: it waits while this many
    /// answers have not been taken.
    fn stalled(base: &Path, editor: &Editor, lease: &Arc<RecoveryLease>) -> SaveWorker {
        let saver = SaveWorker::new(|| {});
        for _ in 0..32 {
            saver
                .request_with_lease(base.to_owned(), editor, true, lease.clone())
                .unwrap();
            saver.flush(Duration::from_secs(20)).unwrap();
        }
        saver
    }

    fn resume(saver: &SaveWorker) {
        while saver.poll().is_some() {}
        saver.flush(Duration::from_secs(20)).unwrap();
        while saver.poll().is_some() {}
    }

    #[test]
    fn the_last_recovery_copy_is_written_by_the_worker_after_an_earlier_one() {
        let dir = scratch();
        let base = dir.join("film.nvp");
        let lease = Arc::new(RecoveryLease::acquire(&base).unwrap());
        let mut editor = blank();
        let saver = SaveWorker::new(|| {});
        renamed(&mut editor, "Earlier");
        saver
            .request_with_lease(base.clone(), &editor, true, lease.clone())
            .unwrap();
        renamed(&mut editor, "Latest");
        part_with_recovery(
            &saver,
            &editor,
            &base,
            Some(lease),
            None,
            Parting::Keep,
            Duration::from_secs(20),
        );
        assert_eq!(kept_name(&base), "Latest");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_wait_that_runs_out_leaves_the_last_copy_with_the_worker() {
        let dir = scratch();
        let base = dir.join("film.nvp");
        let lease = Arc::new(RecoveryLease::acquire(&base).unwrap());
        let mut editor = blank();
        renamed(&mut editor, "Written");
        let saver = stalled(&base, &editor, &lease);
        // An earlier copy is still to be written when the window closes.
        renamed(&mut editor, "Earlier");
        saver
            .request_with_lease(base.clone(), &editor, true, lease.clone())
            .unwrap();
        renamed(&mut editor, "Latest");
        let latest = editor.revision();
        part_with_recovery(
            &saver,
            &editor,
            &base,
            Some(lease),
            None,
            Parting::Keep,
            Duration::from_millis(50),
        );
        // Nothing was written beside the worker.
        assert_eq!(kept_name(&base), "Written");
        // What it writes once it can is the last copy, and nothing after it.
        let mut written = Vec::new();
        for _ in 0..3 {
            while let Some(result) = saver.poll() {
                written.push(result.revision);
            }
            saver.flush(Duration::from_secs(20)).unwrap();
        }
        assert_eq!(written.last(), Some(&latest));
        assert_eq!(written.iter().filter(|r| **r == latest).count(), 1);
        assert_eq!(kept_name(&base), "Latest");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_recovery_copy_is_removed_only_when_the_worker_is_idle() {
        let dir = scratch();
        let base = dir.join("film.nvp");
        let copy = editor_store::recovery_path(&base);
        let lease = Arc::new(RecoveryLease::acquire(&base).unwrap());
        let mut editor = blank();
        renamed(&mut editor, "Written");
        let saver = stalled(&base, &editor, &lease);
        renamed(&mut editor, "Under way");
        saver
            .request_with_lease(base.clone(), &editor, true, lease.clone())
            .unwrap();
        let wait = Duration::from_millis(50);
        part_with_recovery(
            &saver,
            &editor,
            &base,
            Some(lease.clone()),
            None,
            Parting::Remove,
            wait,
        );
        // A write is still to come: the copy is left, and the write lands.
        assert!(copy.exists());
        resume(&saver);
        assert_eq!(kept_name(&base), "Under way");

        // A copy another window owns is never removed.
        part_with_recovery(&saver, &editor, &base, None, None, Parting::Remove, wait);
        assert!(copy.exists());
        part_with_recovery(
            &saver,
            &editor,
            &base,
            Some(lease.clone()),
            None,
            Parting::Leave,
            wait,
        );
        assert!(copy.exists());
        part_with_recovery(
            &saver,
            &editor,
            &base,
            Some(lease),
            None,
            Parting::Remove,
            wait,
        );
        assert!(!copy.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_narrow_window_shows_the_side_panel_asked_for_last() {
        // Both beside the viewer in a window with the room: §2.4, from a
        // window of 820, Terminal's narrowest with its sidebar.
        assert_eq!(LIBRARY_FITS, 820.0 - 2.0 * metrics::GUTTER);
        assert_eq!(side_panels(1428.0, true, true, false), (true, true));
        assert_eq!(side_panels(LIBRARY_FITS, true, true, true), (true, true));
        assert_eq!(side_panels(807.0, true, true, true), (true, false));
        // 760 points across leave 748: one panel, and the library is one of
        // the two it can be.
        assert_eq!(side_panels(748.0, true, true, false), (false, true));
        assert_eq!(side_panels(748.0, true, true, true), (true, false));
        // Alone, either is shown whichever was asked for last.
        assert_eq!(side_panels(748.0, true, false, false), (true, false));
        assert_eq!(side_panels(748.0, false, true, true), (false, true));
        assert_eq!(side_panels(748.0, false, false, true), (false, false));
    }

    #[test]
    fn the_side_panels_shrink_alike_to_keep_the_viewer() {
        // §2.4: the library at 316 and the inspector at 300, as preferred.
        for (window, library, inspector, viewer) in [
            (1440.0, 316.0, 300.0, 824.0),
            (1100.0, 316.0, 300.0, 484.0),
            (1000.0, 268.0, 252.0, 480.0),
            (900.0, 220.0, 240.0, 440.0),
            (820.0, 220.0, 240.0, 360.0),
        ] {
            let widths = panel_widths(window, 316.0, 300.0, (true, true));
            assert_eq!(widths, (library, inspector), "{window}");
            assert_eq!(window - widths.0 - widths.1, viewer, "{window}");
        }
        // One alone leaves the viewer 480 beside the gutter, within its
        // least and the width preferred.
        for (window, alone, viewer) in [(819.0, 316.0, 497.0), (760.0, 274.0, 480.0)] {
            let (library, _) = panel_widths(window, 316.0, 300.0, (true, false));
            assert_eq!(
                (library, window - library - metrics::GUTTER),
                (alone, viewer)
            );
        }
        assert_eq!(panel_widths(760.0, 316.0, 300.0, (false, true)).1, 274.0);
        assert_eq!(panel_widths(600.0, 316.0, 300.0, (false, true)).1, 240.0);
        // One that gives all it can leaves the rest to the other.
        assert_eq!(
            panel_widths(1000.0, 230.0, 400.0, (true, true)),
            (220.0, 300.0)
        );
    }

    #[test]
    fn the_panel_chords_show_and_hide_the_panels_but_not_while_typing() {
        let chord = |key, typing| {
            let ctx = egui::Context::default();
            let modifiers = if cfg!(target_os = "macos") {
                match key {
                    Key::B => Modifiers::COMMAND,
                    _ => Modifiers::COMMAND | Modifiers::ALT,
                }
            } else {
                Modifiers::COMMAND | Modifiers::SHIFT
            };
            let raw = egui::RawInput {
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            };
            let mut answer = (None, false);
            let output = ctx.run_ui(raw, |ui| {
                let action = ui.ctx().input_mut(|i| panel_chords(i, typing));
                let left = ui.ctx().input(|i| {
                    (i.events.iter()).any(|event| matches!(event, egui::Event::Key { .. }))
                });
                answer = (action, left);
            });
            output.drop_without_applying_deltas();
            answer
        };
        assert!(matches!(
            chord(Key::B, false),
            (Some(Action::ToggleLibrary), false)
        ));
        assert!(matches!(
            chord(Key::O, false),
            (Some(Action::ToggleInspector), false)
        ));
        // A field keeps them, and they are left in the input for it.
        assert!(matches!(chord(Key::B, true), (None, true)));
        assert!(matches!(chord(Key::O, true), (None, true)));
        // Nor does Open take the inspector's chord from the field, though
        // its own chord ignores Shift and Option. Cmd+O alone still opens.
        let struck = |modifiers, typing| {
            let ctx = egui::Context::default();
            let raw = egui::RawInput {
                events: vec![egui::Event::Key {
                    key: Key::O,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            };
            let mut struck = Vec::new();
            let output = ctx.run_ui(raw, |ui| {
                struck = ui.ctx().input_mut(|i| {
                    let panels = panel_chords(i, typing);
                    assert!(panels.is_none() || !typing);
                    chords_struck(i, typing)
                });
            });
            output.drop_without_applying_deltas();
            struck
        };
        let inspector = if cfg!(target_os = "macos") {
            Modifiers::COMMAND | Modifiers::ALT
        } else {
            Modifiers::COMMAND | Modifiers::SHIFT
        };
        assert_eq!(struck(inspector, true), Vec::<usize>::new());
        assert_eq!(struck(inspector, false), Vec::<usize>::new());
        assert_eq!(struck(Modifiers::COMMAND, true), vec![4]);
        assert_eq!(struck(Modifiers::COMMAND, false), vec![4]);
    }

    #[test]
    fn a_dragged_side_panel_follows_the_pointer() {
        // §2.4 and §2.5: the library at 316 and the inspector at 300
        // preferred. Drawn widths are derived from the preferred ones, so a
        // drag stores the preference that draws the edge where the pointer
        // took it.
        let drag = |window: f32, shown, library, steps: &[f32]| {
            let mut preferred = (316.0, 300.0);
            let mut drawn = Vec::new();
            for delta in steps {
                let widths = panel_widths(window, preferred.0, preferred.1, shown);
                let most = window - 360.0 - if shown.0 && shown.1 { 240.0 } else { 6.0 };
                if library {
                    preferred.0 =
                        dragged_width(window, preferred, shown, true, widths.0 + delta, most);
                } else {
                    preferred.1 =
                        dragged_width(window, preferred, shown, false, widths.1 - delta, most);
                }
                drawn.push(panel_widths(window, preferred.0, preferred.1, shown));
            }
            (preferred, drawn)
        };
        let near = |a: f32, b: f32| (a - b).abs() < 0.05;
        // A window of 1000, where both have shrunk to 268 and 252: the
        // library's edge moves right with the pointer, two at a time, while
        // the inspector gives the viewer its 480 (§2.4 rule 1), until the
        // inspector is at its least. Then it holds, and never goes back.
        let (_, drawn) = drag(1000.0, (true, true), true, &[2.0; 8]);
        for (step, (library, inspector)) in drawn.iter().enumerate() {
            let expected = (270.0 + 2.0 * step as f32).min(1000.0 - 480.0 - 240.0);
            assert!(near(*library, expected), "{drawn:?}");
            assert!(near(1000.0 - library - inspector, 480.0), "{drawn:?}");
        }
        // And left with it, as the inspector takes back its room.
        let (_, drawn) = drag(1000.0, (true, true), true, &[-10.0; 3]);
        assert!(near(drawn[2].0, 238.0), "{drawn:?}");
        // The inspector's edge moves left with the pointer in the same way.
        let (_, drawn) = drag(1000.0, (true, true), false, &[-10.0; 2]);
        assert!(near(drawn[1].1, 272.0), "{drawn:?}");
        // With the room, the edge is the preference.
        let (preferred, drawn) = drag(1440.0, (true, true), true, &[24.0, -4.0]);
        assert!(near(drawn[1].0, 336.0) && near(preferred.0, 336.0));
        assert_eq!(drawn[1].1, 300.0);
        // A library alone in a window of 760 is held at 274: dragged wider it
        // goes no further and keeps its 316; dragged narrower it follows.
        let (preferred, drawn) = drag(760.0, (true, false), true, &[10.0, 10.0]);
        assert_eq!((preferred.0, drawn[1].0), (316.0, 274.0));
        let (preferred, drawn) = drag(760.0, (true, false), true, &[-10.0]);
        assert!(near(drawn[0].0, 264.0) && near(preferred.0, 264.0));
        // Nor does a drag past the least a panel can be drawn at lose what
        // was preferred.
        let (preferred, _) = drag(820.0, (true, true), true, &[-10.0]);
        assert_eq!(preferred.0, 316.0);
    }

    #[test]
    fn an_arrow_on_a_tab_keeps_the_strip_with_the_keyboard_on_it() {
        // §5.4 and §7.3: Right on the focused "Inspector" chooses "Agent" in
        // the same frame, and the strip is made in it, so that what has the
        // keyboard is among the nodes said to assistive technology.
        let ctx = egui::Context::default();
        let p = Palette::new(Appearance::Dark, theme::Accent::default());
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &p);
        ctx.enable_accesskit();
        let tabs = [
            ui::panel::Tab {
                label: "Inspector",
                icon: Icon::Adjustments,
                waiting: 0,
            },
            ui::panel::Tab {
                label: "Agent",
                icon: Icon::Sparkles,
                waiting: 0,
            },
        ];
        let mut selected = 0;
        let mut frame = |events: Vec<egui::Event>, focus: bool| {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 300.0))),
                events,
                ..Default::default()
            };
            let output = ctx.run_ui(raw, |ui| {
                if focus {
                    let first = ui.id().with(("panel-tab", "Inspector"));
                    ui.ctx().memory_mut(|memory| memory.request_focus(first));
                }
                let strip = Rect::from_min_size(pos2(6.0, 0.0), vec2(300.0, ui::panel::TABS));
                if let Some(chosen) = tab_strip(ui, strip, p, &tabs, selected) {
                    selected = chosen;
                }
            });
            let update = output.platform_output.accesskit_update.clone();
            output.drop_without_applying_deltas();
            update
        };
        frame(Vec::new(), true);
        frame(Vec::new(), false);
        let right = egui::Event::Key {
            key: Key::ArrowRight,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        let update = frame(vec![right], false).expect("accesskit is on");
        assert_eq!(selected, 1);
        assert!(
            (update.nodes.iter()).any(|(id, _)| *id == update.focus),
            "the keyboard is on a node that is not there"
        );
    }

    #[test]
    fn a_panel_beside_one_that_slides_widens_with_it() {
        // §2.5: at 1000 both are 268 and 252, and alone 316 and 300. As the
        // inspector slides away, the library widens with it, not at once.
        let preferred = (316.0, 300.0);
        assert_eq!(
            drawn_widths(1000.0, preferred, true, (1.0, 1.0)),
            (268.0, 252.0)
        );
        assert_eq!(
            drawn_widths(1000.0, preferred, true, (1.0, 0.5)),
            (292.0, 252.0)
        );
        assert_eq!(drawn_widths(1000.0, preferred, true, (1.0, 0.0)).0, 316.0);
        // Where only one fits, each is as wide as it is alone.
        assert_eq!(
            drawn_widths(760.0, preferred, false, (0.3, 0.7)),
            (274.0, 274.0)
        );
    }

    #[test]
    fn what_is_made_in_and_after_the_side_panels_keeps_its_names_as_they_come_and_go() {
        // §2.4 and §7.4: in a narrow window Ctrl+K slides the library out as
        // the right panel slides in, and the command line asks for the
        // keyboard in the first frame of the slide. It keeps it when the
        // library is no longer drawn, as does what is made after the panels.
        let ctx = egui::Context::default();
        let screen = Rect::from_min_size(egui::Pos2::ZERO, vec2(760.0, 560.0));
        let band = Rect::from_min_max(pos2(0.0, 44.0), pos2(760.0, 300.0));
        let library = Rect::from_min_max(pos2(0.0, 44.0), pos2(274.0, 300.0));
        let right = Rect::from_min_max(pos2(486.0, 44.0), pos2(760.0, 300.0));
        let mut text = String::new();
        let mut frame = |reveal: f32, ask: bool| {
            let mut seen = (None, None);
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ui| {
                    if let Some(mut pane) = side_pane(ui, "library-pane", library, band, reveal) {
                        let _ = pane.button("Media");
                    }
                    if let Some(mut pane) = side_pane(ui, "right-pane", right, band, 1.0) {
                        let field = pane.text_edit_singleline(&mut text);
                        if ask {
                            field.request_focus();
                        }
                        seen.0 = Some(field.id);
                    }
                    seen.1 = Some(ui.next_auto_id());
                },
            );
            output.drop_without_applying_deltas();
            let focused = ctx.memory(|memory| memory.focused());
            (seen, focused)
        };
        let ((field, after), focused) = frame(1.0, true);
        assert_eq!(focused, field);
        for reveal in [0.5, 0.0, 0.0] {
            let (seen, focused) = frame(reveal, false);
            assert_eq!(seen, (field, after), "at {reveal}");
            assert_eq!(focused, field, "at {reveal}: the field keeps the keyboard");
        }
    }

    #[test]
    fn a_hidden_panel_hands_its_keyboard_to_the_lanes_unless_a_panel_took_it() {
        // §2.4 rule 6: the keyboard a hidden panel had goes to the lanes, or
        // is given up while the graph editor has their place.
        let ctx = egui::Context::default();
        let (library, field) = (
            egui::Id::new("a library tab"),
            egui::Id::new("the command line"),
        );
        let lanes = egui::Id::new("timeline-lanes");
        let handed = |asked: &[egui::Id], handed: Option<egui::Id>, graph: bool| {
            let mut focused = None;
            ctx.run_ui(egui::RawInput::default(), |ui| {
                for id in asked {
                    ui.memory_mut(|memory| memory.request_focus(*id));
                }
                hand_off_keyboard(ui.ctx(), handed, graph);
                focused = ui.memory(|memory| memory.focused());
            })
            .drop_without_applying_deltas();
            focused
        };
        assert_eq!(handed(&[library], Some(library), false), Some(lanes));
        assert_eq!(handed(&[library], Some(library), true), None);
        // The panel shown in its place asked for it in the same frame, as
        // Ctrl+K's command line does under Reduce motion, and keeps it.
        assert_eq!(handed(&[library, field], Some(library), false), Some(field));
        assert_eq!(handed(&[library, field], Some(library), true), Some(field));
        // Nothing hidden held it.
        assert_eq!(handed(&[field], None, false), Some(field));
    }

    #[test]
    fn reset_layout_brings_back_the_fresh_window() {
        // §2.5: library 316, inspector 300, the timeline at 0.42, and both
        // panels shown. It reads and writes the settings alone, so the
        // project and its history are as they were.
        let mut settings = Settings {
            library_width: 480.0,
            inspector_width: 250.0,
            timeline_share: 0.6,
            show_library: false,
            show_inspector: false,
            ..Settings::default()
        };
        reset_layout(&mut settings);
        assert_eq!(
            (
                settings.library_width,
                settings.inspector_width,
                settings.timeline_share,
                settings.show_library,
                settings.show_inspector,
            ),
            (316.0, 300.0, 0.42, true, true)
        );
    }

    #[test]
    fn cmd_or_ctrl_with_plus_minus_and_0_scale_the_interface() {
        let press = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        // Ctrl as Linux and Windows send it, and Cmd and Ctrl as macOS does.
        let ctrl = Modifiers {
            ctrl: true,
            command: true,
            ..Modifiers::NONE
        };
        let cmd = Modifiers {
            mac_cmd: true,
            command: true,
            ..Modifiers::NONE
        };
        let scaled = |events: Vec<egui::Event>, scale: f32| {
            let ctx = egui::Context::default();
            let mut scaled = None;
            let mut left = Vec::new();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    scaled = ui.input_mut(|i| scaled_by_keys(i, scale));
                    left = ui.input(|i| i.events.clone());
                },
            );
            output.textures_delta.clear();
            (scaled, left)
        };
        for modifiers in [ctrl, cmd, Modifiers::CTRL] {
            assert_eq!(
                scaled(vec![press(Key::Equals, modifiers)], 1.0).0,
                Some(1.1)
            );
            assert_eq!(scaled(vec![press(Key::Minus, modifiers)], 1.0).0, Some(0.9));
            assert_eq!(scaled(vec![press(Key::Num0, modifiers)], 1.7).0, Some(1.0));
        }
        // Plus as most layouts type it, with Shift, and once for each press
        // a frame is handed.
        let plus = press(Key::Plus, ctrl | Modifiers::SHIFT);
        let (twice, left) = scaled(vec![plus.clone(), plus], 1.0);
        assert_eq!(twice, Some(1.2));
        assert!(left.is_empty(), "the chords are taken: {left:?}");
        // The keys alone are the timeline's, and are left for it.
        let (alone, left) = scaled(vec![press(Key::Equals, Modifiers::NONE)], 1.0);
        assert_eq!(alone, None);
        assert_eq!(left.len(), 1);
        // At either end the size stays where it is.
        assert_eq!(scaled(vec![press(Key::Equals, ctrl)], 3.0).0, Some(3.0));
        assert_eq!(scaled(vec![press(Key::Minus, ctrl)], 0.5).0, Some(0.5));
    }

    #[test]
    fn a_key_is_read_with_the_modifiers_of_its_own_event() {
        let key = |key, pressed, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        // A whole Shift+→ inside one frame: Shift is up again by its end.
        let chord = [
            key(Key::ArrowRight, true, Modifiers::SHIFT),
            key(Key::ArrowRight, false, Modifiers::SHIFT),
        ];
        assert!(struck(&chord, Key::ArrowRight, Held::Shift));
        assert!(!struck(&chord, Key::ArrowRight, Held::Alone));
        assert!(!struck(&chord, Key::ArrowLeft, Held::Shift));

        let plain = [key(Key::ArrowRight, true, Modifiers::NONE)];
        assert!(struck(&plain, Key::ArrowRight, Held::Alone));
        assert!(!struck(&plain, Key::ArrowRight, Held::Shift));

        // A release is not a press, and other chords are neither.
        let others = [
            key(Key::ArrowRight, false, Modifiers::NONE),
            key(Key::ArrowLeft, true, Modifiers::SHIFT | Modifiers::COMMAND),
            key(Key::ArrowLeft, true, Modifiers::ALT),
        ];
        assert!(struck(&others, Key::ArrowLeft, Held::Alt));
        assert!(!struck(&others, Key::ArrowRight, Held::Alt));
        for held in [Held::Alone, Held::Shift] {
            assert!(!struck(&others, Key::ArrowRight, held));
            assert!(!struck(&others, Key::ArrowLeft, held));
        }
    }

    #[test]
    fn r_asks_for_a_take_or_ends_it_and_is_a_letter_in_a_field() {
        let ctx = egui::Context::default();
        let mut typed = String::new();
        // One pass with R pressed, read as `shortcuts` reads it, before the
        // panels; a field is drawn, and has the keyboard when `focus`.
        let mut pass = |press: bool, focus: bool, busy: bool| {
            let events = match press {
                true => vec![egui::Event::Key {
                    key: Key::R,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
                false => Vec::new(),
            };
            let input = egui::RawInput {
                events,
                ..Default::default()
            };
            let mut asked = None;
            let output = ctx.run_ui(input, |ui| {
                asked = record_key(ui.ctx(), busy);
                let field = ui.text_edit_singleline(&mut typed);
                if focus {
                    field.request_focus();
                } else {
                    field.surrender_focus();
                }
            });
            output.drop_without_applying_deltas();
            asked
        };
        assert!(matches!(pass(true, false, false), Some(Action::Record)));
        assert!(matches!(pass(true, false, true), Some(Action::StopRecord)));
        assert!(pass(false, false, false).is_none());
        // A field takes the keyboard, and from the next pass R is a letter.
        pass(false, true, false);
        assert!(pass(true, true, false).is_none());
        assert!(pass(true, true, true).is_none());
        // Once it lets go, R records again.
        pass(false, false, false);
        assert!(matches!(pass(true, false, false), Some(Action::Record)));
        // Held with a modifier, it is another chord.
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key: Key::R,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::COMMAND,
            }],
            ..Default::default()
        };
        let mut asked = None;
        ctx.run_ui(input, |ui| asked = record_key(ui.ctx(), false))
            .drop_without_applying_deltas();
        assert!(asked.is_none());
    }

    #[test]
    fn during_a_take_space_ends_it_and_k_and_l_ask_once_and_never_end_it() {
        let stopped = Playback::default();
        let mut playing = Playback::default();
        playing.set_speed(2.0).unwrap();
        playing.play();
        let asked = |key, playback: &Playback, busy| {
            play_key(key, playback, busy)
                .iter()
                .map(|action| match action {
                    Action::StopRecord => "stop record".to_owned(),
                    Action::TogglePlay => "toggle".to_owned(),
                    Action::SetSpeed(speed) => format!("speed {speed}"),
                    other => panic!("{key:?} asked for {other:?}"),
                })
                .collect::<Vec<_>>()
        };
        // Without a take, as ever.
        assert_eq!(asked(Key::Space, &stopped, false), ["toggle"]);
        assert_eq!(asked(Key::K, &stopped, false), Vec::<String>::new());
        assert_eq!(asked(Key::K, &playing, false), ["toggle"]);
        assert_eq!(asked(Key::L, &stopped, false), ["speed 1", "toggle"]);
        assert_eq!(asked(Key::L, &playing, false), ["speed 4"]);
        // During a take, whether the timeline plays or has stopped at its
        // end: only Space ends it, and K or L is one request `perform`
        // refuses in words.
        for playback in [&stopped, &playing] {
            assert_eq!(asked(Key::Space, playback, true), ["stop record"]);
            let l = asked(Key::L, playback, true);
            assert_eq!(l.len(), 1, "{l:?}");
            assert!(!l.contains(&"stop record".to_owned()));
            assert!(!asked(Key::K, playback, true).contains(&"stop record".to_owned()));
        }
        assert_eq!(asked(Key::L, &stopped, true), ["toggle"]);
    }

    #[test]
    fn every_press_of_a_key_inside_one_frame_is_counted() {
        let key = |key, pressed, repeat, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat,
            modifiers,
        };
        // Three taps of → and one of Shift+→ that one slow frame was handed,
        // with their releases and a repeat of a held ←.
        let frame = [
            key(Key::ArrowRight, true, false, Modifiers::NONE),
            key(Key::ArrowRight, false, false, Modifiers::NONE),
            key(Key::ArrowRight, true, false, Modifiers::NONE),
            key(Key::ArrowRight, false, false, Modifiers::NONE),
            key(Key::ArrowRight, true, false, Modifiers::SHIFT),
            key(Key::ArrowRight, true, false, Modifiers::NONE),
            key(Key::ArrowLeft, true, false, Modifiers::NONE),
            key(Key::ArrowLeft, true, true, Modifiers::NONE),
        ];
        assert_eq!(strikes(&frame, Key::ArrowRight, Held::Alone), 3);
        assert_eq!(strikes(&frame, Key::ArrowRight, Held::Shift), 1);
        assert_eq!(strikes(&frame, Key::ArrowLeft, Held::Alone), 2);
        assert_eq!(strikes(&frame, Key::ArrowLeft, Held::Shift), 0);
        assert_eq!(strikes(&frame, Key::J, Held::Alone), 0);
        assert!(struck(&frame, Key::ArrowRight, Held::Alone));
        assert!(!struck(&frame, Key::J, Held::Alone));

        // The playhead goes as many frames as the key went down.
        let mut playback = editor_core::Playback::default();
        let rate = editor_core::FrameRate::default();
        playback.step(strikes(&frame, Key::ArrowRight, Held::Alone) as i64, rate);
        assert_eq!(playback.position, rate.frame_time(3));
    }

    #[test]
    fn waiting_imports_are_handed_back_in_order() {
        let batch = |name: &str| vec![PathBuf::from(name)];
        let waiting = VecDeque::from([batch("/a"), batch("/b"), batch("/c")]);
        let mut asked = Vec::new();
        let (taken, refused) = hand_back(waiting, |paths| {
            asked.push(paths.clone());
            if paths == batch("/b") {
                Err("queue full")
            } else {
                Ok(())
            }
        });
        // One refusal does not stop the batches behind it.
        assert_eq!(asked, [batch("/a"), batch("/b"), batch("/c")]);
        assert_eq!(taken, [batch("/a"), batch("/c")]);
        assert_eq!(refused, ["queue full"]);

        let (taken, refused) = hand_back(VecDeque::new(), |_| -> Result<(), ()> {
            panic!("nothing was waiting")
        });
        assert!(taken.is_empty() && refused.is_empty());
    }

    #[test]
    fn an_import_cancelled_with_the_worker_arrives_once_after_the_hand_back() {
        let dir = scratch();
        let still = dir.join("still.png");
        image::RgbaImage::from_pixel(8, 6, image::Rgba([200, 40, 40, 255]))
            .save(&still)
            .unwrap();
        let media = MediaService::new(|| {}).unwrap();
        let mut waiting = VecDeque::new();
        media.import(vec![still.clone()]).unwrap();
        waiting.push_back(vec![still.clone()]);
        // A changed asset restarts the worker under the waiting import.
        media.cancel();
        let (mut waiting, refused) = hand_back(waiting, |paths| media.import(paths).map(drop));
        assert!(refused.is_empty());
        assert_eq!(waiting.len(), 1);

        // What the window does with the worker's answers: one from before
        // the restart is not about anything that is waiting.
        let mut arrived = Vec::new();
        let until = Instant::now() + Duration::from_secs(20);
        while !waiting.is_empty() && Instant::now() < until {
            match media.poll() {
                Some(MediaEvent::Imported {
                    generation, assets, ..
                }) if generation == media.generation() => {
                    waiting.pop_front();
                    arrived.extend(assets);
                }
                Some(_) => {}
                None => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(waiting.is_empty(), "the import was never answered");
        assert_eq!(arrived.len(), 1);
        assert_eq!(arrived[0].kind, MediaKind::Image);
        assert_eq!((arrived[0].width, arrived[0].height), (8, 6));
        // Nothing more follows it.
        std::thread::sleep(Duration::from_millis(150));
        assert!(!std::iter::from_fn(|| media.poll()).any(|event| matches!(
            event,
            MediaEvent::Imported { generation, .. } if generation == media.generation()
        )));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_recovery_write_is_asked_for_again_without_a_new_edit() {
        let start = Instant::now();
        let later = |intervals: u32| start + RECOVERY_INTERVAL * intervals;
        let mut autosave = Autosave::new(0, start);
        // Nothing changed: nothing to keep.
        assert!(!autosave.due(0, later(5)));
        // An edit is kept once the interval has passed.
        assert!(!autosave.due(1, start + Duration::from_secs(1)));
        assert!(autosave.due(1, later(1)));
        autosave.submitted(later(1));
        assert!(!autosave.due(1, later(9)), "one write at a time");
        // The write failed: the same revision is due again, after a wait.
        autosave.answered(1, false, later(2));
        assert!(!autosave.due(1, later(2) + Duration::from_secs(1)));
        assert!(autosave.due(1, later(3)));
        // Once written, it is not asked for until there is more to keep.
        autosave.submitted(later(3));
        autosave.answered(1, true, later(3));
        assert!(!autosave.due(1, later(9)));
        assert!(autosave.due(2, later(9)));
        // A copy that could not be asked for is also tried again.
        autosave.failed(later(9));
        assert!(!autosave.due(2, later(9)));
        assert!(autosave.due(2, later(10)));
    }

    #[test]
    fn an_answer_about_an_older_copy_leaves_newer_work_due() {
        let start = Instant::now();
        let mut autosave = Autosave::new(0, start);
        autosave.submitted(start);
        autosave.answered(3, true, start);
        assert!(autosave.due(4, start + RECOVERY_INTERVAL));
    }

    #[test]
    fn the_source_transport_stays_inside_the_source() {
        let mut source = Source::open(&video(2.0, true)).unwrap();
        let rate = source.project.settings.frame_rate;
        let last = rate.frame_time(59);
        assert!(source.audible());
        assert_eq!(source.project.tracks.len(), 1);

        source.apply(Transport::Step(-3)).unwrap();
        assert_eq!(source.playback.position, Time::ZERO);
        source.apply(Transport::Step(1)).unwrap();
        assert_eq!(source.playback.position, rate.frame_time(1));
        source.apply(Transport::Step(500)).unwrap();
        assert_eq!(source.playback.position, last);
        source
            .apply(Transport::Seek(Time::from_seconds(90.0)))
            .unwrap();
        assert_eq!(source.playback.position, Time::from_seconds(2.0));
        source.apply(Transport::JumpEnd).unwrap();
        assert_eq!(source.playback.position, last);

        // Play from the end starts over, and asks for the sound.
        assert!(source.apply(Transport::TogglePlay).unwrap());
        assert!(source.playback.playing);
        assert_eq!(source.playback.position, Time::ZERO);
        // A seek while paused needs no sound.
        source.apply(Transport::TogglePlay).unwrap();
        assert!(
            !source
                .apply(Transport::Seek(Time::from_seconds(1.0)))
                .unwrap()
        );
        assert!(source.apply(Transport::SetSpeed(99.0)).is_err());
    }

    #[test]
    fn the_source_clock_stops_at_the_end_or_starts_over_when_looping() {
        let start = Instant::now();
        let mut source = Source::open(&video(2.0, false)).unwrap();
        assert!(!source.audible());
        source.apply(Transport::TogglePlay).unwrap();
        assert!(!source.tick(start, None));
        assert!(!source.tick(start + Duration::from_secs(1), None));
        assert_eq!(source.playback.position, Time::from_seconds(1.0));
        // The device clock is followed when there is one.
        assert!(!source.tick(start, Some(Time::from_seconds(1.5))));
        assert_eq!(source.playback.position, Time::from_seconds(1.5));
        assert!(source.tick(start, Some(Time::from_seconds(5.0))));
        assert!(!source.playback.playing);
        assert_eq!(source.playback.position, Time::from_seconds(2.0));

        source.apply(Transport::ToggleLoop).unwrap();
        source.apply(Transport::TogglePlay).unwrap();
        assert!(source.tick(start, Some(Time::from_seconds(5.0))));
        assert!(source.playback.playing);
        assert_eq!(source.playback.position, Time::ZERO);
    }

    #[test]
    fn a_still_image_has_no_transport() {
        let mut still = video(5.0, false);
        still.kind = MediaKind::Image;
        still.frame_rate = None;
        let mut source = Source::open(&still).unwrap();
        assert!(source.apply(Transport::TogglePlay).is_err());
        assert!(!source.playback.playing);
    }

    #[test]
    fn a_checkpoint_file_name_says_when_it_was_made() {
        let path = Path::new("/p/film.nvp.checkpoints/1760250000123-6f0c.nvp");
        assert_eq!(checkpoint_time(path), Some(1_760_250_000));
        assert_eq!(checkpoint_time(Path::new("/p/notes.nvp")), None);
    }

    /// The worker's answer to the one save that was asked for.
    fn answered(saver: &SaveWorker) -> editor_store::SaveResult {
        saver.flush(Duration::from_secs(20)).unwrap();
        saver.poll().expect("the save was answered")
    }

    #[test]
    fn a_file_another_writer_saved_is_not_replaced_and_the_edits_stay_unsaved() {
        let dir = scratch();
        let file = dir.join("film.nvp");
        editor_store::save_project(&file, &Project::default()).unwrap();
        let saver = SaveWorker::new(|| {});

        // Two windows open the same file; the other one saves first.
        let (project, session) = editor_store::load_project_with_session(&file).unwrap();
        let mut editor = Editor::new(project).unwrap();
        let (project, theirs) = editor_store::load_project_with_session(&file).unwrap();
        let mut other = Editor::new(project).unwrap();
        renamed(&mut other, "Theirs");
        other.register_save();
        saver.request_document(theirs, &other).unwrap();
        assert!(answered(&saver).result.is_ok());

        renamed(&mut editor, "Mine");
        for _ in 0..2 {
            // Asking again changes nothing: the session does not take up
            // what is in the file now.
            editor.register_save();
            saver.request_document(session.clone(), &editor).unwrap();
            let refused = answered(&saver);
            assert_eq!(refused.context_id, editor.instance_id());
            assert_eq!(refused.path, session.path());
            assert_eq!(
                other_writer(refused.result.as_ref().unwrap_err()),
                Some(false)
            );
        }
        assert!(editor.is_dirty());
        assert_eq!(editor.project().name, "Mine");
        assert_eq!(editor_store::load_project(&file).unwrap().name, "Theirs");

        // Choosing the open file in Save As is still the open file.
        let prepared = DocumentSession::prepare(&dir.join(".").join("film.nvp")).unwrap();
        assert!(Arc::ptr_eq(
            &destination(Some(&session), prepared),
            &session
        ));

        // Another file takes the edits; the other writer's file is as it left it.
        let prepared = DocumentSession::prepare(&dir.join("mine.nvp")).unwrap();
        let elsewhere = destination(Some(&session), prepared);
        assert!(!Arc::ptr_eq(&elsewhere, &session));
        let revision = editor.register_save();
        saver.request_document(elsewhere.clone(), &editor).unwrap();
        let written = answered(&saver);
        assert!(written.result.is_ok());
        assert_eq!(written.revision, revision);
        editor.mark_saved(written.revision);
        assert!(!editor.is_dirty());
        assert_eq!(
            editor_store::load_project(elsewhere.path()).unwrap().name,
            "Mine"
        );
        assert_eq!(editor_store::load_project(&file).unwrap().name, "Theirs");

        // From there the new file is saved like any other.
        renamed(&mut editor, "Mine again");
        editor.register_save();
        saver.request_document(elsewhere.clone(), &editor).unwrap();
        assert!(answered(&saver).result.is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_checkpoint_restore_saves_through_the_session_the_file_was_opened_with() {
        let dir = scratch();
        let file = dir.join("film.nvp");
        editor_store::save_project(&file, &Project::default()).unwrap();
        let saver = SaveWorker::new(|| {});
        let (project, session) = editor_store::load_project_with_session(&file).unwrap();
        let mut editor = Editor::new(project.clone()).unwrap();
        renamed(&mut editor, "Later");
        editor.register_save();
        saver.request_document(session.clone(), &editor).unwrap();
        assert!(answered(&saver).result.is_ok());

        // Going back is an edit of the same document, not another file.
        let context = editor.instance_id();
        editor
            .dispatch(Command::RestoreProject {
                project: Box::new(project),
            })
            .unwrap();
        assert_eq!(editor.instance_id(), context);
        editor.register_save();
        saver.request_document(session, &editor).unwrap();
        assert!(answered(&saver).result.is_ok());
        assert_eq!(
            editor_store::load_project(&file).unwrap().name,
            Project::default().name
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_duplicate_never_replaces_a_file_that_is_there() {
        let dir = scratch();
        let project = Project::default();
        let taken = dir.join("taken.nvp");
        std::fs::write(&taken, b"someone's notes").unwrap();
        let refused = create_document(&taken, &project).unwrap_err();
        assert!(refused.contains("already exists"), "{refused}");
        assert_eq!(std::fs::read(&taken).unwrap(), b"someone's notes");

        let session = create_document(&dir.join("copy.nvp"), &project).unwrap();
        assert_eq!(
            editor_store::load_project(session.path()).unwrap().id,
            project.id
        );

        // A file that arrives after the destination was found free is kept.
        let raced = dir.join("raced.nvp");
        let session = DocumentSession::create(&raced).unwrap();
        std::fs::write(&raced, b"arrived first").unwrap();
        let error = editor_store::save_project_with_session(&session, &project).unwrap_err();
        assert_eq!(other_writer(&error), Some(false));
        assert_eq!(std::fs::read(&raced).unwrap(), b"arrived first");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_second_window_keeps_its_recovery_copy_apart_from_the_first() {
        let dir = scratch();
        let file = dir.join("film.nvp");
        let folder = dir.join("recovery");
        let instance = uuid::Uuid::new_v4();
        let home =
            |private| recovery_home(Some(&file), None, Some(&folder), instance, private).unwrap();
        // The first window's copy is beside the file, and it owns it.
        assert_eq!(home(false), file);
        let _first = RecoveryLease::acquire(&home(false)).unwrap();
        assert!(RecoveryLease::acquire(&home(false)).is_err());

        // The second keeps its own, and neither touches the other's.
        let own = home(true);
        assert_eq!(own, folder.join(format!("{instance}.nvp")));
        let _second = RecoveryLease::acquire(&own).unwrap();
        let mut editor = blank();
        renamed(&mut editor, "Second window");
        editor_store::save_recovery(&own, &editor).unwrap();
        assert_eq!(kept_name(&own), "Second window");
        assert!(!editor_store::recovery_path(&file).exists());

        // Without a settings folder there is nowhere private to go.
        assert_eq!(
            recovery_home(Some(&file), None, None, instance, true),
            Some(file.clone())
        );
        // A project without a file was in its own place all along.
        let unsaved = folder.join("earlier.nvp");
        assert_eq!(
            recovery_home(None, Some(&unsaved), Some(&folder), instance, false),
            Some(unsaved)
        );
        assert_eq!(
            recovery_home(None, None, Some(&folder), instance, false),
            Some(own)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_late_removal_leaves_a_copy_another_window_has_taken_up() {
        let dir = scratch();
        let base = dir.join("film.nvp");
        let copy = editor_store::recovery_path(&base);
        let mut editor = blank();
        renamed(&mut editor, "Theirs now");
        editor_store::save_recovery(&base, &editor).unwrap();

        // The window that asked for the removal no longer owns the copy,
        // and another has taken it up before the job ran.
        let theirs = RecoveryLease::acquire(&base).unwrap();
        remove_own_recovery(&base, None).unwrap();
        assert_eq!(kept_name(&base), "Theirs now");
        drop(theirs);

        // Ownership that travelled with the job removes the copy.
        let held = Arc::new(RecoveryLease::acquire(&base).unwrap());
        remove_own_recovery(&base, Some(held.clone())).unwrap();
        assert!(!copy.exists());
        drop(held);

        // An unowned copy is taken first, then removed; none is no error.
        editor_store::save_recovery(&base, &editor).unwrap();
        remove_own_recovery(&base, None).unwrap();
        assert!(!copy.exists());
        remove_own_recovery(&dir.join("never.nvp"), None).unwrap();
        assert!(!dir.join("never.nvp.lock").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A window that opened `file`, renamed the project and went away with
    /// that unsaved, leaving its copy through the worker as a closing window
    /// does. Returns the path the copy was written for.
    fn left_unsaved(file: &Path, name: &str) -> PathBuf {
        let (project, session) = editor_store::load_project_with_session(file).unwrap();
        let base = session.path().to_path_buf();
        let mut editor = Editor::new(project).unwrap();
        renamed(&mut editor, name);
        let saver = SaveWorker::new(|| {});
        let lease = Arc::new(RecoveryLease::acquire(&base).unwrap());
        let document = bound_document(&base, Some(&session));
        assert!(document.is_some());
        assert!(part_with_recovery(
            &saver,
            &editor,
            &base,
            Some(lease),
            document,
            Parting::Keep,
            Duration::from_secs(20),
        ));
        base
    }

    #[test]
    fn restored_work_does_not_replace_what_another_writer_saved_since_the_copy() {
        let dir = scratch();
        let file = dir.join("film.nvp");
        editor_store::save_project(&file, &Project::default()).unwrap();
        let base = left_unsaved(&file, "Mine");

        // Nobody wrote the file: the copy comes back with its file.
        let untouched = editor_store::load_recovery_with_document(&base).unwrap();
        assert!(!untouched.document_changed);
        let document = bound_document(&base, untouched.document.as_ref()).unwrap();
        assert_eq!(
            restored_home(&base, Some(&document)),
            (Some(base.clone()), None)
        );

        // Another writer saves the file before the copy is restored.
        let saver = SaveWorker::new(|| {});
        let (project, theirs) = editor_store::load_project_with_session(&file).unwrap();
        let mut other = Editor::new(project).unwrap();
        renamed(&mut other, "Theirs");
        other.register_save();
        saver.request_document(theirs, &other).unwrap();
        assert!(answered(&saver).result.is_ok());

        // The work and its file both come back, and the change is known
        // before anything is written.
        let restored = editor_store::load_recovery_with_document(&base).unwrap();
        assert!(restored.document_changed);
        let document = bound_document(&base, restored.document.as_ref()).unwrap();
        let mut editor = restored.editor;
        assert_eq!(editor.project().name, "Mine");
        assert!(editor.is_dirty());

        // The first save after the restore is refused.
        editor.register_save();
        saver.request_document(document.clone(), &editor).unwrap();
        let refused = answered(&saver);
        assert_eq!(
            other_writer(refused.result.as_ref().unwrap_err()),
            Some(false)
        );
        assert!(editor.is_dirty());
        assert_eq!(editor_store::load_project(&file).unwrap().name, "Theirs");

        // A copy written after the restore still records the file as it was
        // when the work was made, not as the other writer left it.
        let lease = Arc::new(RecoveryLease::acquire(&base).unwrap());
        saver
            .request_recovery(base.clone(), &editor, lease.clone(), Some(document))
            .unwrap();
        assert!(answered(&saver).result.is_ok());
        drop(lease);
        let again = editor_store::load_recovery_with_document(&base).unwrap();
        assert!(again.document.is_some() && again.document_changed);
        assert_eq!(editor_store::load_project(&file).unwrap().name, "Theirs");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_copy_that_names_no_file_comes_back_as_unsaved_work() {
        let dir = scratch();
        let file = dir.join("film.nvp");
        editor_store::save_project(&file, &Project::default()).unwrap();
        let (project, session) = editor_store::load_project_with_session(&file).unwrap();
        let base = session.path().to_path_buf();
        let mut editor = Editor::new(project).unwrap();
        renamed(&mut editor, "Mine");
        let unsaved = |base: &Path| {
            let restored = editor_store::load_recovery_with_document(base).unwrap();
            assert!(!restored.document_changed);
            let document = bound_document(base, restored.document.as_ref());
            assert!(document.is_none());
            assert_eq!(restored.editor.project().name, "Mine");
            assert!(restored.editor.is_dirty());
            // No file to save to: the copy stays where it was.
            assert_eq!(
                restored_home(base, document.as_ref()),
                (None, Some(base.to_path_buf()))
            );
        };

        // A copy from before copies recorded their file, beside that file.
        editor_store::save_recovery(&base, &editor).unwrap();
        unsaved(&base);

        // A second window's own copy: it is not beside the file, so it may
        // not speak for it, and the worker refuses one that tries.
        let private = dir.join("recovery").join("window.nvp");
        std::fs::create_dir_all(private.parent().unwrap()).unwrap();
        assert!(bound_document(&private, Some(&session)).is_none());
        assert!(bound_document(&base, Some(&session)).is_some());
        let saver = SaveWorker::new(|| {});
        let lease = Arc::new(RecoveryLease::acquire(&private).unwrap());
        assert!(
            saver
                .request_recovery(
                    private.clone(),
                    &editor,
                    lease.clone(),
                    Some(session.clone())
                )
                .is_err()
        );
        assert!(part_with_recovery(
            &saver,
            &editor,
            &private,
            Some(lease),
            bound_document(&private, Some(&session)),
            Parting::Keep,
            Duration::from_secs(20),
        ));
        unsaved(&private);

        // A copy that recorded its file, found beside another one.
        editor_store::remove_recovery(&base).unwrap();
        let base = left_unsaved(&file, "Mine");
        let moved = dir.join("elsewhere.nvp");
        std::fs::copy(
            editor_store::recovery_path(&base),
            editor_store::recovery_path(&moved),
        )
        .unwrap();
        unsaved(&moved);

        assert_eq!(
            editor_store::load_project(&file).unwrap().name,
            Project::default().name
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Work on `file` renamed twice and left in two copies that record no
    /// file: one beside it, as copies were written before they recorded
    /// theirs, and a window's private one. Returns where each was written for.
    fn left_unbound(dir: &Path, file: &Path) -> [PathBuf; 2] {
        let (project, session) = editor_store::load_project_with_session(file).unwrap();
        let base = session.path().to_path_buf();
        let mut editor = Editor::new(project).unwrap();
        renamed(&mut editor, "Draft");
        renamed(&mut editor, "Mine");
        editor_store::save_recovery(&base, &editor).unwrap();

        let private = dir.join("recovery").join("window.nvp");
        std::fs::create_dir_all(private.parent().unwrap()).unwrap();
        let lease = Arc::new(RecoveryLease::acquire(&private).unwrap());
        assert!(part_with_recovery(
            &SaveWorker::new(|| {}),
            &editor,
            &private,
            Some(lease),
            bound_document(&private, Some(&session)),
            Parting::Keep,
            Duration::from_secs(20),
        ));
        [base, private]
    }

    fn named(name: &str) -> Editor {
        let mut editor = blank();
        renamed(&mut editor, name);
        editor
    }

    /// The whole of the restored work: both renames, undone and redone.
    fn history_is_whole(editor: &mut Editor) {
        assert!(editor.is_dirty());
        assert_eq!(editor.history().count(), 2);
        assert!(editor.undo());
        assert_eq!(editor.project().name, "Draft");
        assert!(editor.undo());
        assert_eq!(editor.project().name, Project::default().name);
        assert!(!editor.can_undo());
        assert!(editor.redo() && editor.redo());
        assert_eq!(editor.project().name, "Mine");
        assert!(editor.is_dirty());
    }

    #[test]
    fn restored_work_without_a_file_never_replaces_a_file_that_is_there() {
        let dir = scratch();
        let file = dir.join("film.nvp");
        editor_store::save_project(&file, &Project::default()).unwrap();
        let other = dir.join("other.nvp");
        editor_store::save_project(&other, named("Theirs").project()).unwrap();
        let notes = dir.join("notes.nvp");
        std::fs::write(&notes, b"someone's notes").unwrap();
        let saver = SaveWorker::new(|| {});

        for base in left_unbound(&dir, &file) {
            let copy = editor_store::recovery_path(&base);
            let kept = [&file, &other, &notes, &copy].map(|path| std::fs::read(path).unwrap());
            let unchanged = || {
                let now = [&file, &other, &notes, &copy].map(|path| std::fs::read(path).unwrap());
                assert!(now == kept, "a file was written for {}", base.display());
            };

            let restored = editor_store::load_recovery_with_document(&base).unwrap();
            let document = bound_document(&base, restored.document.as_ref());
            let origin = Origin::restored(document.as_ref());
            assert_eq!(origin, Origin::Restored);
            let mut editor = restored.editor;

            // The file the work was made from, another project, and a file
            // that is no project, under any name they go by: all refused
            // before anything is written.
            for taken in [&file, &other, &notes, &dir.join(".").join("film.nvp")] {
                let refused = origin.prepare(taken).unwrap_err();
                assert!(refused.contains("already exists"), "{refused}");
                unchanged();
            }

            // A file that arrives after the destination was found free is
            // refused by the save, and the work stays unsaved.
            let raced = dir.join("raced.nvp");
            let session = origin.prepare(&raced).unwrap();
            std::fs::write(&raced, b"arrived first").unwrap();
            editor.register_save();
            saver.request_document(session, &editor).unwrap();
            let refused = answered(&saver);
            assert_eq!(
                other_writer(refused.result.as_ref().unwrap_err()),
                Some(false)
            );
            assert_eq!(std::fs::read(&raced).unwrap(), b"arrived first");
            std::fs::remove_file(&raced).unwrap();
            unchanged();

            history_is_whole(&mut editor);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn restored_work_without_a_file_takes_a_new_one_and_saves_there_from_then_on() {
        let dir = scratch();
        let file = dir.join("film.nvp");
        editor_store::save_project(&file, &Project::default()).unwrap();
        let original = std::fs::read(&file).unwrap();
        let saver = SaveWorker::new(|| {});

        for (index, base) in left_unbound(&dir, &file).into_iter().enumerate() {
            let restored = editor_store::load_recovery_with_document(&base).unwrap();
            let document = bound_document(&base, restored.document.as_ref());
            let origin = Origin::restored(document.as_ref());
            let mut editor = restored.editor;

            let session = origin
                .prepare(&dir.join(format!("restored {index}.nvp")))
                .unwrap();
            let revision = editor.register_save();
            saver.request_document(session.clone(), &editor).unwrap();
            let written = answered(&saver);
            assert!(written.result.is_ok());
            assert_eq!(written.path, session.path());
            editor.mark_saved(revision);
            assert!(!editor.is_dirty());
            assert_eq!(
                editor_store::load_project(session.path()).unwrap().name,
                "Mine"
            );

            // The file is the work's own now: it is saved again through the
            // session it was made with, and chosen again in Save As it is the
            // open file, as for any project that has one.
            assert_eq!(Origin::restored(Some(&session)), Origin::Ordinary);
            let again = Origin::Ordinary.prepare(session.path()).unwrap();
            assert!(Arc::ptr_eq(&destination(Some(&session), again), &session));
            renamed(&mut editor, "Mine, later");
            let revision = editor.register_save();
            saver.request_document(session.clone(), &editor).unwrap();
            assert!(answered(&saver).result.is_ok());
            editor.mark_saved(revision);
            assert!(!editor.is_dirty());
            assert_eq!(
                editor_store::load_project(session.path()).unwrap().name,
                "Mine, later"
            );
            // What came before the restore can still be undone.
            assert_eq!(editor.history().count(), 3);
            assert!(editor.undo() && editor.undo() && editor.undo());
            assert_eq!(editor.project().name, Project::default().name);

            assert_eq!(std::fs::read(&file).unwrap(), original);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_deliberate_save_as_still_replaces_the_project_that_was_chosen() {
        let dir = scratch();
        let chosen = dir.join("chosen.nvp");
        editor_store::save_project(&chosen, named("Old").project()).unwrap();
        let saver = SaveWorker::new(|| {});

        // A project made here and one opened from a file are both ordinary,
        // as is restored work that came back with its file.
        assert_eq!(Origin::default(), Origin::Ordinary);
        let mut editor = blank();
        renamed(&mut editor, "New");
        let session = Origin::Ordinary.prepare(&chosen).unwrap();
        assert_eq!(Origin::restored(Some(&session)), Origin::Ordinary);
        let revision = editor.register_save();
        saver.request_document(session.clone(), &editor).unwrap();
        assert!(answered(&saver).result.is_ok());
        editor.mark_saved(revision);
        assert!(!editor.is_dirty());
        assert_eq!(editor_store::load_project(&chosen).unwrap().name, "New");

        // A file that is no project is refused for everyone, and a path
        // that is free is taken.
        let notes = dir.join("notes.nvp");
        std::fs::write(&notes, b"someone's notes").unwrap();
        assert!(Origin::Ordinary.prepare(&notes).is_err());
        assert_eq!(std::fs::read(&notes).unwrap(), b"someone's notes");
        assert!(Origin::Ordinary.prepare(&dir.join("free.nvp")).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_another_writer_brings_up_the_review() {
        let path = PathBuf::from("/p/film.nvp");
        assert_eq!(
            other_writer(&StoreError::DocumentConflict(path.clone())),
            Some(false)
        );
        assert_eq!(other_writer(&StoreError::DocumentBusy(path)), Some(true));
        assert_eq!(other_writer(&StoreError::QueueFull), None);
        assert_eq!(
            other_writer(&StoreError::Io(std::io::Error::other("disk full"))),
            None
        );
    }

    #[test]
    fn a_duplicate_may_not_take_the_file_it_is_made_from() {
        let dir = std::env::temp_dir().join(format!("neptune-video-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let file = dir.join("film.nvp");
        std::fs::write(&file, b"{}").unwrap();
        assert!(same_file(
            &file,
            &dir.join("sub").join("..").join("film.nvp")
        ));
        assert!(!same_file(&file, &dir.join("film copy.nvp")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // ---- Motion tracking -------------------------------------------------

    #[test]
    fn media_only_trackers_still_hold_is_removed_with_them_in_one_step() {
        let (mut editor, tracking, _) = tracking_fixture();
        let asset = tracking.asset;
        let tracker = |name: &str| Tracker {
            id: uuid::Uuid::new_v4(),
            name: name.to_owned(),
            asset_id: asset,
            keys: vec![editor_core::TrackKey {
                source: Time::ZERO,
                center: [0.5, 0.5],
                scale: 1.0,
                rotation: 0.0,
            }],
        };
        // Media nothing holds goes as it always did.
        let (plain, said) = remove_asset_command(editor.project(), asset);
        assert!(matches!(plain, Command::RemoveAssets { .. }) && said.is_none());
        for name in ["Ball", "Face"] {
            let tracker = tracker(name);
            editor.dispatch(Command::SetTracker { tracker }).unwrap();
        }
        // The project refuses the media alone, and there is no clip left to
        // open a Motion section on.
        let alone = editor.dispatch(Command::RemoveAssets {
            asset_ids: vec![asset],
        });
        assert!(alone.is_err());
        let steps = editor.history().count();
        let (command, said) = remove_asset_command(editor.project(), asset);
        let said = said.expect("the trackers are named");
        assert!(said.contains("Ball") && said.contains("Face"), "{said}");
        editor.dispatch(command).unwrap();
        assert_eq!(editor.history().count(), steps + 1);
        assert_eq!(editor.undo_label(), Some("Remove media"));
        assert!(editor.project().assets.is_empty() && editor.project().trackers.is_empty());
        assert!(editor.undo());
        assert_eq!(editor.project().trackers.len(), 2);
        assert!(editor.project().assets.contains_key(&asset));

        // While a clip still shows the media nothing of it is taken: the
        // project refuses, and the trackers stay.
        let shot = editor.project().assets[&asset].clone();
        let track_id = editor.project().tracks[0].id;
        let mut clip = Clip::from_asset(&shot, Time::ZERO);
        clip.duration = Time::from_seconds(2.0);
        editor
            .dispatch(Command::AddClip { track_id, clip })
            .unwrap();
        let (command, said) = remove_asset_command(editor.project(), asset);
        assert!(matches!(command, Command::RemoveAssets { .. }) && said.is_none());
        assert!(editor.dispatch(command).is_err());
        assert_eq!(editor.project().trackers.len(), 2);
    }

    #[test]
    fn a_path_waits_while_adding_it_would_take_the_redo_or_a_plan() {
        assert_eq!(tracker_waits(false, false), None);
        assert!(tracker_waits(true, false).unwrap().contains("undone"));
        assert!(tracker_waits(false, true).unwrap().contains("plan"));
        assert!(tracker_waits(true, true).unwrap().contains("undone"));
    }

    #[test]
    fn captions_wait_when_the_timeline_changed_or_adding_them_would_cost_something() {
        assert_eq!(heard_waits(false, false, false), None);
        assert!(heard_waits(true, false, false).unwrap().contains("changed"));
        assert!(heard_waits(false, true, false).unwrap().contains("undone"));
        assert!(heard_waits(false, false, true).unwrap().contains("plan"));
        assert!(heard_waits(true, true, true).unwrap().contains("changed"));
        assert!(speech_refused(editor_media::MediaError::Busy).contains("still stopping"));
        assert!(
            speech_refused(editor_media::MediaError::Closed).starts_with("Captions cannot be made")
        );
    }

    #[test]
    fn an_arrow_a_control_owns_is_not_the_applications_with_any_modifier() {
        // The reach for a clip (Alt and an arrow) asks this of its arrow as
        // every other arrow shortcut does.
        for key in [
            Key::ArrowLeft,
            Key::ArrowRight,
            Key::ArrowUp,
            Key::ArrowDown,
        ] {
            assert!(key_is_free(false, key));
            assert!(!key_is_free(true, key));
        }
        assert!(!key_is_free(true, Key::Delete) && !key_is_free(true, Key::Backspace));
        assert!(key_is_free(true, Key::C) && key_is_free(true, Key::Space));
        // A tab or a segment with the keyboard takes Left and Right only:
        // the selected clips are still deleted from it.
        assert!(choice_takes(true, Key::ArrowLeft) && choice_takes(true, Key::ArrowRight));
        for key in [Key::ArrowUp, Key::ArrowDown, Key::Delete, Key::Backspace] {
            assert!(!choice_takes(true, key), "{key:?}");
        }
        assert!(!choice_takes(false, Key::ArrowLeft));
    }

    /// An editor with one ten second video, what tracking a region of it
    /// waits for, and a path of three frames.
    fn tracking_fixture() -> (Editor, Tracking, editor_media::Tracked) {
        let mut asset = Asset::new(
            "/media/shot.mp4".into(),
            MediaKind::Video,
            Time::from_seconds(10.0),
        );
        (asset.width, asset.height, asset.file_size) = (1920, 1080, 4000);
        asset.frame_rate = Some(editor_core::FrameRate::default());
        let mut editor = blank();
        let tracking = Tracking {
            request: 3,
            clip: uuid::Uuid::new_v4(),
            asset: asset.id,
            tracker: uuid::Uuid::new_v4(),
            name: "Tracker 1".to_owned(),
            done: 0.5,
            ready: None,
        };
        editor
            .dispatch(Command::AddAssets {
                assets: vec![asset],
            })
            .unwrap();
        let key = |frame: i64, x: f32| editor_core::TrackKey {
            source: Time(frame * 33_333),
            center: [x, 0.5],
            scale: 1.0,
            rotation: 0.0,
        };
        let tracked = editor_media::Tracked {
            keys: vec![key(0, 0.2), key(1, 0.25), key(2, 0.3)],
            frames: 3,
            lost_at: None,
            confidence: 0.97,
        };
        (editor, tracking, tracked)
    }

    #[test]
    fn a_tracked_path_enters_the_project_as_one_reversible_command() {
        let (mut editor, tracking, tracked) = tracking_fixture();
        let asset = editor.project().assets[&tracking.asset].clone();
        let source = SourceIdentity::of(&asset);
        let steps = editor.history().count();
        let (command, said) = tracker_command(
            editor.instance_id(),
            Some(&asset),
            &tracking,
            editor.instance_id(),
            tracking.tracker,
            &source,
            Ok(tracked.clone()),
        )
        .unwrap();
        assert_eq!(said, "Tracker 1 follows the region for 0.07 s.");
        editor.dispatch(command).unwrap();
        assert_eq!(editor.history().count(), steps + 1);
        assert_eq!(editor.undo_label(), Some("Set tracker"));
        let made = &editor.project().trackers[&tracking.tracker];
        assert_eq!((made.name.as_str(), made.asset_id), ("Tracker 1", asset.id));
        assert_eq!(made.keys, tracked.keys);
        assert!(editor.undo());
        assert!(editor.project().trackers.is_empty());

        // A path that ended early says so, and is still a tracker.
        let lost = editor_media::Tracked {
            lost_at: Some(Time(99_999)),
            ..tracked
        };
        let (_, said) = tracker_command(
            editor.instance_id(),
            Some(&asset),
            &tracking,
            editor.instance_id(),
            tracking.tracker,
            &source,
            Ok(lost),
        )
        .unwrap();
        assert_eq!(
            said,
            "Tracker 1 follows the region for 0.07 s, then lost it."
        );
    }

    #[test]
    fn a_stale_or_failed_tracking_answer_changes_nothing() {
        let (editor, tracking, tracked) = tracking_fixture();
        let asset = editor.project().assets[&tracking.asset].clone();
        let source = SourceIdentity::of(&asset);
        let here = editor.instance_id();
        let answer = |instance, now: Option<&Asset>, project, target, result| {
            tracker_command(instance, now, &tracking, project, target, &source, result)
                .map(|(command, _)| command.label().to_owned())
        };
        // An answer for the project this window had before, or for another
        // tracker than the one waited for, is dropped without a word.
        let other = blank().instance_id();
        assert_ne!(other, here);
        assert_eq!(
            answer(
                other,
                Some(&asset),
                here,
                tracking.tracker,
                Ok(tracked.clone())
            ),
            Err(None)
        );
        assert_eq!(
            answer(
                here,
                Some(&asset),
                here,
                uuid::Uuid::new_v4(),
                Ok(tracked.clone())
            ),
            Err(None)
        );
        // Media that left the library, or is now of another file, has no
        // path: the answer was read from what it used to be.
        let gone = answer(here, None, here, tracking.tracker, Ok(tracked.clone()));
        assert!(
            matches!(&gone, Err(Some(why)) if why.contains("media changed")),
            "{gone:?}"
        );
        let mut replaced = asset.clone();
        replaced.file_size += 1;
        let changed = answer(
            here,
            Some(&replaced),
            here,
            tracking.tracker,
            Ok(tracked.clone()),
        );
        assert!(matches!(&changed, Err(Some(why)) if why.contains("media changed")));
        // A failure, and a region lost at once, are said and make nothing.
        let failed = answer(
            here,
            Some(&asset),
            here,
            tracking.tracker,
            Err("ffmpeg: gone".into()),
        );
        assert_eq!(
            failed,
            Err(Some("Tracking failed: ffmpeg: gone".to_owned()))
        );
        let lost = editor_media::Tracked {
            keys: tracked.keys[..1].to_vec(),
            lost_at: Some(Time(33_333)),
            ..tracked
        };
        let lost = answer(here, Some(&asset), here, tracking.tracker, Ok(lost));
        assert!(matches!(&lost, Err(Some(why)) if why.contains("lost in the next frame")));
    }
}
