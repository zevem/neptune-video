//! The sound lane: one worker that reads a media file's sound through
//! FFmpeg, apart from the mixer that plays it.
//!
//! What it makes is derived data: a file's peaks, read once and kept in a
//! bounded cache outside the project, the loudness of a span, measured when
//! asked and kept nowhere, and the beats found in kept peaks, kept nowhere
//! either. Besides, it writes the sound of a span to a WAV file the person
//! chose, which is theirs and no project's, and has a stem separator the
//! person installed split a file's sound into stems in the project's media
//! folder, which are theirs too. The lane has its own thread, queue and
//! FFmpeg processes, beside
//! the analysis lane whose long measurements would hold up a waveform, and a
//! second thread that only reads what the cache keeps and finds beats in it.
//! An answer names the request, the project that asked and the source it
//! read, for the caller to drop it when any of them is no longer the one it
//! has.
use crate::{
    MediaError, SourceIdentity, Wake,
    audio::chain,
    io,
    process::{Process, Processes, tool_command},
};
use editor_core::{Asset, AssetId, MediaKind, Time};
use std::{
    collections::VecDeque,
    io::Read,
    path::PathBuf,
    process::ExitStatus,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

mod beats;
mod extract;
mod loudness;
mod peaks;
mod separate;

pub use beats::{BeatGrid, MAX_BEATS, beats};
pub use extract::{ExtractJob, part_path};
pub(crate) use loudness::summary as loudness_summary;
pub use loudness::{LOUDNESS_FLOOR, Loudness, LoudnessJob};
pub use peaks::{
    KeptPeaks, MAX_PEAK_BUCKETS, PEAK_ALGORITHM, PEAK_CACHE_BYTES, PEAK_CACHE_ENTRIES,
    PEAKS_PER_SECOND, Peak, PeakCache, Peaks,
};
use peaks::{Reducer, SAMPLE_RATE, Stamp, buckets_in};
pub use separate::{
    SEPARATOR, SEPARATOR_VARIABLE, STEMS, SeparateJob, Separator, look_for_separator,
    separator_absent, separator_program, stem_separator,
};

/// Jobs that may wait behind the one being worked on.
pub const SOUND_QUEUE: usize = 16;
/// Readings of kept peaks, and of their beats, that may wait to be read.
pub const PEAK_LOADS: usize = 16;

/// One media file whose sound is to be read as peaks, or whose kept peaks,
/// or their beats, are asked for.
#[derive(Clone, Debug)]
pub struct PeakJob {
    /// The project that asks, handed back with the answer and never read.
    pub project: Uuid,
    pub asset: Asset,
    pub cache: PeakCache,
}

#[derive(Debug)]
pub enum SoundEvent {
    /// How much of a job is done, `0..=1`. Only the newest of a job waits
    /// to be read.
    Progress { request: u64, done: f32 },
    /// A file's sound was read, or could not be. A job that was cancelled
    /// or replaced says nothing.
    Peaked {
        request: u64,
        project: Uuid,
        asset_id: AssetId,
        /// The source as it was when the job was asked for.
        source: SourceIdentity,
        result: Result<Arc<Peaks>, String>,
        /// Why peaks just read could not be kept for another session.
        unkept: Option<String>,
    },
    /// What the cache keeps of a file's peaks, read without its sound.
    Loaded {
        request: u64,
        project: Uuid,
        asset_id: AssetId,
        source: SourceIdentity,
        kept: KeptPeaks,
    },
    /// A span's loudness was measured, or could not be. A job that was
    /// cancelled or replaced says nothing.
    Measured {
        request: u64,
        project: Uuid,
        asset_id: AssetId,
        /// The source as it was when the job was asked for.
        source: SourceIdentity,
        result: Result<Loudness, String>,
    },
    /// The beats of a file's kept peaks were found, or there were none to
    /// find (`Ok(None)`: no clear period), or no peaks are kept. A request
    /// that was cancelled or replaced says nothing.
    Beats {
        request: u64,
        project: Uuid,
        asset_id: AssetId,
        /// The source as it was when the beats were asked for.
        source: SourceIdentity,
        result: Result<Option<BeatGrid>, String>,
    },
    /// A span of a file's sound was written to `to`, and how much of it, or
    /// it was not and `to` is as it was. A job that was cancelled or replaced
    /// before it claimed its last step says nothing and leaves no file.
    Extracted {
        request: u64,
        project: Uuid,
        asset_id: AssetId,
        /// The source as it was when the job was asked for.
        source: SourceIdentity,
        to: PathBuf,
        result: Result<Time, String>,
    },
    /// A file's sound was separated and its stems put in the job's folder
    /// and probed, in [`STEMS`]' order, or it was not and no stem was put
    /// there unless the error names it. A job that was cancelled or replaced
    /// before it claimed its last step says nothing and leaves no file.
    Separated {
        request: u64,
        project: Uuid,
        asset_id: AssetId,
        /// The source as it was when the job was asked for.
        source: SourceIdentity,
        result: Result<Vec<Asset>, String>,
    },
}

/// A file the lane took to write, or stems to put in place, and what it
/// gave up for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extracting {
    pub request: u64,
    /// Files of the same span of the same media, waiting or under way, given
    /// up for this one: they answer nothing and leave no file. One that has
    /// claimed its last step, or has answered, is not given up and is not
    /// here.
    pub replaced: Vec<u64>,
}

