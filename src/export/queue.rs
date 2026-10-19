//! The render queue: exports waiting, rendering and finished, run one at a
//! time on an `neptune-video-render` thread of their own.
//!
//! A job is a snapshot: the project as it was when it was queued, its
//! settings and its destination. Later edits never reach it. The queue keeps
//! every job listed, finished ones too, until it is removed; at most
//! [`MAX_JOBS`] are listed.
//!
//! The interface and the agent read the queue through cheap snapshots
//! ([`RenderQueue::jobs`], [`RenderQueue::summary`]) and drain finished jobs
//! with [`RenderQueue::poll`]. The queue's lock is held only to read or change
//! that state, never while a job works (rule 13): the job reports through a
//! [`Handle`] that takes the lock for a moment. The window is woken at once
//! when a job starts or ends and at most every 100 ms while one renders.
//!
//! Pause takes effect between frames: the render thread waits, the frames
//! already handed to FFmpeg are written, and FFmpeg then waits on its input
//! pipe. A paused job holds the queue: the next job does not start while one
//! is paused half way, so at most one encode's processes and buffers exist.
//! A job paused before it started is skipped until it is resumed, and the
//! jobs after it render.
use super::engine::{self, Engine, Finish, Ticket};
use editor_core::{
    Project,
    export::{Capabilities, ExportError, ExportSettings, RenderJobInfo, RenderState},
};
use editor_media::export::ProbeGate;
use eframe::wgpu;
use std::{
    collections::{HashSet, VecDeque},
    fmt,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};
use uuid::Uuid;

/// The most jobs listed at once, finished ones included.
pub const MAX_JOBS: usize = 64;
/// Finished events kept for the application to take; the oldest go first.
const MAX_EVENTS: usize = 32;
/// The least time between two progress wakes.
const WAKE_EVERY: Duration = Duration::from_millis(100);
/// How long `shutdown` waits for the job it stopped to tidy up.
const SHUTDOWN_GRACE: Duration = Duration::from_millis(750);
/// How long the window has not played before the encoder probe's test
/// encodes run: from launch, and again from each stop.
const PROBE_SETTLE: Duration = Duration::from_secs(5);

/// What this host's FFmpeg can make, as far as it is known.
#[derive(Debug, Clone, PartialEq)]
pub enum CapabilityState {
    /// The probe is running; nothing can be queued yet.
    Probing,
    Ready(Arc<Capabilities>),
    /// FFmpeg is missing or could not be checked; why, in a sentence.
    Failed(String),
}

/// An export to queue: a snapshot of the project and what to make of it.
#[derive(Debug, Clone)]
pub struct NewJob {
    /// The project as it is now, drafts committed.
    pub project: Arc<Project>,
    /// The editing context (`Editor::instance_id`) it comes from.
    pub context: Uuid,
    /// The project revision it was taken at.
    pub revision: u64,
    pub settings: ExportSettings,
    /// Where the file goes: absolute, with the container's extension. An
    /// existing file is replaced when the export succeeds; whether that is
    /// wanted is asked before queueing (the save dialog, or the agent's
    /// `overwrite`), not here.
    pub destination: PathBuf,
    /// Queue it paused: it waits until resumed.
    pub start_paused: bool,
    /// The project's file, when it has one: neither it nor its recovery,
    /// lock and checkpoint files may be the destination.
    pub project_path: Option<PathBuf>,
}

/// Why a job was not queued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnqueueError {
    /// FFmpeg's encoders are still being checked.
    CapabilitiesPending,
    /// FFmpeg is missing or could not be checked.
    CapabilitiesFailed(String),
    /// The settings cannot make a file of this project here.
    Invalid(ExportError),
    /// [`MAX_JOBS`] jobs are listed already.
    QueueFull,
    /// The destination is a file the project reads.
    DestinationIsSource,
    /// The destination is the project's file or one kept beside it.
    DestinationIsProject,
    /// The destination's extension is not the container's.
    WrongExtension { expected: &'static str },
    /// The destination is not an absolute path to a file.
    DestinationNotAbsolute,
    /// The destination is a folder.
    DestinationIsFolder,
    /// The destination's folder does not exist.
    FolderMissing,
    /// A job listed and not finished writes this file already: the export
    /// or its caption file.
    DestinationQueued(String),
    /// The queue has been shut down or could not start.
    Stopped(String),
}

impl EnqueueError {
    /// The agent protocol's error code for this refusal (rule 17).
    pub fn code(&self) -> &'static str {
        match self {
            Self::CapabilitiesPending => "capabilities_pending",
            Self::CapabilitiesFailed(_) | Self::Stopped(_) => "unsupported_export",
            Self::Invalid(ExportError::Unsupported(_)) => "unsupported_export",
            Self::Invalid(ExportError::Empty(_)) => "empty_export",
            Self::QueueFull => "queue_full",
            Self::Invalid(ExportError::Invalid(_))
            | Self::DestinationIsSource
            | Self::DestinationIsProject
            | Self::WrongExtension { .. }
            | Self::DestinationNotAbsolute
            | Self::DestinationIsFolder
            | Self::FolderMissing
            | Self::DestinationQueued(_) => "invalid_export",
        }
    }
}

