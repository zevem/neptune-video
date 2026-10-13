//! Audio I/O lives on its own worker. The device callback reads a bounded SPSC
//! ring with atomics and one monotonic clock read: no locks, allocation,
//! processes or media decoding.
use crate::{
    MediaError, Wake, io,
    process::{Process, Processes, tool_command},
};
use cpal::{
    FromSample, Sample, SampleFormat, SizedSample, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use editor_core::{AssetId, ClipContent, ClipId, MediaKind, Project, Time, TrackKind};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io::Read,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

const CHUNK_FRAMES: usize = 1024;
const MAX_VOICES: usize = 16;
const MAX_AUDIO_RING_BYTES: usize = 8 * 1024 * 1024;
const MIN_CALLBACK_GRACE_NS: u64 = 500_000_000;
const CALLBACK_GRACE_PERIODS: u64 = 4;

struct AudioRequest {
    generation: u64,
    project: Arc<Project>,
    position: Time,
    playing: bool,
    speed: f64,
}

enum AudioEvent {
    Message(String),
    Output(cpal::Error),
}

struct AudioState {
    pending: Mutex<Option<AudioRequest>>,
    worker: OnceLock<thread::Thread>,
    stopped: AtomicBool,
    output_failed: AtomicBool,
    failure_notice_lost: AtomicBool,
    generation: AtomicU64,
    active_generation: AtomicU64,
    position: AtomicI64,
    speed: AtomicU64,
    played: AtomicU64,
    sample_rate: AtomicU32,
    clock_origin: Instant,
    callback_heartbeat_ns: AtomicU64,
    callback_period_ns: AtomicU64,
}
impl Default for AudioState {
    fn default() -> Self {
        Self {
            pending: Mutex::new(None),
            worker: OnceLock::new(),
            stopped: AtomicBool::new(false),
            output_failed: AtomicBool::new(false),
            failure_notice_lost: AtomicBool::new(false),
            generation: AtomicU64::new(1),
            active_generation: AtomicU64::new(0),
            position: AtomicI64::new(0),
            speed: AtomicU64::new(1_f64.to_bits()),
            played: AtomicU64::new(0),
            sample_rate: AtomicU32::new(48_000),
            clock_origin: Instant::now(),
            callback_heartbeat_ns: AtomicU64::new(0),
            callback_period_ns: AtomicU64::new(0),
        }
    }
}

impl AudioState {
    fn unavailable(&self) -> bool {
        self.stopped.load(Ordering::Acquire) || self.output_failed.load(Ordering::Acquire)
    }

    fn notify(&self) {
        if let Some(worker) = self.worker.get() {
            worker.unpark();
        }
    }

    fn fail_output(&self) -> bool {
        // This latch belongs to one native stream lifetime. A seek cannot
        // repair it, and a raced activation stamp cannot override it.
        let first_failure = !self.output_failed.swap(true, Ordering::AcqRel);
        self.active_generation.store(0, Ordering::Release);
        self.notify();
        first_failure
    }

    fn wait_request(&self) -> Result<Option<AudioRequest>, MediaError> {
        self.wait_request_before_park(|| {})
    }

    fn wait_request_before_park(
        &self,
        mut before_park: impl FnMut(),
    ) -> Result<Option<AudioRequest>, MediaError> {
        loop {
            let request = {
                let mut pending = self.pending.lock().map_err(|_| MediaError::Closed)?;
                if self.unavailable() {
                    pending.take();
                    return Ok(None);
                }
                pending.take()
            };
            if request.is_some() {
                return Ok(request);
            }
            // No mutex or unknown callback is held across park. A fatal
            // notification before park retains its token; recheck on wake.
            // Production's hook is empty; tests inject the notification race.
            before_park();
            thread::park();
        }
    }

    fn clock(&self) -> Option<Time> {
        self.clock_at(self.clock_origin.elapsed())
    }

    fn record_callback_at(&self, elapsed: Duration, frames: u64) {
        self.record_callback_at_before_heartbeat(elapsed, frames, || {});
    }

    fn record_callback_at_before_heartbeat(
        &self,
        elapsed: Duration,
        frames: u64,
        before_heartbeat: impl FnOnce(),
    ) {
        if frames == 0 {
            return;
        }
        let period = frames.saturating_mul(1_000_000_000)
            / u64::from(self.sample_rate.load(Ordering::Acquire).max(1));
        // A reader can see this period before the new heartbeat. Retaining
        // the largest period makes that mixed observation conservative when
        // a backend switches from a long buffer to a shorter one.
        self.callback_period_ns.fetch_max(period, Ordering::Relaxed);
        // Production's empty hook disappears when optimized; the regression
        // reads clock state at this actual partial-publication boundary.
        before_heartbeat();
        self.callback_heartbeat_ns
            .store(duration_ns(elapsed).max(1), Ordering::Release);
    }

    fn start_output_watch(&self) {
        self.callback_heartbeat_ns.store(
            duration_ns(self.clock_origin.elapsed()).max(1),
            Ordering::Release,
        );
    }

    fn clock_at(&self, elapsed: Duration) -> Option<Time> {
        let generation = self.generation.load(Ordering::Acquire);
        if self.unavailable() || self.active_generation.load(Ordering::Acquire) != generation {
            return None;
        }
        if !self.output_live_at(elapsed, generation) {
            return None;
        }
        let frames = self.played.load(Ordering::Acquire);
        let rate = self.sample_rate.load(Ordering::Acquire).max(1);
        let speed = f64::from_bits(self.speed.load(Ordering::Acquire));
        let position = Time(self.position.load(Ordering::Acquire))
            + Time::from_seconds(frames as f64 / f64::from(rate) * speed);
        (self.generation.load(Ordering::Acquire) == generation
            && self.active_generation.load(Ordering::Acquire) == generation
            && !self.unavailable())
        .then_some(position)
    }

    fn output_live_at(&self, elapsed: Duration, generation: u64) -> bool {
        let heartbeat = self.callback_heartbeat_ns.load(Ordering::Acquire);
        // Native playback arms the watch. Pure domain/clock fixtures do not
        // imply a stream was started and remain independent of device health.
        if heartbeat == 0 {
            return true;
        }
        let grace = self
            .callback_period_ns
            .load(Ordering::Relaxed)
            .saturating_mul(CALLBACK_GRACE_PERIODS)
            .max(MIN_CALLBACK_GRACE_NS);
        let now = duration_ns(elapsed);
        if now.saturating_sub(heartbeat) < grace
            || now.saturating_sub(self.callback_heartbeat_ns.load(Ordering::Acquire)) < grace
        {
            return true;
        }
        // A completed pause/reset invalidates this clock. If reset overlaps
        // an already observed expiry, failure may win for this same native
        // stream; resetting a request does not recreate output. A renewed
        // heartbeat gets a final check above before expiry is committed.
        if self.generation.load(Ordering::Acquire) == generation
            && self.active_generation.load(Ordering::Acquire) == generation
            && self.fail_output()
        {
            self.failure_notice_lost.store(true, Ordering::Release);
        }
        !self.unavailable()
    }

    fn deactivate(&self, generation: u64) {
        let _ = self.active_generation.compare_exchange(
            generation,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

fn duration_ns(duration: Duration) -> u64 {
    duration.as_nanos().min(u128::from(u64::MAX)) as u64
}

/// Native output with latest-wins timeline snapshots. Errors are asynchronous,
/// so absence of an audio device never prevents silent video editing.
pub struct AudioService {
    state: Arc<AudioState>,
    processes: Processes,
    events: Receiver<AudioEvent>,
}

impl AudioService {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Result<Self, MediaError> {
        let state = Arc::new(AudioState::default());
        let processes = Processes::default();
        let (sender, events) = mpsc::sync_channel(8);
        let (worker_state, worker_processes) = (Arc::clone(&state), processes.clone());
        let wake: Wake = Arc::new(wake);
        thread::Builder::new()
            .name("aster-audio".into())
            .spawn(move || {
                if let Err(error) =
                    audio_worker(Arc::clone(&worker_state), worker_processes, &sender, &wake)
                {
                    worker_state.fail_output();
                    // Reject later sync under the same pending lock, and
                    // release any request queued before startup failed.
                    if let Ok(mut pending) = worker_state.pending.lock() {
                        pending.take();
                    }
                    if sender
                        .try_send(AudioEvent::Message(error.to_string()))
                        .is_err()
                    {
                        worker_state
                            .failure_notice_lost
                            .store(true, Ordering::Release);
                    }
                    wake();
                }
            })
            .map_err(|error| io("Cannot start native audio worker", error))?;
        Ok(Self {
            state,
            processes,
            events,
        })
    }

    /// Call on seek, play/pause, speed or document changes, rather than every frame.
    pub fn sync(&self, project: Arc<Project>, position: Time, playing: bool, speed: f64) {
        if self.state.unavailable() {
            return;
        }
        self.state.active_generation.store(0, Ordering::Release);
        let generation = self.state.generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.processes.kill_all();
        if let Ok(mut pending) = self.state.pending.lock()
            && !self.state.unavailable()
        {
            *pending = Some(AudioRequest {
                generation,
                project,
                position,
                playing,
                speed,
            });
        }
        self.state.notify();
    }

    pub fn poll(&self) -> Option<String> {
        self.events
            .try_recv()
            .ok()
            .map(|event| match event {
                AudioEvent::Message(message) => message,
                AudioEvent::Output(error) => format!("Audio output: {error}"),
            })
            .or_else(|| {
                self.state
                    .failure_notice_lost
                    .swap(false, Ordering::AcqRel)
                    .then(|| "Audio output is unavailable".into())
            })
    }

    /// Generation-tagged output-frame clock after the initial buffer is ready.
    /// It includes underrun silence; DAC/presentation latency is not measured here.
    /// Missing callbacks expire the clock after four largest observed buffer
    /// periods, with a 500 ms floor. Expiry does not recreate the native stream.
    pub fn position(&self) -> Option<Time> {
        self.state.clock()
    }

    pub fn stop(&self) {
        self.state.active_generation.store(0, Ordering::Release);
        self.state.generation.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut pending) = self.state.pending.lock() {
            *pending = None;
        }
        self.processes.kill_all();
        self.state.notify();
    }
}

impl Drop for AudioService {
    fn drop(&mut self) {
        self.state.stopped.store(true, Ordering::Release);
        self.stop();
    }
}

struct Ring {
    values: Box<[AtomicU32]>,
    schedule: Box<[FrameSchedule]>,
    channels: usize,
    period: usize,
    read: AtomicUsize,
    write: AtomicUsize,
    seen_generation: AtomicU64,
}

struct FrameSchedule {
    generation: AtomicU64,
    output_frame: AtomicU64,
}

impl Ring {
    fn new(capacity_samples: usize, channels: usize) -> Result<Self, MediaError> {
        if channels == 0 || channels > 8 {
            return Err(MediaError::Invalid(
                "Invalid audio buffer channel count".into(),
            ));
        }
        let frames = (capacity_samples / channels).max(1);
        let bytes_per_frame =
            channels * std::mem::size_of::<AtomicU32>() + std::mem::size_of::<FrameSchedule>();
        if frames > MAX_AUDIO_RING_BYTES / bytes_per_frame {
            return Err(MediaError::Invalid(
                "Audio buffer exceeds the 8 MiB budget".into(),
            ));
        }
        Ok(Self {
            values: (0..frames * channels).map(|_| AtomicU32::new(0)).collect(),
            schedule: (0..frames)
                .map(|_| FrameSchedule {
                    generation: AtomicU64::new(0),
                    output_frame: AtomicU64::new(0),
                })
                .collect(),
            channels,
            // Cursors wrap at a multiple of capacity, including non-power-of-two
            // buffers. Native usize overflow cannot alias adjacent unread slots.
            period: frames * 2,
            read: AtomicUsize::new(0),
            write: AtomicUsize::new(0),
            seen_generation: AtomicU64::new(0),
        })
    }
    fn advance(&self, cursor: usize, frames: usize) -> usize {
        (cursor + frames) % self.period
    }
    fn queued(&self, read: usize, write: usize) -> usize {
        ((write + self.period - read) % self.period).min(self.schedule.len())
    }
    fn available(&self) -> usize {
        self.queued(
            self.read.load(Ordering::Acquire),
            self.write.load(Ordering::Acquire),
        ) * self.channels
    }
    fn free(&self) -> usize {
        self.values.len() - self.available()
    }
    fn push(&self, generation: u64, first_output_frame: u64, samples: &[f32]) -> bool {
        if samples.is_empty() || !samples.len().is_multiple_of(self.channels) {
            return false;
        }
        let frames = samples.len() / self.channels;
        if first_output_frame.checked_add(frames as u64).is_none() {
            return false;
        }
        let write = self.write.load(Ordering::Relaxed);
        let read = self.read.load(Ordering::Acquire);
        if frames > self.schedule.len() - self.queued(read, write) {
            return false;
        }
        for (frame, samples) in samples.chunks_exact(self.channels).enumerate() {
            let slot = self.advance(write, frame) % self.schedule.len();
            for (channel, sample) in samples.iter().enumerate() {
                self.values[slot * self.channels + channel]
                    .store(sample.to_bits(), Ordering::Relaxed);
            }
            self.schedule[slot]
                .generation
                .store(generation, Ordering::Relaxed);
            self.schedule[slot]
                .output_frame
                .store(first_output_frame + frame as u64, Ordering::Relaxed);
        }
        // Only this release publishes the complete frame headers and samples.
        self.write
            .store(self.advance(write, frames), Ordering::Release);
        true
    }
    fn fill<T: Sample + FromSample<f32>>(
        &self,
        output: &mut [T],
        state: &AudioState,
        channels: usize,
    ) {
        let generation = state.generation.load(Ordering::Acquire);
        output.fill(T::EQUILIBRIUM);
        if state.unavailable()
            || channels != self.channels
            || !output.len().is_multiple_of(channels)
        {
            return;
        }
        state.record_callback_at(
            state.clock_origin.elapsed(),
            (output.len() / channels) as u64,
        );
        if self.seen_generation.load(Ordering::Relaxed) != generation {
            self.read
                .store(self.write.load(Ordering::Acquire), Ordering::Release);
            self.seen_generation.store(generation, Ordering::Release);
            return;
        }
        if state.active_generation.load(Ordering::Acquire) != generation {
            return;
        }
        let read = self.read.load(Ordering::Relaxed);
        let queued = self.queued(read, self.write.load(Ordering::Acquire));
        let first = state.played.load(Ordering::Relaxed);
        let mut consumed = 0;
        for (offset, frame) in output.chunks_exact_mut(channels).enumerate() {
            let expected = first.saturating_add(offset as u64);
            while consumed < queued {
                let slot = self.advance(read, consumed) % self.schedule.len();
                let header = &self.schedule[slot];
                let epoch = header.generation.load(Ordering::Relaxed);
                let scheduled = header.output_frame.load(Ordering::Relaxed);
                if epoch != generation || scheduled < expected {
                    consumed += 1;
                    continue;
                }
                if scheduled == expected {
                    for (channel, sample) in frame.iter_mut().enumerate() {
                        *sample = T::from_sample(f32::from_bits(
                            self.values[slot * channels + channel].load(Ordering::Relaxed),
                        ));
                    }
                    consumed += 1;
                }
                // A future frame stays queued while this callback inserts silence.
                break;
            }
        }
        self.read
            .store(self.advance(read, consumed), Ordering::Release);
        if state.generation.load(Ordering::Acquire) != generation
            || state.active_generation.load(Ordering::Acquire) != generation
            || state.unavailable()
        {
            output.fill(T::EQUILIBRIUM);
            return;
        }
        state
            .played
            .fetch_add((output.len() / channels) as u64, Ordering::Release);
    }
}

fn output_stream<T: Sample + SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: StreamConfig,
    ring: Arc<Ring>,
    state: Arc<AudioState>,
    errors: SyncSender<AudioEvent>,
    wake: Wake,
) -> Result<cpal::Stream, MediaError> {
    let channels = usize::from(config.channels);
    let error_state = Arc::clone(&state);
    device
        .build_output_stream(
            config,
            move |output: &mut [T], _| ring.fill(output, &state, channels),
            move |error| {
                handle_output_error(&error_state, error, &errors, &wake);
            },
            None,
        )
        .map_err(|error| MediaError::Decode(format!("Cannot create native audio output: {error}")))
}

fn handle_output_error(
    state: &AudioState,
    error: cpal::Error,
    errors: &SyncSender<AudioEvent>,
    wake: &Wake,
) {
    let fatal = !matches!(
        error.kind(),
        cpal::ErrorKind::DeviceChanged | cpal::ErrorKind::RealtimeDenied | cpal::ErrorKind::Xrun
    );
    if fatal {
        state.fail_output();
    }
    if errors.try_send(AudioEvent::Output(error)).is_err() && fatal {
        state.failure_notice_lost.store(true, Ordering::Release);
    }
    wake();
}

fn audio_worker(
    state: Arc<AudioState>,
    processes: Processes,
    errors: &SyncSender<AudioEvent>,
    wake: &Wake,
) -> Result<(), MediaError> {
    let _ = state.worker.set(thread::current());
    if state.unavailable() {
        return Ok(());
    }
    let device = cpal::default_host()
        .default_output_device()
        .ok_or_else(|| MediaError::Decode("No native audio output device is available".into()))?;
    let supported = device
        .default_output_config()
        .map_err(|error| MediaError::Decode(format!("Cannot query audio output: {error}")))?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let channels = usize::from(config.channels);
    let sample_rate = config.sample_rate;
    if channels == 0 || channels > 8 || sample_rate == 0 {
        return Err(MediaError::Invalid(
            "Audio device has an unsupported channel layout".into(),
        ));
    }
    state.sample_rate.store(sample_rate, Ordering::Release);
    let ring = Arc::new(Ring::new(sample_rate as usize * channels / 4, channels)?);
    let args = || {
        (
            config,
            Arc::clone(&ring),
            Arc::clone(&state),
            errors.clone(),
            Arc::clone(wake),
        )
    };
    macro_rules! stream {
        ($ty:ty) => {{
            let (config, ring, state, errors, wake) = args();
            output_stream::<$ty>(&device, config, ring, state, errors, wake)?
        }};
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
                "Unsupported audio device format: {format}"
            )));
        }
    };
    while let Some(request) = state.wait_request()? {
        let _ = stream.pause();
        if !request.playing {
            continue;
        }
        if !request.speed.is_finite() || request.speed <= 0.0 || request.speed > 16.0 {
            let _ = errors.try_send(AudioEvent::Message(
                "Audio playback speed must be positive and at most 16×".into(),
            ));
            wake();
            continue;
        }
        if state.unavailable() || state.generation.load(Ordering::Acquire) != request.generation {
            continue;
        }
        state.position.store(request.position.0, Ordering::Release);
        state
            .speed
            .store(request.speed.to_bits(), Ordering::Release);
        stream
            .play()
            .map_err(|error| MediaError::Decode(format!("Cannot start audio device: {error}")))?;
        state.start_output_watch();
        // The callback alone owns the consumer cursor. It acknowledges a reset
        // before a producer writes samples for the new generation.
        let deadline = Instant::now() + Duration::from_secs(1);
        while ring.seen_generation.load(Ordering::Acquire) != request.generation
            && state.generation.load(Ordering::Acquire) == request.generation
            && !state.unavailable()
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(2));
        }
        if state.unavailable() || state.generation.load(Ordering::Acquire) != request.generation {
            continue;
        }
        if ring.seen_generation.load(Ordering::Acquire) != request.generation {
            let _ = errors.try_send(AudioEvent::Message(
                "Audio device did not start producing callbacks".into(),
            ));
            wake();
            continue;
        }
        // The consumer has finished all old-generation frame accounting before
        // acknowledging this epoch. Only now may its playback cursor reset.
        state.played.store(0, Ordering::Release);
        let mut mixer = Mixer::new(
            Arc::clone(&request.project),
            request.speed,
            sample_rate,
            channels,
            processes.clone(),
        );
        let mut rendered = 0_u64;
        let mut enabled = false;
        while state.generation.load(Ordering::Acquire) == request.generation && !state.unavailable()
        {
            if enabled {
                // The existing active mixer loop also observes output health;
                // cleanup does not depend on a visible window polling position.
                let _ = state.clock();
                if state.unavailable() {
                    wake();
                    break;
                }
            }
            if ring.free() < CHUNK_FRAMES * channels {
                thread::sleep(Duration::from_millis(2));
                continue;
            }
            let played = state.played.load(Ordering::Acquire);
            // After an underrun, seek to the device clock instead of playing
            // old samples late. Decoders are restarted only on this discontinuity.
            if played > rendered {
                rendered = played;
                mixer.clear();
            }
            let time = request.position
                + Time::from_seconds(rendered as f64 / f64::from(sample_rate) * request.speed);
            if time >= request.project.duration() {
                if ring.available() == 0 {
                    state.deactivate(request.generation);
                    let _ = stream.pause();
                    break;
                }
                thread::sleep(Duration::from_millis(2));
                continue;
            }
            let samples = match mixer.chunk(time, CHUNK_FRAMES) {
                Ok(samples) => samples,
                Err(error) => {
                    if state.generation.load(Ordering::Acquire) == request.generation {
                        let _ = errors.try_send(AudioEvent::Message(error.to_string()));
                        wake();
                    }
                    state.deactivate(request.generation);
                    break;
                }
            };
            match publish_decoded(
                &ring,
                &state,
                request.generation,
                rendered,
                &samples,
                &mut enabled,
                || {},
            ) {
                DecodedPublication::Cancelled => break,
                DecodedPublication::Full => {}
                DecodedPublication::Expired => {
                    rendered = state.played.load(Ordering::Acquire);
                    mixer.clear();
                }
                DecodedPublication::Published { became_ready } => {
                    rendered += CHUNK_FRAMES as u64;
                    if became_ready {
                        wake();
                    }
                }
            }
        }
        state.deactivate(request.generation);
        let _ = stream.pause();
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum DecodedPublication {
    Cancelled,
    Full,
    Expired,
    Published { became_ready: bool },
}