/// What came of giving up a request by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cancel {
    /// Given up: it answers nothing and leaves no file. So is a file whose
    /// failure waited to be read.
    GivenUp,
    /// A file being put in its place, too late to give up: its answer
    /// comes, written or not.
    Finishing,
    /// A file that was written: its answer waits to be read.
    Written,
}

impl SoundEvent {
    fn request(&self) -> u64 {
        match self {
            Self::Progress { request, .. }
            | Self::Peaked { request, .. }
            | Self::Loaded { request, .. }
            | Self::Measured { request, .. }
            | Self::Beats { request, .. }
            | Self::Extracted { request, .. }
            | Self::Separated { request, .. } => *request,
        }
    }
}

/// What a job under way is told and tells.
pub(crate) struct Control<'a> {
    cancel: &'a AtomicBool,
    report: &'a dyn Fn(f32),
    claim: &'a dyn Fn() -> bool,
}

impl Control<'_> {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }
    fn progress(&self, done: f32) {
        (self.report)(done);
    }
    /// Takes the job's last step, after which it is no longer given up:
    /// `false` when it was given up first.
    fn claim(&self) -> bool {
        (self.claim)()
    }
}

type PeakRunner = Arc<
    dyn Fn(&PeakJob, &Control) -> Result<(Arc<Peaks>, Option<String>), MediaError> + Send + Sync,
>;
type LoudnessRunner =
    Arc<dyn Fn(&LoudnessJob, &Control) -> Result<Loudness, MediaError> + Send + Sync>;
type BeatRunner = Arc<dyn Fn(&PeakJob) -> Result<Option<BeatGrid>, MediaError> + Send + Sync>;
type ExtractRunner = Arc<dyn Fn(&ExtractJob, &Control) -> Result<Time, MediaError> + Send + Sync>;
type SeparateRunner =
    Arc<dyn Fn(&SeparateJob, &Control) -> Result<Vec<Asset>, MediaError> + Send + Sync>;

/// What the lane runs for each kind of job. Tests stand in for them.
struct Runners {
    peaks: PeakRunner,
    loudness: LoudnessRunner,
    extract: ExtractRunner,
    separate: SeparateRunner,
    /// Run on the thread that reads the cache, not the worker.
    beats: BeatRunner,
}

/// What the lane works on, one at a time.
enum Work {
    Peaks(PeakJob),
    Loudness(LoudnessJob),
    Extract(ExtractJob),
    Separate(SeparateJob),
}

/// What the thread that reads the cache is asked for.
enum Load {
    /// What the cache keeps of a file's peaks.
    Kept(PeakJob),
    /// The beats of the peaks it keeps.
    Beats(PeakJob),
}

impl Load {
    fn target(&self) -> Target {
        match self {
            Self::Kept(job) => Target::Kept(job.asset.id),
            Self::Beats(job) => Target::Beats(job.asset.id),
        }
    }
}