impl fmt::Display for EnqueueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapabilitiesPending => {
                f.write_str("FFmpeg's encoders are still being checked; try again in a moment")
            }
            Self::CapabilitiesFailed(why) => write!(f, "Exports are unavailable: {why}"),
            Self::Invalid(error) => write!(f, "{error}"),
            Self::QueueFull => write!(
                f,
                "The render queue holds {MAX_JOBS} jobs; remove finished ones first"
            ),
            Self::DestinationIsSource => {
                f.write_str("The export would replace media the project uses; choose another name")
            }
            Self::DestinationIsProject => {
                f.write_str("The export would replace the project's own files; choose another name")
            }
            Self::WrongExtension { expected } => {
                write!(f, "The file name must end in .{expected}")
            }
            Self::DestinationNotAbsolute => {
                f.write_str("The destination must be a full path to a file")
            }
            Self::DestinationIsFolder => f.write_str("The destination is a folder, not a file"),
            Self::FolderMissing => f.write_str("The destination folder does not exist"),
            Self::DestinationQueued(name) => write!(
                f,
                "Another export in the queue already writes {name}; choose another name"
            ),
            Self::Stopped(why) => write!(f, "The render queue is not running: {why}"),
        }
    }
}

impl std::error::Error for EnqueueError {}

/// The queue at a glance, for the toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct QueueSummary {
    /// Jobs started and not finished, paused ones included: 0 or 1.
    pub active: usize,
    /// Jobs waiting their turn that will take it: not held.
    pub queued: usize,
    /// Jobs waiting that are held, paused before they started (Add to
    /// queue queues them so): they start only once resumed.
    pub held: usize,
    /// The started job's progress, 0..=1.
    pub progress: Option<f32>,
    /// The started job.
    pub rendering: Option<Uuid>,
    /// Whether the started job is paused.
    pub paused: bool,
}

/// What the application hears of the queue.
#[derive(Debug, Clone, PartialEq)]
pub enum RenderEvent {
    /// A job finished: its file, or why it failed. A job the person or the
    /// application cancelled says nothing.
    Finished {
        id: Uuid,
        name: String,
        result: Result<PathBuf, String>,
        output_bytes: Option<u64>,
        /// Something to know about a finished file, such as a fallback to
        /// software encoding.
        note: Option<String>,
    },
}

/// Answers a capability probe, its test encodes giving way to the gate;
/// FFmpeg's in production.
pub(crate) type Probe = Arc<dyn Fn(&dyn ProbeGate) -> Result<Capabilities, String> + Send + Sync>;
/// Runs one job to its end on the render thread.
pub(crate) type Runner = Box<dyn FnMut(&Ticket, &Handle) -> Finish + Send>;

/// The export queue service. Dropping it shuts it down.
pub struct RenderQueue {
    shared: Arc<Shared>,
    worker: Mutex<Option<thread::JoinHandle<()>>>,
}

impl RenderQueue {
    /// Starts the queue: FFmpeg's encoders are probed at once on a thread of
    /// their own, and jobs run on the `neptune-video-render` thread. `gpu` is the
    /// device to draw on, the window's when there is one; without one a
    /// device of the export's own is opened when the first picture is
    /// drawn. `steady` is the folder stabilization paths are kept in.
    ///
    /// Before the first job, the render thread removes the working folders
    /// that exports stopped by a crash left in the system's temporary
    /// folder; each job, as it starts, does the same in its own folder.
    pub fn new(
        wake: impl Fn() + Send + Sync + 'static,
        gpu: Option<(wgpu::Device, wgpu::Queue)>,
        steady: Option<PathBuf>,
    ) -> Self {
        let probe: Probe = Arc::new(|gate| {
            editor_media::export::probe_capabilities_gated(gate).map_err(|error| error.to_string())
        });
        let mut engine = Engine::new(gpu, steady);
        Self::build(
            wake,
            probe,
            Box::new(move |ticket, handle| engine.run(ticket, handle)),
            Some(Box::new(|| {
                engine::sweep_temp(&std::env::temp_dir(), SystemTime::now());
            })),
        )
    }

    /// A queue whose capabilities come from `probe` and whose jobs `run`
    /// does: the seam the tests use.
    #[cfg(test)]
    pub(crate) fn with_parts(
        wake: impl Fn() + Send + Sync + 'static,
        probe: Probe,
        run: Runner,
    ) -> Self {
        Self::build(wake, probe, run, None)
    }

