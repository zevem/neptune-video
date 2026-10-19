//! The analysis lane: one worker that reads a video's frames small and grey
//! through FFmpeg and follows one region through them.
//!
//! What it does is template matching: the region the person chose is kept as
//! a patch of luma, and in each next frame the place whose patch correlates
//! best with it (zero-mean normalized cross-correlation) inside a search
//! window is where the region went. It follows a translation only: no
//! rotation, no change of size, no perspective, no detection of what the
//! region shows, and nothing is found again once it is lost. A match under
//! [`LOST_BELOW`] ends the path there rather than letting it drift.
//!
//! The lane has its own thread, queue and FFmpeg processes, apart from the
//! preview and import workers, so a long analysis delays no frame. An answer
//! names the request, the project that asked and the source it read, for the
//! caller to drop it when any of them is no longer the one it has.
use crate::{
    MediaError, Wake, fitted_dimensions, io,
    process::{Process, Processes, tool_command},
};
use editor_core::{Asset, AssetId, FrameRate, MediaKind, Time, TrackKey, look::MAX_TRACK_KEYS};
use std::{
    collections::VecDeque,
    io::Read,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};
use uuid::Uuid;

/// The longer side of the frames a region is followed in. Positions are
/// refined under a pixel of it, and a full-size frame is never decoded.
pub const ANALYSIS_SIDE: u32 = 384;
/// Jobs that may wait behind the one being worked on.
pub const ANALYSIS_QUEUE: usize = 4;
/// The correlation under which the region is lost and the path ends.
pub const LOST_BELOW: f32 = 0.5;
/// The correlation under which the patch is taken again from where the
/// region was found: what it shows has changed, slowly enough to be followed.
pub const REFRESH_BELOW: f32 = 0.8;

// A patch is at least this many analysis pixels a side, and at most this many.
const MIN_PATCH: usize = 8;
const MAX_PATCH: usize = 160;
// How far from where it is expected a region is looked for, in pixels.
const MIN_REACH: i32 = 8;
const MAX_REACH: i32 = 32;
// A patch with less spread of luma than this (8-bit levels, squared) has
// nothing to tell one place from the next.
const MIN_VARIANCE: f32 = 9.0;
// A coarse score reads about this many texels of a patch.
const COARSE_TEXELS: usize = 1024;
// How far from the straight line between the keys kept a dropped key may
// lie, as a fraction of the frame: a twentieth of an analysis pixel.
const THIN_TOLERANCE: f32 = 0.05 / ANALYSIS_SIDE as f32;

/// What a source is, as far as its pixels are concerned: an answer read from
/// one is about no asset that is now of another file, length or size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceIdentity {
    pub path: PathBuf,
    pub file_size: u64,
    pub duration: Time,
    pub width: u32,
    pub height: u32,
    pub frame_rate: Option<FrameRate>,
}

impl SourceIdentity {
    pub fn of(asset: &Asset) -> Self {
        Self {
            path: asset.path.clone(),
            file_size: asset.file_size,
            duration: asset.duration,
            width: asset.width,
            height: asset.height,
            frame_rate: asset.frame_rate,
        }
    }
}

/// A box on the source frame, in fractions of it with the origin top left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub center: [f32; 2],
    pub size: [f32; 2],
}

impl Region {
    /// The smallest side a region may have, as a fraction of the frame.
    pub const MIN_SIDE: f32 = 0.02;

    /// The region held inside the frame, no smaller than it may be.
    pub fn held(self) -> Self {
        let size = self.size.map(|side| {
            if side.is_finite() {
                side.clamp(Self::MIN_SIDE, 1.0)
            } else {
                Self::MIN_SIDE
            }
        });
        let mut center = self.center;
        for axis in 0..2 {
            let half = size[axis] * 0.5;
            center[axis] = if center[axis].is_finite() {
                center[axis].clamp(half, 1.0 - half)
            } else {
                0.5
            };
        }
        Self { center, size }
    }
}

/// One region to follow through one stretch of one video.
#[derive(Clone, Debug)]
pub struct TrackJob {
    /// The project that asks, handed back with the answer and never read.
    pub project: Uuid,
    /// The tracker the answer is for. A newer job for the same one takes
    /// the place of an older, waiting or under way.
    pub target: Uuid,
    pub asset: Asset,
    /// Where the region is in the frame at `from`.
    pub region: Region,
    /// Source times: the frame the region was chosen on, and where to stop.
    pub from: Time,
    pub until: Time,
}

/// Where a region went.
#[derive(Clone, Debug, PartialEq)]
pub struct Tracked {
    /// The region's centre at source times, thinned to what a straight line
    /// between keys does not already say; `scale` is 1 and `rotation` 0.
    pub keys: Vec<TrackKey>,
    /// How many frames were read, the first included.
    pub frames: usize,
    /// The source time of the frame the region was lost in, when it was:
    /// the path ends on the frame before.
    pub lost_at: Option<Time>,
    /// The weakest match that was accepted, `LOST_BELOW..=1`.
    pub confidence: f32,
}