/// What a newer job takes the place of an older for: its kind and its
/// media file, and for a measurement or a written file its span. Jobs of
/// different targets never give each other up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Peaks(AssetId),
    Loudness {
        asset: AssetId,
        from: Time,
        span: Time,
    },
    Extract {
        asset: AssetId,
        from: Time,
        span: Time,
    },
    Kept(AssetId),
    Beats(AssetId),
    Separate(AssetId),
}

impl Target {
    /// Files being written, which are the person's and no project's.
    fn saves(self) -> bool {
        matches!(self, Self::Extract { .. } | Self::Separate(_))
    }
}

impl Work {
    fn target(&self) -> Target {
        match self {
            Self::Peaks(job) => Target::Peaks(job.asset.id),
            Self::Loudness(job) => Target::Loudness {
                asset: job.asset.id,
                from: job.from,
                span: job.span,
            },
            Self::Extract(job) => Target::Extract {
                asset: job.asset.id,
                from: job.from,
                span: job.span,
            },
            Self::Separate(job) => Target::Separate(job.asset.id),
        }
    }
}

struct Running {
    request: u64,
    target: Target,
    cancel: Arc<AtomicBool>,
    /// The job has taken its last step and is no longer given up.
    claimed: bool,
}

#[derive(Default)]
struct Lane {
    queue: VecDeque<(u64, Work)>,
    running: Option<Running>,
    /// Kept peaks to read, or to find beats in, on a thread of their own so
    /// that none waits behind a file whose sound is being read.
    loads: VecDeque<(u64, Load)>,
    /// The reading under way, until it is given up.
    loading: Option<(u64, Target)>,
    events: VecDeque<SoundEvent>,
    next: u64,
    stopped: bool,
}

impl Lane {
    fn forget_events(&mut self, request: u64) {
        self.events.retain(|event| event.request() != request);
    }
}

#[derive(Default)]
struct Shared {
    lane: Mutex<Lane>,
    ready: Condvar,
    /// Wakes the thread that reads kept peaks.
    to_load: Condvar,
}

/// Nonblocking caller interface of the sound lane.
pub struct SoundService {
    shared: Arc<Shared>,
    processes: Processes,
}

impl SoundService {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Result<Self, MediaError> {
        let processes = Processes::default();
        let (reading, measuring, writing, separating) = (
            processes.clone(),
            processes.clone(),
            processes.clone(),
            processes.clone(),
        );
        Self::with_runners(
            Arc::new(wake),
            processes,
            Runners {
                peaks: Arc::new(move |job, control| peak_asset(job, &reading, control)),
                // The session's probe runs here, on the lane, the first
                // time: a measurement hears the effects this FFmpeg runs.
                loudness: Arc::new(move |job, control| {
                    loudness::measure(job, &chain::session(), &measuring, control)
                }),
                extract: Arc::new(move |job, control| extract::write(job, &writing, control)),
                separate: Arc::new(move |job, control| {
                    separate::separate(job, &separating, control)
                }),
                beats: Arc::new(find_beats),
            },
        )
    }

    fn with_runners(
        wake: Wake,
        processes: Processes,
        runners: Runners,
    ) -> Result<Self, MediaError> {
        let shared = Arc::new(Shared::default());
        let (state, waker) = (Arc::clone(&shared), Arc::clone(&wake));
        let find = Arc::clone(&runners.beats);
        thread::Builder::new()
            .name("aster-sound".into())
            .spawn(move || worker(state, runners, waker))
            .map_err(|error| io("Cannot start sound worker", error))?;
        let state = Arc::clone(&shared);
        let started = thread::Builder::new()
            .name("aster-sound-load".into())
            .spawn(move || loader(state, find, wake));
        if let Err(error) = started {
            // The worker that did start is told to end.
            if let Ok(mut lane) = shared.lane.lock() {
                lane.stopped = true;
            }
            shared.ready.notify_all();
            return Err(io("Cannot start sound reader", error));
        }
        Ok(Self { shared, processes })
    }