    /// The queue, its probe started and its render thread running `first`
    /// before it takes any job.
    fn build(
        wake: impl Fn() + Send + Sync + 'static,
        probe: Probe,
        run: Runner,
        first: Option<Box<dyn FnOnce() + Send>>,
    ) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                jobs: Vec::new(),
                events: VecDeque::new(),
                caps: CapabilityState::Probing,
                probe_generation: 0,
                probe_held: false,
                probe_still: Instant::now(),
                probe_settle: PROBE_SETTLE,
                probe_wanted: false,
                shutdown: false,
                broken: None,
            }),
            work: Condvar::new(),
            wake: Box::new(wake),
            woken: Mutex::new(None),
            probe,
        });
        start_probe(&shared);
        let worker = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("neptune-video-render".into())
            .spawn(move || {
                if let Some(first) = first {
                    // Tidying is never worth a failed queue.
                    let _ = catch_unwind(AssertUnwindSafe(first));
                }
                work(&worker, run);
            });
        let worker = match worker {
            Ok(worker) => Some(worker),
            Err(error) => {
                shared.lock().broken = Some(format!("its thread could not start ({error})"));
                None
            }
        };
        Self {
            shared,
            worker: Mutex::new(worker),
        }
    }

    /// What this host can export, as far as it is known. Asking lets a held
    /// probe go on: see [`Self::hold_probe`].
    pub fn capabilities(&self) -> CapabilityState {
        self.shared.want_probe().caps.clone()
    }

    /// Checks FFmpeg again, for instance after it was installed. Queued jobs
    /// wait for the answer; the one rendering goes on.
    pub fn reprobe(&self) {
        drop(self.shared.want_probe());
        start_probe(&self.shared);
    }

    /// Whether the window plays or records. The encoder probe's test
    /// encodes run once it has not for [`PROBE_SETTLE`], from launch and
    /// from each stop; while it does, the ones running are stopped, to run
    /// again later. A hardware encoder's test opens its GPU, which on a
    /// laptop with two held up the preview for most of a second just after
    /// launch. Asking for the capabilities, or queueing an export, lets the
    /// probe run at once.
    pub fn hold_probe(&self, held: bool) {
        let mut state = self.shared.lock();
        if state.probe_held != held {
            state.probe_held = held;
            if !held {
                state.probe_still = Instant::now();
            }
            drop(state);
            self.shared.work.notify_all();
        }
    }

    /// Queues an export after checking it: FFmpeg is known, the settings make
    /// a file of the project here, the destination is an absolute path with
    /// the container's extension that names none of the project's files,
    /// and no job listed and not finished writes it or its caption file.
    ///
    /// Only names are compared here, without asking the file system, so the
    /// window never waits on a slow disk. What the names resolve to (links,
    /// the folder, the file itself) is checked on the render thread as the
    /// job starts; a job that fails there fails with the same reason.
    pub fn enqueue(&self, job: NewJob) -> Result<Uuid, EnqueueError> {
        let caps = {
            let state = self.shared.want_probe();
            if let Some(why) = &state.broken {
                return Err(EnqueueError::Stopped(why.clone()));
            }
            if state.shutdown {
                return Err(EnqueueError::Stopped("the application is closing".into()));
            }
            if state.jobs.len() >= MAX_JOBS {
                return Err(EnqueueError::QueueFull);
            }
            match &state.caps {
                CapabilityState::Probing => return Err(EnqueueError::CapabilitiesPending),
                CapabilityState::Failed(why) => {
                    return Err(EnqueueError::CapabilitiesFailed(why.clone()));
                }
                CapabilityState::Ready(caps) => Arc::clone(caps),
            }
        };
        // Names are compared and the plan made without the lock.
        let sidecar = engine::sidecar_for(&job.project, &job.settings, &job.destination);
        check_names(&job, sidecar.as_deref())?;
        let plan = job
            .settings
            .plan(&job.project, &caps)
            .map_err(EnqueueError::Invalid)?;
        let id = Uuid::new_v4();
        let name = job
            .destination
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let encoder = engine::encoder_name(&plan);
        let frames_total = plan.video.as_ref().map_or(0, |video| video.frames.count);
        let info = RenderJobInfo {
            id,
            name,
            destination: job.destination,
            settings: job.settings,
            project_id: job.project.id,
            revision: job.revision,
            state: if job.start_paused {
                RenderState::Paused
            } else {
                RenderState::Queued
            },
            progress: 0.0,
            frames_done: 0,
            frames_total,
            speed_fps: 0.0,
            elapsed_seconds: 0.0,
            eta_seconds: None,
            encoder,
            output_bytes: None,
            context_id: job.context,
            message: None,
            attempts: 0,
        };
        let mut state = self.shared.lock();
        if state.shutdown {
            return Err(EnqueueError::Stopped("the application is closing".into()));
        }
        if state.jobs.len() >= MAX_JOBS {
            return Err(EnqueueError::QueueFull);
        }
        let outputs = Outputs {
            destination: info.destination.clone(),
            sidecar,
        };
        if let Some(name) = state.taken(&outputs, None) {
            return Err(EnqueueError::DestinationQueued(name));
        }
        state.jobs.push(Entry {
            info,
            project: job.project,
            phase: RenderState::Queued,
            paused: job.start_paused,
            control: None,
            project_path: job.project_path,
            outputs,
        });
        drop(state);
        self.shared.work.notify_all();
        Ok(id)
    }

    /// Pauses a job: the one rendering stops between frames and holds the
    /// queue; a waiting one is skipped until resumed. `false` when the job
    /// is unknown, finished, finalizing or paused already.
    pub fn pause(&self, id: Uuid) -> bool {
        let mut state = self.shared.lock();
        let Some(entry) = state.entry_mut(id) else {
            return false;
        };
        if entry.paused
            || !matches!(
                entry.phase,
                RenderState::Queued | RenderState::Preparing | RenderState::Rendering
            )
        {
            return false;
        }
        entry.paused = true;
        if let Some(control) = &entry.control {
            control.set_paused(true);
        }
        entry.show();
        true
    }

    /// Resumes a paused job where it stopped, or lets a waiting one take its
    /// turn again. `false` when it is not paused.
    pub fn resume(&self, id: Uuid) -> bool {
        let mut state = self.shared.lock();
        let Some(entry) = state.entry_mut(id) else {
            return false;
        };
        if !entry.paused {
            return false;
        }
        entry.paused = false;
        if let Some(control) = &entry.control {
            control.set_paused(false);
        }
        entry.show();
        drop(state);
        self.shared.work.notify_all();
        true
    }

    /// Resumes every held job, those paused before they started, in the
    /// order listed; the one started and paused is left as it is. Returns
    /// how many were resumed.
    pub fn resume_all(&self) -> usize {
        let mut state = self.shared.lock();
        let mut resumed = 0;
        for entry in state.jobs.iter_mut().filter(|entry| entry.is_held()) {
            entry.paused = false;
            entry.show();
            resumed += 1;
        }
        drop(state);
        if resumed > 0 {
            self.shared.work.notify_all();
        }
        resumed
    }

    /// The jobs waiting their turn, held ones included, in the order they
    /// render: those [`move_by`](Self::move_by) moves among.
    pub fn waiting(&self) -> Vec<Uuid> {
        (self.shared.lock().jobs.iter())
            .filter(|entry| entry.is_waiting())
            .map(|entry| entry.info.id)
            .collect()
    }

    /// Cancels a job: a waiting one at once; the one rendering has its
    /// processes killed and its part and working files removed, then reads
    /// Cancelled. `false` when it is unknown or finished.
    pub fn cancel(&self, id: Uuid) -> bool {
        let control = {
            let mut state = self.shared.lock();
            let Some(entry) = state.entry_mut(id) else {
                return false;
            };
            if entry.phase.is_finished() {
                return false;
            }
            match &entry.control {
                Some(control) => Arc::clone(control),
                None => {
                    entry.phase = RenderState::Cancelled;
                    entry.paused = false;
                    entry.show();
                    return true;
                }
            }
        };
        // Killing happens outside the lock.
        control.cancel();
        true
    }

    /// Queues a failed or cancelled job again, from the same snapshot and
    /// settings. `false` for any other job, and while another job listed and
    /// not finished writes its file.
    pub fn retry(&self, id: Uuid) -> bool {
        let mut state = self.shared.lock();
        if state.shutdown {
            return false;
        }
        let Some(entry) = state.entry(id) else {
            return false;
        };
        if !matches!(entry.phase, RenderState::Failed | RenderState::Cancelled)
            || state.taken(&entry.outputs, Some(id)).is_some()
        {
            return false;
        }
        let Some(entry) = state.entry_mut(id) else {
            return false;
        };
        entry.phase = RenderState::Queued;
        entry.paused = false;
        let info = &mut entry.info;
        info.progress = 0.0;
        info.frames_done = 0;
        info.speed_fps = 0.0;
        info.elapsed_seconds = 0.0;
        info.eta_seconds = None;
        info.output_bytes = None;
        info.message = None;
        entry.show();
        drop(state);
        self.shared.work.notify_all();
        true
    }

    /// Takes a finished or waiting job off the list; a finished file stays
    /// on disk. `false` for a started job: cancel it first.
    pub fn remove(&self, id: Uuid) -> bool {
        let mut state = self.shared.lock();
        let Some(index) = state.index(id) else {
            return false;
        };
        let entry = &state.jobs[index];
        if entry.control.is_some() || !(entry.phase.is_finished() || entry.is_waiting()) {
            return false;
        }
        state.jobs.remove(index);
        true
    }

    /// Moves a waiting job `by` places among the waiting jobs, held ones
    /// included (see [`waiting`](Self::waiting)): negative is sooner. `false`
    /// when it is not waiting or cannot move that far.
    pub fn move_by(&self, id: Uuid, by: i32) -> bool {
        let mut state = self.shared.lock();
        let waiting: Vec<usize> = (state.jobs.iter().enumerate())
            .filter(|(_, entry)| entry.is_waiting())
            .map(|(index, _)| index)
            .collect();
        let Some(at) = waiting
            .iter()
            .position(|&index| state.jobs[index].info.id == id)
        else {
            return false;
        };
        let to = at as i64 + i64::from(by);
        if by == 0 || to < 0 || to >= waiting.len() as i64 {
            return false;
        }
        let from = waiting[at];
        let target = waiting[to as usize];
        let entry = state.jobs.remove(from);
        // Removing shifts later indices down by one.
        state.jobs.insert(target, entry);
        drop(state);
        self.shared.work.notify_all();
        true
    }

    /// Takes every finished job off the list.
    pub fn clear_finished(&self) {
        self.shared
            .lock()
            .jobs
            .retain(|entry| !entry.phase.is_finished());
    }

    /// Every listed job as it stands, in render order.
    pub fn jobs(&self) -> Vec<RenderJobInfo> {
        (self.shared.lock().jobs.iter())
            .map(|entry| entry.info.clone())
            .collect()
    }

    /// One job as it stands.
    pub fn job(&self, id: Uuid) -> Option<RenderJobInfo> {
        let state = self.shared.lock();
        state.entry(id).map(|entry| entry.info.clone())
    }

    pub fn summary(&self) -> QueueSummary {
        let state = self.shared.lock();
        let mut summary = QueueSummary::default();
        for entry in &state.jobs {
            if entry.control.is_some() {
                summary.active += 1;
                summary.progress = Some(entry.info.progress);
                summary.rendering = Some(entry.info.id);
                summary.paused = entry.paused;
            } else if entry.is_held() {
                summary.held += 1;
            } else if entry.is_waiting() {
                summary.queued += 1;
            }
        }
        summary
    }

    /// Whether a job is rendering or waiting, paused ones included: what
    /// quitting asks about.
    pub fn busy(&self) -> bool {
        let state = self.shared.lock();
        (state.jobs.iter()).any(|entry| entry.control.is_some() || entry.is_waiting())
    }

    /// The next finished job to tell of, oldest first.
    pub fn poll(&self) -> Option<RenderEvent> {
        self.shared.lock().events.pop_front()
    }

    /// Cancels every job, kills their processes and removes their part and
    /// working files. Joins no thread: it waits at most a moment for the job
    /// it stopped to say it has stopped. Nothing can be queued afterwards.
    pub fn shutdown(&self) {
        let (control, active) = {
            let mut state = self.shared.lock();
            if state.shutdown {
                // Already done; dropping the queue after `shutdown` waits
                // no longer.
                return;
            }
            state.shutdown = true;
            let mut active = None;
            for entry in &mut state.jobs {
                if entry.phase.is_finished() {
                    continue;
                }
                match &entry.control {
                    Some(control) => {
                        active = Some((
                            Arc::clone(control),
                            entry.info.id,
                            entry.info.destination.clone(),
                        ));
                    }
                    None => {
                        entry.phase = RenderState::Cancelled;
                        entry.paused = false;
                        entry.show();
                    }
                }
            }
            match active {
                Some((control, id, destination)) => (Some(control), Some((id, destination))),
                None => (None, None),
            }
        };
        self.shared.work.notify_all();
        if let Some(control) = control {
            control.cancel();
        }
        if let Some((id, destination)) = active {
            // The render thread tidies up as it stops; this is in case the
            // process ends first.
            let until = Instant::now() + SHUTDOWN_GRACE;
            let mut state = self.shared.lock();
            while state.entry(id).is_some_and(|entry| entry.control.is_some()) {
                let now = Instant::now();
                if now >= until {
                    break;
                }
                state = self
                    .shared
                    .work
                    .wait_timeout(state, until - now)
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .0;
            }
            drop(state);
            engine::clean(id, &destination);
        }
    }

    /// Finish destroying the export's GPU before a headless process enters
    /// native-library exit handlers. Interactive owners keep using the
    /// bounded `shutdown`; this join belongs off their frame thread.
    pub fn shutdown_and_join(&self) {
        self.shutdown();
        if let Some(worker) = self
            .worker
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            let _ = worker.join();
        }
    }
}

