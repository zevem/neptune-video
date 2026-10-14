//! Voiceover capture: the default input device written to a WAV file.
//!
//! The device's callback does one thing: it copies what it is handed into a
//! bounded lock-free ring, or counts the frames it had no room for. One
//! thread (`aster-record`) opens the input and the file, drains the ring to
//! the file every 20 ms, and rewrites the header's two sizes and syncs once a
//! second, so a crash leaves a file that plays up to the last second.
//! Opening the device, making the file and closing it are all the thread's;
//! only `Drop` waits for it, and each answer comes through `poll`.
//!
//! The service is its own clock: the frames that reached the ring, which are
//! their place in the file. Nothing about a timeline bounds a take.
//!
//! The file is the person's. It is made with `create_new`, never over a file
//! that is there, and never deleted, also not when the take fails.
use crate::{MediaError, Wake, io};
use cpal::{
    FromSample, Sample, SampleFormat, SizedSample, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use editor_core::Time;
use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, PoisonError,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// The longest take, in seconds: it ends itself there and finishes as if
/// stopped.
pub const MAX_TAKE_SECONDS: u64 = 2 * 60 * 60;
/// What the ring holds, in seconds at the device's rate and channels.
pub const RECORD_RING_SECONDS: u32 = 2;
/// The bytes before a WAV file's samples, as written here.
pub const WAV_HEADER_BYTES: u64 = 44;
/// The most sample bytes a classic WAV header can count: both of its sizes
/// are `u32`, and the RIFF size is 36 more than the data's.
const WAV_DATA_BYTES: u64 = u32::MAX as u64 - 36;
/// The highest device rate taken, which bounds the ring at 6 MiB.
const MAX_RATE: u32 = 384_000;
/// How often the writer drains the ring.
const DRAIN: Duration = Duration::from_millis(20);
/// How often the sizes are rewritten and the file synced while sound comes.
const CHECKPOINT: Duration = Duration::from_secs(1);
/// The frames written to the file at a time.
const PIECE_FRAMES: usize = 4096;
/// Each sample is a 32-bit float.
const SAMPLE_BYTES: u64 = 4;

/// A take that was written: all the desktop needs to make it an asset,
/// without a probe.
#[derive(Clone, Debug, PartialEq)]
pub struct Take {
    /// The file, canonical as an import makes paths.
    pub path: PathBuf,
    /// The frames in the file; `frames / rate` is its length.
    pub frames: u64,
    pub rate: u32,
    /// 1 or 2, as the device delivered them.
    pub channels: u16,
    /// The file's length, header included.
    pub bytes: u64,
    /// Frames the device delivered that found the ring full and are not in
    /// the file.
    pub dropped: u64,
    /// Why the take ended by itself: the device's error, a failed write, a
    /// full disk or the longest take. `None` when it was asked to stop.
    pub ended: Option<String>,
}

#[derive(Debug)]
pub enum RecordEvent {
    /// The input is playing and the file is made. `frames` and `rate` are
    /// read from just before this is said; `frames` counts from when the
    /// device began to play, before the file was made, so it may be past 0
    /// when this is read. A take is anchored by `frames` read together with
    /// a position, never by this event's arrival.
    Started { rate: u32, channels: u16 },
    /// The loudest sample since the last one said, as a magnitude: 1 is full
    /// scale. Only the newest waits to be read.
    Level(f32),
    /// The take is closed. What was written is in the file, whatever ended it.
    Finished(Take),
    /// Nothing was recorded, `Started` was not said and no file was made: no
    /// microphone (`MediaError::NoMicrophone`), a device that would not open
    /// or play, or a file already at the path, which is left as it was. The
    /// one file left is one just made whose header could not be written: it
    /// holds no sound.
    Failed(MediaError),
}

/// What the thread hands the caller.
#[derive(Default)]
struct Said {
    events: VecDeque<RecordEvent>,
    level: Option<f32>,
}

/// What the service and its thread share.
#[derive(Default)]
struct Shared {
    /// Set once the input is open; read by `frames` and `rate`.
    feed: OnceLock<Arc<Feed>>,
    stop: AtomicBool,
    writer: OnceLock<thread::Thread>,
    said: Mutex<Said>,
    /// Held by a test to stall the writer.
    #[cfg(test)]
    hold: Mutex<()>,
    /// Set by a test to end the writer as a crash would, with nothing closed.
    #[cfg(test)]
    crash: AtomicBool,
    /// Set by a test to fail the next write as a full disk would.
    #[cfg(test)]
    full: AtomicBool,
}

impl Shared {
    fn say(&self, event: RecordEvent, wake: &Wake) {
        let mut said = self.said.lock().unwrap_or_else(PoisonError::into_inner);
        if matches!(event, RecordEvent::Finished(_) | RecordEvent::Failed(_)) {
            // The level of the last sound written is said before the end,
            // not lost to it; none is left waiting after the take.
            if let Some(level) = said.level.take() {
                said.events.push_back(RecordEvent::Level(level));
            }
        }
        said.events.push_back(event);
        drop(said);
        wake();
    }

    /// Kept for `poll`, with no wake: a take's level is read while the
    /// interface repaints for its elapsed time.
    fn level(&self, peak: f32) {
        let mut said = self.said.lock().unwrap_or_else(PoisonError::into_inner);
        said.level = Some(said.level.map_or(peak, |level| level.max(peak)));
    }
}

/// The bounded single-producer, single-consumer ring between the device's
/// callback and the writer, and the take's clock. The callback is the only
/// producer and the writer the only consumer.
struct Feed {
    values: Box<[AtomicU32]>,
    /// Frames the ring holds.
    capacity: u64,
    channels: usize,
    rate: u32,
    /// Frames the take may hold at most; the callback takes none past it.
    limit: u64,
    /// Frames put in the ring since the start, which is also the frame the
    /// next one lands on in the file.
    write: AtomicU64,
    /// Frames the writer has taken out of the ring.
    read: AtomicU64,
    dropped: AtomicU64,
    /// Cleared when the take is asked to end; the callback then takes nothing.
    open: AtomicBool,
    /// The first error the device reported.
    failure: OnceLock<String>,
}

impl Feed {
    fn new(rate: u32, channels: usize, limit: u64) -> Self {
        let capacity = u64::from(rate) * u64::from(RECORD_RING_SECONDS);
        Self {
            values: (0..capacity as usize * channels)
                .map(|_| AtomicU32::new(0))
                .collect(),
            capacity,
            channels,
            rate,
            limit,
            write: AtomicU64::new(0),
            read: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            open: AtomicBool::new(true),
            failure: OnceLock::new(),
        }
    }

    /// The device's callback: copies whole frames into the ring and counts
    /// those that found it full. No lock, allocation or file work.
    fn capture<T>(&self, data: &[T])
    where
        T: Sample,
        f32: FromSample<T>,
    {
        if !self.open.load(Ordering::Acquire) {
            return;
        }
        let offered = (data.len() / self.channels) as u64;
        let write = self.write.load(Ordering::Relaxed);
        let read = self.read.load(Ordering::Acquire);
        // Frames past the longest take are not lost to an overrun: the take
        // has ended there.
        let wanted = offered.min(self.limit.saturating_sub(write));
        let taken = wanted.min(self.capacity - (write - read));
        let first = (write % self.capacity) as usize * self.channels;
        for (index, sample) in data[..taken as usize * self.channels].iter().enumerate() {
            let slot = (first + index) % self.values.len();
            self.values[slot].store(f32::from_sample(*sample).to_bits(), Ordering::Relaxed);
        }
        // Only this release publishes the samples to the writer.
        self.write.store(write + taken, Ordering::Release);
        if wanted > taken {
            self.dropped.fetch_add(wanted - taken, Ordering::Relaxed);
        }
    }

    /// The device's error callback: the first error ends the take.
    fn fail(&self, message: String) {
        let _ = self.failure.set(message);
    }

    fn close(&self) {
        self.open.store(false, Ordering::Release);
    }

    fn frames(&self) -> u64 {
        self.write.load(Ordering::Acquire)
    }

    fn queued(&self) -> u64 {
        self.write.load(Ordering::Acquire) - self.read.load(Ordering::Relaxed)
    }

    /// Takes up to `most` frames out of the ring as little-endian float
    /// bytes, and returns how many and the loudest sample among them.
    fn drain(&self, bytes: &mut Vec<u8>, most: usize) -> (u64, f32) {
        bytes.clear();
        let read = self.read.load(Ordering::Relaxed);
        let frames = (self.write.load(Ordering::Acquire) - read).min(most as u64);
        let first = (read % self.capacity) as usize * self.channels;
        let mut peak = 0_f32;
        for index in 0..frames as usize * self.channels {
            let sample = f32::from_bits(
                self.values[(first + index) % self.values.len()].load(Ordering::Relaxed),
            );
            peak = peak.max(sample.abs());
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        // Only this release gives the slots back to the callback.
        self.read.store(read + frames, Ordering::Release);
        (frames, peak)
    }
}

/// A started source of sound that hands what it captures to
/// `Feed::capture` from its own thread until it is dropped.
trait Source {
    fn play(&self) -> Result<(), MediaError>;
}

impl Source for cpal::Stream {
    fn play(&self) -> Result<(), MediaError> {
        StreamTrait::play(self).map_err(|error| opening(&error))
    }
}

type Build = Box<dyn FnOnce(Arc<Feed>) -> Result<Box<dyn Source>, MediaError>>;

/// An input found but not yet built: its format, and how to build it, still
/// and feeding the ring it is given once played.
struct Input {
    rate: u32,
    channels: u16,
    build: Build,
}

/// Finds the input, on the service's thread. Tests stand in for the device.
type Open = Box<dyn FnOnce() -> Result<Input, MediaError> + Send>;

/// The longest a take may be at this format, in frames, and what says so:
/// the longest take, or what a classic WAV header can count, rounded down
/// to whole frames so that neither of its sizes wraps.
fn limit(rate: u32, channels: u16, longest: Duration) -> (u64, String) {
    let by_time = (u128::from(rate) * longest.as_micros() / 1_000_000) as u64;
    let by_size = WAV_DATA_BYTES / (SAMPLE_BYTES * u64::from(channels));
    if by_time <= by_size {
        let minutes = longest.as_secs() / 60;
        let said = if minutes >= 60 && minutes.is_multiple_of(60) {
            let hours = minutes / 60;
            format!("{hours} hour{}", if hours == 1 { "" } else { "s" })
        } else {
            format!("{} s", longest.as_secs_f64())
        };
        (by_time, format!("The take ended at its limit of {said}"))
    } else {
        (
            by_size,
            "The take ended at the most a WAV file can hold".into(),
        )
    }
}

/// The 44 bytes before the samples of a 32-bit float WAV, with sizes for
/// `data` bytes of samples, held to what the header can count.
fn header(rate: u32, channels: u16, data: u64) -> [u8; 44] {
    let block = u64::from(channels) * SAMPLE_BYTES;
    let data = data.min(WAV_DATA_BYTES / block * block) as u32;
    let mut bytes = [0; 44];
    let mut at = 0;
    let mut put = |value: &[u8]| {
        bytes[at..at + value.len()].copy_from_slice(value);
        at += value.len();
    };
    put(b"RIFF");
    put(&(36 + data).to_le_bytes());
    put(b"WAVE");
    put(b"fmt ");
    put(&16_u32.to_le_bytes());
    // 3: IEEE float.
    put(&3_u16.to_le_bytes());
    put(&channels.to_le_bytes());
    put(&rate.to_le_bytes());
    put(&(rate * block as u32).to_le_bytes());
    put(&(block as u16).to_le_bytes());
    put(&32_u16.to_le_bytes());
    put(b"data");
    put(&data.to_le_bytes());
    bytes
}

/// The file being written and the samples it holds.
struct Wav {
    file: File,
    rate: u32,
    channels: u16,
    /// Bytes of samples written.
    data: u64,
}

impl Wav {
    fn create(mut file: File, rate: u32, channels: u16) -> std::io::Result<Self> {
        file.write_all(&header(rate, channels, 0))?;
        Ok(Self {
            file,
            rate,
            channels,
            data: 0,
        })
    }

    fn append(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.file.write_all(bytes)?;
        self.data += bytes.len() as u64;
        Ok(())
    }

    fn frames(&self) -> u64 {
        self.data / (SAMPLE_BYTES * u64::from(self.channels))
    }

    /// Rewrites the two sizes for what has been written.
    fn sizes(&mut self) -> std::io::Result<()> {
        let header = header(self.rate, self.channels, self.data);
        self.file.seek(SeekFrom::Start(4))?;
        self.file.write_all(&header[4..8])?;
        self.file.seek(SeekFrom::Start(40))?;
        self.file.write_all(&header[40..44])?;
        self.file
            .seek(SeekFrom::Start(WAV_HEADER_BYTES + self.data))?;
        Ok(())
    }

    fn checkpoint(&mut self) -> std::io::Result<()> {
        self.sizes()?;
        self.file.sync_data()
    }

    /// Rewrites the sizes, cuts off what lies past the counted samples (part
    /// of a piece a failed write left), so that the file's length is the
    /// take's `bytes`, and syncs. Each step is tried when one before failed.
    fn finish(&mut self) -> std::io::Result<()> {
        let sized = self.sizes();
        let end = WAV_HEADER_BYTES + self.data;
        let cut = self.file.metadata().and_then(|metadata| {
            // Only ever shorter: a longer length would add bytes never written.
            if metadata.len() > end {
                self.file.set_len(end)
            } else {
                Ok(())
            }
        });
        let synced = self.file.sync_all();
        sized.and(cut).and(synced)
    }
}

/// Records the default input device to a WAV file on a thread of its own.
/// Every call returns at once; what the thread does is told by `poll`.
pub struct RecordService {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl RecordService {
    /// Starts a take at `path`, a file that is not there yet (a file there is
    /// refused and left as it is). The input is opened and the file made on
    /// the service's thread: `Started` or `Failed` follows through `poll`,
    /// with a wake, and `No microphone` is `Failed(MediaError::NoMicrophone)`.
    pub fn start(
        path: PathBuf,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, MediaError> {
        Self::start_with(
            path,
            Arc::new(wake),
            Box::new(open_input),
            Duration::from_secs(MAX_TAKE_SECONDS),
        )
    }

    fn start_with(
        path: PathBuf,
        wake: Wake,
        open: Open,
        longest: Duration,
    ) -> Result<Self, MediaError> {
        if path.file_name().is_none() {
            return Err(MediaError::Invalid("Choose a file to record to".into()));
        }
        let shared = Arc::new(Shared::default());
        let thread_shared = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("aster-record".into())
            .spawn(move || {
                let _ = thread_shared.writer.set(thread::current());
                match record(&thread_shared, &path, open, longest, &wake) {
                    Ok(Some(take)) => thread_shared.say(RecordEvent::Finished(take), &wake),
                    Ok(None) => {}
                    Err(error) => thread_shared.say(RecordEvent::Failed(error), &wake),
                }
            })
            .map_err(|error| io("Cannot start recording", error))?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }

    /// Asks the take to end. It returns at once; the file is closed on the
    /// service's thread and `Finished` follows through `poll`. Frames that
    /// come after are not taken.
    pub fn stop(&self) {
        if let Some(feed) = self.shared.feed.get() {
            feed.close();
        }
        self.shared.stop.store(true, Ordering::Release);
        if let Some(writer) = self.shared.writer.get() {
            writer.unpark();
        }
    }

    pub fn poll(&self) -> Option<RecordEvent> {
        let mut said = self
            .shared
            .said
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        said.events
            .pop_front()
            .or_else(|| said.level.take().map(RecordEvent::Level))
    }

    /// Frames put in the ring since the device began to play, which is also
    /// their place in the file. 0 until the file is made, just before
    /// `Started` is said, and then every frame since the device began,
    /// including those that came while the file was made.
    pub fn frames(&self) -> u64 {
        self.shared.feed.get().map_or(0, |feed| feed.frames())
    }

    /// The device's rate, 0 until the file is made, just before `Started`.
    pub fn rate(&self) -> u32 {
        self.shared.feed.get().map_or(0, |feed| feed.rate)
    }

    /// `frames` over `rate`: how long the take is so far.
    pub fn elapsed(&self) -> Time {
        let rate = self.rate();
        if rate == 0 {
            return Time::ZERO;
        }
        Time((u128::from(self.frames()) * 1_000_000 / u128::from(rate)) as i64)
    }
}

impl Drop for RecordService {
    /// Ends the take and waits for its file to be closed.
    fn drop(&mut self) {
        self.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The service's thread: opens the input and the file, writes the take and
/// closes it. `Err` when nothing was recorded, before `Started`; `None` only
/// when a test ended it as a crash would.
fn record(
    shared: &Shared,
    path: &Path,
    open: Open,
    longest: Duration,
    wake: &Wake,
) -> Result<Option<Take>, MediaError> {
    let Input {
        rate,
        channels,
        build,
    } = open()?;
    if !(1..=2).contains(&channels) {
        return Err(MediaError::Invalid(format!(
            "The microphone delivers {channels} channels; only one or two can be recorded"
        )));
    }
    if rate == 0 || rate > MAX_RATE {
        return Err(MediaError::Invalid(format!(
            "The microphone's rate of {rate} Hz cannot be recorded"
        )));
    }
    let (most, longest_said) = limit(rate, channels, longest);
    let feed = Arc::new(Feed::new(rate, usize::from(channels), most));
    // Built and played before the file is made, so that a device that will
    // not open or play leaves no file behind; the ring holds what comes while
    // the file is made. Each `?` from here drops the source.
    let source = build(Arc::clone(&feed))?;
    if shared.stop.load(Ordering::Acquire) {
        return Err(MediaError::Cancelled);
    }
    source.play()?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| io(format!("Cannot make {}", path.display()), error))?;
    let wav = Wav::create(file, rate, channels)
        .map_err(|error| io(format!("Cannot write {}", path.display()), error))?;
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    // Published only now, so that a take that failed counts no frames.
    let _ = shared.feed.set(Arc::clone(&feed));
    shared.say(RecordEvent::Started { rate, channels }, wake);
    let mut ended = None;
    let mut source = Some(source);
    let mut writing = Writer::new(wav);
    loop {
        #[cfg(test)]
        if shared.crash.load(Ordering::Acquire) {
            return Ok(None);
        }
        #[cfg(test)]
        let held = shared.hold.lock().unwrap_or_else(PoisonError::into_inner);
        if ended.is_none() {
            ended = feed.failure.get().cloned();
        }
        if ended.is_some() || shared.stop.load(Ordering::Acquire) {
            feed.close();
            // No callback runs once the source is dropped, so what the ring
            // holds then is all there will be.
            source = None;
        }
        if let Err(error) = writing.write_out(&feed, shared) {
            // What the ring still holds is not written after a failed write;
            // part of a piece it left behind the counted data is cut off when
            // the take is closed.
            ended = Some(also(ended, format!("Cannot write the take: {error}")));
            break;
        }
        if ended.is_none() && feed.frames() >= feed.limit {
            ended = Some(longest_said.clone());
            continue;
        }
        if source.is_none() && feed.queued() == 0 {
            break;
        }
        #[cfg(test)]
        drop(held);
        thread::park_timeout(DRAIN);
    }
    drop(source);
    if let Err(error) = writing.wav.finish() {
        ended = Some(also(ended, format!("Cannot close the take: {error}")));
    }
    Ok(Some(Take {
        path: canonical,
        frames: writing.wav.frames(),
        rate,
        channels,
        bytes: WAV_HEADER_BYTES + writing.wav.data,
        dropped: feed.dropped.load(Ordering::Acquire),
        ended,
    }))
}

/// Why a take ended, with a later reason after any earlier one, which is kept.
fn also(ended: Option<String>, then: String) -> String {
    match ended {
        Some(ended) => format!("{ended}. {then}"),
        None => then,
    }
}

/// Moves what the ring holds into the file and keeps the header's sizes no
/// more than a second of sound or of the clock behind it.
struct Writer {
    wav: Wav,
    bytes: Vec<u8>,
    unsynced: u64,
    synced: Instant,
}

impl Writer {
    fn new(wav: Wav) -> Self {
        Self {
            bytes: Vec::with_capacity(PIECE_FRAMES * usize::from(wav.channels) * 4),
            wav,
            unsynced: 0,
            synced: Instant::now(),
        }
    }

    fn write_out(&mut self, feed: &Feed, shared: &Shared) -> std::io::Result<()> {
        loop {
            let (frames, peak) = feed.drain(&mut self.bytes, PIECE_FRAMES);
            if frames == 0 {
                break;
            }
            #[cfg(test)]
            if shared.full.load(Ordering::Acquire) {
                // As a full disk would: part of the piece lands, then it fails.
                self.wav
                    .file
                    .write_all(&self.bytes[..self.bytes.len() / 2 + 2])?;
                return Err(std::io::Error::new(
                    std::io::ErrorKind::StorageFull,
                    "No space left on device",
                ));
            }
            self.wav.append(&self.bytes)?;
            shared.level(peak);
            self.unsynced += frames;
            // By sound written as well as by the clock, so that a backlog
            // drained at once is not left uncounted for longer.
            if self.unsynced >= u64::from(feed.rate) {
                self.checkpoint()?;
            }
        }
        if self.unsynced > 0 && self.synced.elapsed() >= CHECKPOINT {
            self.checkpoint()?;
        }
        Ok(())
    }

    fn checkpoint(&mut self) -> std::io::Result<()> {
        self.wav.checkpoint()?;
        (self.unsynced, self.synced) = (0, Instant::now());
        Ok(())
    }
}

/// Words for a device that will not open or play.
fn opening(error: &cpal::Error) -> MediaError {
    match error.kind() {
        cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::HostUnavailable => {
            MediaError::NoMicrophone
        }
        cpal::ErrorKind::PermissionDenied => MediaError::Invalid(
            "Aster may not use the microphone. Allow it in the system's privacy settings".into(),
        ),
        _ => MediaError::Decode(format!("Cannot open the microphone: {error}")),
    }
}

fn open_input() -> Result<Input, MediaError> {
    let device = cpal::default_host()
        .default_input_device()
        .ok_or(MediaError::NoMicrophone)?;
    let supported = device
        .default_input_config()
        .map_err(|error| opening(&error))?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    Ok(Input {
        rate: config.sample_rate,
        channels: config.channels,
        build: Box::new(move |feed| {
            macro_rules! stream {
                ($ty:ty) => {
                    input_stream::<$ty>(&device, config, feed)?
                };
            }
            let stream = match format {
                SampleFormat::F32 => stream!(f32),
                SampleFormat::F64 => stream!(f64),
                SampleFormat::I8 => stream!(i8),
                SampleFormat::I16 => stream!(i16),
                SampleFormat::I32 => stream!(i32),
                SampleFormat::I64 => stream!(i64),
                SampleFormat::U8 => stream!(u8),
                SampleFormat::U16 => stream!(u16),
                SampleFormat::U32 => stream!(u32),
                SampleFormat::U64 => stream!(u64),
                _ => {
                    return Err(MediaError::Invalid(format!(
                        "The microphone's sample format {format} cannot be recorded"
                    )));
                }
            };
            Ok(Box::new(stream) as Box<dyn Source>)
        }),
    })
}

fn input_stream<T>(
    device: &cpal::Device,
    config: StreamConfig,
    feed: Arc<Feed>,
) -> Result<cpal::Stream, MediaError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let errors = Arc::clone(&feed);
    device
        .build_input_stream(
            config,
            move |data: &[T], _| feed.capture(data),
            move |error: cpal::Error| {
                // A route change, a refused real-time promotion or an
                // overrun in the device leaves the stream running.
                if !matches!(
                    error.kind(),
                    cpal::ErrorKind::DeviceChanged
                        | cpal::ErrorKind::RealtimeDenied
                        | cpal::ErrorKind::Xrun
                ) {
                    errors.fail(format!("The microphone stopped: {error}"));
                }
            },
            None,
        )
        .map_err(|error| opening(&error))
}

#[cfg(test)]
mod tests;