    /// Queues a media file whose sound is to be read as peaks and kept in
    /// the job's cache, and names the request. A job for the same file that
    /// waits is dropped, and one under way is stopped; neither answers.
    /// `Busy` when [`SOUND_QUEUE`] others wait.
    pub fn peaks(&self, job: PeakJob) -> Result<u64, MediaError> {
        check_peaks(&job)?;
        self.queue(Work::Peaks(job)).map(|(request, _)| request)
    }

    /// Queues a span of a media file's sound to be measured for loudness
    /// behind the job's audio effects, and names the request. A job for the
    /// same span of the same file that waits is dropped, and one under way
    /// is stopped; neither answers. Other spans, and the file's peaks, are
    /// left alone. `Busy` when [`SOUND_QUEUE`] others wait.
    pub fn loudness(&self, job: LoudnessJob) -> Result<u64, MediaError> {
        loudness::check(&job)?;
        self.queue(Work::Loudness(job)).map(|(request, _)| request)
    }

    /// Queues a span of a media file's sound to be written to the job's
    /// file as WAV, and names the request and those it gave up. A job for
    /// the same span of the same file that waits is dropped, and one under
    /// way is stopped, unless it has claimed its last step; neither answers,
    /// and neither leaves a file. One that has claimed it is written and
    /// answers. Other spans, and the file's other jobs, are left alone, and
    /// [`cancel_all`](Self::cancel_all) leaves it too. `Busy` when
    /// [`SOUND_QUEUE`] others wait.
    pub fn extract(&self, job: ExtractJob) -> Result<Extracting, MediaError> {
        extract::check(&job)?;
        let (request, replaced) = self.queue(Work::Extract(job))?;
        Ok(Extracting { request, replaced })
    }

    /// Queues a media file's whole sound to be separated into stems by the
    /// job's separator and put in the job's folder, and names the request
    /// and those it gave up. A separation of the same file that waits is
    /// dropped, and one under way is stopped, unless it has claimed its last
    /// step; neither answers, and neither leaves a stem. One that has
    /// claimed it puts its stems in place and answers. The file's other
    /// jobs are left alone, and [`cancel_all`](Self::cancel_all) leaves it
    /// too: the stems are the person's. The tool may run for many minutes,
    /// and everything else the lane is asked waits behind it. `Busy` when
    /// [`SOUND_QUEUE`] others wait.
    pub fn separate(&self, job: SeparateJob) -> Result<Extracting, MediaError> {
        separate::check(&job)?;
        let (request, replaced) = self.queue(Work::Separate(job))?;
        Ok(Extracting { request, replaced })
    }

    /// Asks what the job's cache keeps of a file's peaks, and names the
    /// request. The file is read on a thread of its own, so the answer does
    /// not wait for sound being read. A newer request for the same media
    /// takes the place of an older that waits; `Busy` when [`PEAK_LOADS`]
    /// others wait.
    pub fn load_peaks(&self, job: PeakJob) -> Result<u64, MediaError> {
        check_peaks(&job)?;
        let mut lane = self.shared.lane.lock().map_err(|_| MediaError::Closed)?;
        if lane.stopped {
            return Err(MediaError::Closed);
        }
        let target = Target::Kept(job.asset.id);
        lane.loads.retain(|(_, waiting)| waiting.target() != target);
        if lane.loads.len() >= PEAK_LOADS {
            return Err(MediaError::Busy);
        }
        lane.next += 1;
        let request = lane.next;
        lane.loads.push_back((request, Load::Kept(job)));
        self.shared.to_load.notify_all();
        Ok(request)
    }