impl Drop for RenderQueue {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// State shared by the queue's owner, its render thread and its probe.
pub(crate) struct Shared {
    state: Mutex<State>,
    /// Signalled when a job may be runnable and when one ends.
    work: Condvar,
    wake: Box<dyn Fn() + Send + Sync>,
    woken: Mutex<Option<Instant>>,
    probe: Probe,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The state, the probe no longer held for playback.
    fn want_probe(&self) -> MutexGuard<'_, State> {
        let mut state = self.lock();
        if !state.probe_wanted {
            state.probe_wanted = true;
            self.work.notify_all();
        }
        state
    }

    /// Wakes the window: at once for a change of state, at most every
    /// [`WAKE_EVERY`] for progress.
    fn signal(&self, now: bool) {
        let at = Instant::now();
        {
            let mut woken = self
                .woken
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !now && woken.is_some_and(|last| at.duration_since(last) < WAKE_EVERY) {
                return;
            }
            *woken = Some(at);
        }
        (self.wake)();
    }
}

struct State {
    jobs: Vec<Entry>,
    events: VecDeque<RenderEvent>,
    caps: CapabilityState,
    probe_generation: u64,
    /// The window is playing: see [`RenderQueue::hold_probe`].
    probe_held: bool,
    /// When the window was launched or last stopped playing.
    probe_still: Instant,
    /// [`PROBE_SETTLE`], shorter in tests.
    probe_settle: Duration,
    /// The capabilities were asked for or an export queued: the probe is
    /// no longer held.
    probe_wanted: bool,
    shutdown: bool,
    /// Why the render thread is not running, if it could not start.
    broken: Option<String>,
}