mod steady;

pub use steady::{
    Fix, MAX_STEADY_FRAMES, Motion, STEADY_ALGORITHM, STEADY_CACHE_BYTES, STEADY_CACHE_ENTRIES,
    STEADY_SECONDS, STEADY_SUGGEST_FRAMES, Sampling, Steadied, SteadyCache, SteadyJob,
};

/// Readings of kept camera paths that may wait to be read.
pub const STEADY_LOADS: usize = 16;

/// What the lane says of a camera path: apart from [`AnalysisEvent`], and
/// read with [`AnalysisService::poll_steady`].
#[derive(Debug)]
pub enum SteadyEvent {
    /// How much of a video has been measured, `0..=1`. Only the newest of a
    /// job waits to be read.
    Progress { request: u64, done: f32 },
    /// A measuring or a reading ended by itself. One that was cancelled or
    /// replaced says nothing.
    Done {
        request: u64,
        project: Uuid,
        asset_id: AssetId,
        /// The source as it was when the job was asked for.
        source: SourceIdentity,
        /// Whether the video was measured now, or what was kept of it read.
        measured: bool,
        result: Result<Steadied, String>,
        /// Why a path just measured could not be kept for another session.
        unkept: Option<String>,
    },
}

impl SteadyEvent {
    fn request(&self) -> u64 {
        match self {
            Self::Progress { request, .. } | Self::Done { request, .. } => *request,
        }
    }
}

#[derive(Debug)]
pub enum AnalysisEvent {
    /// How much of a job is done, `0..=1`. Only the newest of a job waits
    /// to be read.
    Progress { request: u64, done: f32 },
    /// A job ended by itself. One that was cancelled or replaced says
    /// nothing.
    Tracked {
        request: u64,
        project: Uuid,
        target: Uuid,
        asset_id: AssetId,
        /// The source as it was when the job was asked for.
        source: SourceIdentity,
        result: Result<Tracked, String>,
    },
}

/// What a job under way is told and tells.
pub(crate) struct Control<'a> {
    cancel: &'a AtomicBool,
    report: &'a dyn Fn(f32),
}

impl Control<'_> {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }
    fn progress(&self, done: f32) {
        (self.report)(done);
    }
}

type Runner = Arc<dyn Fn(&TrackJob, &Control) -> Result<Tracked, MediaError> + Send + Sync>;
type SteadyRunner = Arc<
    dyn Fn(&SteadyJob, &Control) -> Result<(Arc<Motion>, Option<String>), MediaError> + Send + Sync,
>;

/// What the lane works on, one at a time.
enum Work {
    Track(TrackJob),
    Steady(SteadyJob),
}

impl Work {
    /// What a newer job takes the place of an older for: a tracker, or the
    /// video whose camera is measured.
    fn target(&self) -> Uuid {
        match self {
            Self::Track(job) => job.target,
            Self::Steady(job) => job.asset.id,
        }
    }
}

struct Running {
    request: u64,
    target: Uuid,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct Lane {
    queue: VecDeque<(u64, Work)>,
    running: Option<Running>,
    events: VecDeque<AnalysisEvent>,
    /// Kept camera paths to read, on a thread of their own so that none
    /// waits behind a video being measured.
    loads: VecDeque<(u64, SteadyJob)>,
    /// The reading under way, until it is given up.
    loading: Option<u64>,
    steady: VecDeque<SteadyEvent>,
    next: u64,
    stopped: bool,
}

impl Lane {
    fn forget_events(&mut self, request: u64) {
        self.events.retain(|event| match event {
            AnalysisEvent::Progress { request: of, .. }
            | AnalysisEvent::Tracked { request: of, .. } => *of != request,
        });
        self.steady.retain(|event| event.request() != request);
    }
}

#[derive(Default)]
struct Shared {
    lane: Mutex<Lane>,
    ready: Condvar,
    /// Wakes the thread that reads kept camera paths.
    to_load: Condvar,
}

/// Nonblocking caller interface of the analysis lane.
pub struct AnalysisService {
    shared: Arc<Shared>,
    processes: Processes,
}

impl AnalysisService {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Result<Self, MediaError> {
        let processes = Processes::default();
        let (children, others) = (processes.clone(), processes.clone());
        Self::with_runners(
            Arc::new(wake),
            processes,
            Arc::new(move |job, control| track_asset(job, &children, control)),
            Arc::new(move |job, control| steady::steady_asset(job, &others, control)),
        )
    }

    #[cfg(test)]
    fn with_runner(wake: Wake, processes: Processes, run: Runner) -> Result<Self, MediaError> {
        let children = processes.clone();
        Self::with_runners(
            wake,
            processes,
            run,
            Arc::new(move |job, control| steady::steady_asset(job, &children, control)),
        )
    }