    /// Asks for the beats of the peaks the job's cache keeps of a file, and
    /// names the request. They are found on the thread that reads the cache,
    /// so the answer does not wait for sound being read, and a file whose
    /// peaks are not kept is an error in words: its peaks are asked for
    /// first. A request for the same file's beats that waits or is under way
    /// is given up and answers nothing; its peaks and kept peaks are left
    /// alone. `Busy` when [`PEAK_LOADS`] others wait.
    pub fn beats(&self, job: PeakJob) -> Result<u64, MediaError> {
        check_beats(&job)?;
        let mut lane = self.shared.lane.lock().map_err(|_| MediaError::Closed)?;
        if lane.stopped {
            return Err(MediaError::Closed);
        }
        let target = Target::Beats(job.asset.id);
        let replaced = (lane.loads.iter())
            .filter(|(_, waiting)| waiting.target() == target)
            .count();
        if lane.loads.len() - replaced >= PEAK_LOADS {
            return Err(MediaError::Busy);
        }
        lane.loads.retain(|(_, waiting)| waiting.target() != target);
        // The one under way finishes and says nothing.
        lane.loading.take_if(|(_, loading)| *loading == target);
        lane.next += 1;
        let request = lane.next;
        lane.loads.push_back((request, Load::Beats(job)));
        self.shared.to_load.notify_all();
        Ok(request)
    }

    /// Queues `work`, and names it and the requests of its target it gave
    /// up: the one that waited, and the one under way unless it had claimed
    /// its last step or was given up already.
    fn queue(&self, work: Work) -> Result<(u64, Vec<u64>), MediaError> {
        let target = work.target();
        let mut lane = self.shared.lane.lock().map_err(|_| MediaError::Closed)?;
        if lane.stopped {
            return Err(MediaError::Closed);
        }
        let mut replaced: Vec<u64> = (lane.queue.iter())
            .filter(|(_, waiting)| waiting.target() == target)
            .map(|(request, _)| *request)
            .collect();
        if lane.queue.len() - replaced.len() >= SOUND_QUEUE {
            return Err(MediaError::Busy);
        }
        lane.queue.retain(|(_, waiting)| waiting.target() != target);
        for request in &replaced {
            lane.forget_events(*request);
        }
        if let Some(request) = (lane.running.as_ref())
            .filter(|running| running.target == target)
            .map(|running| running.request)
            && self.stop_running(&mut lane, request)
        {
            // What it said and nobody read goes with it, as when it is
            // given up by name: the worker forgets nothing for a stopped job.
            lane.forget_events(request);
            replaced.push(request);
        }
        lane.next += 1;
        let request = lane.next;
        lane.queue.push_back((request, work));
        self.shared.ready.notify_all();
        Ok((request, replaced))
    }

    /// Gives up one request, waiting or under way. It answers nothing, and
    /// what it had answered and nobody read is dropped. A file that was
    /// written, or is being put in its place, is not given up, and its
    /// answer waits to be read or comes: see [`Cancel`].
    pub fn cancel(&self, request: u64) -> Cancel {
        let Ok(mut lane) = self.shared.lane.lock() else {
            return Cancel::GivenUp;
        };
        let written = (lane.events.iter()).any(|event| match event {
            SoundEvent::Extracted {
                request: of,
                result: Ok(_),
                ..
            }
            | SoundEvent::Separated {
                request: of,
                result: Ok(_),
                ..
            } => *of == request,
            _ => false,
        });
        if written {
            return Cancel::Written;
        }
        let claimed = (lane.running.as_ref())
            .is_some_and(|running| running.request == request && running.claimed);
        if claimed {
            return Cancel::Finishing;
        }
        lane.queue.retain(|(waiting, _)| *waiting != request);
        lane.loads.retain(|(waiting, _)| *waiting != request);
        lane.loading.take_if(|(loading, _)| *loading == request);
        lane.forget_events(request);
        self.stop_running(&mut lane, request);
        Cancel::GivenUp
    }

    /// Gives up every request but the files being written: the project is
    /// another. A written file is the person's, asked of no project, and is
    /// given up by its name alone. It does not wait.
    pub fn cancel_all(&self) {
        let Ok(mut lane) = self.shared.lane.lock() else {
            return;
        };
        lane.queue.retain(|(_, work)| work.target().saves());
        lane.loads.clear();
        lane.loading = None;
        let saving = (lane.running.as_ref())
            .filter(|running| running.target.saves())
            .map(|running| running.request);
        lane.events.retain(|event| match event {
            SoundEvent::Extracted { .. } | SoundEvent::Separated { .. } => true,
            SoundEvent::Progress { request, .. } => Some(*request) == saving,
            _ => false,
        });
        if let Some(request) = lane.running.as_ref().map(|running| running.request)
            && saving.is_none()
        {
            self.stop_running(&mut lane, request);
        }
    }