impl State {
    /// Whether the probe's running test encodes give way now.
    fn probe_yields(&self) -> bool {
        self.probe_held && !self.probe_wanted && !self.shutdown
    }

    /// How long the probe's next test encode waits: `None` when it may
    /// start, `Some(None)` until playback stops.
    fn probe_wait(&self) -> Option<Option<Duration>> {
        if self.probe_wanted || self.shutdown {
            return None;
        }
        if self.probe_held {
            return Some(None);
        }
        let left = (self.probe_still + self.probe_settle).saturating_duration_since(Instant::now());
        (!left.is_zero()).then_some(Some(left))
    }

    fn index(&self, id: Uuid) -> Option<usize> {
        self.jobs.iter().position(|entry| entry.info.id == id)
    }

    fn entry(&self, id: Uuid) -> Option<&Entry> {
        self.jobs.iter().find(|entry| entry.info.id == id)
    }

    fn entry_mut(&mut self, id: Uuid) -> Option<&mut Entry> {
        self.jobs.iter_mut().find(|entry| entry.info.id == id)
    }

    /// The name of a file `outputs` writes that a job listed and not
    /// finished, other than `except`, writes too.
    fn taken(&self, outputs: &Outputs, except: Option<Uuid>) -> Option<String> {
        let ours = outputs.paths().map(normalized).collect::<Vec<_>>();
        (self.jobs.iter())
            .filter(|entry| Some(entry.info.id) != except && !entry.phase.is_finished())
            .flat_map(|entry| entry.outputs.paths())
            .find_map(|theirs| {
                let theirs = normalized(theirs);
                ours.iter().find(|ours| same(ours, &theirs)).map(|ours| {
                    ours.file_name().map_or_else(
                        || ours.display().to_string(),
                        |name| name.to_string_lossy().into_owned(),
                    )
                })
            })
    }

    fn push_event(&mut self, event: RenderEvent) {
        while self.events.len() >= MAX_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }
}

struct Entry {
    /// What is shown; `info.state` is kept from `phase` and `paused`.
    info: RenderJobInfo,
    project: Arc<Project>,
    /// Where the job is: waiting, a stage of its run, or how it ended.
    phase: RenderState,
    paused: bool,
    /// The started job's controls; `None` unless it is the one running.
    control: Option<Arc<Control>>,
    /// The project's file, whose own files the export may not replace.
    project_path: Option<PathBuf>,
    outputs: Outputs,
}

