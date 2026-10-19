//! The proxy lane: one worker that writes smaller copies of large videos for
//! the preview to read in their place, and the cache that keeps them.
//!
//! A proxy is derived data. It has the original's frame rate on the same
//! `fps=` grid the preview decodes with, so it has the same frames at the
//! same times; its picture is at most [`PROXY_SIDE`] on the long side with
//! the original's aspect and its rotation applied; it has no sound. It is
//! H.264 with a short group of pictures, or MJPEG when FFmpeg here has no
//! libx264. Its frame grid starts at the start of the source file, so that a
//! time seeks to the same frame in both even when the picture starts after
//! the sound. A video with transparency has none, as neither codec keeps
//! it: asked for one, the lane fails and the preview reads the original.
//! Export never reads one, and none is named in a project.
//!
//! The cache is a directory of `<key>.mp4` (or `.mov`) files, each with a
//! `<key>.json` manifest beside it. The key is a SHA-256 of the source's
//! place, its length and modification time on disk, what the asset says of
//! it and [`PROXY_PROFILE`]: a changed source or a changed recipe has another
//! key. The manifests are read on the worker when the service starts; after
//! that a lookup reads only memory. The least recently used proxies are
//! removed to hold the directory to a budget, except those the preview has
//! open, and the caller is told which were.
//!
//! The lane has its own thread, queue and FFmpeg process, apart from the
//! preview, import and analysis workers. An answer names the asset and the
//! source it was read from, for the caller to drop it when the asset is no
//! longer that file.
use crate::{
    MediaError, SourceIdentity, Wake, io,
    process::{Process, Processes, capture, tool_command},
};
use editor_core::{Asset, AssetId, FrameRate, MediaKind, Time};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

/// Proxies that may wait behind the one being written.
pub const PROXY_QUEUE: usize = 64;
/// The longest side of a proxy's picture.
pub const PROXY_SIDE: u32 = 1280;
/// How many bytes of proxies the cache keeps unless told otherwise.
pub const PROXY_BUDGET: u64 = 20 * 1024 * 1024 * 1024;
/// The recipe a proxy was written with. A proxy of another is not used.
pub const PROXY_PROFILE: u32 = 2;
/// A video longer than this on a side wants a proxy.
pub const PROXY_ABOVE_SIDE: u32 = 1920;
/// A video read at more than this many bits a second wants a proxy.
pub const PROXY_ABOVE_BITRATE: u64 = 50_000_000;

// What a write that never ended leaves behind is removed when it is older.
const LEFT_OVER: Duration = Duration::from_secs(3600);
// Failures remembered for lookups; the oldest is forgotten first.
const FAILURES: usize = 256;
// Answers that may wait to be read.
const EVENTS: usize = 1024;
// How long FFmpeg may say nothing while it writes, and take to exit after.
const SILENCE: Duration = Duration::from_secs(120);
const EXIT: Duration = Duration::from_secs(30);
// No manifest is larger.
const MANIFEST_BYTES: u64 = 64 * 1024;
// How often FFmpeg tells how far it has come, in seconds.
const STATS_PERIOD: &str = "0.25";
// How long `ProxyService::shutdown` waits for the proxy being written to
// stop.
const SHUTDOWN_GRACE: Duration = Duration::from_millis(750);

/// Whether a video is large enough that the preview should read a proxy of
/// it when proxies are automatic: longer than [`PROXY_ABOVE_SIDE`] on a side,
/// or more than [`PROXY_ABOVE_BITRATE`] bits a second over its length.
pub fn wants_proxy(asset: &Asset) -> bool {
    if asset.kind != MediaKind::Video || asset.width == 0 || asset.height == 0 {
        return false;
    }
    if asset.width.max(asset.height) > PROXY_ABOVE_SIDE {
        return true;
    }
    let seconds = asset.duration.as_seconds();
    seconds > 0.0 && asset.file_size as f64 * 8.0 / seconds > PROXY_ABOVE_BITRATE as f64
}

/// The picture size of a proxy of a `width` by `height` video: no longer
/// than [`PROXY_SIDE`] on a side and never larger than the video, with its
/// aspect, and even each way for 4:2:0.
pub fn proxy_size(width: u32, height: u32) -> [u32; 2] {
    let long = width.max(height).max(1);
    let ratio = (f64::from(PROXY_SIDE) / f64::from(long)).min(1.0);
    let even = |side: u32| (((f64::from(side) * ratio) / 2.0).round() as u32 * 2).max(2);
    [even(width), even(height)]
}

/// A proxy that may be read for its source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proxy {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    /// The source as it was when the proxy was written.
    pub source: SourceIdentity,
}

/// What there is of a proxy for an asset as it is now.
#[derive(Clone, Debug, PartialEq)]
pub enum ProxyStatus {
    None,
    Queued,
    /// Being written, `0..=1`.
    Generating(f32),
    Ready(Proxy),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProxyEvent {
    /// How much of a proxy is written, `0..=1`. Only the newest of an asset
    /// waits to be read.
    Progress {
        asset: AssetId,
        source: SourceIdentity,
        done: f32,
    },
    /// A proxy can be read: one just written, or one the cache had.
    Ready { asset: AssetId, proxy: Proxy },
    /// A proxy could not be written. One that was cancelled or replaced
    /// says nothing.
    Failed {
        asset: AssetId,
        source: SourceIdentity,
        message: String,
    },
    /// The budget took these proxies out of the cache to make room, and
    /// their files are gone or going. An asset said to have one of them
    /// ready has none now; asked again, it is written again. Proxies taken
    /// out by `remove` or `clear`, or written over, are not told of.
    Removed { proxies: Vec<Proxy> },
}

impl ProxyEvent {
    fn asset(&self) -> Option<AssetId> {
        match self {
            Self::Progress { asset, .. }
            | Self::Ready { asset, .. }
            | Self::Failed { asset, .. } => Some(*asset),
            Self::Removed { .. } => None,
        }
    }
}

/// Whether `source` is what `asset` says of its file now.
fn is_of(source: &SourceIdentity, asset: &Asset) -> bool {
    source.path == asset.path
        && source.file_size == asset.file_size
        && source.duration == asset.duration
        && source.width == asset.width
        && source.height == asset.height
        && source.frame_rate == asset.frame_rate
}

/// What a source file is on disk when its proxy is written.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Stamp {
    canonical: PathBuf,
    length: u64,
    /// Nanoseconds since the epoch; nought when the disk cannot say.
    modified: u64,
}