    pub fn poll(&self) -> Option<SoundEvent> {
        self.shared.lane.lock().ok()?.events.pop_front()
    }

    /// Whether anything waits or is under way.
    pub fn busy(&self) -> bool {
        (self.shared.lane.lock()).is_ok_and(|lane| {
            lane.running.is_some()
                || !lane.queue.is_empty()
                || !lane.loads.is_empty()
                || lane.loading.is_some()
        })
    }

    // The lane has one process at a time, and the worker names the job it
    // is on under the lock before it starts one: killing every process here,
    // with the lock held, kills that job's and no later one's. Whether it
    // stopped it now: not when it had claimed its last step, or was stopped.
    fn stop_running(&self, lane: &mut Lane, request: u64) -> bool {
        let Some(running) = (lane.running.as_ref())
            .filter(|running| running.request == request && !running.claimed)
        else {
            return false;
        };
        if running.cancel.swap(true, Ordering::AcqRel) {
            return false;
        }
        self.processes.kill_all();
        true
    }
}

impl Drop for SoundService {
    fn drop(&mut self) {
        if let Ok(mut lane) = self.shared.lane.lock() {
            lane.stopped = true;
            lane.queue.clear();
            lane.loads.clear();
            if let Some(running) = &lane.running {
                running.cancel.store(true, Ordering::Release);
            }
        }
        self.shared.ready.notify_all();
        self.shared.to_load.notify_all();
        self.processes.kill_all();
        // The worker cleans up after itself; a closing window joins nothing.
    }
}

// Refused here, before any process: media without sound, and media whose
// peaks would be more than a file of them may hold.
fn check_peaks(job: &PeakJob) -> Result<(), MediaError> {
    check_sound(
        &job.asset,
        "Media has no sound to draw",
        "Media is too long for a waveform: the limit is 5.8 hours",
    )
}

// Refused here, before the cache is read: media that can have no peaks.
fn check_beats(job: &PeakJob) -> Result<(), MediaError> {
    check_sound(
        &job.asset,
        "Media has no sound to find beats in",
        "Media is too long to find beats in: the limit is 5.8 hours",
    )
}

fn check_sound(asset: &Asset, silent: &str, too_long: &str) -> Result<(), MediaError> {
    let invalid = |why: &str| Err(MediaError::Invalid(why.into()));
    if !asset.has_audio && asset.kind != MediaKind::Audio {
        return invalid(silent);
    }
    if !asset.duration.is_valid() || asset.duration <= Time::ZERO {
        return invalid("Media has no length");
    }
    if buckets_in(asset.duration) > MAX_PEAK_BUCKETS as u64 {
        return invalid(too_long);
    }
    Ok(())
}

/// Reads kept peaks, and finds their beats, as they are asked for. A
/// reading that was given up or replaced while it ran answers nothing.
fn loader(shared: Arc<Shared>, find_beats: BeatRunner, wake: Wake) {
    loop {
        let (request, load) = {
            let Ok(mut lane) = shared.lane.lock() else {
                return;
            };
            loop {
                if lane.stopped {
                    return;
                }
                if let Some((request, load)) = lane.loads.pop_front() {
                    lane.loading = Some((request, load.target()));
                    break (request, load);
                }
                let Ok(next) = shared.to_load.wait(lane) else {
                    return;
                };
                lane = next;
            }
        };
        let answer = match &load {
            Load::Kept(job) => SoundEvent::Loaded {
                request,
                project: job.project,
                asset_id: job.asset.id,
                source: SourceIdentity::of(&job.asset),
                kept: job.cache.load(&job.asset),
            },
            Load::Beats(job) => SoundEvent::Beats {
                request,
                project: job.project,
                asset_id: job.asset.id,
                source: SourceIdentity::of(&job.asset),
                result: find_beats(job).map_err(|error| error.to_string()),
            },
        };
        let Ok(mut lane) = shared.lane.lock() else {
            return;
        };
        if lane.stopped {
            return;
        }
        if lane.loading.take().map(|(loading, _)| loading) != Some(request) {
            continue;
        }
        lane.events.push_back(answer);
        drop(lane);
        wake();
    }
}