    fn with_runners(
        wake: Wake,
        processes: Processes,
        run: Runner,
        steady: SteadyRunner,
    ) -> Result<Self, MediaError> {
        let shared = Arc::new(Shared::default());
        let (state, waker) = (Arc::clone(&shared), Arc::clone(&wake));
        thread::Builder::new()
            .name("neptune-video-analysis".into())
            .spawn(move || worker(state, run, steady, waker))
            .map_err(|error| io("Cannot start analysis worker", error))?;
        let state = Arc::clone(&shared);
        let started = thread::Builder::new()
            .name("neptune-video-analysis-load".into())
            .spawn(move || loader(state, wake));
        if let Err(error) = started {
            // The worker that did start is told to end.
            if let Ok(mut lane) = shared.lane.lock() {
                lane.stopped = true;
            }
            shared.ready.notify_all();
            return Err(io("Cannot start analysis reader", error));
        }
        Ok(Self { shared, processes })
    }

    /// Queues a region to follow and names the request. A job for the same
    /// target that waits is dropped, and one under way is stopped; neither
    /// answers. `Busy` when [`ANALYSIS_QUEUE`] others wait.
    pub fn track(&self, job: TrackJob) -> Result<u64, MediaError> {
        check(&job)?;
        self.queue(Work::Track(job))
    }

    /// Queues a video whose camera path is to be measured from its frames
    /// and kept in the job's cache, and names the request. It waits its turn
    /// with the regions to follow, and a newer one for the same video takes
    /// the place of an older as theirs do. The answer is read with
    /// [`Self::poll_steady`].
    pub fn steady(&self, job: SteadyJob) -> Result<u64, MediaError> {
        steady::check_steady(&job)?;
        self.queue(Work::Steady(job))
    }

    /// Asks what the job's cache keeps of a video's camera path, and names
    /// the request. The file is read on a thread of its own, so the answer
    /// does not wait for a video being measured. A newer request for the
    /// same video takes the place of an older that waits; `Busy` when
    /// [`STEADY_LOADS`] others wait.
    pub fn load_steady(&self, job: SteadyJob) -> Result<u64, MediaError> {
        steady::check_steady(&job)?;
        let mut lane = self.shared.lane.lock().map_err(|_| MediaError::Closed)?;
        if lane.stopped {
            return Err(MediaError::Closed);
        }
        lane.loads
            .retain(|(_, waiting)| waiting.asset.id != job.asset.id);
        if lane.loads.len() >= STEADY_LOADS {
            return Err(MediaError::Busy);
        }
        lane.next += 1;
        let request = lane.next;
        lane.loads.push_back((request, job));
        self.shared.to_load.notify_all();
        Ok(request)
    }

    fn queue(&self, work: Work) -> Result<u64, MediaError> {
        let target = work.target();
        let mut lane = self.shared.lane.lock().map_err(|_| MediaError::Closed)?;
        if lane.stopped {
            return Err(MediaError::Closed);
        }
        let replaced: Vec<u64> = (lane.queue.iter())
            .filter(|(_, waiting)| waiting.target() == target)
            .map(|(request, _)| *request)
            .collect();
        if lane.queue.len() - replaced.len() >= ANALYSIS_QUEUE {
            return Err(MediaError::Busy);
        }
        lane.queue.retain(|(_, waiting)| waiting.target() != target);
        for request in replaced {
            lane.forget_events(request);
        }
        if let Some(request) = (lane.running.as_ref())
            .filter(|running| running.target == target)
            .map(|running| running.request)
        {
            self.stop_running(&mut lane, request);
        }
        lane.next += 1;
        let request = lane.next;
        lane.queue.push_back((request, work));
        self.shared.ready.notify_all();
        Ok(request)
    }

    /// Gives up one request, waiting or under way. It answers nothing, and
    /// what it had answered and nobody read is dropped.
    pub fn cancel(&self, request: u64) {
        let Ok(mut lane) = self.shared.lane.lock() else {
            return;
        };
        lane.queue.retain(|(waiting, _)| *waiting != request);
        lane.loads.retain(|(waiting, _)| *waiting != request);
        lane.loading.take_if(|loading| *loading == request);
        lane.forget_events(request);
        self.stop_running(&mut lane, request);
    }

    /// Gives up every request: the project is another. It does not wait.
    pub fn cancel_all(&self) {
        let Ok(mut lane) = self.shared.lane.lock() else {
            return;
        };
        lane.queue.clear();
        lane.loads.clear();
        lane.loading = None;
        lane.events.clear();
        lane.steady.clear();
        if let Some(request) = lane.running.as_ref().map(|running| running.request) {
            self.stop_running(&mut lane, request);
        }
    }

    pub fn poll(&self) -> Option<AnalysisEvent> {
        self.shared.lane.lock().ok()?.events.pop_front()
    }

