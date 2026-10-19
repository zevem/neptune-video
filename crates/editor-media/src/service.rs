use crate::{
    MediaError, MediaEvent, PreviewConsumer, Wake, decode::DecoderPool, frame_bytes,
    import::import_paths, io, lut, process::Processes,
};
use editor_core::{Asset, AssetId, MediaKind, Time};
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Lane {
    Legacy { asset_id: AssetId, size: [u32; 2] },
    Consumer(PreviewConsumer),
}
impl Lane {
    pub(crate) fn protected_by(self, protected: &[Option<PreviewConsumer>; 3]) -> bool {
        match self {
            Self::Consumer(consumer) => protected.contains(&Some(consumer)),
            Self::Legacy { .. } => false,
        }
    }
}

fn normalized_retention(protected: [Option<PreviewConsumer>; 3]) -> [Option<PreviewConsumer>; 3] {
    let mut normalized = [None; 3];
    let mut count = 0;
    for consumer in protected.into_iter().flatten() {
        if !normalized.contains(&Some(consumer)) {
            normalized[count] = Some(consumer);
            count += 1;
        }
    }
    normalized.sort_unstable();
    normalized
}

struct PreviewMaintenance {
    retained: Vec<Lane>,
    protected: [Option<PreviewConsumer>; 3],
}
#[derive(Clone)]
pub(crate) struct FrameJob {
    pub generation: u64,
    pub sequence: u64,
    pub asset: Asset,
    pub time: Time,
    pub lane: Lane,
    pub size: [u32; 2],
}
struct ImportJob {
    generation: u64,
    paths: Vec<PathBuf>,
}
/// One lookup table to read, for the asset as it was when asked about.
#[derive(Clone, PartialEq, Eq)]
struct LutJob {
    generation: u64,
    asset_id: AssetId,
    path: PathBuf,
    file_size: u64,
}
/// Tables that may wait to be read, and answers that may wait to be taken.
const MAX_LUT_JOBS: usize = 8;
const MAX_LUT_EVENTS: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq)]
struct FrameIdentity {
    generation: u64,
    sequence: u64,
    lane: Lane,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FramePurpose {
    Interactive,
    Playback,
    Poster,
}

#[derive(Clone)]
struct PendingFrame {
    job: FrameJob,
    purpose: FramePurpose,
}

impl FrameJob {
    fn identity(&self) -> FrameIdentity {
        FrameIdentity {
            generation: self.generation,
            sequence: self.sequence,
            lane: self.lane,
        }
    }
}

struct Queue {
    generation: u64,
    sequence: u64,
    imports: VecDeque<ImportJob>,
    luts: VecDeque<LutJob>,
    frames: BTreeMap<Lane, PendingFrame>,
    pending_order: VecDeque<Lane>,
    interactive_bypasses: u8,
    maintenance: u64,
    preview_retention: [Option<PreviewConsumer>; 3],
    in_flight: Option<FrameIdentity>,
    latest: BTreeMap<Lane, u64>,
    events: VecDeque<MediaEvent>,
    event_bytes: usize,
    stopped: bool,
}
impl Default for Queue {
    fn default() -> Self {
        Self {
            generation: 1,
            sequence: 0,
            imports: VecDeque::new(),
            luts: VecDeque::new(),
            frames: BTreeMap::new(),
            pending_order: VecDeque::new(),
            interactive_bypasses: 0,
            maintenance: 0,
            preview_retention: [None; 3],
            in_flight: None,
            latest: BTreeMap::new(),
            events: VecDeque::new(),
            event_bytes: 0,
            stopped: false,
        }
    }
}

impl Queue {
    #[cfg(test)]
    fn take_frame(&mut self) -> Option<FrameJob> {
        self.take_pending_frame().map(|pending| pending.job)
    }

    fn take_pending_frame(&mut self) -> Option<PendingFrame> {
        let oldest = self.pending_order.front().copied()?;
        let interactive = self.pending_order.iter().copied().find(|lane| {
            self.frames.get(lane).is_some_and(|pending| {
                matches!(
                    pending.purpose,
                    FramePurpose::Interactive | FramePurpose::Playback
                )
            })
        });
        let lane = if self.interactive_bypasses < 2 {
            interactive.unwrap_or(oldest)
        } else {
            oldest
        };
        if lane == oldest {
            self.interactive_bypasses = 0;
        } else {
            self.interactive_bypasses += 1;
        }
        self.pending_order.retain(|pending| *pending != lane);
        if self.pending_order.is_empty() {
            self.interactive_bypasses = 0;
        }
        let pending = self.frames.remove(&lane);
        if let Some(pending) = &pending {
            self.in_flight = Some(pending.job.identity());
        }
        pending
    }

    fn frame_current(&self, job: &FrameJob) -> bool {
        !self.stopped
            && self.generation == job.generation
            && self.latest.get(&job.lane) == Some(&job.sequence)
    }