impl Stamp {
    fn read(path: &Path) -> std::io::Result<Self> {
        let canonical = fs::canonicalize(path)?;
        let file = fs::metadata(&canonical)?;
        if !file.is_file() {
            return Err(std::io::Error::new(ErrorKind::InvalidInput, "not a file"));
        }
        let modified = (file.modified().ok())
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |since| since.as_nanos().min(u128::from(u64::MAX)) as u64);
        Ok(Self {
            canonical,
            length: file.len(),
            modified,
        })
    }
}

/// The name a proxy of `source`, as `stamp` finds it, is kept under.
fn key(source: &SourceIdentity, stamp: &Stamp) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"neptune-video-proxy\0");
    hasher.update(PROXY_PROFILE.to_le_bytes());
    for path in [&stamp.canonical, &source.path] {
        let bytes = path.as_os_str().as_encoded_bytes();
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    for word in [stamp.length, stamp.modified, source.file_size] {
        hasher.update(word.to_le_bytes());
    }
    hasher.update(source.duration.0.to_le_bytes());
    let rate = (source.frame_rate).map_or([0, 0], |rate| [rate.numerator, rate.denominator]);
    for word in [source.width, source.height, rate[0], rate[1]] {
        hasher.update(word.to_le_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_key(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// How a proxy's picture is coded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Codec {
    /// libx264 in MP4.
    H264,
    /// MJPEG in QuickTime, when FFmpeg has no libx264.
    Mjpeg,
}

impl Codec {
    fn extension(self) -> &'static str {
        match self {
            Self::H264 => "mp4",
            Self::Mjpeg => "mov",
        }
    }
}

/// What is written beside a proxy: enough to know, without reading the
/// proxy or its source, whether it is still of the source.
#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    profile: u32,
    file: String,
    codec: Codec,
    width: u32,
    height: u32,
    frames: u64,
    bytes: u64,
    stamp: Stamp,
    source: KeptSource,
}

/// A [`SourceIdentity`] as a manifest holds it.
#[derive(Debug, Serialize, Deserialize)]
struct KeptSource {
    path: PathBuf,
    file_size: u64,
    duration: i64,
    width: u32,
    height: u32,
    frame_rate: Option<[u32; 2]>,
}

impl KeptSource {
    fn of(source: &SourceIdentity) -> Self {
        Self {
            path: source.path.clone(),
            file_size: source.file_size,
            duration: source.duration.0,
            width: source.width,
            height: source.height,
            frame_rate: (source.frame_rate).map(|rate| [rate.numerator, rate.denominator]),
        }
    }

    fn identity(&self) -> SourceIdentity {
        SourceIdentity {
            path: self.path.clone(),
            file_size: self.file_size,
            duration: Time(self.duration),
            width: self.width,
            height: self.height,
            frame_rate: (self.frame_rate).map(|[numerator, denominator]| FrameRate {
                numerator,
                denominator,
            }),
        }
    }
}

/// A proxy the cache holds.
#[derive(Clone, Debug)]
struct Entry {
    proxy: Proxy,
    manifest: PathBuf,
    bytes: u64,
    /// When the preview last opened it, or when it was written.
    used: SystemTime,
    stamp: Stamp,
}

/// One proxy to write, as the runner is told it.
pub(crate) struct Encode {
    pub asset: Asset,
    /// Where to write it; the worker moves it into place.
    pub part: PathBuf,
    pub size: [u32; 2],
    /// How many frames the source's grid has over its length.
    pub frames: u64,
}

/// What the runner wrote.
pub(crate) struct Encoded {
    pub codec: Codec,
    pub frames: u64,
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

type Runner = Arc<dyn Fn(&Encode, &Control) -> Result<Encoded, MediaError> + Send + Sync>;

struct Job {
    asset: Asset,
    source: SourceIdentity,
}

struct Running {
    asset: AssetId,
    source: SourceIdentity,
    done: f32,
    cancel: Arc<AtomicBool>,
}

/// What was asked of the cache before its directory was read.
enum Op {
    Remove(SourceIdentity),
    Clear,
}

struct Lane {
    queue: VecDeque<Job>,
    running: Option<Running>,
    events: VecDeque<ProxyEvent>,
    /// The proxies kept, by the path of their source.
    index: HashMap<PathBuf, Vec<Entry>>,
    loaded: bool,
    pending: Vec<Op>,
    failed: VecDeque<(AssetId, SourceIdentity, String)>,
    /// Proxies the preview has open: never removed while they are.
    in_use: HashSet<PathBuf>,
    /// Files to remove, once nothing has them open.
    doomed: Vec<PathBuf>,
    /// Manifests whose proxies were just opened: marked as used now.
    touch: Vec<PathBuf>,
    /// Stale part files are looked for at the next tidying.
    sweep: bool,
    /// The proxy written last: kept by the budget even when it alone is
    /// over it, as it was just asked for.
    newest: Option<PathBuf>,
    budget: u64,
    /// The worker has files to remove, to mark or to make room among.
    tidy: bool,
    stopped: bool,
}

impl Lane {
    fn new(budget: u64) -> Self {
        Self {
            queue: VecDeque::new(),
            running: None,
            events: VecDeque::new(),
            index: HashMap::new(),
            loaded: false,
            pending: Vec::new(),
            failed: VecDeque::new(),
            in_use: HashSet::new(),
            doomed: Vec::new(),
            touch: Vec::new(),
            sweep: false,
            newest: None,
            budget,
            tidy: false,
            stopped: false,
        }
    }

    fn find(&self, asset: &Asset) -> Option<&Entry> {
        (self.index.get(&asset.path)?.iter()).find(|entry| is_of(&entry.proxy.source, asset))
    }

    fn take(&mut self, source: &SourceIdentity) -> Option<Entry> {
        let entries = self.index.get_mut(&source.path)?;
        let at = entries
            .iter()
            .position(|entry| entry.proxy.source == *source)?;
        let entry = entries.swap_remove(at);
        if entries.is_empty() {
            self.index.remove(&source.path);
        }
        Some(entry)
    }

    fn insert(&mut self, entry: Entry) {
        if let Some(old) = self.take(&entry.proxy.source) {
            self.doom(old);
        }
        // Files of the same name taken out before were written over.
        (self.doomed).retain(|path| *path != entry.proxy.path && *path != entry.manifest);
        (self.index.entry(entry.proxy.source.path.clone()))
            .or_default()
            .push(entry);
    }

    /// Takes a proxy out of the cache at once; its files go when they can.
    /// The manifest goes first, so that a proxy whose file is held open is
    /// not found again by the next session.
    fn doom(&mut self, entry: Entry) {
        self.doomed.push(entry.manifest);
        self.doomed.push(entry.proxy.path);
        self.tidy = true;
    }

    fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.index.values().flatten()
    }

    fn usage(&self) -> (u64, usize) {
        self.entries().fold((0, 0), |(bytes, count), entry| {
            (bytes + entry.bytes, count + 1)
        })
    }

    fn apply(&mut self, op: Op) {
        match op {
            Op::Remove(source) => {
                if let Some(entry) = self.take(&source) {
                    self.doom(entry);
                }
            }
            Op::Clear => {
                let entries: Vec<Entry> = self.index.drain().flat_map(|(_, all)| all).collect();
                for entry in entries {
                    self.doom(entry);
                }
                self.sweep = true;
                self.tidy = true;
            }
        }
    }

    /// Takes the least recently used proxies out until the rest fit the
    /// budget. Those open in the preview, and the newest, stay. Those taken
    /// out are told of in one [`ProxyEvent::Removed`]; true when there are
    /// any.
    fn trim(&mut self) -> bool {
        let (mut bytes, _) = self.usage();
        if bytes <= self.budget {
            return false;
        }
        let mut order: Vec<(SystemTime, SourceIdentity, u64)> = (self.entries())
            .filter(|entry| {
                !self.in_use.contains(&entry.proxy.path)
                    && self.newest.as_ref() != Some(&entry.proxy.path)
            })
            .map(|entry| (entry.used, entry.proxy.source.clone(), entry.bytes))
            .collect();
        order.sort_by_key(|(used, ..)| *used);
        let mut removed = Vec::new();
        for (_, source, size) in order {
            if bytes <= self.budget {
                break;
            }
            if let Some(entry) = self.take(&source) {
                bytes -= size;
                removed.push(entry.proxy.clone());
                self.doom(entry);
            }
        }
        if removed.is_empty() {
            return false;
        }
        self.push(ProxyEvent::Removed { proxies: removed });
        true
    }

    fn forget_events(&mut self, asset: AssetId) {
        self.events.retain(|event| event.asset() != Some(asset));
    }

    fn forget_failure(&mut self, asset: AssetId) {
        self.failed.retain(|(of, ..)| *of != asset);
    }

    fn push(&mut self, event: ProxyEvent) {
        if self.events.len() >= EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }
}

struct Shared {
    lane: Mutex<Lane>,
    ready: Condvar,
}

/// Nonblocking caller interface of the proxy lane and its cache.
pub struct ProxyService {
    shared: Arc<Shared>,
    processes: Processes,
}

impl ProxyService {
    /// A lane that keeps its proxies in `dir`, at most `budget` bytes of
    /// them. The directory is made and read on the lane's thread.
    pub fn new(
        wake: impl Fn() + Send + Sync + 'static,
        dir: PathBuf,
        budget: u64,
    ) -> Result<Self, MediaError> {
        let processes = Processes::default();
        let children = processes.clone();
        let codec = Arc::new(Mutex::new(None));
        Self::with_runner(
            Arc::new(wake),
            dir,
            budget,
            processes,
            Arc::new(move |job, control| encode(job, &children, &codec, false, control)),
        )
    }

    fn with_runner(
        wake: Wake,
        dir: PathBuf,
        budget: u64,
        processes: Processes,
        run: Runner,
    ) -> Result<Self, MediaError> {
        let shared = Arc::new(Shared {
            lane: Mutex::new(Lane::new(budget)),
            ready: Condvar::new(),
        });
        let state = Arc::clone(&shared);
        thread::Builder::new()
            .name("neptune-video-proxy".into())
            .spawn(move || worker(state, dir, run, wake))
            .map_err(|error| io("Cannot start proxy worker", error))?;
        Ok(Self { shared, processes })
    }

    /// Asks for a proxy of a video. One waiting or being written for the
    /// asset as it is now is not asked for again; one for what the asset
    /// was before is given up. When the cache has one, the worker checks
    /// that its source is unchanged on disk and says it is ready, or writes
    /// it again. `Busy` when [`PROXY_QUEUE`] others wait.
    pub fn request(&self, asset: &Asset) -> Result<(), MediaError> {
        if asset.kind != MediaKind::Video {
            return Err(MediaError::Invalid("Only a video has a proxy".into()));
        }
        if asset.width == 0 || asset.height == 0 {
            return Err(MediaError::Invalid("Media has no video dimensions".into()));
        }
        let mut lane = self.shared.lane.lock().map_err(|_| MediaError::Closed)?;
        if lane.stopped {
            return Err(MediaError::Closed);
        }
        let running = lane.running.as_ref();
        if running.is_some_and(|running| running.asset == asset.id && is_of(&running.source, asset))
            || (lane.queue.iter()).any(|job| job.asset.id == asset.id && is_of(&job.source, asset))
        {
            return Ok(());
        }
        let replaced = (lane.queue.iter())
            .filter(|job| job.asset.id == asset.id)
            .count();
        if lane.queue.len() - replaced >= PROXY_QUEUE {
            return Err(MediaError::Busy);
        }
        lane.queue.retain(|job| job.asset.id != asset.id);
        lane.forget_events(asset.id);
        lane.forget_failure(asset.id);
        self.stop_running(&mut lane, asset.id);
        lane.queue.push_back(Job {
            asset: asset.clone(),
            source: SourceIdentity::of(asset),
        });
        self.shared.ready.notify_all();
        Ok(())
    }

    /// Gives up the proxy asked for an asset, waiting or being written. It
    /// answers nothing, and what it had answered and nobody read is dropped.
    pub fn cancel(&self, asset: AssetId) {
        let Ok(mut lane) = self.shared.lane.lock() else {
            return;
        };
        lane.queue.retain(|job| job.asset.id != asset);
        lane.forget_events(asset);
        self.stop_running(&mut lane, asset);
    }

    /// Gives up every proxy asked for: the project is another. Proxies the
    /// cache has stay. It does not wait.
    pub fn cancel_all(&self) {
        let Ok(mut lane) = self.shared.lane.lock() else {
            return;
        };
        lane.queue.clear();
        lane.events.clear();
        lane.failed.clear();
        if let Some(asset) = lane.running.as_ref().map(|running| running.asset) {
            self.stop_running(&mut lane, asset);
        }
    }

    /// Gives up what is asked for an asset and takes the proxy of it as it
    /// is now out of the cache. Its files are removed once the preview no
    /// longer has them open.
    pub fn remove(&self, asset: &Asset) {
        self.cancel(asset.id);
        let Ok(mut lane) = self.shared.lane.lock() else {
            return;
        };
        lane.forget_failure(asset.id);
        let op = Op::Remove(SourceIdentity::of(asset));
        if lane.loaded {
            lane.apply(op);
        } else {
            lane.pending.push(op);
        }
        self.shared.ready.notify_all();
    }

    /// Gives up everything asked for and empties the cache. Proxies the
    /// preview has open are removed once it lets them go.
    pub fn clear(&self) {
        self.cancel_all();
        let Ok(mut lane) = self.shared.lane.lock() else {
            return;
        };
        if lane.loaded {
            lane.apply(Op::Clear);
        } else {
            lane.pending.push(Op::Clear);
        }
        self.shared.ready.notify_all();
    }

    /// Holds the cache to `bytes`, removing the least recently used proxies
    /// that are not open.
    pub fn set_budget(&self, bytes: u64) {
        let Ok(mut lane) = self.shared.lane.lock() else {
            return;
        };
        if lane.budget != bytes {
            lane.budget = bytes;
            lane.tidy = true;
            self.shared.ready.notify_all();
        }
    }

    /// The proxies the preview has open now, by [`Proxy::path`]. They are
    /// not removed while they are, by the budget, `remove` or `clear`; one
    /// that is opened counts as used now. Cheap when nothing changed.
    pub fn set_in_use(&self, open: &[PathBuf]) {
        let Ok(mut lane) = self.shared.lane.lock() else {
            return;
        };
        if open.len() == lane.in_use.len() && open.iter().all(|path| lane.in_use.contains(path)) {
            return;
        }
        let now = SystemTime::now();
        let opened: HashSet<&PathBuf> = (open.iter())
            .filter(|path| !lane.in_use.contains(*path))
            .collect();
        let mut touch = Vec::new();
        for entry in lane.index.values_mut().flatten() {
            if opened.contains(&entry.proxy.path) {
                entry.used = now;
                touch.push(entry.manifest.clone());
            }
        }
        lane.touch.extend(touch);
        lane.in_use = open.iter().cloned().collect();
        lane.tidy = true;
        self.shared.ready.notify_all();
    }

    /// What there is of a proxy for the asset as it is now. It reads only
    /// memory: before the cache's directory has been read it knows none.
    pub fn lookup(&self, asset: &Asset) -> ProxyStatus {
        let Ok(lane) = self.shared.lane.lock() else {
            return ProxyStatus::None;
        };
        if let Some(entry) = lane.find(asset) {
            return ProxyStatus::Ready(entry.proxy.clone());
        }
        if let Some(running) = (lane.running.as_ref())
            .filter(|running| running.asset == asset.id && is_of(&running.source, asset))
        {
            return ProxyStatus::Generating(running.done);
        }
        if (lane.queue.iter()).any(|job| job.asset.id == asset.id && is_of(&job.source, asset)) {
            return ProxyStatus::Queued;
        }
        (lane.failed.iter())
            .find(|(of, source, _)| *of == asset.id && is_of(source, asset))
            .map_or(ProxyStatus::None, |(.., message)| {
                ProxyStatus::Failed(message.clone())
            })
    }

    pub fn poll(&self) -> Option<ProxyEvent> {
        self.shared.lane.lock().ok()?.events.pop_front()
    }

    /// The bytes and the number of proxies the cache holds.
    pub fn usage(&self) -> (u64, usize) {
        (self.shared.lane.lock()).map_or((0, 0), |lane| lane.usage())
    }

    /// Whether the cache's directory has been read, so that a lookup that
    /// finds nothing means there is nothing.
    pub fn loaded(&self) -> bool {
        (self.shared.lane.lock()).is_ok_and(|lane| lane.loaded)
    }

    /// Whether anything waits or is being written.
    pub fn busy(&self) -> bool {
        (self.shared.lane.lock()).is_ok_and(|lane| lane.running.is_some() || !lane.queue.is_empty())
    }

    /// Stops the lane for good, for a closing window: gives up what waits
    /// and what is being written and kills its FFmpeg, then waits at most
    /// 750 ms for the worker to reap it and remove its part
    /// file. Joins no thread. Nothing can be asked afterwards; the cache
    /// stays as it is.
    pub fn shutdown(&self) {
        let Ok(mut lane) = self.shared.lane.lock() else {
            return;
        };
        lane.stopped = true;
        lane.queue.clear();
        lane.events.clear();
        if let Some(running) = &lane.running {
            running.cancel.store(true, Ordering::Release);
        }
        // Under the lock, as `stop_running` does: no later job is started.
        self.processes.kill_all();
        self.shared.ready.notify_all();
        let until = Instant::now() + SHUTDOWN_GRACE;
        while lane.running.is_some() {
            let now = Instant::now();
            if now >= until {
                break;
            }
            let Ok((next, _)) = self.shared.ready.wait_timeout(lane, until - now) else {
                return;
            };
            lane = next;
        }
    }

    // The lane has one process at a time, and the worker names the job it
    // is on under the lock before it starts one: killing every process here,
    // with the lock held, kills that job's and no later one's.
    fn stop_running(&self, lane: &mut Lane, asset: AssetId) {
        if let Some(running) = (lane.running.as_ref()).filter(|running| running.asset == asset) {
            running.cancel.store(true, Ordering::Release);
            self.processes.kill_all();
        }
    }
}

impl Drop for ProxyService {
    fn drop(&mut self) {
        if let Ok(mut lane) = self.shared.lane.lock() {
            lane.stopped = true;
            lane.queue.clear();
            if let Some(running) = &lane.running {
                running.cancel.store(true, Ordering::Release);
            }
        }
        self.shared.ready.notify_all();
        self.processes.kill_all();
        // The worker removes its part file itself; a closing window joins
        // nothing.
    }
}

/// What the worker does next.
enum Work {
    Tidy,
    Make(Job, Arc<AtomicBool>),
}

fn worker(shared: Arc<Shared>, dir: PathBuf, run: Runner, wake: Wake) {
    let found = load(&dir);
    {
        let Ok(mut lane) = shared.lane.lock() else {
            return;
        };
        if lane.stopped {
            return;
        }
        for entry in found.entries {
            lane.insert(entry);
        }
        lane.doomed.extend(found.discard);
        lane.loaded = true;
        for op in std::mem::take(&mut lane.pending) {
            lane.apply(op);
        }
        lane.tidy = true;
    }
    wake();
    loop {
        let work = {
            let Ok(mut lane) = shared.lane.lock() else {
                return;
            };
            loop {
                if lane.stopped {
                    return;
                }
                if lane.tidy {
                    lane.tidy = false;
                    break Work::Tidy;
                }
                if let Some(job) = lane.queue.pop_front() {
                    let cancel = Arc::new(AtomicBool::new(false));
                    lane.running = Some(Running {
                        asset: job.asset.id,
                        source: job.source.clone(),
                        done: 0.0,
                        cancel: Arc::clone(&cancel),
                    });
                    break Work::Make(job, cancel);
                }
                let Ok(next) = shared.ready.wait(lane) else {
                    return;
                };
                lane = next;
            }
        };
        match work {
            Work::Tidy => {
                if tidy(&shared, &dir) {
                    wake();
                }
            }
            Work::Make(job, cancel) => {
                if !make(&shared, &dir, &run, &wake, job, &cancel) {
                    return;
                }
            }
        }
    }
}

/// Writes one proxy, or finds the one kept still good, and says so. False
/// when the service has gone.
fn make(
    shared: &Shared,
    dir: &Path,
    run: &Runner,
    wake: &Wake,
    job: Job,
    cancel: &AtomicBool,
) -> bool {
    let asset = job.asset.id;
    let report = |done: f32| {
        let Ok(mut lane) = shared.lane.lock() else {
            return;
        };
        if cancel.load(Ordering::Acquire) || lane.stopped {
            return;
        }
        if let Some(running) = lane.running.as_mut() {
            running.done = done;
        }
        // One figure of a proxy waits to be read, the newest.
        let waiting = lane.events.iter_mut().find_map(|event| match event {
            ProxyEvent::Progress {
                asset: of, done, ..
            } if *of == asset => Some(done),
            _ => None,
        });
        match waiting {
            Some(waiting) => *waiting = done,
            None => lane.push(ProxyEvent::Progress {
                asset,
                source: job.source.clone(),
                done,
            }),
        }
        drop(lane);
        wake();
    };
    let control = Control {
        cancel,
        report: &report,
    };
    let made = produce(shared, dir, run, &job, &control);
    let Ok(mut lane) = shared.lane.lock() else {
        return false;
    };
    lane.running = None;
    // `shutdown` waits for this.
    shared.ready.notify_all();
    if lane.stopped {
        return false;
    }
    // The flag is read under the lock that set it: a job given up before
    // this answers nothing, and one given up after has its answer taken out
    // of the queue by whoever gave it up.
    if cancel.load(Ordering::Acquire) {
        if let Ok(Made::New(entry)) = made {
            lane.doom(entry);
        }
        return true;
    }
    lane.forget_events(asset);
    match made {
        Ok(Made::Kept(proxy)) => lane.push(ProxyEvent::Ready { asset, proxy }),
        Ok(Made::New(entry)) => {
            let proxy = entry.proxy.clone();
            lane.insert(entry);
            lane.newest = Some(proxy.path.clone());
            lane.tidy = true;
            lane.push(ProxyEvent::Ready { asset, proxy });
        }
        Err(error) => {
            let message = error.to_string();
            if lane.failed.len() >= FAILURES {
                lane.failed.pop_front();
            }
            (lane.failed).push_back((asset, job.source.clone(), message.clone()));
            lane.push(ProxyEvent::Failed {
                asset,
                source: job.source,
                message,
            });
        }
    }
    drop(lane);
    wake();
    true
}

enum Made {
    /// The cache had it, and its source is as it was.
    Kept(Proxy),
    New(Entry),
}

fn produce(
    shared: &Shared,
    dir: &Path,
    run: &Runner,
    job: &Job,
    control: &Control,
) -> Result<Made, MediaError> {
    let unreadable = |error: std::io::Error| {
        MediaError::Invalid(format!(
            "{} cannot be read: {error}",
            job.asset.path.display()
        ))
    };
    let kept = (shared.lane.lock())
        .map_err(|_| MediaError::Closed)?
        .find(&job.asset)
        .cloned();
    let stamp = Stamp::read(&job.asset.path).map_err(unreadable)?;
    if let Some(kept) = kept {
        if kept.stamp == stamp && kept.proxy.path.is_file() {
            return Ok(Made::Kept(kept.proxy));
        }
        // The file changed on disk under the same name and size.
        let mut lane = shared.lane.lock().map_err(|_| MediaError::Closed)?;
        if let Some(entry) = lane.take(&kept.proxy.source) {
            lane.doom(entry);
        }
    }
    if control.cancelled() {
        return Err(MediaError::Cancelled);
    }
    let name = key(&job.source, &stamp);
    fs::create_dir_all(dir).map_err(|error| io("Cannot make the proxy folder", error))?;
    let part = dir.join(format!("{name}.{}.part", Uuid::new_v4().simple()));
    let size = proxy_size(job.asset.width, job.asset.height);
    let rate = job.asset.frame_rate.unwrap_or_default();
    let frames = (rate.frame_at(Time(job.asset.duration.0.max(1) - 1)) + 1).max(1) as u64;
    let encode = Encode {
        asset: job.asset.clone(),
        part: part.clone(),
        size,
        frames,
    };
    let encoded = match run(&encode, control) {
        Ok(_) if control.cancelled() => Err(MediaError::Cancelled),
        outcome => outcome,
    };
    let encoded = match encoded {
        Ok(encoded) => encoded,
        Err(error) => {
            let _ = fs::remove_file(&part);
            return Err(error);
        }
    };
    let file = format!("{name}.{}", encoded.codec.extension());
    let path = dir.join(&file);
    let placed = fs::rename(&part, &path).and_then(|()| fs::metadata(&path));
    let bytes = match placed {
        Ok(found) => found.len(),
        Err(error) => {
            let _ = fs::remove_file(&part);
            return Err(io("Cannot keep the proxy", error));
        }
    };
    // A proxy of the same source written with the other codec is of no use.
    for codec in [Codec::H264, Codec::Mjpeg] {
        if codec != encoded.codec {
            let _ = fs::remove_file(dir.join(format!("{name}.{}", codec.extension())));
        }
    }
    let manifest = Manifest {
        profile: PROXY_PROFILE,
        file,
        codec: encoded.codec,
        width: size[0],
        height: size[1],
        frames: encoded.frames,
        bytes,
        stamp: stamp.clone(),
        source: KeptSource::of(&job.source),
    };
    let manifest_path = dir.join(format!("{name}.json"));
    if let Err(error) = write_manifest(&manifest_path, &manifest) {
        let _ = fs::remove_file(&path);
        return Err(error);
    }
    Ok(Made::New(Entry {
        proxy: Proxy {
            path,
            width: size[0],
            height: size[1],
            source: job.source.clone(),
        },
        manifest: manifest_path,
        bytes,
        used: SystemTime::now(),
        stamp,
    }))
}

/// Writes a manifest whole beside its place and then moves it there, so a
/// reader never finds half of one.
fn write_manifest(path: &Path, manifest: &Manifest) -> Result<(), MediaError> {
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|error| MediaError::Invalid(format!("Cannot describe the proxy: {error}")))?;
    let mut part = path.as_os_str().to_owned();
    part.push(format!(".{}.part", Uuid::new_v4().simple()));
    let part = PathBuf::from(part);
    let written = fs::write(&part, bytes).and_then(|()| fs::rename(&part, path));
    if let Err(error) = written {
        let _ = fs::remove_file(&part);
        return Err(io("Cannot keep the proxy's description", error));
    }
    Ok(())
}