/// Worker-owned decoded-publication boundary. Producer checks avoid wholly
/// expired work; callback schedule validation closes the remaining publish race.
/// The no-op production hook allows tests to interleave the real callback reset
/// between the request guard and publication without a device or timing sleeps.
fn publish_decoded(
    ring: &Ring,
    state: &AudioState,
    generation: u64,
    first_output_frame: u64,
    samples: &[f32],
    enabled: &mut bool,
    before_publish: impl FnOnce(),
) -> DecodedPublication {
    if state.unavailable() || state.generation.load(Ordering::Acquire) != generation {
        return DecodedPublication::Cancelled;
    }
    before_publish();
    if state.unavailable() || state.generation.load(Ordering::Acquire) != generation {
        return DecodedPublication::Cancelled;
    }
    let frames = samples.len() / ring.channels;
    if first_output_frame.saturating_add(frames as u64) <= state.played.load(Ordering::Acquire) {
        return DecodedPublication::Expired;
    }
    if !ring.push(generation, first_output_frame, samples) {
        return DecodedPublication::Full;
    }
    if state.unavailable() || state.generation.load(Ordering::Acquire) != generation {
        return DecodedPublication::Cancelled;
    }
    let became_ready = !*enabled;
    if became_ready {
        // An older epoch stamp cannot enable a reset consumer or its clock,
        // even if cancellation races this store after the generation check.
        state.active_generation.store(generation, Ordering::Release);
        if state.unavailable() || state.generation.load(Ordering::Acquire) != generation {
            state.deactivate(generation);
            return DecodedPublication::Cancelled;
        }
        *enabled = true;
    }
    DecodedPublication::Published { became_ready }
}