    fn video_warmup_jobs(&self, job: &FrameJob) -> Vec<PendingFrame> {
        if !self.frame_current(job) {
            return Vec::new();
        }
        self.pending_order
            .iter()
            .filter_map(|lane| self.frames.get(lane))
            .filter(|pending| {
                pending.purpose != FramePurpose::Poster
                    && pending.job.asset.kind == MediaKind::Video
            })
            .take(4)
            .cloned()
            .collect()
    }

    fn prune_latest(&mut self) {
        if self.latest.len() > 64
            && let Some(oldest) = self
                .latest
                .iter()
                .filter(|(lane, _)| {
                    !self.frames.contains_key(*lane)
                        && self.in_flight.is_none_or(|flight| flight.lane != **lane)
                        && !lane.protected_by(&self.preview_retention)
                })
                .min_by_key(|(_, sequence)| **sequence)
                .map(|(lane, _)| *lane)
        {
            self.latest.remove(&oldest);
        }
    }

    fn retained_lanes(&self) -> Vec<Lane> {
        let mut retained: Vec<_> = self.latest.keys().copied().collect();
        // A newly hinted warm pipe can outlive its completed latest tag. This
        // allows retention without recreating demand or result currentness.
        for consumer in self.preview_retention.into_iter().flatten() {
            let lane = Lane::Consumer(consumer);
            if !retained.contains(&lane) {
                retained.push(lane);
            }
        }
        retained
    }

    fn preview_maintenance(&self) -> PreviewMaintenance {
        PreviewMaintenance {
            retained: self.retained_lanes(),
            protected: self.preview_retention,
        }
    }

    fn import_event_capacity(&self) -> bool {
        self.events.len() < 64
            && self
                .events
                .iter()
                .filter(|event| matches!(event, MediaEvent::Imported { .. }))
                .count()
                < 4
    }

    fn lut_event_capacity(&self) -> bool {
        self.events.len() < 64
            && self
                .events
                .iter()
                .filter(|event| matches!(event, MediaEvent::LutLoaded { .. }))
                .count()
                < MAX_LUT_EVENTS
    }

    fn finish_frame(&mut self, job: &FrameJob) {
        if self.in_flight == Some(job.identity()) {
            self.in_flight = None;
        }
    }

    fn frame_event_capacity(&self, bytes: usize) -> bool {
        self.events.len() < 64
            && self
                .event_bytes
                .checked_add(bytes)
                .is_some_and(|total| total <= crate::FRAME_CACHE_BYTES)
    }
}
#[derive(Default)]
struct Shared {
    queue: Mutex<Queue>,
    ready: Condvar,
}

/// Nonblocking caller interface; import and preview decoding own separate workers.
pub struct MediaService {
    shared: Arc<Shared>,
    processes: Processes,
}

impl MediaService {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Result<Self, MediaError> {
        let shared = Arc::new(Shared::default());
        let processes = Processes::default();
        let wake: Wake = Arc::new(wake);
        for (name, imports) in [
            ("neptune-video-import", true),
            ("neptune-video-preview", false),
        ] {
            let (state, children, callback) =
                (Arc::clone(&shared), processes.clone(), Arc::clone(&wake));
            if let Err(error) = thread::Builder::new().name(name.into()).spawn(move || {
                if imports {
                    import_worker(state, children, callback);
                } else {
                    preview_worker(state, children, callback);
                }
            }) {
                if let Ok(mut queue) = shared.queue.lock() {
                    queue.stopped = true;
                }
                shared.ready.notify_all();
                processes.kill_all();
                return Err(io("Cannot start media worker", error));
            }
        }
        Ok(Self { shared, processes })
    }

    pub fn import(&self, paths: Vec<PathBuf>) -> Result<u64, MediaError> {
        if paths.is_empty() {
            return Err(MediaError::Invalid(
                "Select at least one media file or folder".into(),
            ));
        }
        if paths.len() > 10_000 {
            return Err(MediaError::Invalid(
                "Import at most 10,000 paths per batch".into(),
            ));
        }
        let mut queue = self.shared.queue.lock().map_err(|_| MediaError::Closed)?;
        if queue.stopped {
            return Err(MediaError::Closed);
        }
        if queue.imports.len() >= 8 {
            return Err(MediaError::Busy);
        }
        let generation = queue.generation;
        queue.imports.push_back(ImportJob { generation, paths });
        self.shared.ready.notify_all();
        Ok(generation)
    }

