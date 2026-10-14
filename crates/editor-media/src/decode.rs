use crate::{
    FRAME_CACHE_BYTES, Frame, MediaError, fitted_dimensions, frame_bytes,
    import::{gif_delay_us, image_reader, limits},
    io,
    process::{Process, Processes, tool_command},
    service::{FrameJob, Lane},
};
use editor_core::{Asset, AssetId, FrameRate, MediaKind, Time};
use image::{
    AnimationDecoder, DynamicImage, ImageDecoder, codecs::gif::GifDecoder, imageops::FilterType,
};
use std::{
    collections::VecDeque,
    fs::File,
    io::{BufReader, Read},
    path::PathBuf,
    process::{Child, Command},
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};

/// A checkpoint can reuse an asset UUID for a different source. Compare the
/// complete requested metadata rather than trusting that UUID or a hash alone.
/// Construction is pure; external file I/O remains in the decode worker.
#[derive(Clone, PartialEq, Eq)]
struct Source {
    path: PathBuf,
    kind: MediaKind,
    duration: Time,
    width: u32,
    height: u32,
    frame_rate: Option<FrameRate>,
    file_size: u64,
}

impl From<&Asset> for Source {
    fn from(asset: &Asset) -> Self {
        Self {
            path: asset.path.clone(),
            kind: asset.kind,
            duration: asset.duration,
            width: asset.width,
            height: asset.height,
            frame_rate: asset.frame_rate,
            file_size: asset.file_size,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Key {
    asset: AssetId,
    dimensions: [u32; 2],
    frame: i64,
    source: Source,
}
struct Cached {
    key: Key,
    frame: Frame,
    span: Option<(Time, Time)>,
}

// Account for the iterator's original canvas and its current held preview;
// their aggregate reservation is independent of the global frame cache.
const GIF_STATE_BUDGET: usize = 64 * 1024 * 1024;
// Keep the target five independent streams continuous. Native output threads
// and packet queues are bounded separately; decoded caches retain their 64 MiB
// byte limit regardless of how many pipes are warm.
const VIDEO_DECODER_LIMIT: usize = 5;
// A speculative read has only four milliseconds. Large partial reads can
// exhaust that deadline and destroy a useful sequential pipe; leave those
// streams demand-driven instead of risking their continuity for future pixels.
const READ_AHEAD_FRAME_BYTES: usize = 1024 * 1024;
// A pipe runs forwards only. A request a little behind its lane's pipe is a
// step backwards: the frames leading up to it are decoded in one run and kept
// in the frame cache, so the steps that follow are served from there instead
// of each starting a process. While those are shown the run before them is
// decoded by a second process on its own thread (the look-behind), so that a
// steady walk backwards finds each run already in the cache. The frames of a
// walk not yet shown, read or still to be handed over, are never more than
// half the cache, which leaves the other lanes their pictures: the
// look-behind reads a frame only when one of the run being shown has made
// room for it. A run is at most this many frames, and two short of that
// half, so that the last frames of a run are read a few steps before the
// walk reaches them.
const REVERSE_GROUP_FRAMES: usize = 24;
const REVERSE_GROUP_MARGIN: usize = 2;
// A walk nothing has stepped for this long gives its look-behind to another
// lane that starts to step backwards.
const WALK_IDLE: Duration = Duration::from_secs(1);
struct GifDecoderState {
    lane: Lane,
    dimensions: [u32; 2],
    source: Source,
    frames: image::Frames<'static>,
    next_index: i64,
    next_time: Time,
    reserved_bytes: usize,
    last: Option<(Frame, (Time, Time), i64)>,
}
struct VideoDecoder {
    lane: Lane,
    process: Process,
    next_index: i64,
    dimensions: [u32; 2],
    source: Source,
    unread: bool,
    // The frame a run decoded for a step backwards leads up to: every frame
    // read on the way to it is kept.
    fill: Option<i64>,
}

impl VideoDecoder {
    fn compatible(&self, target: i64, dimensions: [u32; 2], source: &Source) -> bool {
        target >= self.next_index
            && (target - self.next_index <= 12 || self.fill.is_some_and(|until| target <= until))
            && self.dimensions == dimensions
            && self.source == *source
    }

    /// The frame a run for `target` starts at, when `target` is a step back
    /// from where this pipe is: behind it, and no further than two runs.
    fn run_start(&self, target: i64, dimensions: [u32; 2], source: &Source) -> Option<i64> {
        let frames = reverse_group(dimensions)?;
        (self.dimensions == dimensions
            && self.source == *source
            && target < self.next_index
            && self.next_index - target <= 2 * frames + 2)
            .then(|| (target - (frames - 1)).max(0))
    }
}

/// How many frames of a backwards walk may be held ahead of it at these
/// dimensions: half the cache.
fn reverse_room(dimensions: [u32; 2]) -> usize {
    frame_bytes(dimensions[0], dimensions[1])
        .map_or(0, |bytes| FRAME_CACHE_BYTES / bytes.max(1) / 2)
}

/// How many frames a run for a step backwards holds at these dimensions;
/// none when half the cache has no room for a run of two.
fn reverse_group(dimensions: [u32; 2]) -> Option<i64> {
    let frames = reverse_room(dimensions)
        .saturating_sub(REVERSE_GROUP_MARGIN)
        .min(REVERSE_GROUP_FRAMES);
    (frames > 1).then_some(frames as i64)
}

/// Whose pictures a request is for: all of a cache key but the frame, and
/// the lane that asks. Look-behind work is for one of these and for no other.
#[derive(Clone, PartialEq, Eq)]
struct Stream {
    lane: Lane,
    asset: AssetId,
    dimensions: [u32; 2],
    source: Source,
}

impl Stream {
    fn key(&self, frame: i64) -> Key {
        Key {
            asset: self.asset,
            dimensions: self.dimensions,
            frame,
            source: self.source.clone(),
        }
    }

    fn holds(&self, key: &Key) -> bool {
        key.asset == self.asset && key.dimensions == self.dimensions && key.source == self.source
    }
}

/// A lane being stepped backwards through one source: `last` is the frame
/// it asked for last.
struct Walk {
    stream: Stream,
    last: i64,
    at: Instant,
}

/// What a look-behind run has read and the pool has not yet taken.
#[derive(Default)]
struct Staged {
    frames: Vec<(i64, Frame)>,
    /// How many frames may wait here at once: the run reads no further
    /// until the pool has taken some or made room for more.
    budget: usize,
    /// The run has ended and its process is gone.
    done: bool,
    /// The pool gave the run up: nothing more is staged.
    cancelled: bool,
    child: Option<Arc<Mutex<Child>>>,
}
type StagedShared = Arc<(Mutex<Staged>, Condvar)>;

/// The run before the one a backwards walk is being shown, decoded off the
/// frame path: frames `from..=until` of `stream`. Its frames wait in `shared`
/// until the pool takes them into the cache, where room is kept for as many
/// as the run may read before the pool looks again. Dropping it gives the
/// run up: its process is ended and what it staged is never cached.
struct Behind {
    stream: Stream,
    from: i64,
    until: i64,
    shared: StagedShared,
    /// How many of its frames the pool has taken.
    taken: usize,
    /// The bytes of one frame.
    bytes: usize,
    /// Cache bytes kept free for the frames it may stage.
    reserved: usize,
}

impl Behind {
    fn covers(&self, stream: &Stream, frame: i64) -> bool {
        self.stream == *stream && self.from <= frame && frame <= self.until
    }

    fn frames(&self) -> usize {
        (self.until - self.from + 1) as usize
    }

    /// Whether the run has nothing more to hand over: every frame it was
    /// started for was taken, or it ended short of that.
    fn done(&self) -> bool {
        self.taken >= self.frames() || self.shared.0.lock().is_ok_and(|staged| staged.done)
    }
}

impl Drop for Behind {
    fn drop(&mut self) {
        let child = self.shared.0.lock().ok().and_then(|mut staged| {
            staged.cancelled = true;
            staged.frames.clear();
            staged.child.take()
        });
        self.shared.1.notify_all();
        // The run is blocked in a read of its pipe: ending the process ends
        // the read, and the run's own thread reaps it.
        if let Some(child) = child
            && let Ok(mut child) = child.lock()
        {
            let _ = child.kill();
        }
    }
}

/// A look-behind run, on its own thread: starts the process, reads its
/// frames in order and stages each. Nothing here is on the frame path.
fn run_behind(
    mut command: Command,
    processes: Processes,
    shared: StagedShared,
    dimensions: [u32; 2],
    bytes: usize,
    from: i64,
    until: i64,
) {
    let finish = |shared: &StagedShared| {
        if let Ok(mut staged) = shared.0.lock() {
            staged.done = true;
            staged.child = None;
        }
        shared.1.notify_all();
    };
    let Ok(mut process) = Process::spawn("ffmpeg", &mut command, &processes) else {
        finish(&shared);
        return;
    };
    let given_up = shared.0.lock().map_or(true, |mut staged| {
        staged.child = Some(Arc::clone(&process.child));
        staged.cancelled
    });
    if !given_up {
        'run: for index in from..=until {
            // No further than there is room for: the process waits in its
            // pipe meanwhile, with the frame after this one decoded.
            let Ok(mut staged) = shared.0.lock() else {
                break;
            };
            while staged.frames.len() >= staged.budget {
                if staged.cancelled {
                    break 'run;
                }
                let Ok(next) = shared.1.wait(staged) else {
                    break 'run;
                };
                staged = next;
            }
            if staged.cancelled {
                break;
            }
            drop(staged);
            let mut rgba = vec![0; bytes];
            process.begin_read();
            let read = process.stdout.read_exact(&mut rgba);
            process.end_read();
            let Ok(frame) = read
                .map_err(|_| ())
                .and_then(|()| Frame::new(dimensions[0], dimensions[1], rgba).map_err(|_| ()))
            else {
                break;
            };
            let Ok(mut staged) = shared.0.lock() else {
                break;
            };
            if staged.cancelled {
                break;
            }
            staged.frames.push((index, frame));
            drop(staged);
            shared.1.notify_all();
        }
    }
    // The process is reaped before the run says it is done, so the pipe it
    // counted for in the decoder bank is free by then.
    drop(process);
    finish(&shared);
}

struct ReadAhead {
    generation: u64,
    sequence: u64,
    lane: Lane,
    size: [u32; 2],
    time: Time,
    key: Key,
}

pub(crate) struct DecoderPool {
    processes: Processes,
    decoders: VecDeque<VideoDecoder>,
    gifs: VecDeque<GifDecoderState>,
    gif_state_bytes: usize,
    cache: VecDeque<Cached>,
    bytes: usize,
    #[cfg(test)]
    starts: usize,
    preview_retention: [Option<crate::PreviewConsumer>; 3],
    read_ahead: Option<ReadAhead>,
    /// The frame each lane asked for last, of a few lanes: a request a
    /// little before it is a step backwards.
    steps: VecDeque<(Lane, AssetId, i64)>,
    walk: Option<Walk>,
    behind: Option<Behind>,
    #[cfg(test)]
    behind_starts: usize,
}

impl DecoderPool {
    pub fn new(processes: Processes) -> Self {
        Self {
            processes,
            decoders: VecDeque::new(),
            gifs: VecDeque::new(),
            gif_state_bytes: 0,
            cache: VecDeque::new(),
            bytes: 0,
            #[cfg(test)]
            starts: 0,
            preview_retention: [None; 3],
            read_ahead: None,
            steps: VecDeque::new(),
            walk: None,
            behind: None,
            #[cfg(test)]
            behind_starts: 0,
        }
    }

    pub fn clear(&mut self) {
        self.steps.clear();
        self.walk = None;
        self.behind = None;
        self.decoders.clear();
        self.gifs.clear();
        self.gif_state_bytes = 0;
        self.cache.clear();
        self.bytes = 0;
        self.preview_retention = [None; 3];
        self.read_ahead = None;
    }

    pub fn set_preview_retention(&mut self, protected: [Option<crate::PreviewConsumer>; 3]) {
        self.preview_retention = protected;
    }

    pub fn retain_lanes(&mut self, lanes: &[Lane]) {
        if self
            .read_ahead
            .as_ref()
            .is_some_and(|hint| !lanes.contains(&hint.lane))
        {
            self.read_ahead = None;
        }
        self.steps.retain(|(lane, ..)| lanes.contains(lane));
        if (self.walk.as_ref()).is_some_and(|walk| !lanes.contains(&walk.stream.lane)) {
            self.walk = None;
            self.behind = None;
        }
        self.decoders
            .retain(|decoder| lanes.contains(&decoder.lane));
        self.gifs.retain(|decoder| lanes.contains(&decoder.lane));
        self.gif_state_bytes = self.gifs.iter().map(|decoder| decoder.reserved_bytes).sum();
    }

    /// Prepare an admitted video's pipe without reading pixels or evicting a
    /// different lane. Only the preview worker may call this method.
    pub fn prepare_video(
        &mut self,
        job: &FrameJob,
        current: impl Fn() -> bool,
    ) -> Result<bool, MediaError> {
        if !current() {
            return Err(MediaError::Cancelled);
        }
        if job.asset.kind != MediaKind::Video {
            return Ok(false);
        }
        let dimensions = fitted_dimensions([job.asset.width, job.asset.height], job.size)?;
        let target = job
            .asset
            .frame_rate
            .unwrap_or_default()
            .frame_at(source_time(job));
        let source = Source::from(&job.asset);
        let key = Key {
            asset: job.asset.id,
            dimensions,
            frame: target,
            source: source.clone(),
        };
        let stream = Stream {
            lane: job.lane,
            asset: job.asset.id,
            dimensions,
            source: source.clone(),
        };
        self.note_step(&stream, target);
        self.take_staged();
        if self.cache.iter().any(|entry| entry.key == key) {
            return Ok(false);
        }
        // The look-behind is already decoding this frame: no pipe is opened
        // for it.
        if (self.behind.as_ref()).is_some_and(|behind| behind.covers(&stream, target)) {
            return Ok(false);
        }
        // A step backwards opens its pipe where its run begins.
        let mut run = None;
        if let Some(index) = self
            .decoders
            .iter()
            .position(|decoder| decoder.lane == job.lane)
        {
            if self.decoders[index].compatible(target, dimensions, &source) {
                return Ok(false);
            }
            if !current() {
                return Err(MediaError::Cancelled);
            }
            run = self.decoders[index].run_start(target, dimensions, &source);
            // Replacing this admitted lane is safe even in a full bank. An
            // unrelated lane is never evicted merely to prepare future work.
            self.decoders.remove(index);
        }
        let run = run.or_else(|| self.walk_run(&stream, target));
        if self.decoders.len() + self.behind_pipes() >= VIDEO_DECODER_LIMIT {
            // A pipe for a picture that was asked for outranks one that
            // decodes ahead of need.
            self.behind = None;
        }
        if self.decoders.len() >= VIDEO_DECODER_LIMIT {
            return Ok(false);
        }
        if !current() {
            return Err(MediaError::Cancelled);
        }
        let mut decoder = self.start_video(job, run.unwrap_or(target), dimensions, &source)?;
        decoder.fill = run.map(|_| target);
        if !current() {
            // Reap outside the caller's queue mutex before accepting a child
            // whose captured request was superseded while spawn was in flight.
            return Err(MediaError::Cancelled);
        }
        self.decoders.push_back(decoder);
        Ok(true)
    }

    pub fn frame(
        &mut self,
        job: &FrameJob,
        current: impl Fn() -> bool,
    ) -> Result<Frame, MediaError> {
        // A hint belongs only to this successful cold decode, never a cache hit
        // or a subsequent request which happens to reuse the same owner.
        self.read_ahead = None;
        let dimensions = fitted_dimensions([job.asset.width, job.asset.height], job.size)?;
        let source_time = source_time(job);
        let frame_number = match job.asset.kind {
            MediaKind::Image => 0,
            MediaKind::Gif => source_time.0 / 10_000,
            _ => job
                .asset
                .frame_rate
                .unwrap_or_default()
                .frame_at(source_time),
        };
        let mut key = Key {
            asset: job.asset.id,
            dimensions,
            frame: frame_number,
            source: Source::from(&job.asset),
        };
        let stream = (job.asset.kind == MediaKind::Video).then(|| Stream {
            lane: job.lane,
            asset: job.asset.id,
            dimensions,
            source: key.source.clone(),
        });
        if let Some(stream) = &stream {
            self.note_step(stream, frame_number);
            self.take_staged();
        }
        if let Some(frame) = self.cached(&key, job.asset.kind, source_time) {
            return Ok(frame);
        }
        if !current() {
            return Err(MediaError::Cancelled);
        }
        // A frame the look-behind is on its way to is waited for: starting
        // over for it would take longer than the run has left.
        if let Some(stream) = &stream
            && self.await_behind(stream, &key, &current)?
            && let Some(frame) = self.cached(&key, job.asset.kind, source_time)
        {
            return Ok(frame);
        }
        let mut span = None;
        let frame = match job.asset.kind {
            MediaKind::Image => {
                let image = image_reader(&job.asset.path)?
                    .decode()
                    .map_err(|error| MediaError::Decode(error.to_string()))?;
                scaled(image, dimensions)?
            }
            MediaKind::Gif => {
                let (frame, interval, index) =
                    self.gif(job, source_time, dimensions, &key.source, &current)?;
                span = Some(interval);
                key.frame = index;
                frame
            }
            MediaKind::Video => self.video(job, frame_number, dimensions, &key.source, &current)?,
            MediaKind::Audio => return Err(MediaError::Invalid("Audio has no video frame".into())),
            MediaKind::Lut => return Err(MediaError::Invalid("A LUT has no video frame".into())),
            MediaKind::Font => return Err(MediaError::Invalid("A font has no video frame".into())),
        };
        if !current() {
            self.read_ahead = None;
            return Err(MediaError::Cancelled);
        }
        self.cache_frame(key, frame.clone(), span);
        Ok(frame)
    }

    /// The cached picture for `key`, which becomes the one used last.
    fn cached(&mut self, key: &Key, kind: MediaKind, source_time: Time) -> Option<Frame> {
        let index = self.cache.iter().position(|entry| {
            if kind == MediaKind::Gif {
                entry.key.asset == key.asset
                    && entry.key.dimensions == key.dimensions
                    && entry.key.source == key.source
                    && entry
                        .span
                        .is_some_and(|(start, end)| start <= source_time && source_time < end)
            } else {
                entry.key == *key
            }
        })?;
        let entry = self.cache.remove(index)?;
        let frame = entry.frame.clone();
        self.cache.push_back(entry);
        Some(frame)
    }

    fn cache_frame(&mut self, key: Key, frame: Frame, span: Option<(Time, Time)>) {
        self.make_room(frame.byte_len(), 1);
        self.bytes += frame.byte_len();
        self.cache.push_back(Cached { key, frame, span });
    }

    /// Evicts until `bytes` more and `entries` more fit beside what the
    /// look-behind has yet to hand over. The picture used longest ago goes
    /// first, but for the frames a backwards walk has still to show: those
    /// were decoded before the ones shown since and would otherwise be the
    /// first to go.
    fn make_room(&mut self, bytes: usize, entries: usize) {
        let reserved = self.behind.as_ref().map_or(0, |behind| behind.reserved);
        while self.bytes + reserved + bytes > FRAME_CACHE_BYTES || self.cache.len() + entries > 256
        {
            let ahead = self.walk.as_ref().and_then(|walk| {
                let group = reverse_group(walk.stream.dimensions)?;
                // The frame asked for last may be the one about to be read.
                Some((&walk.stream, walk.last - 2 * group..=walk.last))
            });
            let index = (self.cache.iter())
                .position(|entry| {
                    !ahead.as_ref().is_some_and(|(stream, frames)| {
                        stream.holds(&entry.key) && frames.contains(&entry.key.frame)
                    })
                })
                .unwrap_or(0);
            let Some(old) = self.cache.remove(index) else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(old.frame.byte_len());
        }
    }

    // ---- Stepping backwards: the walk and its look-behind ------------------

    /// Told of every video frame asked for. A lane that asks for a frame a
    /// little before its last is walking backwards; one that turns forwards
    /// or jumps is not, and the look-behind decoded for its walk is given up.
    fn note_step(&mut self, stream: &Stream, target: i64) {
        let slot = (self.steps.iter())
            .position(|(lane, asset, _)| *lane == stream.lane && *asset == stream.asset);
        let previous = slot
            .and_then(|index| self.steps.remove(index))
            .map(|step| step.2);
        while self.steps.len() >= 8 {
            self.steps.pop_front();
        }
        self.steps.push_back((stream.lane, stream.asset, target));

        let group = reverse_group(stream.dimensions);
        let back = |from: i64| group.is_some_and(|group| target < from && from - target <= group);
        let now = Instant::now();
        match &mut self.walk {
            Some(walk) if walk.stream == *stream => {
                if target == walk.last || back(walk.last) {
                    walk.last = target;
                    walk.at = now;
                } else {
                    self.walk = None;
                    self.behind = None;
                }
            }
            // The same lane, shown another source or at another size.
            Some(walk) if walk.stream.lane == stream.lane => {
                self.walk = None;
                self.behind = None;
            }
            _ => {}
        }
        let free = (self.walk.as_ref()).is_none_or(|walk| now.duration_since(walk.at) > WALK_IDLE);
        if free && previous.is_some_and(back) {
            self.behind = None;
            self.walk = Some(Walk {
                stream: stream.clone(),
                last: target,
                at: now,
            });
        }
    }

    /// Where a run for `target` starts when the lane is walking backwards
    /// and no pipe of its own says so: the walk goes on in runs.
    fn walk_run(&self, stream: &Stream, target: i64) -> Option<i64> {
        let walk = self.walk.as_ref().filter(|walk| walk.stream == *stream)?;
        let group = reverse_group(stream.dimensions)?;
        (walk.last == target).then(|| (target - (group - 1)).max(0))
    }

    /// The pipes the look-behind holds: one while its run is not done.
    fn behind_pipes(&self) -> usize {
        usize::from((self.behind.as_ref()).is_some_and(|behind| !behind.done()))
    }

    /// Takes what the look-behind has read into the cache, and tells it how
    /// many frames it may read before the pool looks again: the frames of
    /// the walk held ahead of it, with those, are half the cache at most.
    fn take_staged(&mut self) {
        let Some(behind) = &mut self.behind else {
            return;
        };
        let last = (self.walk.as_ref())
            .filter(|walk| walk.stream == behind.stream)
            .map_or(behind.until + 1, |walk| walk.last);
        let held = (self.cache.iter())
            .filter(|entry| {
                behind.stream.holds(&entry.key) && (behind.from..last).contains(&entry.key.frame)
            })
            .count();
        let frames = {
            let Ok(mut staged) = behind.shared.0.lock() else {
                return;
            };
            let frames = std::mem::take(&mut staged.frames);
            behind.taken += frames.len();
            staged.budget = reverse_room(behind.stream.dimensions)
                .saturating_sub(held + frames.len())
                .min(behind.frames().saturating_sub(behind.taken));
            behind.reserved = (frames.len() + staged.budget) * behind.bytes;
            frames
        };
        behind.shared.1.notify_all();
        let stream = behind.stream.clone();
        self.make_room(0, 0);
        for (index, frame) in frames {
            if let Some(behind) = &mut self.behind {
                behind.reserved = behind.reserved.saturating_sub(frame.byte_len());
            }
            let key = stream.key(index);
            if !self.cache.iter().any(|entry| entry.key == key) {
                self.cache_frame(key, frame, None);
            }
        }
    }

    /// Waits for the look-behind while it is on its way to `key`. True when
    /// the frame is in the cache.
    fn await_behind(
        &mut self,
        stream: &Stream,
        key: &Key,
        current: &impl Fn() -> bool,
    ) -> Result<bool, MediaError> {
        loop {
            let Some(behind) = (self.behind.as_ref()).filter(|b| b.covers(stream, key.frame))
            else {
                return Ok(false);
            };
            let (lock, ready) = &*behind.shared;
            let done = match lock.lock() {
                Ok(staged) if staged.frames.is_empty() && !staged.done => ready
                    .wait_timeout(staged, Duration::from_millis(2))
                    .map_or(true, |(staged, _)| staged.done),
                Ok(staged) => staged.done,
                Err(_) => true,
            };
            self.take_staged();
            if self.cache.iter().any(|entry| entry.key == *key) {
                return Ok(true);
            }
            if done {
                // The run ended without it.
                self.behind = None;
                return Ok(false);
            }
            if !current() {
                return Err(MediaError::Cancelled);
            }
        }
    }

    /// After a frame of a backwards walk was served: starts decoding the run
    /// before the frames already held, on its own thread, unless one is under
    /// way or the frames held still reach a run ahead. Called off the frame
    /// path, once the frame asked for has been published. The run takes a
    /// pipe of the decoder bank only when one is free, and reads a frame only
    /// when the cache has been given room for it.
    pub fn look_behind(&mut self, job: &FrameJob, current: impl Fn() -> bool) {
        if job.asset.kind != MediaKind::Video {
            return;
        }
        self.take_staged();
        let Some(walk) = &self.walk else {
            return;
        };
        let stream = walk.stream.clone();
        let target = walk.last;
        let asked = fitted_dimensions([job.asset.width, job.asset.height], job.size).ok();
        if stream.lane != job.lane
            || stream.asset != job.asset.id
            || asked != Some(stream.dimensions)
            || stream.source != Source::from(&job.asset)
        {
            return;
        }
        let Some(group) = reverse_group(stream.dimensions) else {
            return;
        };
        if let Some(behind) = &self.behind {
            if target < behind.from {
                // The walk overtook it.
                self.behind = None;
            } else if !behind.done() || target > behind.until {
                // Still decoding, or the walk has not reached what it read.
                return;
            }
        }
        // The lowest frame held without a gap below the one shown.
        let mut low = target;
        while low > 0
            && target - low < 2 * group
            && (self.cache.iter()).any(|entry| entry.key == stream.key(low - 1))
        {
            low -= 1;
        }
        if low == 0 || target - low >= group {
            return;
        }
        let (from, until) = ((low - group).max(0), low - 1);
        let Ok(bytes) = frame_bytes(stream.dimensions[0], stream.dimensions[1]) else {
            return;
        };
        // A run that has handed everything over holds no pipe that counts:
        // its process is being reaped on its own thread.
        if self.decoders.len() >= VIDEO_DECODER_LIMIT || !current() {
            return;
        }
        let command = video_command(
            &job.asset.path,
            job.asset.frame_rate.unwrap_or_default(),
            from,
            stream.dimensions,
        );
        let shared = StagedShared::default();
        let (processes, staged) = (self.processes.clone(), Arc::clone(&shared));
        let dimensions = stream.dimensions;
        let started = thread::Builder::new()
            .name("aster-look-behind".into())
            .spawn(move || {
                run_behind(command, processes, staged, dimensions, bytes, from, until);
            });
        if started.is_err() {
            return;
        }
        #[cfg(test)]
        {
            self.behind_starts += 1;
        }
        self.behind = Some(Behind {
            stream,
            from,
            until,
            shared,
            taken: 0,
            bytes,
            reserved: 0,
        });
        // Room is made, and the run told of it, before any of it is read.
        self.take_staged();
    }

    /// True only for the exact job which just opened an unprotected video pipe.
    /// The worker must still check currentness and publish its requested result
    /// before spending the separate speculative-read budget.
    pub fn has_read_ahead(&self, job: &FrameJob) -> bool {
        let Some(hint) = &self.read_ahead else {
            return false;
        };
        if hint.generation != job.generation
            || hint.sequence != job.sequence
            || hint.lane != job.lane
            || hint.size != job.size
            || hint.time != job.time
            || hint.key.asset != job.asset.id
            || hint.key.source != Source::from(&job.asset)
            || job.asset.kind != MediaKind::Video
            || job.lane.protected_by(&self.preview_retention)
            || !fitted_dimensions([job.asset.width, job.asset.height], job.size)
                .is_ok_and(|dimensions| dimensions == hint.key.dimensions)
        {
            return false;
        }
        self.decoders.iter().any(|decoder| {
            decoder.lane == hint.lane
                && decoder.source == hint.key.source
                && decoder.dimensions == hint.key.dimensions
                && decoder.next_index == hint.key.frame + 1
        })
    }

    /// Consume one cold-video hint after publication. All future reads share
    /// the caller's single absolute deadline; failures affect only speculation.
    pub fn read_ahead(&mut self, job: &FrameJob, deadline: Instant, current: impl Fn() -> bool) {
        let eligible = self.has_read_ahead(job);
        let Some(hint) = self.read_ahead.take() else {
            return;
        };
        if !eligible || !current() || Instant::now() >= deadline {
            return;
        }
        let Ok(bytes) = frame_bytes(hint.key.dimensions[0], hint.key.dimensions[1]) else {
            return;
        };
        // Leave room for the already published requested frame, both by bytes
        // and entries. Each completed future frame enters the same LRU at once.
        let count = (FRAME_CACHE_BYTES / bytes)
            .saturating_sub(1)
            .min(255)
            .min(12);
        if count == 0 {
            return;
        }
        let Some(index) = self
            .decoders
            .iter()
            .position(|decoder| decoder.lane == hint.lane)
        else {
            return;
        };
        let Some(mut decoder) = self.decoders.remove(index) else {
            return;
        };
        let rate = job.asset.frame_rate.unwrap_or_default();
        for _ in 0..count {
            if !current()
                || Instant::now() >= deadline
                || rate.frame_time(decoder.next_index) >= decoder.source.duration
            {
                break;
            }
            let index = decoder.next_index;
            let mut rgba = vec![0; bytes];
            if !decoder.process.begin_read_until(deadline) {
                return;
            }
            let read = decoder.process.stdout.read_exact(&mut rgba);
            let healthy = decoder.process.end_read_checked();
            if read.is_err() || !healthy {
                // Partial/EOF/expired reads leave an unsafe pipe. Dropping it
                // cannot revoke the requested frame which was already published.
                return;
            }
            decoder.next_index += 1;
            if !current() {
                break;
            }
            let Ok(frame) = Frame::new(hint.key.dimensions[0], hint.key.dimensions[1], rgba) else {
                break;
            };
            let key = Key {
                frame: index,
                ..hint.key.clone()
            };
            if !self.cache.iter().any(|entry| entry.key == key) {
                self.cache_frame(key, frame, None);
            }
        }
        self.decoders.push_back(decoder);
    }

    fn gif(
        &mut self,
        job: &FrameJob,
        time: Time,
        dimensions: [u32; 2],
        source: &Source,
        current: &impl Fn() -> bool,
    ) -> Result<(Frame, (Time, Time), i64), MediaError> {
        let mut decoder = self
            .gifs
            .iter()
            .position(|decoder| decoder.lane == job.lane)
            .and_then(|index| self.gifs.remove(index));
        if let Some(decoder) = &decoder {
            self.gif_state_bytes = self.gif_state_bytes.saturating_sub(decoder.reserved_bytes);
        }
        if decoder.as_ref().is_some_and(|decoder| {
            decoder.source != *source
                || decoder.dimensions != dimensions
                || (time < decoder.next_time
                    && !decoder
                        .last
                        .as_ref()
                        .is_some_and(|(_, (start, end), _)| *start <= time && time < *end))
        }) {
            decoder = None;
        }
        if decoder.is_none() {
            let file = File::open(&job.asset.path).map_err(|error| io("Cannot open GIF", error))?;
            let mut native = GifDecoder::new(BufReader::new(file))
                .map_err(|error| MediaError::Decode(error.to_string()))?;
            native
                .set_limits(limits())
                .map_err(|error| MediaError::Decode(error.to_string()))?;
            let (width, height) = native.dimensions();
            let preview_bytes = frame_bytes(dimensions[0], dimensions[1])?;
            let reserved_bytes = (width as usize).checked_mul(height as usize)
                .and_then(|n| n.checked_mul(4)).and_then(|n| n.checked_add(preview_bytes))
                .filter(|bytes| *bytes <= GIF_STATE_BUDGET)
                .ok_or_else(|| MediaError::Invalid("GIF canvas and current preview exceed the 64 MiB working budget; resize the source or use a proxy".into()))?;
            while self.gifs.len() >= 4 || self.gif_state_bytes + reserved_bytes > GIF_STATE_BUDGET {
                let Some(old) = self.gifs.pop_front() else {
                    break;
                };
                self.gif_state_bytes = self.gif_state_bytes.saturating_sub(old.reserved_bytes);
            }
            decoder = Some(GifDecoderState {
                lane: job.lane,
                dimensions,
                source: source.clone(),
                frames: native.into_frames(),
                next_index: 0,
                next_time: Time::ZERO,
                reserved_bytes,
                last: None,
            });
        }
        let Some(mut decoder) = decoder else {
            return Err(MediaError::Closed);
        };
        let result = loop {
            if !current() {
                break Err(MediaError::Cancelled);
            }
            if let Some((frame, (start, end), index)) = &decoder.last
                && *start <= time
                && time < *end
            {
                break Ok((frame.clone(), (*start, *end), *index));
            }
            if decoder.next_index >= 10_000 {
                break Err(MediaError::Invalid("GIF exceeds 10,000 frames".into()));
            }
            let image = match decoder.frames.next() {
                Some(Ok(frame)) => frame,
                Some(Err(error)) => break Err(MediaError::Decode(error.to_string())),
                None => {
                    break decoder
                        .last
                        .as_ref()
                        .map(|(frame, interval, index)| (frame.clone(), *interval, *index))
                        .ok_or_else(|| MediaError::Decode("GIF has no frames".into()));
                }
            };
            let start = decoder.next_time;
            let end = start + Time(gif_delay_us(image.delay()));
            let index = decoder.next_index;
            decoder.next_index += 1;
            decoder.next_time = end;
            if time < end {
                match scaled(DynamicImage::ImageRgba8(image.into_buffer()), dimensions) {
                    Ok(frame) => {
                        decoder.last = Some((frame.clone(), (start, end), index));
                        break Ok((frame, (start, end), index));
                    }
                    Err(error) => break Err(error),
                }
            }
            // Skipped frames update disposal state but never allocate previews.
        };
        // A newer playback request must keep already-decoded progress. Dropping
        // it would repeatedly restart frame0 and starve every later frame.
        if result.is_ok() || matches!(result, Err(MediaError::Cancelled)) {
            self.gif_state_bytes += decoder.reserved_bytes;
            self.gifs.push_back(decoder);
        }
        result
    }

    fn video(
        &mut self,
        job: &FrameJob,
        target: i64,
        dimensions: [u32; 2],
        source: &Source,
        current: &impl Fn() -> bool,
    ) -> Result<Frame, MediaError> {
        let mut decoder = self
            .decoders
            .iter()
            .position(|decoder| decoder.lane == job.lane)
            .and_then(|index| self.decoders.remove(index));
        // A frame behind the lane's pipe is a step backwards: the pipe that
        // replaces it starts a run earlier and keeps what it reads on the way.
        let mut run = None;
        if let Some(old) =
            (decoder.as_ref()).filter(|decoder| !decoder.compatible(target, dimensions, source))
        {
            run = old.run_start(target, dimensions, source);
            decoder = None;
        }
        let bytes = frame_bytes(dimensions[0], dimensions[1])?;
        if decoder.is_none() {
            // A walk whose look-behind did not have this frame goes on in
            // runs, wherever the lane's own pipe was left.
            run = run.or_else(|| {
                let stream = Stream {
                    lane: job.lane,
                    asset: job.asset.id,
                    dimensions,
                    source: source.clone(),
                };
                self.walk_run(&stream, target)
            });
            if self.decoders.len() + self.behind_pipes() >= VIDEO_DECODER_LIMIT {
                // The frame asked for outranks the run decoded ahead of need.
                self.behind = None;
            }
            while self.decoders.len() >= VIDEO_DECODER_LIMIT {
                // Three protected identities cannot fill the five-pipe bank.
                // Remove/reap the oldest eligible pipe before opening another.
                let index = self
                    .decoders
                    .iter()
                    .position(|decoder| !decoder.lane.protected_by(&self.preview_retention))
                    .ok_or_else(|| {
                        MediaError::Invalid("No unprotected video decoder slot".into())
                    })?;
                self.decoders.remove(index);
            }
            let mut started = self.start_video(job, run.unwrap_or(target), dimensions, source)?;
            started.fill = run.map(|_| target);
            decoder = Some(started);
        }
        let Some(mut decoder) = decoder else {
            return Err(MediaError::Closed);
        };
        // Reading on past a run would move the pipe away from the steps back
        // that follow it.
        let read_ahead = decoder.unread
            && decoder.fill.is_none()
            && !job.lane.protected_by(&self.preview_retention)
            && bytes <= READ_AHEAD_FRAME_BYTES;
        decoder.unread = false;
        let mut rgba = vec![0; bytes];
        loop {
            if !current() {
                self.decoders.push_back(decoder);
                return Err(MediaError::Cancelled);
            }
            decoder.process.begin_read();
            if let Err(error) = decoder.process.stdout.read_exact(&mut rgba) {
                return Err(decoder.process.failure(&format!(
                    "Could not decode frame at {:.3}s ({error})",
                    job.time.as_seconds()
                )));
            }
            decoder.process.end_read();
            let index = decoder.next_index;
            decoder.next_index += 1;
            if index >= target {
                break;
            }
            if decoder.fill.is_some_and(|until| index <= until) {
                let key = Key {
                    asset: job.asset.id,
                    dimensions,
                    frame: index,
                    source: source.clone(),
                };
                if !self.cache.iter().any(|entry| entry.key == key) {
                    let kept = Frame::new(dimensions[0], dimensions[1], rgba.clone())?;
                    self.cache_frame(key, kept, None);
                }
            }
        }
        let frame = Frame::new(dimensions[0], dimensions[1], rgba)?;
        self.decoders.push_back(decoder);
        if read_ahead {
            self.read_ahead = Some(ReadAhead {
                generation: job.generation,
                sequence: job.sequence,
                lane: job.lane,
                size: job.size,
                time: job.time,
                key: Key {
                    asset: job.asset.id,
                    dimensions,
                    frame: target,
                    source: source.clone(),
                },
            });
        }
        Ok(frame)
    }
    fn start_video(
        &mut self,
        job: &FrameJob,
        target: i64,
        dimensions: [u32; 2],
        source: &Source,
    ) -> Result<VideoDecoder, MediaError> {
        let rate = job.asset.frame_rate.unwrap_or_default();
        let mut command = video_command(&job.asset.path, rate, target, dimensions);
        let process = Process::spawn("ffmpeg", &mut command, &self.processes)?;
        #[cfg(test)]
        {
            self.starts += 1;
        }
        Ok(VideoDecoder {
            lane: job.lane,
            process,
            next_index: target,
            dimensions,
            source: source.clone(),
            unread: true,
            fill: None,
        })
    }
}

/// The process that decodes a video from frame `target` on, scaled to
/// `dimensions`, as raw RGBA on its output.
pub(crate) fn video_command(
    path: &std::path::Path,
    rate: FrameRate,
    target: i64,
    dimensions: [u32; 2],
) -> Command {
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
        .arg(format!("{:.9}", target as f64 / rate.fps()))
        .arg("-i")
        .arg(path)
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
            "scale={}:{}:flags=fast_bilinear,fps={}/{}",
            dimensions[0], dimensions[1], rate.numerator, rate.denominator
        ))
        // Input -threads does not constrain the rawvideo encoder.
        // Automatic frame threading and the default eight-packet mux
        // queue otherwise retain many full RGBA frames per pipe.
        .args([
            "-pix_fmt",
            "rgba",
            "-threads:v",
            "1",
            "-thread_queue_size",
            "1",
            "-f",
            "rawvideo",
            "pipe:1",
        ]);
    command
}