/// What the cache's directory holds.
#[derive(Default)]
struct Found {
    entries: Vec<Entry>,
    /// Files of no use: stale proxies and their manifests, and what a write
    /// that never ended left.
    discard: Vec<PathBuf>,
}

fn age(file: &fs::Metadata) -> Duration {
    (file.modified().ok())
        .and_then(|at| at.elapsed().ok())
        .unwrap_or_default()
}

/// Reads the cache's directory. A proxy is kept when its manifest is whole,
/// of this profile and of its name, its file is the size the manifest says,
/// and its source is unchanged on disk or cannot be read now (a drive that
/// is not attached). Anything else of the cache's is discarded; a proxy
/// without a manifest and a part file only once an hour old, as another
/// session may be writing it.
fn load(dir: &Path) -> Found {
    let mut found = Found::default();
    let Ok(listing) = fs::read_dir(dir) else {
        return found;
    };
    let mut manifests = Vec::new();
    let mut media: HashMap<String, (PathBuf, Duration)> = HashMap::new();
    for item in listing.flatten() {
        let path = item.path();
        let Some(name) = item.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(file) = item.metadata() else {
            continue;
        };
        if !file.is_file() {
            continue;
        }
        if name.ends_with(".part") {
            if age(&file) > LEFT_OVER && is_key(name.get(..64).unwrap_or_default()) {
                found.discard.push(path);
            }
            continue;
        }
        let Some((stem, extension)) = name.split_once('.') else {
            continue;
        };
        if !is_key(stem) {
            continue;
        }
        match extension {
            "json" => manifests.push((stem.to_owned(), path, file)),
            "mp4" | "mov" => {
                media.insert(name, (path, age(&file)));
            }
            _ => {}
        }
    }
    for (stem, path, file) in manifests {
        let entry = admit(dir, &stem, &path, &file);
        let claimed =
            [Codec::H264, Codec::Mjpeg].map(|codec| format!("{stem}.{}", codec.extension()));
        match entry {
            Some(entry) => {
                let name = entry
                    .proxy
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned());
                if let Some(name) = name {
                    media.remove(&name);
                }
                found.entries.push(entry);
            }
            None => {
                found.discard.push(path);
                for name in claimed {
                    if let Some((file, _)) = media.remove(&name) {
                        found.discard.push(file);
                    }
                }
            }
        }
    }
    for (_, (path, age)) in media {
        if age > LEFT_OVER {
            found.discard.push(path);
        }
    }
    found
}