    /// Asks for the table of a LUT asset to be read from its file. The
    /// answer is one `MediaEvent::LutLoaded` naming the asset, its path and
    /// its file size as they are now. A table already waiting to be read
    /// for the same is not read twice; `Busy` when eight others wait.
    ///
    /// The import worker reads tables, ahead of any import that waits but
    /// behind the one it is on.
    pub fn request_lut(&self, asset: &Asset) -> Result<u64, MediaError> {
        if asset.kind != MediaKind::Lut {
            return Err(MediaError::Invalid("Only a LUT asset has a table".into()));
        }
        let mut queue = self.shared.queue.lock().map_err(|_| MediaError::Closed)?;
        if queue.stopped {
            return Err(MediaError::Closed);
        }
        let generation = queue.generation;
        let job = LutJob {
            generation,
            asset_id: asset.id,
            path: asset.path.clone(),
            file_size: asset.file_size,
        };
        if queue.luts.contains(&job) {
            return Ok(generation);
        }
        if queue.luts.len() >= MAX_LUT_JOBS {
            return Err(MediaError::Busy);
        }
        queue.luts.push_back(job);
        self.shared.ready.notify_all();
        Ok(generation)
    }

    pub fn request_frame(
        &self,
        asset: &Asset,
        time: Time,
        size: [u32; 2],
    ) -> Result<u64, MediaError> {
        self.request(
            asset,
            time,
            size,
            FramePurpose::Interactive,
            Lane::Legacy {
                asset_id: asset.id,
                size,
            },
        )
    }

    /// Declares library, timeline or inspector poster demand without reserving
    /// capacity. It coalesces with the same asset/size lane as `request_frame`.
    pub fn request_poster(
        &self,
        asset: &Asset,
        time: Time,
        size: [u32; 2],
    ) -> Result<u64, MediaError> {
        self.request(
            asset,
            time,
            size,
            FramePurpose::Poster,
            Lane::Legacy {
                asset_id: asset.id,
                size,
            },
        )
    }

    /// Requests the latest picture for one owner, superseding that owner's
    /// earlier time, bounds or source without affecting another consumer.
    pub fn request_preview(
        &self,
        consumer: PreviewConsumer,
        asset: &Asset,
        time: Time,
        size: [u32; 2],
    ) -> Result<u64, MediaError> {
        self.request(
            asset,
            time,
            size,
            FramePurpose::Interactive,
            Lane::Consumer(consumer),
        )
    }

    /// Requests advancing playback without replacing this owner's unfinished
    /// picture. Returns `Busy` while that owner has pending or in-flight work,
    /// leaving its admitted request unchanged. Use `request_preview` for seeks
    /// and source or bounds changes, which supersede immediately.
    /// Retirement invalidates old publications, but playback reuse may remain
    /// busy until the retired owner's in-flight job finishes.
    pub fn request_playback_preview(
        &self,
        consumer: PreviewConsumer,
        asset: &Asset,
        time: Time,
        size: [u32; 2],
    ) -> Result<u64, MediaError> {
        self.request(
            asset,
            time,
            size,
            FramePurpose::Playback,
            Lane::Consumer(consumer),
        )
    }

    /// Replaces an optional protected subset of at most three preview owners.
    /// The worker retains five video pipes and evicts unprotected pipes first.
    /// Empty hints use ordinary LRU; hints neither request nor authorize frames.
    /// Callers must replace or clear the subset when the desired owners change.
    pub fn set_preview_retention(
        &self,
        protected: [Option<PreviewConsumer>; 3],
    ) -> Result<(), MediaError> {
        let protected = normalized_retention(protected);
        let mut queue = self.shared.queue.lock().map_err(|_| MediaError::Closed)?;
        if queue.stopped {
            return Err(MediaError::Closed);
        }
        if queue.preview_retention != protected {
            queue.preview_retention = protected;
            queue.maintenance = queue.maintenance.wrapping_add(1);
            self.shared.ready.notify_all();
        }
        Ok(())
    }

    /// Releases this consumer's demand and unpolled pictures without waiting
    /// for I/O. The preview worker reclaims owned decoder state after waking.
    pub fn retire_preview(&self, consumer: PreviewConsumer) {
        if let Ok(mut queue) = self.shared.queue.lock() {
            let lane = Lane::Consumer(consumer);
            queue.frames.remove(&lane);
            queue.latest.remove(&lane);
            for protected in &mut queue.preview_retention {
                if *protected == Some(consumer) {
                    *protected = None;
                }
            }
            queue.preview_retention = normalized_retention(queue.preview_retention);
            queue.pending_order.retain(|pending| *pending != lane);
            if queue.pending_order.is_empty() {
                queue.interactive_bypasses = 0;
            }
            queue
                .events
                .retain(|event| !event_matches_lane(event, lane));
            queue.event_bytes = queue.events.iter().map(event_bytes).sum();
            queue.maintenance = queue.maintenance.wrapping_add(1);
        }
        self.shared.ready.notify_all();
    }