/// The files a job writes.
#[derive(Debug, Clone)]
struct Outputs {
    destination: PathBuf,
    /// The caption file beside it, when one is asked for.
    sidecar: Option<PathBuf>,
}

impl Outputs {
    fn paths(&self) -> impl Iterator<Item = &Path> {
        std::iter::once(self.destination.as_path()).chain(self.sidecar.as_deref())
    }
}

impl Entry {
    fn show(&mut self) {
        self.info.state = if self.paused && !self.phase.is_finished() {
            RenderState::Paused
        } else {
            self.phase
        };
    }

    /// Waiting its turn, paused or not.
    fn is_waiting(&self) -> bool {
        self.phase == RenderState::Queued && self.control.is_none()
    }

    /// Waiting and paused: it starts only once resumed.
    fn is_held(&self) -> bool {
        self.is_waiting() && self.paused
    }

    fn runnable(&self) -> bool {
        self.is_waiting() && !self.paused
    }
}

/// The started job's controls, shared by the queue and its render thread.
pub(crate) struct Control {
    /// Set once on cancel; the frame renderer's stop flag too.
    stop: Arc<AtomicBool>,
    paused: Mutex<bool>,
    resumed: Condvar,
    /// What kills the job's processes, called once on cancel.
    cancellers: Mutex<Vec<Box<dyn Fn() + Send + Sync>>>,
}

impl Control {
    fn new(paused: bool) -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            paused: Mutex::new(paused),
            resumed: Condvar::new(),
            cancellers: Mutex::new(Vec::new()),
        }
    }

    fn cancel(&self) {
        self.stop.store(true, Ordering::Release);
        {
            let cancellers = self
                .cancellers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for cancel in cancellers.iter() {
                cancel();
            }
        }
        let _paused = self
            .paused
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.resumed.notify_all();
    }

    fn cancelled(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    fn set_paused(&self, on: bool) {
        let mut paused = self
            .paused
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *paused = on;
        self.resumed.notify_all();
    }

    /// Waits while paused. `None` when cancelled, otherwise how long it
    /// waited: exactly zero when it was not paused, so the render thread can
    /// tell a pause from the time taken to look.
    fn hold(&self) -> Option<Duration> {
        let mut paused = self
            .paused
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !*paused {
            return (!self.cancelled()).then_some(Duration::ZERO);
        }
        let from = Instant::now();
        while *paused && !self.cancelled() {
            paused = self
                .resumed
                .wait(paused)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        (!self.cancelled()).then(|| from.elapsed())
    }

    fn on_cancel(&self, cancel: Box<dyn Fn() + Send + Sync>) {
        let mut cancellers = self
            .cancellers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.cancelled() {
            drop(cancellers);
            cancel();
        } else {
            cancellers.push(cancel);
        }
    }
}

/// How a running job reports and is controlled, from the render thread.
pub(crate) struct Handle {
    shared: Arc<Shared>,
    id: Uuid,
    control: Arc<Control>,
    started: Instant,
    /// Time spent paused, which elapsed time leaves out.
    paused_for: std::cell::Cell<Duration>,
}

/// A running job's progress.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Report {
    pub progress: f32,
    pub frames_done: u64,
    pub frames_total: u64,
    pub speed_fps: f32,
    pub eta_seconds: Option<f64>,
}

impl Handle {
    pub fn cancelled(&self) -> bool {
        self.control.cancelled()
    }

    /// The flag cancel sets, for what checks it while it waits.
    pub fn stop_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.control.stop)
    }

    /// Runs `cancel` when the job is cancelled, at once if it is already.
    pub fn on_cancel(&self, cancel: impl Fn() + Send + Sync + 'static) {
        self.control.on_cancel(Box::new(cancel));
    }

    /// Waits while the job is paused. `None` when it is cancelled,
    /// otherwise how long it waited.
    pub fn hold(&self) -> Option<Duration> {
        let waited = self.control.hold()?;
        if !waited.is_zero() {
            self.paused_for.set(self.paused_for.get() + waited);
        }
        Some(waited)
    }

    /// Working time since the job started, pauses left out.
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed().saturating_sub(self.paused_for.get())
    }

    /// Moves the job to a stage of its run: preparing, rendering or
    /// finalizing.
    pub fn phase(&self, phase: RenderState) {
        {
            let mut state = self.shared.lock();
            let Some(entry) = state.entry_mut(self.id) else {
                return;
            };
            if entry.phase == phase {
                return;
            }
            entry.phase = phase;
            if phase == RenderState::Finalizing {
                // The end of a file cannot be paused.
                entry.paused = false;
                if let Some(control) = &entry.control {
                    control.set_paused(false);
                }
            }
            entry.show();
        }
        self.shared.signal(true);
    }

    /// The encoder now in use.
    pub fn encoder(&self, name: Option<String>) {
        let mut state = self.shared.lock();
        if let Some(entry) = state.entry_mut(self.id) {
            entry.info.encoder = name;
        }
    }

    /// Progress, shown at most every 100 ms.
    pub fn report(&self, report: Report) {
        let elapsed = self.elapsed().as_secs_f64();
        {
            let mut state = self.shared.lock();
            let Some(entry) = state.entry_mut(self.id) else {
                return;
            };
            let info = &mut entry.info;
            info.progress = report.progress.clamp(0.0, 1.0);
            info.frames_done = report.frames_done;
            info.frames_total = report.frames_total;
            info.speed_fps = report.speed_fps;
            info.eta_seconds = report.eta_seconds;
            info.elapsed_seconds = elapsed;
        }
        self.shared.signal(false);
    }
}