/// The proxy a manifest describes, when it may be used.
fn admit(dir: &Path, stem: &str, path: &Path, file: &fs::Metadata) -> Option<Entry> {
    if file.len() > MANIFEST_BYTES {
        return None;
    }
    let manifest: Manifest = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    let expected = format!("{stem}.{}", manifest.codec.extension());
    if manifest.profile != PROXY_PROFILE
        || manifest.file != expected
        || manifest.width == 0
        || manifest.height == 0
        || manifest.width.max(manifest.height) > PROXY_SIDE
    {
        return None;
    }
    let source = manifest.source.identity();
    if key(&source, &manifest.stamp) != stem {
        return None;
    }
    let proxy = dir.join(&manifest.file);
    if !fs::metadata(&proxy).is_ok_and(|kept| kept.is_file() && kept.len() == manifest.bytes) {
        return None;
    }
    match Stamp::read(&source.path) {
        Ok(now) if now != manifest.stamp => return None,
        _ => {}
    }
    Some(Entry {
        proxy: Proxy {
            path: proxy,
            width: manifest.width,
            height: manifest.height,
            source,
        },
        manifest: path.to_owned(),
        bytes: manifest.bytes,
        used: file.modified().unwrap_or(UNIX_EPOCH),
        stamp: manifest.stamp,
    })
}