    fn request(
        &self,
        asset: &Asset,
        time: Time,
        size: [u32; 2],
        purpose: FramePurpose,
        lane: Lane,
    ) -> Result<u64, MediaError> {
        frame_bytes(size[0], size[1])?;
        if asset.kind == MediaKind::Audio {
            return Err(MediaError::Invalid(
                "Audio assets have no video frame".into(),
            ));
        }
        if asset.kind == MediaKind::Lut {
            return Err(MediaError::Invalid("LUT assets have no video frame".into()));
        }
        let mut queue = self.shared.queue.lock().map_err(|_| MediaError::Closed)?;
        if queue.stopped {
            return Err(MediaError::Closed);
        }
        if purpose == FramePurpose::Playback
            && (queue.frames.contains_key(&lane)
                || queue.in_flight.is_some_and(|flight| flight.lane == lane))
        {
            return Err(MediaError::Busy);
        }
        if queue.frames.len() >= 32 && !queue.frames.contains_key(&lane) {
            return Err(MediaError::Busy);
        }
        queue.sequence = queue.sequence.wrapping_add(1);
        let sequence = queue.sequence;
        let generation = queue.generation;
        queue.latest.insert(lane, sequence);
        // Replacement keeps first admission, independent of sequence wrap and
        // caller purpose changes. There are at most 32 pending positions.
        if !queue.frames.contains_key(&lane) {
            queue.pending_order.push_back(lane);
        }
        queue.frames.insert(
            lane,
            PendingFrame {
                purpose,
                job: FrameJob {
                    generation,
                    sequence,
                    asset: asset.clone(),
                    time: Time(time.0.max(0)),
                    lane,
                    size,
                },
            },
        );
        // At most 32 pending lanes, one worker-owned lane and three hinted
        // consumers are protected;
        // the 64-entry history still has evictable completed entries.
        queue.prune_latest();
        self.shared.ready.notify_all();
        Ok(generation)
    }

    pub fn poll(&self) -> Option<MediaEvent> {
        let mut queue = self.shared.queue.lock().ok()?;
        let event = queue.events.pop_front()?;
        queue.event_bytes = queue.event_bytes.saturating_sub(event_bytes(&event));
        self.shared.ready.notify_all();
        Some(event)
    }

    pub fn generation(&self) -> u64 {
        self.shared
            .queue
            .lock()
            .map(|queue| queue.generation)
            .unwrap_or(0)
    }