struct Voice {
    process: Process,
    eof: bool,
    last_frame: usize,
    /// Source speed the decoder was started at.
    speed: f64,
}

mod plan;

#[cfg(test)]
#[derive(Default)]
struct DecoderAudit {
    children: Vec<Arc<Mutex<std::process::Child>>>,
    opened: usize,
    peak_live: usize,
    peak_reserved: usize,
}
#[cfg(test)]
impl DecoderAudit {
    fn opened(&mut self, child: &Arc<Mutex<std::process::Child>>, reserved: usize) {
        // Owned handles outlive map removal, so this observes an unreaped old
        // child even if an implementation accidentally reuses its slot early.
        self.children
            .retain(|child| child.lock().unwrap().try_wait().unwrap().is_none());
        self.children.push(Arc::clone(child));
        self.opened += 1;
        self.peak_live = self.peak_live.max(self.children.len());
        self.peak_reserved = self.peak_reserved.max(reserved);
    }
}

struct Mixer {
    project: Arc<Project>,
    speed: f64,
    rate: u32,
    channels: usize,
    voices: BTreeMap<ClipId, Voice>,
    /// Explicit Audio-track members own sound for their same-asset link.
    /// Ownership includes muted members, so mute cannot reveal embedded sound.
    audio_owners: BTreeSet<(AssetId, [u8; 16])>,
    processes: Processes,
    plan: plan::Plan,
    #[cfg(test)]
    decoder_audit: DecoderAudit,
}
impl Mixer {
    fn new(
        project: Arc<Project>,
        speed: f64,
        rate: u32,
        channels: usize,
        processes: Processes,
    ) -> Self {
        let audio_owners = project
            .tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Audio)
            .flat_map(|track| track.clips.iter())
            .filter_map(|clip| clip.asset_id().zip(clip.linked_id))
            .map(|(asset_id, link_id)| (asset_id, link_id.into_bytes()))
            .collect();
        Self {
            project,
            speed,
            rate,
            channels,
            voices: BTreeMap::new(),
            audio_owners,
            processes,
            plan: plan::Plan::default(),
            #[cfg(test)]
            decoder_audit: DecoderAudit::default(),
        }
    }
    fn clear(&mut self) {
        self.voices.clear();
    }

    fn chunk(&mut self, start: Time, frames: usize) -> Result<Vec<f32>, MediaError> {
        if !(1..=8).contains(&self.channels) {
            return Err(MediaError::Invalid(
                "Invalid preview audio channel count".into(),
            ));
        }
        let project = Arc::clone(&self.project);
        // Reject the whole audible schedule before source I/O or changing any
        // retained decoder. Planning and mixing use the same sample bounds.
        self.plan.prepare(
            &project,
            &self.audio_owners,
            plan::Window {
                start,
                frames,
                rate: self.rate,
                speed: self.speed,
            },
        )?;
        let plan = &self.plan;
        self.voices.retain(|id, voice| {
            // Only sample-zero intervals can carry a sequential decoder from
            // the preceding chunk. Preflight bounds this search at 16 entries.
            let Some(interval) = plan
                .intervals()
                .take_while(|interval| interval.first == 0)
                .find(|interval| interval.id(&project) == *id)
            else {
                return false;
            };
            voice.last_frame = interval.last;
            true
        });
        let mut output = vec![0_f32; frames * self.channels];
        for interval in self.plan.intervals() {
            let clip = &project.tracks[interval.track].clips[interval.clip];
            let ClipContent::Media { asset_id } = &clip.content else {
                continue;
            };
            let Some(asset) = project.assets.get(asset_id) else {
                continue;
            };
            // Earlier intervals have already been mixed into output. Drop
            // waits for each ended owned child before a replacement slot opens.
            self.voices
                .retain(|_, voice| voice.last_frame > interval.first);
            let position = start
                + Time::from_seconds(interval.first as f64 / f64::from(self.rate) * self.speed);
            // A decoder runs forward at one speed. Where a time map freezes,
            // reverses or ramps the clip is silent, and a new straight segment
            // starts a new decoder at its own source position.
            let Some(clip_speed) = plan::steady_speed(clip, position) else {
                self.voices.remove(&clip.id);
                continue;
            };
            if self
                .voices
                .get(&clip.id)
                .is_some_and(|voice| voice.speed != clip_speed)
            {
                self.voices.remove(&clip.id);
            }
            if !self.voices.contains_key(&clip.id) {
                // Independently guard actual decoder reservations; admission
                // must never become a way around the process resource budget.
                if self.voices.len() >= MAX_VOICES {
                    return Err(plan::voice_limit());
                }
                let source = clip.source_time(position);
                let effective = self.speed * clip_speed;
                let changed_rate = (f64::from(self.rate) * effective).round();
                if !(100.0..=1_536_000.0).contains(&changed_rate) {
                    return Err(MediaError::Invalid(
                        "Clip and preview speeds exceed the audio resampling range".into(),
                    ));
                }
                let mut command = tool_command("ffmpeg");
                command
                    .args([
                        "-hide_banner",
                        "-loglevel",
                        "error",
                        "-nostdin",
                        "-threads",
                        "1",
                        "-ss",
                    ])
                    .arg(format!("{:.9}", source.as_seconds()))
                    .arg("-i")
                    .arg(&asset.path)
                    .args([
                        "-map",
                        "0:a:0",
                        "-vn",
                        "-sn",
                        "-dn",
                        "-filter_threads",
                        "1",
                        "-ac",
                    ])
                    .arg(self.channels.to_string())
                    .arg("-af")
                    .arg(format!(
                        "aresample={},asetrate={},aresample={}",
                        self.rate, changed_rate as u32, self.rate
                    ))
                    .args(["-f", "f32le", "-c:a", "pcm_f32le", "pipe:1"]);
                self.voices.insert(
                    clip.id,
                    Voice {
                        process: Process::spawn("ffmpeg", &mut command, &self.processes)?,
                        eof: false,
                        last_frame: interval.last,
                        speed: clip_speed,
                    },
                );
                #[cfg(test)]
                if let Some(voice) = self.voices.get(&clip.id) {
                    self.decoder_audit
                        .opened(&voice.process.child, self.voices.len());
                }
            }
            let Some(voice) = self.voices.get_mut(&clip.id) else {
                continue;
            };
            let count = (interval.last - interval.first) * self.channels;
            let mut bytes = vec![0; count * 4];
            if !voice.eof {
                voice.process.begin_read();
                let mut read = 0;
                while read < bytes.len() {
                    match voice.process.stdout.read(&mut bytes[read..]) {
                        Ok(0) => {
                            voice.eof = true;
                            break;
                        }
                        Ok(count) => read += count,
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(error) => {
                            return Err(voice
                                .process
                                .failure(&format!("Cannot decode audio: {error}")));
                        }
                    }
                }
                voice.process.end_read();
                if read == 0
                    && voice
                        .process
                        .child
                        .lock()
                        .ok()
                        .and_then(|mut child| child.try_wait().ok().flatten())
                        .is_some_and(|status| !status.success())
                {
                    return Err(voice.process.failure("Audio decoder failed"));
                }
            }
            let animated = plan::gain_animated(clip);
            let mut gain = clip.gain;
            for (index, bytes) in bytes.chunks_exact(4).enumerate() {
                if animated && index % self.channels == 0 {
                    let frame = interval.first + index / self.channels;
                    let at = start
                        + Time::from_seconds(frame as f64 / f64::from(self.rate) * self.speed);
                    gain = editor_core::gain_at(clip, at);
                }
                let sample = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                if sample.is_finite() {
                    output[interval.first * self.channels + index] += sample * gain;
                }
            }
        }
        self.voices.retain(|_, voice| voice.last_frame == frames);
        for sample in &mut output {
            *sample = sample.clamp(-1.0, 1.0);
        }
        Ok(output)
    }
}