/// Marks proxies just opened as used, makes room within the budget and
/// removes the files of proxies taken out that nothing has open. True when
/// room was made, which is told of.
fn tidy(shared: &Shared, dir: &Path) -> bool {
    let (touch, doomed, in_use, sweep, removed) = {
        let Ok(mut lane) = shared.lane.lock() else {
            return false;
        };
        let removed = lane.trim();
        lane.tidy = false;
        (
            std::mem::take(&mut lane.touch),
            std::mem::take(&mut lane.doomed),
            lane.in_use.clone(),
            std::mem::take(&mut lane.sweep),
            removed,
        )
    };
    let now = SystemTime::now();
    for manifest in touch {
        if let Ok(file) = fs::File::options().write(true).open(&manifest) {
            let _ = file.set_modified(now);
        }
    }
    let mut waiting = Vec::new();
    for path in doomed {
        if in_use.contains(&path) {
            waiting.push(path);
            continue;
        }
        // One that cannot be removed now (held open on Windows) is left for
        // the next session to find without its manifest.
        let _ = fs::remove_file(&path);
    }
    if sweep && let Ok(listing) = fs::read_dir(dir) {
        for item in listing.flatten() {
            let name = item.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(".part")
                && is_key(name.get(..64).unwrap_or_default())
                && item.metadata().is_ok_and(|file| age(&file) > LEFT_OVER)
            {
                let _ = fs::remove_file(item.path());
            }
        }
    }
    if !waiting.is_empty()
        && let Ok(mut lane) = shared.lane.lock()
    {
        lane.doomed.extend(waiting);
    }
    removed
}