    /// Invalidates jobs and results when switching projects. It does not wait for I/O.
    pub fn cancel(&self) -> u64 {
        let generation = if let Ok(mut queue) = self.shared.queue.lock() {
            queue.generation = queue.generation.wrapping_add(1);
            queue.imports.clear();
            queue.luts.clear();
            queue.frames.clear();
            queue.pending_order.clear();
            queue.interactive_bypasses = 0;
            queue.in_flight = None;
            queue.latest.clear();
            queue.preview_retention = [None; 3];
            queue.events.clear();
            queue.event_bytes = 0;
            queue.generation
        } else {
            0
        };
        self.shared.ready.notify_all();
        self.processes.kill_all();
        generation
    }
}

impl Drop for MediaService {
    fn drop(&mut self) {
        if let Ok(mut queue) = self.shared.queue.lock() {
            queue.stopped = true;
        }
        self.shared.ready.notify_all();
        self.processes.kill_all();
        // Workers own cleanup. Joining here would freeze a closing native window.
    }
}

fn current(shared: &Shared, generation: u64) -> bool {
    shared
        .queue
        .lock()
        .is_ok_and(|queue| !queue.stopped && queue.generation == generation)
}

fn event_bytes(event: &MediaEvent) -> usize {
    if let MediaEvent::FrameReady { frame, .. } = event {
        frame.byte_len()
    } else {
        0
    }
}

fn event_matches_lane(event: &MediaEvent, lane: Lane) -> bool {
    if let MediaEvent::Error {
        consumer: Some(consumer),
        ..
    } = event
    {
        return lane == Lane::Consumer(*consumer);
    }
    let MediaEvent::FrameReady {
        asset_id,
        size,
        consumer,
        ..
    } = event
    else {
        return false;
    };
    match lane {
        Lane::Legacy {
            asset_id: id,
            size: bounds,
        } => consumer.is_none() && *asset_id == id && *size == bounds,
        Lane::Consumer(id) => *consumer == Some(id),
    }
}

fn import_worker(shared: Arc<Shared>, processes: Processes, wake: Wake) {
    loop {
        let job = {
            let Ok(mut queue) = shared.queue.lock() else {
                return;
            };
            while queue.imports.is_empty() && queue.luts.is_empty() && !queue.stopped {
                let Ok(next) = shared.ready.wait(queue) else {
                    return;
                };
                queue = next;
            }
            if queue.stopped {
                return;
            }
            // A table is one small file; an import may be thousands.
            if let Some(table) = queue.luts.pop_front() {
                drop(queue);
                match publish_lut(&shared, table) {
                    Ok(true) => wake(),
                    Ok(false) => {}
                    Err(()) => return,
                }
                continue;
            }
            queue.imports.pop_front()
        };
        let Some(job) = job else {
            continue;
        };
        let result = import_paths(job.paths, &processes, || !current(&shared, job.generation));
        let Ok(mut queue) = shared.queue.lock() else {
            return;
        };
        while !queue.import_event_capacity() && !queue.stopped && queue.generation == job.generation
        {
            let Ok(next) = shared.ready.wait(queue) else {
                return;
            };
            queue = next;
        }
        if queue.stopped {
            return;
        }
        if queue.generation != job.generation {
            continue;
        }
        queue.events.push_back(MediaEvent::Imported {
            generation: job.generation,
            assets: result.assets,
            folders: result.folders,
            errors: result.errors,
        });
        drop(queue);
        wake();
    }
}

/// Reads the table of `job` and publishes the answer, waiting for room for
/// it. `Ok(false)` when the worker was started over meanwhile, and `Err`
/// when it is to stop.
fn publish_lut(shared: &Shared, job: LutJob) -> Result<bool, ()> {
    let table = lut::read(&job.path)
        .map(Arc::new)
        .map_err(|error| error.to_string());
    let mut queue = shared.queue.lock().map_err(drop)?;
    while !queue.lut_event_capacity() && !queue.stopped && queue.generation == job.generation {
        queue = shared.ready.wait(queue).map_err(drop)?;
    }
    if queue.stopped {
        return Err(());
    }
    if queue.generation != job.generation {
        return Ok(false);
    }
    queue.events.push_back(MediaEvent::LutLoaded {
        generation: job.generation,
        asset_id: job.asset_id,
        path: job.path,
        file_size: job.file_size,
        table,
    });
    Ok(true)
}

fn preview_worker(shared: Arc<Shared>, processes: Processes, wake: Wake) {
    let mut decoders = DecoderPool::new(processes);
    let mut generation = 0;
    let mut maintenance = 0;
    loop {
        let (next_generation, next_maintenance, retained, job) = {
            let Ok(mut queue) = shared.queue.lock() else {
                return;
            };
            while queue.frames.is_empty()
                && !queue.stopped
                && queue.generation == generation
                && queue.maintenance == maintenance
            {
                let Ok(next) = shared.ready.wait(queue) else {
                    return;
                };
                queue = next;
            }
            if queue.stopped {
                return;
            }
            // Latest source time within each lane; bounded interactive preference
            // across lanes cannot repeatedly displace the global oldest.
            let retained = (queue.generation != generation || queue.maintenance != maintenance)
                .then(|| queue.preview_maintenance());
            (
                queue.generation,
                queue.maintenance,
                retained,
                queue.take_pending_frame(),
            )
        };
        if generation != next_generation {
            decoders.clear();
            generation = next_generation;
        }
        if let Some(retained) = retained {
            // Dropping decoder processes can wait; never do it under the
            // caller's queue mutex or on the native UI thread.
            decoders.set_preview_retention(retained.protected);
            decoders.retain_lanes(&retained.retained);
        }
        maintenance = next_maintenance;
        let Some(PendingFrame { job, purpose }) = job else {
            continue;
        };
        let result = prepare_video_cohort(&shared, &mut decoders, &job, purpose, maintenance)
            .and_then(|()| {
                let current = || {
                    shared
                        .queue
                        .lock()
                        .is_ok_and(|queue| queue.frame_current(&job))
                };
                if purpose == FramePurpose::Playback {
                    decoders.playback_frame(&job, current)
                } else {
                    decoders.frame(&job, current)
                }
            });
        let read_ahead =
            purpose == FramePurpose::Playback && result.is_ok() && decoders.has_read_ahead(&job);
        match publish_result_with_admission(
            &shared,
            &job,
            result,
            || {},
            &mut maintenance,
            |snapshot| {
                decoders.set_preview_retention(snapshot.protected);
                decoders.retain_lanes(&snapshot.retained);
            },
            !read_ahead,
        ) {
            Ok(true) => {
                wake();
                if read_ahead {
                    let deadline = Instant::now() + Duration::from_millis(4);
                    decoders.read_ahead(&job, deadline, || {
                        read_ahead_current(&shared, &job, maintenance)
                    });
                    let Ok(mut queue) = shared.queue.lock() else {
                        return;
                    };
                    queue.finish_frame(&job);
                    shared.ready.notify_all();
                }
                if purpose != FramePurpose::Poster {
                    // A walk backwards has the run before it decoded while
                    // this one is shown. The picture asked for is published;
                    // this starts a thread at most and reads no pixels here.
                    decoders.look_behind(&job, || {
                        shared.queue.lock().is_ok_and(|queue| {
                            queue.frame_current(&job) && queue.maintenance == maintenance
                        })
                    });
                }
            }
            Ok(false) => {}
            Err(_) => return,
        }
    }
}

fn prepare_video_cohort(
    shared: &Shared,
    decoders: &mut DecoderPool,
    job: &FrameJob,
    purpose: FramePurpose,
    maintenance: u64,
) -> Result<(), MediaError> {
    if purpose == FramePurpose::Poster || job.asset.kind != MediaKind::Video {
        return Ok(());
    }
    let current = || {
        shared
            .queue
            .lock()
            .is_ok_and(|queue| queue.frame_current(job) && queue.maintenance == maintenance)
    };
    let prepared = if purpose == FramePurpose::Playback {
        decoders.prepare_playback_video(job, current)
    } else {
        decoders.prepare_video(job, current)
    };
    match prepared {
        Ok(_) => {}
        // Preparation is optional. An unrelated maintenance change must not
        // discard a still-current finite paused request; ordinary decode below
        // independently checks that request's identity before publication.
        Err(MediaError::Cancelled) => return Ok(()),
        Err(error) => return Err(error),
    }
    // The first process starts before this bounded snapshot. Other owners can
    // finish admission during its spawn and overlap their cold codec startup
    // before the worker performs any blocking requested pixel read.
    let (sequence, jobs) = {
        let queue = shared.queue.lock().map_err(|_| MediaError::Closed)?;
        if !queue.frame_current(job) || queue.maintenance != maintenance {
            return Ok(());
        }
        (queue.sequence, queue.video_warmup_jobs(job))
    };
    for pending in &jobs {
        let current = || video_warmup_current(shared, job, &jobs, sequence, maintenance);
        let prepared = if pending.purpose == FramePurpose::Playback {
            decoders.prepare_playback_video(&pending.job, current)
        } else {
            decoders.prepare_video(&pending.job, current)
        };
        if prepared.is_err() {
            // The pending request remains admitted and will receive its own
            // ordinary result. Preparation neither dispatches nor publishes it.
            break;
        }
    }
    Ok(())
}

fn video_warmup_current(
    shared: &Shared,
    job: &FrameJob,
    pending: &[PendingFrame],
    sequence: u64,
    maintenance: u64,
) -> bool {
    shared.queue.lock().is_ok_and(|queue| {
        queue.sequence == sequence
            && queue.maintenance == maintenance
            && queue.frame_current(job)
            && pending
                .iter()
                .all(|pending| queue.frame_current(&pending.job))
    })
}

fn read_ahead_current(shared: &Shared, job: &FrameJob, maintenance: u64) -> bool {
    shared.queue.lock().is_ok_and(|queue| {
        queue.frame_current(job)
            && queue.maintenance == maintenance
            // Requested pictures outrank speculative future cache entries,
            // including another playback owner waiting for its first picture.
            && queue.frames.is_empty()
    })
}

#[cfg(test)]
fn publish_result(
    shared: &Shared,
    job: &FrameJob,
    result: Result<crate::Frame, MediaError>,
) -> Result<bool, MediaError> {
    publish_result_before_wait(shared, job, result, || {})
}

#[cfg(test)]
fn publish_result_before_wait(
    shared: &Shared,
    job: &FrameJob,
    result: Result<crate::Frame, MediaError>,
    before_wait: impl FnMut(),
) -> Result<bool, MediaError> {
    publish_result_before_wait_with_maintenance(shared, job, result, before_wait, &mut 0, |_| {})
}

#[cfg(test)]
fn publish_result_before_wait_with_maintenance(
    shared: &Shared,
    job: &FrameJob,
    result: Result<crate::Frame, MediaError>,
    before_wait: impl FnMut(),
    maintenance: &mut u64,
    reclaim: impl FnMut(&PreviewMaintenance),
) -> Result<bool, MediaError> {
    publish_result_with_admission(shared, job, result, before_wait, maintenance, reclaim, true)
}

fn publish_result_with_admission(
    shared: &Shared,
    job: &FrameJob,
    result: Result<crate::Frame, MediaError>,
    mut before_wait: impl FnMut(),
    maintenance: &mut u64,
    mut reclaim: impl FnMut(&PreviewMaintenance),
    finish_on_success: bool,
) -> Result<bool, MediaError> {
    let mut queue = shared.queue.lock().map_err(|_| MediaError::Closed)?;
    if queue.stopped {
        queue.finish_frame(job);
        return Err(MediaError::Closed);
    }
    if !queue.frame_current(job) {
        queue.finish_frame(job);
        return Ok(false);
    }
    let event = match result {
        Ok(frame) => MediaEvent::FrameReady {
            generation: job.generation,
            asset_id: job.asset.id,
            time: job.time,
            size: job.size,
            consumer: match job.lane {
                Lane::Legacy { .. } => None,
                Lane::Consumer(id) => Some(id),
            },
            frame,
        },
        Err(MediaError::Cancelled) => {
            queue.finish_frame(job);
            return Ok(false);
        }
        Err(error) => MediaEvent::Error {
            generation: job.generation,
            asset_id: Some(job.asset.id),
            consumer: match job.lane {
                Lane::Legacy { .. } => None,
                Lane::Consumer(id) => Some(id),
            },
            size: Some(job.size),
            message: error.to_string(),
        },
    };
    // Only the same lane's obsolete result can be superseded. Other current
    // outcomes remain until the consumer polls or the generation is cancelled.
    let old_count = queue.events.len();
    queue
        .events
        .retain(|event| !event_matches_lane(event, job.lane));
    queue.event_bytes = queue.events.iter().map(event_bytes).sum();
    if queue.events.len() != old_count {
        // A count-blocked importer can now use this slot even if this frame
        // remains blocked by the byte budget. No user callback runs here.
        shared.ready.notify_all();
    }
    let bytes = event_bytes(&event);
    loop {
        if queue.stopped {
            queue.finish_frame(job);
            return Err(MediaError::Closed);
        }
        if !queue.frame_current(job) {
            queue.finish_frame(job);
            return Ok(false);
        }
        if queue.maintenance != *maintenance {
            let next_maintenance = queue.maintenance;
            let retained = queue.preview_maintenance();
            // Keep this result while reclaiming unrelated retired pipes. A
            // full consumer mailbox must not prevent retirement, and process
            // teardown must not run under the caller's queue mutex.
            drop(queue);
            reclaim(&retained);
            *maintenance = next_maintenance;
            queue = shared.queue.lock().map_err(|_| MediaError::Closed)?;
            continue;
        }
        if queue.frame_event_capacity(bytes) {
            break;
        }
        // One held result is separate from the bounded mailbox/cache. Wait
        // releases the mutex; poll, supersession, cancel and close all notify.
        before_wait();
        queue = shared.ready.wait(queue).map_err(|_| MediaError::Closed)?;
    }
    let frame_ready = matches!(&event, MediaEvent::FrameReady { .. });
    queue.event_bytes += bytes;
    queue.events.push_back(event);
    if finish_on_success || !frame_ready {
        queue.finish_frame(job);
    }
    Ok(true)
}

#[cfg(test)]
mod delivery_tests;

#[cfg(test)]
mod dispatch_tests;

#[cfg(test)]
mod consumer_tests;

#[cfg(test)]
mod playback_tests;

#[cfg(test)]
mod prefetch_tests;

#[cfg(test)]
mod warmup_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    fn wait(service: &MediaService) -> MediaEvent {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(event) = service.poll() {
                return event;
            }
            assert!(Instant::now() < until, "media worker timed out");
            thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn native_image_import_and_preview_work_without_external_tools() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("red.png");
        image::RgbaImage::from_pixel(32, 16, image::Rgba([255, 0, 0, 255]))
            .save(&path)
            .unwrap();
        let service = MediaService::new(|| {}).unwrap();
        service.import(vec![path]).unwrap();
        let MediaEvent::Imported { assets, .. } = wait(&service) else {
            panic!("expected import");
        };
        service
            .request_frame(&assets[0], Time::ZERO, [16, 16])
            .unwrap();
        let MediaEvent::FrameReady { frame, .. } = wait(&service) else {
            panic!("expected preview");
        };
        assert_eq!((frame.width, frame.height), (16, 8));
        assert_eq!(&frame.rgba[..4], &[255, 0, 0, 255]);
        let old = service.generation();
        assert_ne!(service.cancel(), old);
        assert!(service.poll().is_none());
    }