#[cfg(test)]
mod fault_tests;

#[cfg(test)]
mod liveness_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::{Asset, Clip, Track, TrackKind};
    use std::{path::Path, process::Stdio};

    fn constant_pcm_asset(path: &Path) -> Asset {
        const RATE: u32 = 48_000;
        const CHANNELS: u16 = 2;
        const FRAMES: u32 = RATE / 4;
        let sample = 8192_i16.to_le_bytes(); // exactly 0.25 in signed 16-bit PCM
        let data_bytes = FRAMES * u32::from(CHANNELS) * 2;
        let mut wave = Vec::with_capacity((44 + data_bytes) as usize);
        wave.extend_from_slice(b"RIFF");
        wave.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        wave.extend_from_slice(b"WAVEfmt ");
        wave.extend_from_slice(&16_u32.to_le_bytes());
        wave.extend_from_slice(&1_u16.to_le_bytes()); // PCM
        wave.extend_from_slice(&CHANNELS.to_le_bytes());
        wave.extend_from_slice(&RATE.to_le_bytes());
        wave.extend_from_slice(&(RATE * u32::from(CHANNELS) * 2).to_le_bytes());
        wave.extend_from_slice(&(CHANNELS * 2).to_le_bytes());
        wave.extend_from_slice(&16_u16.to_le_bytes());
        wave.extend_from_slice(b"data");
        wave.extend_from_slice(&data_bytes.to_le_bytes());
        for _ in 0..FRAMES * u32::from(CHANNELS) {
            wave.extend_from_slice(&sample);
        }
        std::fs::write(path, wave).unwrap();
        Asset::new(path.to_path_buf(), MediaKind::Audio, Time(250_000))
    }
    #[test]
    fn audio_ring_is_bounded_and_never_replays_samples_after_reset() {
        let state = AudioState::default();
        let ring = Ring::new(4, 2).unwrap();
        let mut output = [1_f32; 2];
        ring.fill(&mut output, &state, 2); // acknowledge generation
        assert_eq!(output, [0.0; 2]);
        assert!(ring.push(
            state.generation.load(Ordering::Acquire),
            0,
            &[0.25, -0.25, 0.5, -0.5]
        ));
        assert!(!ring.push(state.generation.load(Ordering::Acquire), 2, &[1.0, -1.0]));
        state
            .active_generation
            .store(state.generation.load(Ordering::Acquire), Ordering::Release);
        ring.fill(&mut output, &state, 2);
        assert_eq!(output, [0.25, -0.25]);
        state.generation.fetch_add(1, Ordering::AcqRel);
        ring.fill(&mut output, &state, 2);
        assert_eq!(output, [0.0; 2]);
        assert_eq!(ring.available(), 0);
    }

    #[test]
    fn decoded_publication_rejects_pcm_expired_during_decode() {
        let state = AudioState::default();
        let ring = Ring::new(8, 2).unwrap();
        let mut callback = [1_f32; 8];
        ring.fill(&mut callback, &state, 2); // acknowledge the captured generation
        let generation = state.generation.load(Ordering::Acquire);
        state
            .active_generation
            .store(state.generation.load(Ordering::Acquire), Ordering::Release);
        let mut enabled = true;
        let first_output_frame = 0;

        // This deterministic decode operation completes only after a callback
        // consumes the entire intended interval as silence. The completed PCM
        // carries a nonzero marker that must never enter the future FIFO.
        let samples = {
            ring.fill(&mut callback, &state, 2);
            assert_eq!(callback, [0.0; 8]);
            assert_eq!(state.played.load(Ordering::Acquire), 4);
            [0.25_f32; 8]
        };
        let publication = publish_decoded(
            &ring,
            &state,
            generation,
            first_output_frame,
            &samples,
            &mut enabled,
            || {},
        );
        assert!(
            !matches!(publication, DecodedPublication::Published { .. }),
            "expired decoded PCM must not be published after the callback clock passed it"
        );
        assert_eq!(ring.available(), 0);
        ring.fill(&mut callback, &state, 2);
        assert_eq!(callback, [0.0; 8]);
        assert_eq!(state.played.load(Ordering::Acquire), 8);
    }

    #[test]
    fn decoded_publication_rejects_an_already_stale_request_without_reactivating_audio() {
        let state = AudioState::default();
        let ring = Ring::new(8, 2).unwrap();
        let generation = state.generation.load(Ordering::Acquire);
        state.generation.fetch_add(1, Ordering::AcqRel);
        let mut callback = [1_f32; 8];
        ring.fill(&mut callback, &state, 2); // acknowledge the newer seek
        let mut enabled = false;
        assert_eq!(
            publish_decoded(
                &ring,
                &state,
                generation,
                0,
                &[0.25; 8],
                &mut enabled,
                || {},
            ),
            DecodedPublication::Cancelled
        );
        assert!(!enabled);
        assert_eq!(state.active_generation.load(Ordering::Acquire), 0);
        assert_eq!(ring.available(), 0);
        assert_eq!(callback, [0.0; 8]);
        assert_eq!(state.played.load(Ordering::Acquire), 0);
    }

    #[test]
    fn decoded_publication_callback_race_plays_only_the_current_suffix() {
        let state = AudioState::default();
        let ring = Ring::new(16, 2).unwrap();
        let mut callback = [1_f32; 4];
        ring.fill(&mut callback, &state, 2);
        let generation = state.generation.load(Ordering::Acquire);
        state
            .active_generation
            .store(state.generation.load(Ordering::Acquire), Ordering::Release);
        let mut enabled = true;
        let samples = [0.1_f32, -0.1, 0.2, -0.2, 0.3, -0.3, 0.4, -0.4];
        let publication =
            publish_decoded(&ring, &state, generation, 0, &samples, &mut enabled, || {
                // Consume the first two scheduled frames after the producer's
                // final check. Consumer schedule validation must trim them.
                ring.fill(&mut callback, &state, 2);
                assert_eq!(callback, [0.0; 4]);
                assert_eq!(state.played.load(Ordering::Acquire), 2);
            });
        assert!(matches!(publication, DecodedPublication::Published { .. }));
        ring.fill(&mut callback, &state, 2);
        assert_eq!(callback, [0.3, -0.3, 0.4, -0.4]);
        assert_eq!(ring.available(), 0);
        assert_eq!(state.played.load(Ordering::Acquire), 4);
    }

    #[test]
    fn decoded_publication_future_pcm_stays_queued_until_its_output_frame() {
        let state = AudioState::default();
        let ring = Ring::new(8, 2).unwrap();
        let mut callback = [1_f32; 4];
        ring.fill(&mut callback, &state, 2);
        let generation = state.generation.load(Ordering::Acquire);
        state
            .active_generation
            .store(state.generation.load(Ordering::Acquire), Ordering::Release);
        let mut enabled = true;
        let samples = [0.25_f32, -0.25, 0.5, -0.5];
        assert!(matches!(
            publish_decoded(&ring, &state, generation, 2, &samples, &mut enabled, || {},),
            DecodedPublication::Published { .. }
        ));
        ring.fill(&mut callback, &state, 2);
        assert_eq!(callback, [0.0; 4]);
        assert_eq!(ring.available(), samples.len());
        assert_eq!(state.played.load(Ordering::Acquire), 2);
        ring.fill(&mut callback, &state, 2);
        assert_eq!(callback, samples);
        assert_eq!(ring.available(), 0);
        assert_eq!(state.played.load(Ordering::Acquire), 4);
    }

    #[test]
    fn decoded_publication_rejects_reset_after_request_check_before_publish() {
        let state = AudioState::default();
        let ring = Ring::new(8, 2).unwrap();
        let mut callback = [1_f32; 8];
        ring.fill(&mut callback, &state, 2);
        let generation = state.generation.load(Ordering::Acquire);
        let mut enabled = false;
        let publication = publish_decoded(
            &ring,
            &state,
            generation,
            0,
            &[0.25; 8],
            &mut enabled,
            || {
                // The real callback acknowledges the newer request after the
                // worker's final guard but before old PCM/enable publication.
                state.active_generation.store(0, Ordering::Release);
                state.generation.fetch_add(1, Ordering::AcqRel);
                ring.fill(&mut callback, &state, 2);
            },
        );
        assert!(
            !matches!(publication, DecodedPublication::Published { .. }),
            "a reset between the check and publication must reject the old request"
        );
        assert!(!enabled);
        assert_eq!(state.active_generation.load(Ordering::Acquire), 0);
        assert_eq!(ring.available(), 0);
        ring.fill(&mut callback, &state, 2);
        assert_eq!(callback, [0.0; 8]);
        assert_eq!(state.played.load(Ordering::Acquire), 0);
    }

    #[test]
    fn scheduled_callback_discards_whole_pcm_that_expires_after_publication_check() {
        let state = AudioState::default();
        let ring = Ring::new(8, 2).unwrap();
        let generation = state.generation.load(Ordering::Acquire);
        let mut callback = [1_f32; 8];
        ring.fill(&mut callback, &state, 2);
        state.active_generation.store(generation, Ordering::Release);
        ring.fill(&mut callback, &state, 2);
        assert_eq!(state.played.load(Ordering::Acquire), 4);
        // The producer's check could have passed before that callback ran. Its
        // release happens afterward; only consumer scheduling closes this race.
        assert!(ring.push(generation, 0, &[0.25; 8]));
        ring.fill(&mut callback, &state, 2);
        assert_eq!(callback, [0.0; 8]);
        assert_eq!(ring.available(), 0);
        assert_eq!(state.played.load(Ordering::Acquire), 8);
    }

    #[test]
    fn scheduled_mono_frames_and_non_power_of_two_queue_wrap_remain_exact() {
        for channels in [1, 2] {
            let state = AudioState::default();
            let ring = Ring::new(3 * channels, channels).unwrap();
            let generation = state.generation.load(Ordering::Acquire);
            let mut output = vec![1_f32; 2 * channels];
            ring.fill(&mut output, &state, channels);
            state.active_generation.store(generation, Ordering::Release);
            for iteration in 0..12 {
                let samples: Vec<_> = (0..2 * channels)
                    .map(|sample| (iteration * 2 * channels + sample) as f32 / 64.0)
                    .collect();
                let first_frame = (iteration * 2) as u64;
                assert!(ring.push(generation, first_frame, &samples));
                ring.fill(&mut output, &state, channels);
                assert_eq!(output, samples);
                assert_eq!(ring.available(), 0);
                assert_eq!(state.played.load(Ordering::Acquire), first_frame + 2);
            }
            assert_eq!(ring.read.load(Ordering::Acquire), 0);
            assert_eq!(ring.write.load(Ordering::Acquire), 0);
        }
    }

    #[test]
    fn obsolete_samples_and_clock_stamp_after_reset_cannot_activate_a_new_epoch() {
        let state = AudioState::default();
        let ring = Ring::new(8, 2).unwrap();
        let old_generation = state.generation.load(Ordering::Acquire);
        let mut output = [1_f32; 4];
        ring.fill(&mut output, &state, 2);
        state
            .active_generation
            .store(old_generation, Ordering::Release);
        ring.fill(&mut output, &state, 2);
        assert_eq!(state.played.load(Ordering::Acquire), 2);

        state.active_generation.store(0, Ordering::Release);
        let generation = state.generation.fetch_add(1, Ordering::AcqRel) + 1;
        ring.fill(&mut output, &state, 2); // consumer acknowledges the new epoch
        assert_eq!(ring.seen_generation.load(Ordering::Acquire), generation);
        // Reset only after acknowledgement, as the actual worker now does.
        state.played.store(0, Ordering::Release);
        assert!(ring.push(old_generation, 0, &[0.9; 4]));
        state
            .active_generation
            .store(old_generation, Ordering::Release);
        assert!(state.clock().is_none());
        ring.fill(&mut output, &state, 2);
        assert_eq!(output, [0.0; 4]);
        assert_eq!(state.played.load(Ordering::Acquire), 0);

        state.active_generation.store(0, Ordering::Release);
        let mut enabled = false;
        assert_eq!(
            publish_decoded(
                &ring,
                &state,
                generation,
                0,
                &[0.25, -0.25, 0.5, -0.5],
                &mut enabled,
                || {},
            ),
            DecodedPublication::Published { became_ready: true }
        );
        ring.fill(&mut output, &state, 2);
        assert_eq!(output, [0.25, -0.25, 0.5, -0.5]);
        assert_eq!(ring.available(), 0);
        assert_eq!(state.played.load(Ordering::Acquire), 2);
        assert!(state.clock().is_some());
    }

    #[test]
    fn scheduled_ring_rejects_partial_channels_and_unbounded_storage() {
        assert!(Ring::new(8, 0).is_err());
        assert!(Ring::new(8, 9).is_err());
        assert!(Ring::new(usize::MAX, 1).is_err());
        let state = AudioState::default();
        let generation = state.generation.load(Ordering::Acquire);
        let ring = Ring::new(8, 2).unwrap();
        assert!(!ring.push(generation, 0, &[0.25, 0.5, 0.75]));
        assert!(!ring.push(generation, 0, &[]));
        assert_eq!(ring.available(), 0);
        assert!(ring.push(generation, 0, &[0.25, -0.25]));
        let mut malformed = [1_f32; 3];
        ring.fill(&mut malformed, &state, 2);
        assert_eq!(malformed, [0.0; 3]);
        assert_eq!(ring.available(), 2);
        assert_eq!(state.played.load(Ordering::Acquire), 0);
    }

    #[test]
    fn generation_clock_maps_output_frames_with_the_captured_anchor_and_speed() {
        let state = AudioState::default();
        let generation = state.generation.load(Ordering::Acquire);
        state.position.store(2_000_000, Ordering::Release);
        state.speed.store(0.5_f64.to_bits(), Ordering::Release);
        state.sample_rate.store(48_000, Ordering::Release);
        state.played.store(48_000, Ordering::Release);
        assert!(state.clock().is_none());
        state.active_generation.store(generation, Ordering::Release);
        assert_eq!(state.clock(), Some(Time(2_500_000)));
        state.generation.fetch_add(1, Ordering::AcqRel);
        assert!(state.clock().is_none());
    }

    #[test]
    fn zero_gain_offline_clips_do_not_open_decoders_or_consume_audible_voice_slots() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("never-created.wav");
        assert!(!missing.exists());
        let asset = Asset::new(missing, MediaKind::Audio, Time::from_seconds(1.0));
        let mut project = Project::new("Silent offline audio fixture");
        Arc::make_mut(&mut project.assets).insert(asset.id, asset.clone());
        let mut track = Track::new("Silent clips", TrackKind::Audio);
        for _ in 0..MAX_VOICES + 1 {
            let mut clip = Clip::from_asset(&asset, Time::ZERO);
            clip.gain = 0.0;
            track.clips.push(clip);
        }
        project.tracks = vec![Arc::new(track)];
        project.validate().unwrap();
        assert_eq!(project.duration(), Time::from_seconds(1.0));
        let mut mixer = Mixer::new(Arc::new(project), 1.0, 48_000, 2, Processes::default());
        let samples = mixer
            .chunk(Time::ZERO, 1024)
            .expect("zero-gain offline clips need no source I/O or audible decoder slots");
        assert_eq!(samples, vec![0.0; 2048]);
        assert!(mixer.voices.is_empty());
    }

    #[test]
    fn voice_budget_rejects_new_earlier_clip_before_io_with_sixteen_retained_decoders() {
        let temp = tempfile::tempdir().unwrap();
        let asset = constant_pcm_asset(&temp.path().join("constant.wav"));
        let missing = temp.path().join("never-created.wav");
        assert!(!missing.exists());
        let offline = Asset::new(missing, MediaKind::Audio, Time(250_000));
        let mut project = Project::new("Retained audible voice budget fixture");
        Arc::make_mut(&mut project.assets).insert(asset.id, asset.clone());
        Arc::make_mut(&mut project.assets).insert(offline.id, offline.clone());
        let mut earlier = Track::new("New earlier-track clip", TrackKind::Audio);
        earlier.clips.push(Clip::from_asset(&offline, Time(30_000)));
        project.tracks = vec![Arc::new(earlier)];
        for index in 0..MAX_VOICES {
            let mut track = Track::new(format!("Retained {index}"), TrackKind::Audio);
            let mut clip = Clip::from_asset(&asset, Time::ZERO);
            clip.gain = 0.04;
            track.clips.push(clip);
            project.tracks.push(Arc::new(track));
        }
        project.validate().unwrap();
        let mut mixer = Mixer::new(Arc::new(project), 1.0, 48_000, 2, Processes::default());
        let first = mixer.chunk(Time::ZERO, CHUNK_FRAMES).unwrap();
        assert!(first.iter().all(|sample| (*sample - 0.16).abs() < 0.00001));
        let retained: Vec<_> = mixer
            .voices
            .values()
            .map(|voice| voice.process.child.lock().unwrap().id())
            .collect();
        assert_eq!(retained.len(), MAX_VOICES);

        let error = mixer
            .chunk(
                Time::from_seconds(CHUNK_FRAMES as f64 / 48_000.0),
                CHUNK_FRAMES,
            )
            .expect_err("a seventeenth overlapping voice must be rejected");
        assert!(
            matches!(error, MediaError::Invalid(ref message) if message == "Preview audio supports at most 16 simultaneous audible clips; mute extra tracks"),
            "the voice budget must reject the missing clip before any decoder I/O"
        );
        let remaining: Vec<_> = mixer
            .voices
            .values()
            .map(|voice| voice.process.child.lock().unwrap().id())
            .collect();
        assert_eq!(remaining, retained, "no new decoder reservation is spent");
        assert_eq!(mixer.decoder_audit.opened, MAX_VOICES);
    }

    #[test]
    fn voice_budget_accepts_seventeen_disjoint_clips_in_one_chunk_with_exact_pcm_coverage() {
        let temp = tempfile::tempdir().unwrap();
        let asset = constant_pcm_asset(&temp.path().join("constant.wav"));
        let mut project = Project::new("Sequential audible voice budget fixture");
        Arc::make_mut(&mut project.assets).insert(asset.id, asset.clone());
        let mut track = Track::new("Sequential audio", TrackKind::Audio);
        for index in 0..17 {
            let mut clip = Clip::from_asset(&asset, Time(index * 1000));
            clip.duration = Time(1000);
            track.clips.push(clip);
        }
        project.tracks = vec![Arc::new(track)];
        project.validate().unwrap();
        let mut mixer = Mixer::new(Arc::new(project), 1.0, 48_000, 2, Processes::default());
        let samples = mixer
            .chunk(Time::ZERO, CHUNK_FRAMES)
            .expect("seventeen sequential clips have only one simultaneously audible voice");
        assert_eq!(samples.len(), CHUNK_FRAMES * 2);
        assert!(
            samples[..816 * 2]
                .iter()
                .all(|sample| (*sample - 0.25).abs() < 0.00001),
            "seventeen one-millisecond clips cover exactly 816 stereo frames"
        );
        assert!(samples[816 * 2..].iter().all(|sample| *sample == 0.0));
        assert_eq!(mixer.decoder_audit.opened, 17);
        assert_eq!(mixer.decoder_audit.peak_reserved, 1);
        assert!(mixer.decoder_audit.peak_live <= 1);
        assert!(mixer.voices.is_empty());
    }

    #[test]
    fn voice_budget_reaps_sixteen_cut_decoders_before_opening_sixteen_replacements() {
        let temp = tempfile::tempdir().unwrap();
        let asset = constant_pcm_asset(&temp.path().join("constant.wav"));
        let mut project = Project::new("Sixteen voice cut fixture");
        Arc::make_mut(&mut project.assets).insert(asset.id, asset.clone());
        project.tracks.clear();
        for index in 0..MAX_VOICES {
            let mut track = Track::new(format!("Audio {index}"), TrackKind::Audio);
            for start in [0, 10_000] {
                let mut clip = Clip::from_asset(&asset, Time(start));
                clip.duration = Time(10_000);
                clip.gain = 0.04;
                track.clips.push(clip);
            }
            project.tracks.push(Arc::new(track));
        }
        project.validate().unwrap();
        let mut mixer = Mixer::new(Arc::new(project), 1.0, 48_000, 2, Processes::default());
        let samples = mixer.chunk(Time::ZERO, CHUNK_FRAMES).unwrap();
        assert!(
            samples[..960 * 2]
                .iter()
                .all(|sample| (*sample - 0.16).abs() < 0.00001)
        );
        assert!(samples[960 * 2..].iter().all(|sample| *sample == 0.0));
        assert_eq!(mixer.decoder_audit.opened, MAX_VOICES * 2);
        assert_eq!(mixer.decoder_audit.peak_reserved, MAX_VOICES);
        assert!(mixer.decoder_audit.peak_live <= MAX_VOICES);
        assert!(mixer.decoder_audit.peak_live > 0);
        assert!(mixer.voices.is_empty());
        for child in &mixer.decoder_audit.children {
            assert!(child.lock().unwrap().try_wait().unwrap().is_some());
        }
    }

    #[test]
    fn voice_budget_keeps_continuing_decoder_ids_and_pcm_while_other_voices_cross_a_cut() {
        let temp = tempfile::tempdir().unwrap();
        let asset = constant_pcm_asset(&temp.path().join("constant.wav"));
        let mut project = Project::new("Continuing decoder fixture");
        Arc::make_mut(&mut project.assets).insert(asset.id, asset.clone());
        project.tracks.clear();
        let mut continuing = Vec::new();
        for index in 0..MAX_VOICES / 2 {
            let mut track = Track::new(format!("Continuing {index}"), TrackKind::Audio);
            let mut clip = Clip::from_asset(&asset, Time::ZERO);
            clip.gain = 0.04;
            continuing.push(clip.id);
            track.clips.push(clip);
            project.tracks.push(Arc::new(track));
        }
        let mut retiring = Vec::new();
        for index in 0..MAX_VOICES / 2 {
            let mut track = Track::new(format!("Cut {index}"), TrackKind::Audio);
            let mut first = Clip::from_asset(&asset, Time::ZERO);
            first.duration = Time(30_000);
            first.gain = 0.04;
            retiring.push(first.id);
            let mut second = Clip::from_asset(&asset, Time(30_000));
            second.source_in = Time(50_000);
            second.duration = Time(50_000);
            second.gain = 0.04;
            track.clips = vec![first, second];
            project.tracks.push(Arc::new(track));
        }
        project.validate().unwrap();
        let mut mixer = Mixer::new(Arc::new(project), 1.0, 48_000, 2, Processes::default());
        let first = mixer.chunk(Time::ZERO, CHUNK_FRAMES).unwrap();
        assert!(first.iter().all(|sample| (*sample - 0.16).abs() < 0.00001));
        let continuing_children: Vec<_> = continuing
            .iter()
            .map(|id| mixer.voices[id].process.child.lock().unwrap().id())
            .collect();
        let retired_handles: Vec<_> = retiring
            .iter()
            .map(|id| Arc::clone(&mixer.voices[id].process.child))
            .collect();
        for chunk in [1, 2] {
            let samples = mixer
                .chunk(
                    Time::from_seconds((chunk * CHUNK_FRAMES) as f64 / 48_000.0),
                    CHUNK_FRAMES,
                )
                .unwrap();
            assert!(
                samples
                    .iter()
                    .all(|sample| (*sample - 0.16).abs() < 0.00001)
            );
            let current: Vec<_> = continuing
                .iter()
                .map(|id| mixer.voices[id].process.child.lock().unwrap().id())
                .collect();
            assert_eq!(current, continuing_children);
            assert_eq!(mixer.voices.len(), MAX_VOICES);
        }
        assert_eq!(mixer.decoder_audit.opened, MAX_VOICES + MAX_VOICES / 2);
        assert_eq!(mixer.decoder_audit.peak_reserved, MAX_VOICES);
        assert!(mixer.decoder_audit.peak_live <= MAX_VOICES);
        for child in retired_handles {
            assert!(child.lock().unwrap().try_wait().unwrap().is_some());
        }
        let remaining = mixer.decoder_audit.children.clone();
        mixer.clear();
        for child in remaining {
            assert!(child.lock().unwrap().try_wait().unwrap().is_some());
        }
    }

    #[test]
    fn real_pcm_mixing_follows_gain_keyframes_and_time_maps() {
        if !tool_command("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
        {
            eprintln!("FFmpeg is unavailable; animated PCM mixing fixture skipped");
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("constant.wav");
        let status = tool_command("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
            .args(["aevalsrc=0.25|0.25:s=48000:c=stereo", "-t", "1"])
            .args(["-c:a", "pcm_f32le"])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
        let asset = Asset::new(path, MediaKind::Audio, Time::from_seconds(1.0));
        let mut project = Project::new("Animated PCM fixture");
        Arc::make_mut(&mut project.assets).insert(asset.id, asset.clone());
        project.tracks.clear();
        let mut track = Track::new("A", TrackKind::Audio);
        let mut clip = Clip::from_asset(&asset, Time::ZERO);
        clip.duration = Time::from_seconds(0.4);
        clip.gain = 0.0;
        clip.animation.push(editor_core::AnimTrack {
            target: None,
            key: editor_core::ParamKey::Gain,
            keys: [(0.0, 0.0), (0.2, 2.0)]
                .map(|(at, value)| editor_core::Keyframe {
                    at: Time::from_seconds(at),
                    value,
                    ease: editor_core::Ease::Linear,
                })
                .to_vec(),
        });
        track.clips.push(clip);
        project.tracks.push(Arc::new(track));
        project.validate().unwrap();

        // A base gain of zero is audible once its keyframes raise it.
        let mut mixer = Mixer::new(
            Arc::new(project.clone()),
            1.0,
            48_000,
            2,
            Processes::default(),
        );
        let samples = mixer.chunk(Time::ZERO, 960).unwrap();
        for (frame, pair) in samples.chunks_exact(2).enumerate() {
            let expected = 0.25 * 2.0 * frame as f32 / 9600.0;
            assert!((pair[0] - expected).abs() < 0.002, "frame {frame}");
            assert_eq!(pair[0], pair[1]);
        }

        // Forward at double speed, a freeze, then backwards.
        let key = |at: f64, source: f64| editor_core::TimeKey {
            at: Time::from_seconds(at),
            source: Time::from_seconds(source),
            ease: editor_core::Ease::Linear,
        };
        let clip = &mut Arc::make_mut(&mut project.tracks[0]).clips[0];
        clip.animation.clear();
        clip.gain = 1.0;
        clip.retime = Some(editor_core::Retime {
            keys: vec![key(0.0, 0.0), key(0.1, 0.2), key(0.2, 0.2), key(0.4, 0.0)],
        });
        project.validate().unwrap();
        let mut mixer = Mixer::new(Arc::new(project), 1.0, 48_000, 2, Processes::default());
        let samples = mixer.chunk(Time::ZERO, 960).unwrap();
        assert!(samples.iter().all(|sample| (*sample - 0.25).abs() < 0.002));
        assert_eq!(mixer.voices.values().next().unwrap().speed, 2.0);
        for start in [0.1, 0.25] {
            let samples = mixer.chunk(Time::from_seconds(start), 960).unwrap();
            assert!(samples.iter().all(|sample| *sample == 0.0), "at {start}");
            assert!(mixer.voices.is_empty());
        }

        // A chunk that crosses into the freeze is silent from the sample the
        // freeze starts on, and its decoder is not kept for the next chunk.
        mixer.clear();
        let samples = mixer.chunk(Time::from_seconds(0.09), 960).unwrap();
        let (sounding, frozen) = samples.split_at(480 * 2);
        assert!(sounding.iter().all(|sample| (*sample - 0.25).abs() < 0.002));
        assert!(frozen.iter().all(|sample| *sample == 0.0));
        assert!(mixer.voices.is_empty());
    }

    #[test]
    fn real_pcm_mixing_honors_gain_mute_hidden_tracks_and_reuses_decoders() {
        if !tool_command("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
        {
            eprintln!("FFmpeg is unavailable; PCM mixing fixture skipped");
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("constant.wav");
        let status = tool_command("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "aevalsrc=0.25|0.25:s=48000:c=stereo",
                "-t",
                "0.25",
                "-c:a",
                "pcm_f32le",
            ])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
        let asset = Asset::new(path, MediaKind::Audio, Time::from_seconds(0.25));
        let mut project = Project::new("PCM fixture");
        Arc::make_mut(&mut project.assets).insert(asset.id, asset.clone());
        project.tracks.clear();
        for name in ["A", "B"] {
            let mut track = Track::new(name, TrackKind::Audio);
            let mut clip = Clip::from_asset(&asset, Time::ZERO);
            clip.source_in = Time::from_seconds(0.05);
            clip.duration = Time::from_seconds(0.1);
            clip.gain = 0.5;
            track.hidden = true;
            track.clips.push(clip);
            project.tracks.push(Arc::new(track));
        }
        let mut mixer = Mixer::new(
            Arc::new(project.clone()),
            1.0,
            48_000,
            2,
            Processes::default(),
        );
        let samples = mixer.chunk(Time::ZERO, 1024).unwrap();
        assert!(samples.iter().all(|sample| (*sample - 0.25).abs() < 0.002));
        let child_ids: Vec<_> = mixer
            .voices
            .values()
            .map(|voice| voice.process.child.lock().unwrap().id())
            .collect();
        mixer
            .chunk(Time::from_seconds(1024.0 / 48_000.0), 1024)
            .unwrap();
        let next_ids: Vec<_> = mixer
            .voices
            .values()
            .map(|voice| voice.process.child.lock().unwrap().id())
            .collect();
        assert_eq!(child_ids, next_ids);
        Arc::make_mut(&mut project.tracks[1]).muted = true;
        let mut muted = Mixer::new(Arc::new(project), 1.0, 48_000, 2, Processes::default());
        let samples = muted.chunk(Time::ZERO, 1024).unwrap();
        assert!(samples.iter().all(|sample| (*sample - 0.125).abs() < 0.002));
        assert_eq!(muted.voices.len(), 1);
        assert!(
            muted
                .chunk(Time::from_seconds(0.2), 1024)
                .unwrap()
                .iter()
                .all(|sample| *sample == 0.0)
        );
        assert!(muted.voices.is_empty());
    }

    #[test]
    fn linked_same_asset_audio_owns_sound_and_mute_does_not_reveal_embedded_audio() {
        if !tool_command("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
        {
            eprintln!("FFmpeg is unavailable; linked PCM fixture skipped");
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("linked.mkv");
        let status = tool_command("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=16x16:r=30",
                "-f",
                "lavfi",
                "-i",
                "aevalsrc=1|1:s=48000:c=stereo",
                "-t",
                "0.25",
                "-c:v",
                "ffv1",
                "-c:a",
                "pcm_f32le",
            ])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
        let mut asset = Asset::new(path.clone(), MediaKind::Video, Time::from_seconds(0.25));
        asset.width = 16;
        asset.height = 16;
        asset.has_audio = true;
        let link = ClipId::new_v4();
        let mut project = Project::new("Linked PCM fixture");
        Arc::make_mut(&mut project.assets).insert(asset.id, asset.clone());
        project.tracks.clear();
        for kind in [TrackKind::Video, TrackKind::Audio] {
            let mut track = Track::new("Linked", kind);
            let mut clip = Clip::from_asset(&asset, Time::ZERO);
            clip.source_in = Time::from_seconds(0.05);
            clip.duration = Time::from_seconds(0.1);
            clip.linked_id = Some(link);
            clip.gain = 0.25;
            track.clips.push(clip);
            project.tracks.push(Arc::new(track));
        }
        let mut paired = Mixer::new(
            Arc::new(project.clone()),
            1.0,
            48_000,
            2,
            Processes::default(),
        );
        let samples = paired.chunk(Time::ZERO, 1024).unwrap();
        assert!(
            samples.iter().all(|sample| (*sample - 0.25).abs() < 0.002),
            "embedded audio must not double the explicit member"
        );
        assert_eq!(paired.voices.len(), 1);

        let mut zero_gain_project = project.clone();
        Arc::make_mut(&mut zero_gain_project.tracks[1]).clips[0].gain = 0.0;
        zero_gain_project.validate().unwrap();
        let mut zero_gain = Mixer::new(
            Arc::new(zero_gain_project.clone()),
            1.0,
            48_000,
            2,
            Processes::default(),
        );
        assert!(
            zero_gain
                .chunk(Time::ZERO, 1024)
                .unwrap()
                .iter()
                .all(|sample| *sample == 0.0),
            "zero gain on the explicit linked audio must not reveal embedded video sound"
        );
        assert!(zero_gain.voices.is_empty());

        // Also remove an already opened voice when its clip becomes silent.
        // Link ownership remains unchanged when the explicit member gains zero.
        paired.project = Arc::new(zero_gain_project);
        assert!(
            paired
                .chunk(Time::from_seconds(1024.0 / 48_000.0), 1024)
                .unwrap()
                .iter()
                .all(|sample| *sample == 0.0)
        );
        assert!(paired.voices.is_empty());

        Arc::make_mut(&mut project.tracks[1]).muted = true;
        let mut muted = Mixer::new(
            Arc::new(project.clone()),
            1.0,
            48_000,
            2,
            Processes::default(),
        );
        assert!(
            muted
                .chunk(Time::ZERO, 1024)
                .unwrap()
                .iter()
                .all(|sample| *sample == 0.0)
        );
        assert!(muted.voices.is_empty());

        // A link across different assets retains both sounds, even if their
        // paths happen to refer to the same physical fixture.
        let unrelated = Asset::new(path, MediaKind::Audio, Time::from_seconds(0.25));
        Arc::make_mut(&mut project.assets).insert(unrelated.id, unrelated.clone());
        let explicit = Arc::make_mut(&mut project.tracks[1]);
        explicit.muted = false;
        explicit.clips[0].content = ClipContent::Media {
            asset_id: unrelated.id,
        };
        let mut different = Mixer::new(Arc::new(project), 1.0, 48_000, 2, Processes::default());
        let samples = different.chunk(Time::ZERO, 1024).unwrap();
        assert!(samples.iter().all(|sample| (*sample - 0.5).abs() < 0.002));
        assert_eq!(different.voices.len(), 2);
    }
}