fn start_probe(shared: &Arc<Shared>) {
    let generation = {
        let mut state = shared.lock();
        state.probe_generation += 1;
        state.caps = CapabilityState::Probing;
        state.probe_generation
    };
    let probing = Arc::clone(shared);
    let spawned = thread::Builder::new()
        .name("neptune-video-render-probe".into())
        .spawn(move || {
            let answer = catch_unwind(AssertUnwindSafe(|| (probing.probe)(&Gate(&probing))))
                .unwrap_or_else(|panic| Err(format!("Internal error: {}", panic_text(&*panic))));
            {
                let mut state = probing.lock();
                if state.probe_generation != generation {
                    return;
                }
                state.caps = match answer {
                    Ok(caps) => CapabilityState::Ready(Arc::new(caps)),
                    Err(why) => CapabilityState::Failed(why),
                };
            }
            probing.work.notify_all();
            probing.signal(true);
        });
    if let Err(error) = spawned {
        let mut state = shared.lock();
        if state.probe_generation == generation {
            state.caps = CapabilityState::Failed(format!("FFmpeg could not be checked ({error})"));
        }
    }
}

/// The probe's gate: see [`RenderQueue::hold_probe`].
struct Gate<'a>(&'a Shared);

impl ProbeGate for Gate<'_> {
    fn held(&self) -> bool {
        self.0.lock().probe_yields()
    }

    fn wait(&self) {
        let mut state = self.0.lock();
        while let Some(left) = state.probe_wait() {
            state = match left {
                None => (self.0.work.wait(state)).unwrap_or_else(|poisoned| poisoned.into_inner()),
                Some(left) => {
                    (self.0.work.wait_timeout(state, left))
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .0
                }
            };
        }
    }
}

/// The render thread: the first runnable job in list order, one at a time.
fn work(shared: &Arc<Shared>, mut run: Runner) {
    loop {
        let (ticket, handle, live) = {
            let mut state = shared.lock();
            loop {
                if state.shutdown {
                    return;
                }
                if let Some(index) = state.jobs.iter().position(Entry::runnable) {
                    match state.caps.clone() {
                        CapabilityState::Ready(caps) => {
                            break start(shared, &mut state, index, caps);
                        }
                        CapabilityState::Failed(why) => {
                            // Nothing waiting can be made until FFmpeg is
                            // checked again.
                            let entry = &mut state.jobs[index];
                            entry.phase = RenderState::Failed;
                            entry.info.message = Some(format!("Exports are unavailable: {why}"));
                            entry.show();
                            let event = finished_event(&entry.info, Err(why), None, None);
                            state.push_event(event);
                            drop(state);
                            shared.signal(true);
                            state = shared.lock();
                            continue;
                        }
                        CapabilityState::Probing => {}
                    }
                }
                state = shared
                    .work
                    .wait(state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        };
        shared.signal(true);
        let finish = catch_unwind(AssertUnwindSafe(|| {
            // What the names resolve to is looked at here, off the window's
            // thread; then what stopped exports left in the folder goes.
            if let Err(refused) = check_files(&ticket) {
                return Finish::Failed(refused.to_string());
            }
            if let Some(folder) = ticket.destination.parent() {
                engine::sweep_folder(folder, &live, SystemTime::now());
            }
            run(&ticket, &handle)
        }))
        .unwrap_or_else(|panic| {
            engine::clean(ticket.id, &ticket.destination);
            Finish::Failed(format!("Internal error: {}", panic_text(&*panic)))
        });
        end(shared, &handle, finish);
    }
}

/// Marks the job at `index` started and makes what its run is given, with
/// the ids of every job listed.
fn start(
    shared: &Arc<Shared>,
    state: &mut State,
    index: usize,
    caps: Arc<Capabilities>,
) -> (Ticket, Handle, HashSet<Uuid>) {
    let live = state.jobs.iter().map(|entry| entry.info.id).collect();
    let entry = &mut state.jobs[index];
    let control = Arc::new(Control::new(false));
    entry.control = Some(Arc::clone(&control));
    entry.phase = RenderState::Preparing;
    entry.info.attempts += 1;
    entry.info.message = None;
    entry.show();
    let ticket = Ticket {
        id: entry.info.id,
        project: Arc::clone(&entry.project),
        settings: entry.info.settings.clone(),
        destination: entry.info.destination.clone(),
        caps,
        sidecar: entry.outputs.sidecar.clone(),
        project_path: entry.project_path.clone(),
    };
    let handle = Handle {
        shared: Arc::clone(shared),
        id: entry.info.id,
        control,
        started: Instant::now(),
        paused_for: std::cell::Cell::new(Duration::ZERO),
    };
    (ticket, handle, live)
}

/// Records how a job ended and tells the window.
fn end(shared: &Arc<Shared>, handle: &Handle, finish: Finish) {
    // A failure that a cancel caused is a cancel.
    let finish = match finish {
        Finish::Failed(_) if handle.cancelled() => Finish::Cancelled,
        finish => finish,
    };
    let elapsed = handle.elapsed().as_secs_f64();
    {
        let mut state = shared.lock();
        let Some(entry) = state.entry_mut(handle.id) else {
            return;
        };
        entry.control = None;
        entry.paused = false;
        let info = &mut entry.info;
        info.elapsed_seconds = elapsed;
        info.eta_seconds = None;
        info.speed_fps = 0.0;
        let event = match finish {
            Finish::Done { path, bytes, note } => {
                entry.phase = RenderState::Done;
                info.progress = 1.0;
                info.frames_done = info.frames_total;
                info.output_bytes = bytes;
                info.message = note.clone();
                Some(finished_event(info, Ok(path), bytes, note))
            }
            Finish::Failed(why) => {
                entry.phase = RenderState::Failed;
                info.message = Some(why.clone());
                Some(finished_event(info, Err(why), None, None))
            }
            Finish::Cancelled => {
                entry.phase = RenderState::Cancelled;
                info.message = None;
                None
            }
        };
        entry.show();
        if let Some(event) = event {
            state.push_event(event);
        }
    }
    shared.work.notify_all();
    shared.signal(true);
}

fn finished_event(
    info: &RenderJobInfo,
    result: Result<PathBuf, String>,
    output_bytes: Option<u64>,
    note: Option<String>,
) -> RenderEvent {
    RenderEvent::Finished {
        id: info.id,
        name: info.name.clone(),
        result,
        output_bytes,
        note,
    }
}

fn panic_text(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "the export stopped unexpectedly".into())
}

// ---------------------------------------------------------------------------
// Destination rules (rule 16)
// ---------------------------------------------------------------------------

/// What `enqueue` checks without the file system: the destination is an
/// absolute file path with the container's extension, and neither it nor
/// the caption file beside it names media the project reads or the
/// project's own files.
fn check_names(job: &NewJob, sidecar: Option<&Path>) -> Result<(), EnqueueError> {
    let destination = &job.destination;
    if !destination.is_absolute() || destination.file_name().is_none() {
        return Err(EnqueueError::DestinationNotAbsolute);
    }
    let expected = job.settings.container.extension();
    let extension = destination
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase());
    if extension.as_deref() != Some(expected) {
        return Err(EnqueueError::WrongExtension { expected });
    }
    for output in std::iter::once(destination.as_path()).chain(sidecar) {
        collides(
            output,
            &job.project,
            job.project_path.as_deref(),
            &normalized,
        )?;
    }
    Ok(())
}