/// Whether FFmpeg here has libx264, asked once and remembered.
fn codec_here(processes: &Processes, known: &Mutex<Option<Codec>>) -> Result<Codec, MediaError> {
    if let Some(codec) = *known.lock().map_err(|_| MediaError::Closed)? {
        return Ok(codec);
    }
    let mut command = tool_command("ffmpeg");
    command.args(["-hide_banner", "-encoders"]);
    let listed = capture("ffmpeg", &mut command, processes, 1024 * 1024)?;
    let x264 = String::from_utf8_lossy(&listed)
        .lines()
        .any(|line| line.split_whitespace().nth(1) == Some("libx264"));
    let codec = if x264 { Codec::H264 } else { Codec::Mjpeg };
    *known.lock().map_err(|_| MediaError::Closed)? = Some(codec);
    Ok(codec)
}

/// Whether the picture of the file at `path` has an alpha channel, as
/// ffprobe names its pixel format. A proxy is H.264 or MJPEG, opaque both,
/// while the preview and the export keep a video's transparency: such a
/// video is read from its original. A file ffprobe cannot read is not
/// called transparent; the encode then says what is wrong with it.
fn transparent(path: &Path, processes: &Processes) -> bool {
    let mut command = tool_command("ffprobe");
    command
        .args([
            "-v",
            "error",
            "-select_streams",
            "V:0",
            "-show_entries",
            "stream=pix_fmt",
            "-of",
            "csv=p=0",
        ])
        .arg(path);
    capture("ffprobe", &mut command, processes, 4096).is_ok_and(|bytes| {
        String::from_utf8_lossy(&bytes)
            .lines()
            .next()
            .is_some_and(|format| has_alpha(format.trim()))
    })
}