    /// What was said of camera paths, in order.
    pub fn poll_steady(&self) -> Option<SteadyEvent> {
        self.shared.lane.lock().ok()?.steady.pop_front()
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
    // with the lock held, kills that job's and no later one's.
    fn stop_running(&self, lane: &mut Lane, request: u64) {
        if let Some(running) = (lane.running.as_ref()).filter(|running| running.request == request)
        {
            running.cancel.store(true, Ordering::Release);
            self.processes.kill_all();
        }
    }
}

impl Drop for AnalysisService {
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

fn check(job: &TrackJob) -> Result<(), MediaError> {
    let invalid = |why: &str| Err(MediaError::Invalid(why.into()));
    if job.asset.kind != MediaKind::Video {
        return invalid("Only a video has motion to track");
    }
    if job.asset.width == 0 || job.asset.height == 0 {
        return invalid("Media has no video dimensions");
    }
    let region = job.region;
    if !(region.center.iter().chain(&region.size)).all(|value| value.is_finite())
        || region.size.iter().any(|side| *side <= 0.0)
    {
        return invalid("The region to track is not a box");
    }
    if !job.from.is_valid() || !job.until.is_valid() || job.from < Time::ZERO {
        return invalid("The stretch to track is not a time");
    }
    if job.until <= job.from {
        return invalid("There is nothing after the playhead to track");
    }
    Ok(())
}

/// Reads kept camera paths as they are asked for. A reading that was given
/// up while its file was read answers nothing.
fn loader(shared: Arc<Shared>, wake: Wake) {
    loop {
        let (request, job) = {
            let Ok(mut lane) = shared.lane.lock() else {
                return;
            };
            loop {
                if lane.stopped {
                    return;
                }
                if let Some(next) = lane.loads.pop_front() {
                    lane.loading = Some(next.0);
                    break next;
                }
                let Ok(next) = shared.to_load.wait(lane) else {
                    return;
                };
                lane = next;
            }
        };
        let found = job.cache.load(&job.asset);
        let Ok(mut lane) = shared.lane.lock() else {
            return;
        };
        if lane.stopped {
            return;
        }
        if lane.loading.take() != Some(request) {
            continue;
        }
        lane.steady.push_back(SteadyEvent::Done {
            request,
            project: job.project,
            asset_id: job.asset.id,
            source: SourceIdentity::of(&job.asset),
            measured: false,
            result: Ok(found),
            unkept: None,
        });
        drop(lane);
        wake();
    }
}

fn worker(shared: Arc<Shared>, run: Runner, steady: SteadyRunner, wake: Wake) {
    loop {
        let (request, job, cancel) = {
            let Ok(mut lane) = shared.lane.lock() else {
                return;
            };
            loop {
                if lane.stopped {
                    return;
                }
                if let Some((request, job)) = lane.queue.pop_front() {
                    let cancel = Arc::new(AtomicBool::new(false));
                    lane.running = Some(Running {
                        request,
                        target: job.target(),
                        cancel: Arc::clone(&cancel),
                    });
                    break (request, job, cancel);
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
            if matches!(job, Work::Steady(_)) {
                let waiting = lane.steady.iter_mut().find_map(|event| match event {
                    SteadyEvent::Progress { request: of, done } if *of == request => Some(done),
                    _ => None,
                });
                match waiting {
                    Some(waiting) => *waiting = done,
                    None => (lane.steady).push_back(SteadyEvent::Progress { request, done }),
                }
            } else {
                let waiting = lane.events.iter_mut().find_map(|event| match event {
                    AnalysisEvent::Progress { request: of, done } if *of == request => Some(done),
                    _ => None,
                });
                match waiting {
                    Some(waiting) => *waiting = done,
                    None => (lane.events).push_back(AnalysisEvent::Progress { request, done }),
                }
            }
            drop(lane);
            wake();
        };
        let control = Control {
            cancel: &cancel,
            report: &report,
        };
        let done = match &job {
            Work::Track(job) => Done::Tracked(run(job, &control)),
            Work::Steady(job) => Done::Steadied(steady(job, &control)),
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
        match (job, done) {
            (Work::Track(job), Done::Tracked(result)) => {
                lane.events.push_back(AnalysisEvent::Tracked {
                    request,
                    project: job.project,
                    target: job.target,
                    asset_id: job.asset.id,
                    source: SourceIdentity::of(&job.asset),
                    result: result.map_err(|error| error.to_string()),
                });
            }
            (Work::Steady(job), Done::Steadied(result)) => {
                let (result, unkept) = match result {
                    Ok((motion, unkept)) => (Ok(Steadied::Ready(motion)), unkept),
                    Err(error) => (Err(error.to_string()), None),
                };
                lane.steady.push_back(SteadyEvent::Done {
                    request,
                    project: job.project,
                    asset_id: job.asset.id,
                    source: SourceIdentity::of(&job.asset),
                    measured: true,
                    result,
                    unkept,
                });
            }
            // A job is answered by its own runner.
            (Work::Track(_), Done::Steadied(_)) | (Work::Steady(_), Done::Tracked(_)) => {}
        }
        drop(lane);
        wake();
    }
}

/// What a job came to.
enum Done {
    Tracked(Result<Tracked, MediaError>),
    Steadied(Result<(Arc<Motion>, Option<String>), MediaError>),
}

/// An asset's frames as luma, read in order from one FFmpeg process at the
/// asset's own frame rate and no larger than [`ANALYSIS_SIDE`] a side.
pub(crate) struct Luma {
    process: Process,
    /// The frame last read.
    pub gray: Gray,
}

impl Luma {
    /// Starts reading `frames` frames from the frame `first`.
    pub fn open(
        asset: &Asset,
        first: i64,
        frames: usize,
        processes: &Processes,
    ) -> Result<Self, MediaError> {
        let rate = asset.frame_rate.unwrap_or_default();
        let [width, height] =
            fitted_dimensions([asset.width, asset.height], [ANALYSIS_SIDE, ANALYSIS_SIDE])?;
        let mut command = tool_command("ffmpeg");
        command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-threads",
                "2",
                "-ss",
            ])
            .arg(format!("{:.9}", first as f64 / rate.fps()))
            .arg("-i")
            .arg(&asset.path)
            .args([
                "-map",
                "0:V:0",
                "-an",
                "-sn",
                "-dn",
                "-filter_threads",
                "1",
                "-vf",
            ])
            .arg(format!(
                "scale={width}:{height}:flags=bilinear,fps={}/{}",
                rate.numerator, rate.denominator
            ))
            .args(["-frames:v"])
            .arg(frames.to_string())
            .args([
                "-pix_fmt",
                "gray",
                "-threads:v",
                "1",
                "-thread_queue_size",
                "1",
                "-f",
                "rawvideo",
                "pipe:1",
            ]);
        let process = Process::spawn("ffmpeg", &mut command, processes)?;
        Ok(Self {
            process,
            gray: Gray {
                width: width as usize,
                height: height as usize,
                luma: vec![0u8; width as usize * height as usize],
            },
        })
    }

    /// Reads the next frame into [`Self::gray`]. False when the stream has
    /// no more: it is shorter than its container said.
    pub fn next(&mut self, control: &Control) -> Result<bool, MediaError> {
        if control.cancelled() {
            return Err(MediaError::Cancelled);
        }
        self.process.begin_read();
        let outcome = self.process.stdout.read_exact(&mut self.gray.luma);
        let in_time = self.process.end_read_checked();
        if control.cancelled() {
            return Err(MediaError::Cancelled);
        }
        match outcome {
            Ok(()) if in_time => Ok(true),
            Ok(()) => Err(self.failure("a frame took too long to decode")),
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
            Err(error) => Err(self.failure(&format!("could not read a frame ({error})"))),
        }
    }

    pub fn failure(&self, detail: &str) -> MediaError {
        self.process.failure(detail)
    }
}

/// Follows the job's region through its asset's frames, read from FFmpeg at
/// the asset's own frame rate and no larger than [`ANALYSIS_SIDE`].
fn track_asset(
    job: &TrackJob,
    processes: &Processes,
    control: &Control,
) -> Result<Tracked, MediaError> {
    let asset = &job.asset;
    let rate = asset.frame_rate.unwrap_or_default();
    let first = rate.frame_at(job.from).max(0);
    let end = Time(job.until.0.min(asset.duration.0.max(1)) - 1);
    let frames = (rate.frame_at(end) - first + 1).max(1) as usize;
    let mut luma = Luma::open(asset, first, frames, processes)?;
    if !luma.next(control)? {
        return Err(luma.failure("no frame could be read at the playhead"));
    }
    let mut path = Path::begin(&luma.gray, job.region).map_err(MediaError::Invalid)?;
    let mut read_frames = 1;
    let mut said = 0.0;
    while read_frames < frames {
        if !luma.next(control)? {
            break;
        }
        read_frames += 1;
        if !path.step(&luma.gray) {
            break;
        }
        let done = read_frames as f32 / frames as f32;
        if done - said >= 0.01 {
            said = done;
            control.progress(done);
        }
    }
    Ok(path.finish(read_frames, |index| rate.frame_time(first + index as i64)))
}

/// A frame's luma, a byte a pixel.
#[derive(Clone, Debug)]
pub(crate) struct Gray {
    pub width: usize,
    pub height: usize,
    pub luma: Vec<u8>,
}

impl Gray {
    fn at(&self, x: usize, y: usize) -> f32 {
        f32::from(self.luma[y * self.width + x])
    }

    // Luma at a place between pixels, held at the frame's edge.
    fn sample(&self, x: f32, y: f32) -> f32 {
        let x = x.clamp(0.0, (self.width - 1) as f32);
        let y = y.clamp(0.0, (self.height - 1) as f32);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let top = self.at(x0, y0) * (1.0 - fx) + self.at(x1, y0) * fx;
        let bottom = self.at(x0, y1) * (1.0 - fx) + self.at(x1, y1) * fx;
        top * (1.0 - fy) + bottom * fy
    }
}

/// A patch of luma and where it was last found.
struct Matcher {
    template: Vec<f32>,
    size: [usize; 2],
    /// The patch's centre, in pixel indices: pixel `i` spans `i - 0.5` to
    /// `i + 0.5`.
    center: [f32; 2],
    /// How far it went in the last step: where it is looked for next.
    velocity: [f32; 2],
    reach: i32,
}

impl Matcher {
    fn new(frame: &Gray, region: Region) -> Result<Self, String> {
        if frame.width < MIN_PATCH || frame.height < MIN_PATCH {
            return Err("The picture is too small to track a region of".into());
        }
        let region = region.held();
        let extent = [frame.width, frame.height];
        let mut size = [0; 2];
        let mut center = [0.0; 2];
        for axis in 0..2 {
            let side = (region.size[axis] * extent[axis] as f32).round() as usize;
            size[axis] = side.clamp(MIN_PATCH, MAX_PATCH.min(extent[axis]));
            // The whole patch starts inside the frame.
            let half = (size[axis] - 1) as f32 * 0.5;
            center[axis] = (region.center[axis] * extent[axis] as f32 - 0.5)
                .clamp(half, (extent[axis] - 1) as f32 - half);
        }
        let mut matcher = Self {
            template: Vec::new(),
            size,
            center,
            velocity: [0.0; 2],
            reach: (size[0].max(size[1]) as i32 / 2).clamp(MIN_REACH, MAX_REACH),
        };
        matcher.take(frame);
        let count = matcher.template.len() as f32;
        let mean = matcher.template.iter().sum::<f32>() / count;
        let variance = (matcher.template.iter())
            .map(|value| (value - mean) * (value - mean))
            .sum::<f32>()
            / count;
        if variance < MIN_VARIANCE {
            return Err(
                "The region is too even to follow: choose one with an edge or a texture in it"
                    .into(),
            );
        }
        Ok(matcher)
    }

    fn half(&self) -> [f32; 2] {
        self.size.map(|side| (side - 1) as f32 * 0.5)
    }

    // The patch under the centre, read between pixels so that taking it
    // again moves nothing by a rounding.
    fn take(&mut self, frame: &Gray) {
        let [width, height] = self.size;
        let half = self.half();
        let (left, top) = (self.center[0] - half[0], self.center[1] - half[1]);
        self.template = (0..height)
            .flat_map(|row| {
                (0..width).map(move |column| frame.sample(left + column as f32, top + row as f32))
            })
            .collect();
    }

    // How alike the patch and the frame at `(left, top)` are, `-1..=1`,
    // reading every `stride`th texel each way. None where the patch would
    // reach outside the frame.
    fn score(&self, frame: &Gray, left: i32, top: i32, stride: usize) -> Option<f32> {
        let [width, height] = self.size;
        if left < 0
            || top < 0
            || left as usize + width > frame.width
            || top as usize + height > frame.height
        {
            return None;
        }
        let (left, top) = (left as usize, top as usize);
        let (mut st, mut si, mut stt, mut sii, mut sti) = (0f64, 0f64, 0f64, 0f64, 0f64);
        let mut count = 0f64;
        for row in (0..height).step_by(stride) {
            let line = &frame.luma[(top + row) * frame.width + left..][..width];
            let template = &self.template[row * width..][..width];
            for column in (0..width).step_by(stride) {
                let (t, i) = (f64::from(template[column]), f64::from(line[column]));
                st += t;
                si += i;
                stt += t * t;
                sii += i * i;
                sti += t * i;
                count += 1.0;
            }
        }
        let spread = (count * stt - st * st) * (count * sii - si * si);
        if spread <= 1e-6 {
            // An even stretch is like nothing.
            return Some(0.0);
        }
        Some(((count * sti - st * si) / spread.sqrt()) as f32)
    }

    // The frame under the patch with its corner at `corner`, a place
    // between pixels, with a pixel more all round for its slopes.
    fn window(&self, frame: &Gray, corner: [f32; 2]) -> Vec<f32> {
        let [width, height] = self.size;
        (0..height + 2)
            .flat_map(|row| {
                (0..width + 2).map(move |column| {
                    frame.sample(
                        corner[0] + column as f32 - 1.0,
                        corner[1] + row as f32 - 1.0,
                    )
                })
            })
            .collect()
    }

    // Mean and spread of a patch.
    fn level(values: impl Iterator<Item = f32> + Clone) -> (f32, f32) {
        let count = values.clone().count().max(1) as f32;
        let mean = values.clone().sum::<f32>() / count;
        let spread = (values.map(|v| (v - mean) * (v - mean)).sum::<f32>() / count).sqrt();
        (mean, spread)
    }

    // How alike the patch and the frame are with the patch's corner at a
    // place between pixels.
    fn score_between(&self, frame: &Gray, corner: [f32; 2]) -> f32 {
        let [width, height] = self.size;
        let window = self.window(frame, corner);
        let inner = |row: usize, column: usize| window[(row + 1) * (width + 2) + column + 1];
        let cells = (0..height).flat_map(|row| (0..width).map(move |column| (row, column)));
        let (mean_t, spread_t) = Self::level(self.template.iter().copied());
        let (mean_i, spread_i) = Self::level(cells.clone().map(|(row, column)| inner(row, column)));
        if spread_t <= 1e-3 || spread_i <= 1e-3 {
            return 0.0;
        }
        let sum: f32 = cells
            .map(|(row, column)| {
                (self.template[row * width + column] - mean_t) * (inner(row, column) - mean_i)
            })
            .sum();
        sum / (self.template.len() as f32 * spread_t * spread_i)
    }

    // The corner between pixels at which the frame is most like the patch,
    // from the whole-pixel corner the search found: a few steps of least
    // squares on the patch's luma against the frame's and its slopes, each
    // brought to the same level and spread first. A step that would go
    // more than a pixel and a half from where it began is not believed.
    fn refined(&self, frame: &Gray, from: [f32; 2]) -> [f32; 2] {
        let [width, height] = self.size;
        let (mean_t, spread_t) = Self::level(self.template.iter().copied());
        if spread_t <= 1e-3 {
            return from;
        }
        let mut corner = from;
        for _ in 0..8 {
            let window = self.window(frame, corner);
            let at = |row: usize, column: usize| window[row * (width + 2) + column];
            let cells = (0..height).flat_map(|row| (0..width).map(move |column| (row, column)));
            let (mean_i, spread_i) =
                Self::level(cells.clone().map(|(row, column)| at(row + 1, column + 1)));
            if spread_i <= 1e-3 {
                return from;
            }
            let (mut xx, mut xy, mut yy, mut xr, mut yr) = (0f64, 0f64, 0f64, 0f64, 0f64);
            for (row, column) in cells {
                let across = (at(row + 1, column + 2) - at(row + 1, column)) * 0.5 / spread_i;
                let down = (at(row + 2, column + 1) - at(row, column + 1)) * 0.5 / spread_i;
                let left = (self.template[row * width + column] - mean_t) / spread_t
                    - (at(row + 1, column + 1) - mean_i) / spread_i;
                let (across, down, left) = (f64::from(across), f64::from(down), f64::from(left));
                xx += across * across;
                xy += across * down;
                yy += down * down;
                xr += across * left;
                yr += down * left;
            }
            let det = xx * yy - xy * xy;
            if det.abs() <= 1e-9 {
                break;
            }
            // What the patch has over the frame here is the frame's slope
            // times how far there is still to go.
            let step = [
                ((yy * xr - xy * yr) / det) as f32,
                ((xx * yr - xy * xr) / det) as f32,
            ];
            corner = [corner[0] + step[0], corner[1] + step[1]];
            if (corner[0] - from[0]).abs() > 1.5 || (corner[1] - from[1]).abs() > 1.5 {
                return from;
            }
            if step[0].abs() < 0.005 && step[1].abs() < 0.005 {
                break;
            }
        }
        corner
    }

    /// Finds the patch in `frame` and moves to it. Hands back how alike the
    /// place is; under [`LOST_BELOW`] the matcher has not moved.
    fn step(&mut self, frame: &Gray) -> f32 {
        let half = self.half();
        let expected = [
            self.center[0] + self.velocity[0],
            self.center[1] + self.velocity[1],
        ];
        let origin = [
            (expected[0] - half[0]).round() as i32,
            (expected[1] - half[1]).round() as i32,
        ];
        // A large patch is found coarsely first, on every other place and
        // a thinned patch, then exactly around the best of those.
        let texels = self.size[0] * self.size[1];
        let stride = ((texels as f32 / COARSE_TEXELS as f32).sqrt().ceil() as usize).clamp(1, 4);
        let pace = if stride > 1 { 2 } else { 1 };
        let mut best: Option<(f32, i32, i32)> = None;
        let mut consider = |score: Option<f32>, x: i32, y: i32| {
            if let Some(score) = score
                && best.is_none_or(|(held, _, _)| score > held)
            {
                best = Some((score, x, y));
            }
        };
        for dy in (-self.reach..=self.reach).step_by(pace) {
            for dx in (-self.reach..=self.reach).step_by(pace) {
                let (x, y) = (origin[0] + dx, origin[1] + dy);
                consider(self.score(frame, x, y, stride), x, y);
            }
        }
        if stride > 1
            && let Some((_, x, y)) = best
        {
            best = None;
            let mut consider = |score: Option<f32>, x: i32, y: i32| {
                if let Some(score) = score
                    && best.is_none_or(|(held, _, _)| score > held)
                {
                    best = Some((score, x, y));
                }
            };
            for dy in -2..=2 {
                for dx in -2..=2 {
                    consider(self.score(frame, x + dx, y + dy, 1), x + dx, y + dy);
                }
            }
        }
        let Some((_, x, y)) = best else {
            return 0.0;
        };
        // Between pixels, and how alike the patch is there.
        let corner = self.refined(frame, [x as f32, y as f32]);
        let score = self.score_between(frame, corner);
        if score < LOST_BELOW {
            return score;
        }
        let found = [corner[0] + half[0], corner[1] + half[1]];
        self.velocity = [found[0] - self.center[0], found[1] - self.center[1]];
        self.center = found;
        if score < REFRESH_BELOW {
            self.take(frame);
        }
        score
    }
}

/// A region followed from the frame it was chosen on.
pub(crate) struct Path {
    matcher: Matcher,
    extent: [f32; 2],
    /// The centre in each frame read so far, as fractions of the frame.
    centers: Vec<[f32; 2]>,
    lost: bool,
    confidence: f32,
}

impl Path {
    pub fn begin(frame: &Gray, region: Region) -> Result<Self, String> {
        let matcher = Matcher::new(frame, region)?;
        let mut path = Self {
            matcher,
            extent: [frame.width as f32, frame.height as f32],
            centers: Vec::new(),
            lost: false,
            confidence: 1.0,
        };
        path.push();
        Ok(path)
    }

    fn push(&mut self) {
        let center = self.matcher.center;
        self.centers.push([
            (center[0] + 0.5) / self.extent[0],
            (center[1] + 0.5) / self.extent[1],
        ]);
    }

    /// Follows the region into the next frame. False when it is lost there:
    /// the path has ended on the frame before.
    pub fn step(&mut self, frame: &Gray) -> bool {
        if self.lost {
            return false;
        }
        let score = self.matcher.step(frame);
        if score < LOST_BELOW {
            self.lost = true;
            return false;
        }
        self.confidence = self.confidence.min(score);
        self.push();
        true
    }

    /// The path as keys, with `time` giving the source time of each frame
    /// read, counted from the first.
    pub fn finish(self, frames: usize, time: impl Fn(usize) -> Time) -> Tracked {
        let keys: Vec<TrackKey> = (self.centers.iter().enumerate())
            .map(|(index, center)| TrackKey {
                source: time(index),
                center: *center,
                scale: 1.0,
                rotation: 0.0,
            })
            .collect();
        let lost_at = self.lost.then(|| time(keys.len()));
        Tracked {
            keys: thinned(keys, THIN_TOLERANCE, MAX_TRACK_KEYS),
            frames,
            lost_at,
            confidence: self.confidence,
        }
    }
}

/// `keys` without those a straight line between the ones kept already says
/// within `tolerance`, and then evenly fewer when more than `most` are left.
/// The first and the last are always kept, and keys whose time does not
/// come after the one before are dropped.
pub(crate) fn thinned(keys: Vec<TrackKey>, tolerance: f32, most: usize) -> Vec<TrackKey> {
    let mut ordered: Vec<TrackKey> = Vec::with_capacity(keys.len());
    for key in keys {
        if ordered.last().is_none_or(|last| last.source < key.source) {
            ordered.push(key);
        }
    }
    if ordered.len() <= 2 {
        return ordered;
    }
    let says = |from: &TrackKey, to: &TrackKey, key: &TrackKey| {
        let t = (key.source.0 - from.source.0) as f32 / (to.source.0 - from.source.0) as f32;
        (0..2).all(|axis| {
            let line = from.center[axis] + (to.center[axis] - from.center[axis]) * t;
            (line - key.center[axis]).abs() <= tolerance
        })
    };
    // A kept key reaches for the furthest one it can draw a line to, no
    // further than 64 on, so a pass stays linear in the path's length; the
    // passes go on while they still drop keys.
    let mut kept = ordered;
    loop {
        let before = kept.len();
        let mut next_kept = vec![kept[0]];
        let mut anchor = 0;
        while anchor + 1 < kept.len() {
            let mut next = anchor + 1;
            let furthest = (anchor + 64).min(kept.len() - 1);
            for candidate in anchor + 2..=furthest {
                let between = &kept[anchor + 1..candidate];
                if (between.iter()).all(|key| says(&kept[anchor], &kept[candidate], key)) {
                    next = candidate;
                } else {
                    break;
                }
            }
            next_kept.push(kept[next]);
            anchor = next;
        }
        kept = next_kept;
        if kept.len() == before || kept.len() <= 2 {
            break;
        }
    }
    let most = most.max(2);
    if kept.len() <= most {
        return kept;
    }
    let last = kept.len() - 1;
    (0..most)
        .map(|index| kept[index * last / (most - 1)])
        .collect()
}

#[cfg(test)]
mod tests;