/// What the render thread checks as a job starts, asking the file system:
/// the destination is not a folder, its folder exists, and neither it nor
/// its caption file resolves (through links, `..` or another case of the
/// same letters) to media the project reads or the project's own files,
/// nor is that same file under another name (a hard link).
fn check_files(ticket: &Ticket) -> Result<(), EnqueueError> {
    let destination = &ticket.destination;
    if destination.is_dir() {
        return Err(EnqueueError::DestinationIsFolder);
    }
    if !destination.parent().is_some_and(Path::is_dir) {
        return Err(EnqueueError::FolderMissing);
    }
    for output in std::iter::once(destination.as_path()).chain(ticket.sidecar.as_deref()) {
        collides(
            output,
            &ticket.project,
            ticket.project_path.as_deref(),
            &identity,
        )?;
        let Some(file) = file_id(output) else {
            continue;
        };
        if (ticket.project.assets.values()).any(|asset| file_id(&asset.path) == Some(file)) {
            return Err(EnqueueError::DestinationIsSource);
        }
        if let Some(project) = &ticket.project_path
            && project_files(project).any(|path| file_id(&path) == Some(file))
        {
            return Err(EnqueueError::DestinationIsProject);
        }
    }
    Ok(())
}

/// Whether `output`, resolved by `resolve`, names media the project reads
/// or the project's own files.
fn collides(
    output: &Path,
    project: &Project,
    project_path: Option<&Path>,
    resolve: &dyn Fn(&Path) -> PathBuf,
) -> Result<(), EnqueueError> {
    let target = resolve(output);
    if (project.assets.values()).any(|asset| same(&resolve(&asset.path), &target)) {
        return Err(EnqueueError::DestinationIsSource);
    }
    if let Some(project) = project_path {
        let project = resolve(project);
        let checkpoints = beside(&project, ".checkpoints");
        if project_files(&project).any(|path| same(&path, &target))
            || inside(&target, &checkpoints)
            || same(&target, &checkpoints)
        {
            return Err(EnqueueError::DestinationIsProject);
        }
    }
    Ok(())
}

/// The project's file and the recovery and lock files kept beside it.
fn project_files(project: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    std::iter::once(project.to_path_buf()).chain(
        [".recovery", ".lock", ".write-lock"]
            .into_iter()
            .map(|suffix| beside(project, suffix)),
    )
}

/// `path` with `suffix` added to its name.
fn beside(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// What a path names: canonical when it exists, otherwise its folder's
/// canonical path and its name, otherwise the path made lexically normal.
fn identity(path: &Path) -> PathBuf {
    if let Ok(path) = path.canonicalize() {
        return path;
    }
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name())
        && let Ok(parent) = parent.canonicalize()
    {
        return parent.join(name);
    }
    normalized(path)
}

/// The file an existing path is, whatever it is called: its device and
/// index. Unknown where the system does not say.
fn file_id(path: &Path) -> Option<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(path).ok()?;
        Some((meta.dev(), meta.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// `.` and `..` resolved without asking the file system.
fn normalized(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Two identities name the same file, regardless of case: most volumes on
/// Windows and macOS ignore it, and so do the FAT and exFAT cards and drives
/// Linux mounts, whose names `canonicalize` leaves as given. Refusing two
/// names that differ only in case on a volume that keeps them apart costs a
/// rename; missing it there could replace a source.
fn same(a: &Path, b: &Path) -> bool {
    a == b || a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

fn inside(path: &Path, folder: &Path) -> bool {
    let path = path.to_string_lossy().to_lowercase();
    let folder = folder.to_string_lossy().to_lowercase();
    Path::new(&path).starts_with(Path::new(&folder))
}

#[cfg(test)]
#[path = "queue_tests.rs"]
mod tests;