/// Whether an FFmpeg pixel format carries alpha: every format FFmpeg lists
/// with four components, or two for grey, and a palette, which may.
fn has_alpha(format: &str) -> bool {
    const ALPHA: [&str; 13] = [
        "yuva", "rgba", "bgra", "argb", "abgr", "gbrap", "ya8", "ya16", "yaf", "ayuv", "vuya",
        "uyva", "pal8",
    ];
    ALPHA.iter().any(|prefix| format.starts_with(prefix))
}

/// The FFmpeg process that writes a proxy. `paced` reads the source no
/// faster than it plays, for tests that need an encode under way.
fn proxy_command(job: &Encode, codec: Codec, paced: bool) -> std::process::Command {
    let rate = job.asset.frame_rate.unwrap_or_default();
    // Half the cores, so that the preview keeps its share.
    let threads = (thread::available_parallelism().map_or(4, |count| count.get()) / 2)
        .clamp(2, 8)
        .to_string();
    let mut command = tool_command("ffmpeg");
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-y",
        "-threads",
        &threads,
    ]);
    if paced {
        command.args(["-readrate", "1"]);
    }
    command
        .arg("-i")
        .arg(&job.asset.path)
        .args([
            "-map",
            "0:V:0",
            "-an",
            "-sn",
            "-dn",
            "-map_metadata",
            "-1",
            "-map_chapters",
            "-1",
            "-filter_threads",
            "2",
            "-vf",
        ])
        // The preview's frame grid, then the proxy's size. FFmpeg turns
        // the picture upright before this, as the asset's sizes already are.
        // The grid starts at the start of the file, not of its picture: a
        // picture that starts after the sound has its first frame repeated
        // until then, so a time seeks to the same frame in proxy and source.
        .arg(format!(
            "fps={}/{}:start_time=0,scale={}:{}:flags=bicubic",
            rate.numerator, rate.denominator, job.size[0], job.size[1]
        ));
    match codec {
        Codec::H264 => command.args([
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-tune",
            "fastdecode",
            "-crf",
            "23",
            "-g",
            "15",
            "-bf",
            "0",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
        ]),
        Codec::Mjpeg => command.args([
            "-c:v", "mjpeg", "-q:v", "4", "-pix_fmt", "yuvj420p", "-f", "mov",
        ]),
    };
    command
        .args(["-threads", &threads])
        .args([
            "-progress",
            "pipe:1",
            "-stats_period",
            STATS_PERIOD,
            "-nostats",
        ])
        .arg(&job.part);
    command
}