fn source_time(job: &FrameJob) -> Time {
    Time(
        job.time
            .0
            .min(job.asset.duration.0.saturating_sub(1))
            .max(0),
    )
}

fn scaled(image: DynamicImage, dimensions: [u32; 2]) -> Result<Frame, MediaError> {
    let rgba = if image.width() == dimensions[0] && image.height() == dimensions[1] {
        image.into_rgba8()
    } else {
        image
            .resize_exact(dimensions[0], dimensions[1], FilterType::Triangle)
            .into_rgba8()
    };
    Frame::new(dimensions[0], dimensions[1], rgba.into_raw())
}

#[cfg(test)]
mod retention_tests;

#[cfg(test)]
pub(crate) mod warmup_tests;

#[cfg(test)]
mod consumer_tests;

#[cfg(test)]
mod prefetch_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::asset_from_probe;
    use std::process::Stdio;

    #[test]
    fn gif_requests_inside_one_native_delay_reuse_pixels_and_honor_variable_boundaries() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("variable.gif");
        let mut encoder = image::codecs::gif::GifEncoder::new(File::create(&path).unwrap());
        for (delay, color) in [
            (100, [255, 0, 0, 255]),
            (200, [0, 255, 0, 255]),
            (50, [0, 0, 255, 255]),
        ] {
            encoder
                .encode_frame(image::Frame::from_parts(
                    image::RgbaImage::from_pixel(8, 4, image::Rgba(color)),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(delay, 1),
                ))
                .unwrap();
        }
        drop(encoder);
        let mut asset = Asset::new(path, MediaKind::Gif, Time(350_000));
        asset.width = 8;
        asset.height = 4;
        let mut job = FrameJob {
            generation: 1,
            sequence: 1,
            lane: Lane::Legacy {
                asset_id: asset.id,
                size: [8, 4],
            },
            size: [8, 4],
            asset,
            time: Time::ZERO,
        };
        let mut pool = DecoderPool::new(Processes::default());
        let first = pool.frame(&job, || true).unwrap();
        job.time = Time(33_333);
        let repeated = pool.frame(&job, || true).unwrap();
        assert!(
            std::sync::Arc::ptr_eq(&first.rgba, &repeated.rgba),
            "30fps requests must reuse a GIF's unchanged display interval"
        );
        job.time = Time(100_000);
        let middle = pool.frame(&job, || true).unwrap();
        assert_eq!(&middle.rgba[..4], &[0, 255, 0, 255]);
        job.time = Time(299_999);
        assert!(std::sync::Arc::ptr_eq(
            &middle.rgba,
            &pool.frame(&job, || true).unwrap().rgba
        ));
        job.time = Time(300_000);
        assert_eq!(
            &pool.frame(&job, || true).unwrap().rgba[..4],
            &[0, 0, 255, 255]
        );
        job.time = Time(30_000); // A backwards seek is still exact.
        assert_eq!(
            &pool.frame(&job, || true).unwrap().rgba[..4],
            &[255, 0, 0, 255]
        );
        assert!(pool.gif_state_bytes <= GIF_STATE_BUDGET);
        assert!(pool.gifs.len() <= 4);
        pool.clear();
        assert_eq!(pool.gif_state_bytes, 0);
        assert!(pool.gifs.is_empty());
    }

    #[test]
    fn restoring_an_asset_uuid_with_a_different_image_path_invalidates_cached_pixels() {
        let temp = tempfile::tempdir().unwrap();
        let red = temp.path().join("red.png");
        let green = temp.path().join("green.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255]))
            .save(&red)
            .unwrap();
        image::RgbaImage::from_pixel(8, 8, image::Rgba([0, 255, 0, 255]))
            .save(&green)
            .unwrap();
        let mut asset = Asset::new(red, MediaKind::Image, Time::from_seconds(5.0));
        asset.width = 8;
        asset.height = 8;
        let asset_id = asset.id;
        let mut job = FrameJob {
            generation: 1,
            sequence: 1,
            asset,
            time: Time::ZERO,
            lane: Lane::Legacy {
                asset_id,
                size: [8, 8],
            },
            size: [8, 8],
        };
        let mut pool = DecoderPool::new(Processes::default());
        let first = pool.frame(&job, || true).unwrap();
        assert_eq!(&first.rgba[..4], &[255, 0, 0, 255]);
        job.asset.path = green;
        let restored = pool.frame(&job, || true).unwrap();
        assert_eq!(&restored.rgba[..4], &[0, 255, 0, 255]);
        assert_eq!(
            &pool.frame(&job, || true).unwrap().rgba[..4],
            &[0, 255, 0, 255]
        );
    }

    #[test]
    fn sequential_video_requests_reuse_one_ffmpeg_process() {
        let mut version = tool_command("ffmpeg");
        if !version
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
        {
            eprintln!("FFmpeg is unavailable; sequential decoder fixture skipped");
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("video.mkv");
        let status = tool_command("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=64x32:rate=30",
                "-t",
                "1",
                "-c:v",
                "ffv1",
            ])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
        let asset = asset_from_probe(&path, br#"{"format":{"duration":"1"},"streams":[{"codec_type":"video","width":64,"height":32,"avg_frame_rate":"30/1"}]}"#).unwrap();
        let mut pool = DecoderPool::new(Processes::default());
        for index in 0..10 {
            let job = FrameJob {
                generation: 1,
                sequence: index,
                time: Time::from_seconds(index as f64 / 30.0),
                lane: Lane::Legacy {
                    asset_id: asset.id,
                    size: [64, 32],
                },
                size: [64, 32],
                asset: asset.clone(),
            };
            let frame = pool.frame(&job, || true).unwrap();
            assert_eq!(frame.byte_len(), 64 * 32 * 4);
        }
        assert_eq!(
            pool.starts, 1,
            "playback must not spawn FFmpeg for each frame"
        );
        let thumbnail = FrameJob {
            generation: 1,
            sequence: 11,
            time: Time::ZERO,
            lane: Lane::Legacy {
                asset_id: asset.id,
                size: [32, 16],
            },
            size: [32, 16],
            asset: asset.clone(),
        };
        assert_eq!(
            pool.frame(&thumbnail, || true).unwrap().byte_len(),
            32 * 16 * 4
        );
        let preview = FrameJob {
            generation: 1,
            sequence: 12,
            time: Time::from_seconds(10.0 / 30.0),
            lane: Lane::Legacy {
                asset_id: asset.id,
                size: [64, 32],
            },
            size: [64, 32],
            asset,
        };
        pool.frame(&preview, || true).unwrap();
        assert_eq!(
            pool.starts, 2,
            "thumbnail and playback retain separate sequential pipes"
        );
        assert!(pool.bytes <= FRAME_CACHE_BYTES);
        assert_eq!(pool.decoders.len(), 2);

        let mut changed = preview;
        changed.time = Time::from_seconds(11.0 / 30.0);
        // Restoring changed metadata also invalidates a pipe whose path and
        // fitted dimensions are unchanged; it must not silently retain it.
        changed.asset.file_size = changed.asset.file_size.saturating_add(1);
        pool.frame(&changed, || true).unwrap();
        assert_eq!(
            pool.starts, 3,
            "changed source metadata restarts retained decoding"
        );
        changed.time = Time::from_seconds(12.0 / 30.0);
        pool.frame(&changed, || true).unwrap();
        assert_eq!(
            pool.starts, 3,
            "unchanged restored metadata retains the replacement pipe"
        );
    }

    /// Reverse preview through the sequential decoder: the pictures are right,
    /// and what each step back costs is printed for whoever reads the run. A
    /// pipe runs forwards only, so a step back starts another a run of frames
    /// earlier, and the steps after it are served from what that run kept.
    #[test]
    fn stepping_backwards_decodes_the_frames_decoded_forwards() {
        let _heavy = crate::heavy_test();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("long-gop.mp4");
        // One keyframe for the whole file, as camera footage has few.
        let encoded = tool_command("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=640x360:rate=30",
                "-t",
                "3",
                "-c:v",
                "libx264",
                "-g",
                "250",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !encoded {
            eprintln!("FFmpeg with libx264 is unavailable; reverse decoder fixture skipped");
            return;
        }
        let asset = asset_from_probe(&path, br#"{"format":{"duration":"3"},"streams":[{"codec_type":"video","width":640,"height":360,"avg_frame_rate":"30/1"}]}"#).unwrap();
        let job = |index: i64| FrameJob {
            generation: 1,
            sequence: index as u64,
            time: FrameRate::default().frame_time(index),
            lane: Lane::Legacy {
                asset_id: asset.id,
                size: [640, 360],
            },
            size: [640, 360],
            asset: asset.clone(),
        };
        const STEPS: i64 = 30;
        let mut forwards = DecoderPool::new(Processes::default());
        let began = Instant::now();
        let expected: Vec<_> = (60..60 + STEPS)
            .map(|index| forwards.frame(&job(index), || true).unwrap())
            .collect();
        let forward_time = began.elapsed();
        assert_eq!(forwards.starts, 1);

        let mut pool = DecoderPool::new(Processes::default());
        let began = Instant::now();
        for index in (60..60 + STEPS).rev() {
            let frame = pool.frame(&job(index), || true).unwrap();
            assert!(
                frame.rgba == expected[(index - 60) as usize].rgba,
                "frame {index} stepped back to is not the frame played forwards"
            );
        }
        let reverse_time = began.elapsed();
        let cold_starts = pool.starts;
        // The first frame, and then one run for every group of steps back.
        let runs = (STEPS - 1 + REVERSE_GROUP_FRAMES as i64 - 1) / REVERSE_GROUP_FRAMES as i64;
        assert_eq!(cold_starts as i64, 1 + runs);
        assert!(pool.decoders.len() <= VIDEO_DECODER_LIMIT);
        assert!(pool.bytes <= FRAME_CACHE_BYTES);

        // The frames just shown are kept, within the cache: back over them
        // again starts nothing.
        let began = Instant::now();
        for index in (60..60 + STEPS).rev() {
            pool.frame(&job(index), || true).unwrap();
        }
        let cached_time = began.elapsed();
        assert_eq!(pool.starts, cold_starts);
        eprintln!(
            "MEASURE: {STEPS} frames of 640x360 H.264 with one keyframe, from frame 60: \
             forwards {} decoder start, {:.1} ms a frame; backwards {} decoder starts, \
             {:.1} ms a frame; backwards again from the cache {} starts, {:.3} ms a frame",
            forwards.starts,
            forward_time.as_secs_f64() * 1000.0 / STEPS as f64,
            cold_starts,
            reverse_time.as_secs_f64() * 1000.0 / STEPS as f64,
            pool.starts - cold_starts,
            cached_time.as_secs_f64() * 1000.0 / STEPS as f64,
        );
    }

    /// A lossless clip of `frames` frames whose pictures all differ.
    fn counted_clip(dir: &std::path::Path, size: [u32; 2], frames: u32) -> Option<Asset> {
        let path = dir.join("counted.mkv");
        let made = tool_command("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
            .arg(format!("testsrc2=size={}x{}:rate=30", size[0], size[1]))
            .args(["-frames:v", &frames.to_string(), "-c:v", "ffv1"])
            .arg(&path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !made {
            eprintln!("FFmpeg is unavailable; reverse run fixture skipped");
            return None;
        }
        let probe = format!(
            r#"{{"format":{{"duration":"{}"}},"streams":[{{"codec_type":"video","width":{},"height":{},"avg_frame_rate":"30/1"}}]}}"#,
            f64::from(frames) / 30.0,
            size[0],
            size[1]
        );
        asset_from_probe(&path, probe.as_bytes()).ok()
    }

    fn step_job(asset: &Asset, size: [u32; 2], index: i64) -> FrameJob {
        FrameJob {
            generation: 1,
            sequence: index as u64,
            time: FrameRate::default().frame_time(index),
            lane: Lane::Legacy {
                asset_id: asset.id,
                size,
            },
            size,
            asset: asset.clone(),
        }
    }

    #[test]
    fn a_run_for_a_step_back_is_bounded_and_a_jump_far_back_decodes_one_frame() {
        let temp = tempfile::tempdir().unwrap();
        let size = [64, 32];
        let Some(asset) = counted_clip(temp.path(), size, 120) else {
            return;
        };
        let group = REVERSE_GROUP_FRAMES as i64;
        let mut pool = DecoderPool::new(Processes::default());
        pool.frame(&step_job(&asset, size, 100), || true).unwrap();
        assert_eq!((pool.starts, pool.cache.len()), (1, 1));

        // One step back keeps the run that leads up to it, and no more.
        pool.frame(&step_job(&asset, size, 99), || true).unwrap();
        assert_eq!(pool.starts, 2);
        assert_eq!(pool.cache.len() as i64, 1 + group);
        assert!(pool.bytes <= FRAME_CACHE_BYTES);
        for index in (100 - group..99).rev() {
            pool.frame(&step_job(&asset, size, index), || true).unwrap();
        }
        assert_eq!(pool.starts, 2, "the run serves the steps that follow it");

        // Forwards again from the run's end reuses its pipe.
        pool.frame(&step_job(&asset, size, 101), || true).unwrap();
        assert_eq!(pool.starts, 2);

        // A jump far behind the pipe is a seek: one frame, as before.
        let kept = pool.cache.len();
        pool.frame(&step_job(&asset, size, 10), || true).unwrap();
        assert_eq!(pool.starts, 3);
        assert_eq!(pool.cache.len(), kept + 1);
        // A run near the start of the file does not reach before it.
        pool.frame(&step_job(&asset, size, 9), || true).unwrap();
        assert_eq!(pool.starts, 4);
        assert_eq!(pool.cache.len(), kept + 1 + 10);
    }

    #[test]
    fn a_run_given_up_part_way_keeps_its_frames_and_is_carried_on() {
        let temp = tempfile::tempdir().unwrap();
        let size = [64, 32];
        let Some(asset) = counted_clip(temp.path(), size, 120) else {
            return;
        };
        let mut forwards = DecoderPool::new(Processes::default());
        let expected: Vec<_> = (70..=100)
            .map(|index| {
                forwards
                    .frame(&step_job(&asset, size, index), || true)
                    .unwrap()
            })
            .collect();

        let mut pool = DecoderPool::new(Processes::default());
        pool.frame(&step_job(&asset, size, 100), || true).unwrap();
        // The request is replaced after a few frames of its run were read.
        let asked = std::cell::Cell::new(0);
        let result = pool.frame(&step_job(&asset, size, 99), || {
            asked.set(asked.get() + 1);
            asked.get() <= 6
        });
        assert!(matches!(result, Err(MediaError::Cancelled)));
        assert_eq!(pool.starts, 2);
        let kept = pool.cache.len();
        assert!(kept > 1, "what the run read before it was given up is kept");

        // The step back that replaced it carries the same pipe on.
        for index in (76..=98).rev() {
            let frame = pool.frame(&step_job(&asset, size, index), || true).unwrap();
            assert!(
                frame.rgba == expected[(index - 70) as usize].rgba,
                "frame {index} of a run is not the frame played forwards"
            );
        }
        assert_eq!(pool.starts, 2);
        assert!(pool.bytes <= FRAME_CACHE_BYTES);
    }

    #[test]
    fn a_frame_too_large_for_a_run_steps_back_one_at_a_time() {
        assert_eq!(reverse_group([640, 360]), Some(REVERSE_GROUP_FRAMES as i64));
        // Half the cache holds sixteen frames of the first and four of the
        // second, and a run is two short of that; it does not hold two of
        // the last.
        assert_eq!(reverse_group([960, 540]), Some(14));
        assert_eq!(reverse_group([1920, 1080]), Some(2));
        assert_eq!(reverse_group([4096, 2304]), None);
    }

    impl DecoderPool {
        /// Waits until the look-behind run under way has ended, as it has in
        /// playback by the time its frames are wanted.
        fn settle_behind(&self) {
            let Some(behind) = &self.behind else {
                return;
            };
            let (lock, ready) = &*behind.shared;
            let mut staged = lock.lock().unwrap();
            while !staged.done {
                staged = ready.wait(staged).unwrap();
            }
        }

        /// What is decoded and held: the cache and the room kept for the
        /// look-behind's frames.
        fn held_bytes(&self) -> usize {
            self.bytes + self.behind.as_ref().map_or(0, |behind| behind.reserved)
        }
    }

    /// One step of a walk as the preview worker makes it: the frame, and
    /// then, with the frame published, the look-behind.
    fn walk_step(pool: &mut DecoderPool, asset: &Asset, size: [u32; 2], index: i64) -> Frame {
        let job = step_job(asset, size, index);
        let frame = pool.frame(&job, || true).unwrap();
        pool.look_behind(&job, || true);
        assert!(pool.held_bytes() <= FRAME_CACHE_BYTES, "at frame {index}");
        assert!(pool.decoders.len() + pool.behind_pipes() <= VIDEO_DECODER_LIMIT);
        frame
    }

    #[test]
    fn a_steady_walk_backwards_starts_no_decoder_on_the_frame_path_once_warmed() {
        let temp = tempfile::tempdir().unwrap();
        let size = [64, 32];
        let Some(asset) = counted_clip(temp.path(), size, 240) else {
            return;
        };
        let group = REVERSE_GROUP_FRAMES as i64;
        let mut forwards = DecoderPool::new(Processes::default());
        let expected: Vec<_> = (60..=200)
            .map(|index| {
                forwards
                    .frame(&step_job(&asset, size, index), || true)
                    .unwrap()
            })
            .collect();

        let mut pool = DecoderPool::new(Processes::default());
        walk_step(&mut pool, &asset, size, 200);
        assert_eq!(
            (pool.starts, pool.behind_starts),
            (1, 0),
            "one frame: no walk"
        );
        // The first step back is a run on the frame path, as before; from
        // then on the run before the one shown is decoded behind it.
        walk_step(&mut pool, &asset, size, 199);
        assert_eq!((pool.starts, pool.behind_starts), (2, 1));
        let warmed = pool.starts;
        let mut boundaries = 0;
        for index in (60..199).rev() {
            // Playback shows a run for longer than the next takes to decode.
            pool.settle_behind();
            let behind = pool.behind_starts;
            let frame = walk_step(&mut pool, &asset, size, index);
            assert!(
                frame.rgba == expected[(index - 60) as usize].rgba,
                "frame {index} walked back to is not the frame played forwards"
            );
            assert_eq!(
                pool.starts, warmed,
                "frame {index} waited for a decoder to start"
            );
            boundaries += usize::from(pool.behind_starts != behind);
        }
        assert!(
            boundaries >= ((199 - 60) / group) as usize,
            "{boundaries} runs were decoded behind the walk"
        );
        assert_eq!(pool.behind_starts, 1 + boundaries);
    }

    #[test]
    fn a_frame_the_look_behind_is_on_its_way_to_is_waited_for_and_not_started_over() {
        let temp = tempfile::tempdir().unwrap();
        let size = [64, 32];
        let Some(asset) = counted_clip(temp.path(), size, 240) else {
            return;
        };
        let group = REVERSE_GROUP_FRAMES as i64;
        let mut pool = DecoderPool::new(Processes::default());
        walk_step(&mut pool, &asset, size, 200);
        walk_step(&mut pool, &asset, size, 199);
        let warmed = pool.starts;
        // Straight to the far end of the run being decoded, without letting
        // it finish first.
        for index in [199 - group, 199 - 2 * group, 199 - 3 * group] {
            walk_step(&mut pool, &asset, size, index);
            assert_eq!(pool.starts, warmed, "frame {index} started a decoder");
        }
        // And a request replaced while it waits is given up, not decoded.
        pool.settle_behind();
        walk_step(&mut pool, &asset, size, 198 - 3 * group);
        let job = step_job(&asset, size, 198 - 4 * group);
        if !pool.behind.as_ref().is_some_and(Behind::done) {
            let result = pool.frame(&job, || false);
            assert!(matches!(result, Err(MediaError::Cancelled)));
        }
        assert_eq!(pool.starts, warmed);
    }

    #[test]
    fn a_seek_or_a_turn_forwards_gives_up_the_look_behind_and_what_it_read() {
        let temp = tempfile::tempdir().unwrap();
        let size = [64, 32];
        let Some(asset) = counted_clip(temp.path(), size, 240) else {
            return;
        };
        let group = REVERSE_GROUP_FRAMES as i64;
        let given_up = |shared: &StagedShared| {
            let (lock, ready) = &**shared;
            let mut staged = lock.lock().unwrap();
            assert!(staged.cancelled, "the run was told it is given up");
            while !staged.done {
                staged = ready.wait(staged).unwrap();
            }
            assert!(staged.frames.is_empty(), "and stages nothing more");
        };
        let walking = |pool: &mut DecoderPool| {
            walk_step(pool, &asset, size, 200);
            walk_step(pool, &asset, size, 199);
            walk_step(pool, &asset, size, 198);
            let behind = pool.behind.as_ref().expect("a run is decoded behind");
            assert_eq!((behind.from, behind.until), (200 - 2 * group, 199 - group));
            Arc::clone(&behind.shared)
        };
        let below = |pool: &DecoderPool| {
            (pool.cache.iter())
                .filter(|entry| entry.key.frame <= 199 - group)
                .count()
        };

        // A seek: a frame far from the walk.
        let mut pool = DecoderPool::new(Processes::default());
        let shared = walking(&mut pool);
        let taken = below(&pool);
        walk_step(&mut pool, &asset, size, 20);
        assert!(pool.behind.is_none() && pool.walk.is_none());
        given_up(&shared);
        pool.frame(&step_job(&asset, size, 21), || true).unwrap();
        // Frame 20 and 21, and of the run only what was taken before.
        assert_eq!(below(&pool), taken + 2, "nothing stale reached the cache");
        assert!(pool.held_bytes() == pool.bytes, "no room is kept for it");

        // A turn forwards.
        let mut pool = DecoderPool::new(Processes::default());
        let shared = walking(&mut pool);
        walk_step(&mut pool, &asset, size, 199);
        assert!(pool.behind.is_none() && pool.walk.is_none());
        given_up(&shared);

        // The lane is retired, or everything is started over.
        let mut pool = DecoderPool::new(Processes::default());
        let shared = walking(&mut pool);
        pool.retain_lanes(&[]);
        assert!(pool.behind.is_none() && pool.walk.is_none());
        given_up(&shared);
        let mut pool = DecoderPool::new(Processes::default());
        let shared = walking(&mut pool);
        pool.clear();
        assert!(pool.behind.is_none() && pool.walk.is_none() && pool.cache.is_empty());
        given_up(&shared);
    }

    #[test]
    fn the_look_behind_takes_a_free_pipe_only_and_gives_it_up_for_a_frame_asked_for() {
        let temp = tempfile::tempdir().unwrap();
        let size = [64, 32];
        let Some(asset) = counted_clip(temp.path(), size, 240) else {
            return;
        };
        let other = |lane: u32, index: i64| FrameJob {
            lane: Lane::Legacy {
                asset_id: asset.id,
                size: [64, 32 + lane],
            },
            ..step_job(&asset, size, index)
        };
        let mut pool = DecoderPool::new(Processes::default());
        // Four other lanes and the walking one fill the bank.
        for lane in 1..=4 {
            pool.frame(&other(lane, 10 * i64::from(lane)), || true)
                .unwrap();
        }
        walk_step(&mut pool, &asset, size, 200);
        walk_step(&mut pool, &asset, size, 199);
        assert_eq!(pool.decoders.len(), VIDEO_DECODER_LIMIT);
        assert!(
            pool.behind.is_none(),
            "no pipe is free: nothing is decoded behind"
        );
        assert_eq!(pool.behind_starts, 0);

        // With one free it is taken, and given back when a lane needs it.
        pool.retain_lanes(&[
            other(1, 0).lane,
            other(2, 0).lane,
            other(3, 0).lane,
            step_job(&asset, size, 0).lane,
        ]);
        walk_step(&mut pool, &asset, size, 198);
        assert_eq!(pool.behind_starts, 1);
        if pool.behind_pipes() == 1 {
            pool.frame(&other(4, 50), || true).unwrap();
            assert!(pool.behind.is_none(), "the frame asked for took its pipe");
        }
        assert!(pool.decoders.len() <= VIDEO_DECODER_LIMIT);
    }

    /// What a steady walk backwards costs a step at the size a 1080p clip is
    /// previewed at, printed for whoever reads the run: each step is timed
    /// on the frame path alone, and the walk is paced as playback paces it.
    #[test]
    fn a_paced_walk_backwards_at_preview_size_is_measured() {
        let _heavy = crate::heavy_test();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("hd.mp4");
        let encoded = tool_command("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
            .arg("testsrc2=size=1920x1080:rate=30")
            .args([
                "-t", "5", "-c:v", "libx264", "-g", "250", "-pix_fmt", "yuv420p",
            ])
            .arg(&path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !encoded {
            eprintln!("FFmpeg with libx264 is unavailable; paced reverse fixture skipped");
            return;
        }
        let asset = asset_from_probe(&path, br#"{"format":{"duration":"5"},"streams":[{"codec_type":"video","width":1920,"height":1080,"avg_frame_rate":"30/1"}]}"#).unwrap();
        let size = [960, 540];
        let group = reverse_group(size).unwrap();
        const TICKS: i64 = 90;
        let tick = Duration::from_secs_f64(1.0 / 30.0);
        // As playback asks: at each thirtieth of a second, the frame that is
        // due then. A step that took longer than that skips the frames it
        // made late.
        let paced = |look_behind: bool| {
            let mut pool = DecoderPool::new(Processes::default());
            let began = Instant::now();
            let mut waits = Vec::new();
            let mut due = 0;
            while due < TICKS {
                thread::sleep(
                    (began + tick * due as u32).saturating_duration_since(Instant::now()),
                );
                let job = step_job(&asset, size, 140 - due);
                let asked = Instant::now();
                pool.frame(&job, || true).unwrap();
                waits.push(asked.elapsed());
                if look_behind {
                    pool.look_behind(&job, || true);
                }
                assert!(pool.held_bytes() <= FRAME_CACHE_BYTES);
                due = (due + 1)
                    .max((began.elapsed().as_secs_f64() / tick.as_secs_f64()).ceil() as i64);
            }
            // The first two steps are the seek and the first run.
            let steady = &waits[2..];
            let longest = steady.iter().max().unwrap().as_secs_f64() * 1000.0;
            let late: Vec<usize> = (steady.iter().enumerate())
                .filter(|(_, wait)| **wait > tick)
                .map(|(step, _)| step + 3)
                .collect();
            (pool.starts, pool.behind_starts, waits.len(), longest, late)
        };
        let (starts, _, shown, longest, late) = paced(false);
        let (ahead_starts, behind, ahead_shown, ahead_longest, ahead_late) = paced(true);
        eprintln!(
            "MEASURE: {TICKS} thirtieths of a second walked back through 1920x1080 H.264 with \
             one keyframe, previewed at 960x540 in runs of {group}: without the look-behind \
             {shown} frames shown, {starts} decoder starts on the frame path, longest step \
             {longest:.1} ms, steps over 33 ms {late:?}; with it {ahead_shown} frames shown, \
             {ahead_starts} starts on the frame path and {behind} behind it, longest step \
             {ahead_longest:.1} ms, steps over 33 ms {ahead_late:?} (both after the first two \
             steps, which are the seek and the first run; steps are numbered from one)"
        );
        assert_eq!(ahead_starts, 2, "the seek and the first run, and no more");
    }
}