/// The beats of the peaks the job's cache keeps of its file. Peaks that are
/// not kept, or are of the file as it was, are not read again here: that is
/// a job of the lane's worker, and the caller's to ask for.
fn find_beats(job: &PeakJob) -> Result<Option<BeatGrid>, MediaError> {
    match job.cache.load(&job.asset) {
        KeptPeaks::Held(peaks) => Ok(beats(&peaks)),
        KeptPeaks::Stale => Err(MediaError::Invalid(
            "The media changed since its sound was read: its peaks are needed again to find its beats"
                .into(),
        )),
        KeptPeaks::Missing => Err(MediaError::Invalid(
            "The media's sound has not been read: its peaks are needed to find its beats".into(),
        )),
    }
}

fn worker(shared: Arc<Shared>, runners: Runners, wake: Wake) {
    loop {
        let (request, work, cancel) = {
            let Ok(mut lane) = shared.lane.lock() else {
                return;
            };
            loop {
                if lane.stopped {
                    return;
                }
                if let Some((request, work)) = lane.queue.pop_front() {
                    let cancel = Arc::new(AtomicBool::new(false));
                    lane.running = Some(Running {
                        request,
                        target: work.target(),
                        cancel: Arc::clone(&cancel),
                        claimed: false,
                    });
                    break (request, work, cancel);
                }
                let Ok(next) = shared.ready.wait(lane) else {
                    return;
                };
                lane = next;
            }
        };
        let report = |done: f32| {
            let Ok(mut lane) = shared.lane.lock() else {
                return;
            };
            if cancel.load(Ordering::Acquire) || lane.stopped {
                return;
            }
            // One figure of a job waits to be read, the newest.
            let waiting = lane.events.iter_mut().find_map(|event| match event {
                SoundEvent::Progress { request: of, done } if *of == request => Some(done),
                _ => None,
            });
            match waiting {
                Some(waiting) => *waiting = done,
                None => (lane.events).push_back(SoundEvent::Progress { request, done }),
            }
            drop(lane);
            wake();
        };
        // Under the lock that gives jobs up: a job given up before this
        // takes no last step, and one given up after is not stopped.
        let claim = || {
            let Ok(mut lane) = shared.lane.lock() else {
                return false;
            };
            if cancel.load(Ordering::Acquire) || lane.stopped {
                return false;
            }
            if let Some(running) = lane.running.as_mut() {
                running.claimed = true;
            }
            true
        };
        let control = Control {
            cancel: &cancel,
            report: &report,
            claim: &claim,
        };
        let answer = match &work {
            Work::Peaks(job) => {
                let (result, unkept) = match (runners.peaks)(job, &control) {
                    Ok((peaks, unkept)) => (Ok(peaks), unkept),
                    Err(error) => (Err(error.to_string()), None),
                };
                SoundEvent::Peaked {
                    request,
                    project: job.project,
                    asset_id: job.asset.id,
                    source: SourceIdentity::of(&job.asset),
                    result,
                    unkept,
                }
            }
            Work::Loudness(job) => SoundEvent::Measured {
                request,
                project: job.project,
                asset_id: job.asset.id,
                source: SourceIdentity::of(&job.asset),
                result: (runners.loudness)(job, &control).map_err(|error| error.to_string()),
            },
            Work::Extract(job) => SoundEvent::Extracted {
                request,
                project: job.project,
                asset_id: job.asset.id,
                source: SourceIdentity::of(&job.asset),
                to: job.to.clone(),
                result: (runners.extract)(job, &control).map_err(|error| error.to_string()),
            },
            Work::Separate(job) => SoundEvent::Separated {
                request,
                project: job.project,
                asset_id: job.asset.id,
                source: SourceIdentity::of(&job.asset),
                result: (runners.separate)(job, &control).map_err(|error| error.to_string()),
            },
        };
        let Ok(mut lane) = shared.lane.lock() else {
            return;
        };
        lane.running = None;
        if lane.stopped {
            return;
        }
        // The flag is read under the lock that set it: a job given up
        // before this answers nothing, and one given up after has its
        // answer taken out of the queue by whoever gave it up.
        if cancel.load(Ordering::Acquire) {
            continue;
        }
        lane.forget_events(request);
        lane.events.push_back(answer);
        drop(lane);
        wake();
    }
}