/// Writes a proxy with FFmpeg to `job.part`, telling how many of the
/// source's frames are written from what FFmpeg says on its output.
pub(crate) fn encode(
    job: &Encode,
    processes: &Processes,
    known: &Mutex<Option<Codec>>,
    paced: bool,
    control: &Control,
) -> Result<Encoded, MediaError> {
    if control.cancelled() {
        return Err(MediaError::Cancelled);
    }
    if transparent(&job.asset.path, processes) {
        return Err(MediaError::Invalid(
            "it has transparency, which a proxy cannot keep, so the original is read".into(),
        ));
    }
    let codec = codec_here(processes, known)?;
    if control.cancelled() {
        return Err(MediaError::Cancelled);
    }
    let mut command = proxy_command(job, codec, paced);
    let mut process = Process::spawn("ffmpeg", &mut command, processes)?;
    let mut said = 0.0;
    let mut written = 0u64;
    let mut line = Vec::new();
    let mut bytes = [0u8; 4096];
    loop {
        if control.cancelled() {
            return Err(MediaError::Cancelled);
        }
        process.begin_read_until(Instant::now() + SILENCE);
        let read = process.stdout.read(&mut bytes);
        let in_time = process.end_read_checked();
        if control.cancelled() {
            return Err(MediaError::Cancelled);
        }
        let count = match read {
            Ok(0) => break,
            Ok(count) if in_time => count,
            Ok(_) => return Err(process.failure("the proxy encoder stopped answering")),
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(process.failure(&format!("could not read its progress ({error})")));
            }
        };
        for byte in &bytes[..count] {
            if *byte != b'\n' {
                line.push(*byte);
                continue;
            }
            if let Some(frame) = (line.strip_prefix(b"frame="))
                .and_then(|value| std::str::from_utf8(value).ok())
                .and_then(|value| value.trim().parse::<u64>().ok())
            {
                written = frame;
                let done = (frame as f32 / job.frames.max(1) as f32).clamp(0.0, 1.0);
                if done - said >= 0.01 {
                    said = done;
                    control.progress(done);
                }
            }
            line.clear();
        }
    }
    // Never hold the child mutex while waiting: cancellation needs it.
    let until = Instant::now() + EXIT;
    let status = loop {
        if control.cancelled() {
            return Err(MediaError::Cancelled);
        }
        let exited = (process.child.lock())
            .map_err(|_| MediaError::Closed)?
            .try_wait()
            .map_err(|error| io("Cannot wait for ffmpeg", error))?;
        if let Some(status) = exited {
            break status;
        }
        if Instant::now() >= until {
            return Err(process.failure("the proxy encoder did not finish"));
        }
        thread::sleep(Duration::from_millis(5));
    };
    if !status.success() {
        return Err(process.failure("could not write the proxy"));
    }
    if written == 0 {
        return Err(process.failure("no frame of the video could be read"));
    }
    Ok(Encoded {
        codec,
        frames: written,
    })
}

#[cfg(test)]
mod tests;