    #[test]
    fn a_cube_file_is_imported_and_its_table_read_by_the_worker() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("swap.cube");
        std::fs::write(&path, lut::tests::text(4, |[r, g, b]| [b, g, r])).unwrap();
        let service = MediaService::new(|| {}).unwrap();
        service.import(vec![path.clone()]).unwrap();
        let MediaEvent::Imported { assets, errors, .. } = wait(&service) else {
            panic!("expected import");
        };
        assert!(errors.is_empty());
        let asset = assets[0].clone();
        assert_eq!(asset.kind, MediaKind::Lut);
        assert_eq!(asset.width, 4);
        // A table is not a picture, and a picture has no table.
        assert!(service.request_frame(&asset, Time::ZERO, [16, 16]).is_err());
        assert!(
            service
                .request_poster(&asset, Time::ZERO, [16, 16])
                .is_err()
        );
        let still = Asset::new("still.png".into(), MediaKind::Image, Time(1));
        assert!(service.request_lut(&still).is_err());

        let generation = service.request_lut(&asset).unwrap();
        let MediaEvent::LutLoaded {
            generation: from,
            asset_id,
            path: read,
            file_size,
            table,
        } = wait(&service)
        else {
            panic!("expected a table");
        };
        assert_eq!((from, asset_id), (generation, asset.id));
        assert_eq!((&read, file_size), (&asset.path, asset.file_size));
        let table = table.unwrap();
        assert_eq!(table.size, 4);
        assert_eq!(table.at(3, 0, 0), [0.0, 0.0, 1.0]);