/// Reads the whole of the job's first sound stream through one FFmpeg
/// process, mixed to one channel at 8 kHz, reduces it to peaks as it comes
/// and keeps them in the job's cache. The peaks are the answer whether or
/// not they could be kept: the second value says why they were not.
fn peak_asset(
    job: &PeakJob,
    processes: &Processes,
    control: &Control,
) -> Result<(Arc<Peaks>, Option<String>), MediaError> {
    let asset = &job.asset;
    let expected = buckets_in(asset.duration).max(1);
    // What the file is now, before it is read: one that changes while it
    // is read is kept as what it was, and is stale the next time it is asked.
    let stamp = Stamp::of(asset);
    let mut command = tool_command("ffmpeg");
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-threads",
            "1",
            "-i",
        ])
        .arg(&asset.path)
        .args(["-map", "0:a:0", "-vn", "-sn", "-dn", "-ac", "1", "-ar"])
        .arg(SAMPLE_RATE.to_string())
        // Sound is padded from the container's start, so a stream that
        // begins late keeps its place: the mixer seeks in container time.
        .args(["-af", "aresample=async=1:first_pts=0"])
        .args(["-f", "s16le", "pipe:1"]);
    let mut process = Process::spawn("ffmpeg", &mut command, processes)?;
    let mut reducer = Reducer::default();
    let mut bytes = vec![0u8; 32 * 1024];
    let mut said = 0.0;
    loop {
        if control.cancelled() {
            return Err(MediaError::Cancelled);
        }
        process.begin_read();
        let outcome = process.stdout.read(&mut bytes);
        let in_time = process.end_read_checked();
        if control.cancelled() {
            return Err(MediaError::Cancelled);
        }
        match outcome {
            Ok(_) if !in_time => return Err(process.failure("the sound took too long to decode")),
            Ok(0) => break,
            Ok(count) => reducer.push(&bytes[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(process.failure(&format!("could not read the sound ({error})")));
            }
        }
        if reducer.len() > MAX_PEAK_BUCKETS {
            return Err(MediaError::Invalid(
                "Media has more sound than its length says: too much for a waveform".into(),
            ));
        }
        let done = (reducer.len() as f32 / expected as f32).min(1.0);
        if done - said >= 0.01 {
            said = done;
            control.progress(done);
        }
    }
    // The stream has ended; how the process ended says whether it was read
    // whole.
    let status = await_exit(&process, control)?;
    if !status.success() {
        return Err(process.failure("could not read the sound"));
    }
    let peaks = reducer.finish();
    if peaks.buckets.is_empty() {
        return Err(process.failure("no sound could be read"));
    }
    if peaks.buckets.len() > MAX_PEAK_BUCKETS {
        return Err(MediaError::Invalid(
            "Media has more sound than its length says: too much for a waveform".into(),
        ));
    }
    let peaks = Arc::new(peaks);
    let unkept = (job.cache.store(asset, &stamp, &peaks)).err();
    Ok((peaks, unkept))
}

/// How a process whose output has ended ended, waited for up to 10 s. The
/// child's lock is never held while waiting: a watchdog or a cancellation
/// needs it.
fn await_exit(process: &Process, control: &Control) -> Result<ExitStatus, MediaError> {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        if control.cancelled() {
            return Err(MediaError::Cancelled);
        }
        let ended = (process.child.lock().map_err(|_| MediaError::Closed)?)
            .try_wait()
            .map_err(|error| io("Cannot wait for ffmpeg", error))?;
        if let Some(status) = ended {
            return Ok(status);
        }
        if Instant::now() >= until {
            return Err(process.failure("the process did not end"));
        }
        thread::sleep(Duration::from_millis(2));
    }
}

#[cfg(test)]
mod tests;