        // A file that is gone, or no table any more, is answered with why.
        std::fs::write(&path, "LUT_3D_SIZE 2\n0 0 0\n").unwrap();
        service.request_lut(&asset).unwrap();
        let MediaEvent::LutLoaded { table, .. } = wait(&service) else {
            panic!("expected an answer");
        };
        assert!(table.unwrap_err().contains("fewer rows"));
        std::fs::remove_file(&path).unwrap();
        service.request_lut(&asset).unwrap();
        let MediaEvent::LutLoaded { table, .. } = wait(&service) else {
            panic!("expected an answer");
        };
        assert!(table.is_err());
    }

    #[test]
    fn a_table_asked_for_before_the_worker_started_over_is_never_answered() {
        let temp = tempfile::tempdir().unwrap();
        let mut asset = Asset::new(temp.path().join("look.cube"), MediaKind::Lut, Time(1));
        std::fs::write(&asset.path, lut::tests::text(2, |c| c)).unwrap();
        asset.width = 2;
        // No worker takes the queue here, so the order is the test's own.
        let shared = Shared::default();
        let job = |generation| LutJob {
            generation,
            asset_id: asset.id,
            path: asset.path.clone(),
            file_size: asset.file_size,
        };
        assert_eq!(publish_lut(&shared, job(0)), Ok(false));
        assert!(shared.queue.lock().unwrap().events.is_empty());
        assert_eq!(publish_lut(&shared, job(1)), Ok(true));
        assert_eq!(shared.queue.lock().unwrap().events.len(), 1);

        // The service drops what waits and what was answered alike.
        let service = MediaService::new(|| {}).unwrap();
        for _ in 0..MAX_LUT_JOBS + 4 {
            // Whatever the worker has taken, the queue never grows past
            // its bound: the same table is not queued twice.
            service.request_lut(&asset).unwrap();
        }
        let old = service.generation();
        assert_ne!(service.cancel(), old);
        assert!(service.shared.queue.lock().unwrap().luts.is_empty());
        service.request_lut(&asset).unwrap();
        // One read from before the restart may still have been in hand; its
        // answer is dropped by its generation, never published.
        let MediaEvent::LutLoaded { generation, .. } = wait(&service) else {
            panic!("expected a table");
        };
        assert_eq!(generation, service.generation());
        thread::sleep(Duration::from_millis(50));
        assert!(service.poll().is_none());
    }

    #[test]
    fn native_gif_import_preserves_animation_timing_and_decodes_source_frames() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("animation.gif");
        let mut encoder =
            image::codecs::gif::GifEncoder::new(std::fs::File::create(&path).unwrap());
        encoder
            .set_repeat(image::codecs::gif::Repeat::Infinite)
            .unwrap();
        for color in [[255, 0, 0, 255], [0, 255, 0, 255]] {
            encoder
                .encode_frame(image::Frame::from_parts(
                    image::RgbaImage::from_pixel(4, 4, image::Rgba(color)),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(100, 1),
                ))
                .unwrap();
        }
        drop(encoder);
        let service = MediaService::new(|| {}).unwrap();
        service.import(vec![path]).unwrap();
        let MediaEvent::Imported { assets, errors, .. } = wait(&service) else {
            panic!("expected import");
        };
        assert!(errors.is_empty());
        assert_eq!(assets[0].kind, MediaKind::Gif);
        assert_eq!(assets[0].duration, Time(200_000));
        for (time, color) in [
            (Time::ZERO, [255, 0, 0, 255]),
            (Time(150_000), [0, 255, 0, 255]),
        ] {
            service.request_frame(&assets[0], time, [4, 4]).unwrap();
            let MediaEvent::FrameReady { frame, .. } = wait(&service) else {
                panic!("expected frame");
            };
            assert_eq!(&frame.rgba[..4], &color);
        }
    }

    #[test]
    fn paced_gif_playback_keeps_up_when_latest_requests_arrive_at_project_frame_rate() {
        let _heavy = crate::heavy_test();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("paced.gif");
        let mut encoder =
            image::codecs::gif::GifEncoder::new(std::fs::File::create(&path).unwrap());
        for index in 0..30_u8 {
            encoder
                .encode_frame(image::Frame::from_parts(
                    image::RgbaImage::from_pixel(320, 180, image::Rgba([index, 0, 0, 255])),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(100, 1),
                ))
                .unwrap();
        }
        drop(encoder);
        let service = MediaService::new(|| {}).unwrap();
        service.import(vec![path]).unwrap();
        let MediaEvent::Imported { assets, errors, .. } = wait(&service) else {
            panic!("expected GIF import");
        };
        assert!(errors.is_empty());
        let started = Instant::now();
        let mut shown = 0_u8;
        for index in 0..60_i64 {
            let time = editor_core::FrameRate::default().frame_time(index);
            service.request_frame(&assets[0], time, [320, 180]).unwrap();
            let until = started + Duration::from_secs_f64((index + 1) as f64 / 30.0);
            while Instant::now() < until {
                while let Some(event) = service.poll() {
                    match event {
                        MediaEvent::FrameReady { frame, .. } => shown = shown.max(frame.rgba[0]),
                        MediaEvent::Error { message, .. } => panic!("{message}"),
                        _ => {}
                    }
                }
                thread::sleep(Duration::from_millis(1));
            }
        }
        assert!(
            shown >= 17,
            "after 2 seconds at 10 GIF fps, picture should be near frame19; received frame{shown}"
        );
    }
}
