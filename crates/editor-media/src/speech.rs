//! Speech on the timeline as words with their times, for captions:
//! whisper.cpp on its own worker, its models downloaded on request from a
//! fixed list and checked against their SHA-256, and the timeline's sound
//! mixed offline at 16 kHz.
//!
//! One job runs at a time, a download or a transcription. Its progress is
//! one value the newest report replaces, and every job ends in one event.
//! A cancel stops the download, the decoders and whisper.cpp at their next
//! check; the caller never waits for the worker.
use crate::{MediaError, Wake, audio::OfflineMix, io, process::Processes};
use editor_core::{Project, Time, captions::SpokenWord};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    fs,
    io::Read,
    ops::Range,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Condvar, Mutex, Once,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use whisper_rs::{
    DtwMode, DtwModelPreset, DtwParameters, FullParams, SamplingStrategy, WhisperContext,
    WhisperContextParameters, WhisperState, WhisperVadContext, WhisperVadContextParams,
    WhisperVadParams,
};

mod timing;

/// A model file the person can download, with what it weighs and the sum
/// that proves it arrived whole.
#[derive(Debug, PartialEq, Eq)]
pub struct Model {
    pub id: &'static str,
    pub name: &'static str,
    /// What it is good for, in a few words.
    pub note: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

macro_rules! whisper_model {
    ($id:literal, $name:literal, $note:literal, $file:literal, $bytes:literal, $sha:literal) => {
        Model {
            id: $id,
            name: $name,
            note: $note,
            file: $file,
            url: concat!(
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/",
                $file
            ),
            bytes: $bytes,
            sha256: $sha,
        }
    };
}

/// The Whisper models offered, smallest first. All are multilingual and
/// quantized; the sums are those of whisper.cpp's own model repository.
pub const MODELS: [Model; 5] = [
    whisper_model!(
        "tiny",
        "Tiny",
        "Fastest, rough",
        "ggml-tiny-q5_1.bin",
        32_152_673,
        "818710568da3ca15689e31a743197b520007872ff9576237bda97bd1b469c3d7"
    ),
    whisper_model!(
        "base",
        "Base",
        "Fast, clear speech",
        "ggml-base-q5_1.bin",
        59_707_625,
        "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898"
    ),
    whisper_model!(
        "small",
        "Small",
        "Better, most languages",
        "ggml-small-q5_1.bin",
        190_085_487,
        "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb"
    ),
    whisper_model!(
        "medium",
        "Medium",
        "Accurate, slow",
        "ggml-medium-q5_0.bin",
        539_212_467,
        "19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f"
    ),
    whisper_model!(
        "large-v3-turbo",
        "Large v3 Turbo",
        "Best, for fast computers",
        "ggml-large-v3-turbo-q5_0.bin",
        574_041_195,
        "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2"
    ),
];

/// The speech detector downloaded with every model. It keeps music and
/// silence from Whisper, which would otherwise write words into them.
pub const DETECTOR: Model = Model {
    id: "silero-vad",
    name: "Speech detector",
    note: "Finds where people speak",
    file: "ggml-silero-v5.1.2.bin",
    url: "https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v5.1.2.bin",
    bytes: 885_098,
    sha256: "29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf",
};

/// The model offered first: quick to download and to run.
pub const DEFAULT_MODEL: &str = "base";

/// The sample rate Whisper hears.
pub const SPEECH_RATE: u32 = 16_000;

/// The longest stretch of timeline one job hears.
pub const MAX_SPEECH: Time = Time(6 * 3600 * 1_000_000);

/// How much sound Whisper is given at once: a few minutes, cut in a quiet
/// moment of the last few seconds. It bounds the samples held to about
/// 20 MB whatever the length of the timeline.
const PIECE: usize = 300 * SPEECH_RATE as usize;
const CUT_SEARCH: usize = 10 * SPEECH_RATE as usize;
const CUT_WINDOW: usize = SPEECH_RATE as usize / 10;
/// The silence put between two stretches of speech.
const GAP: usize = SPEECH_RATE as usize * 3 / 10;

const CURL_MISSING: &str = "curl was not found. It comes with Windows 10 and later, macOS and most Linux systems; install it, or download a model yourself and choose its file.";

impl Model {
    /// The model with this id, from [`MODELS`].
    pub fn by_id(id: &str) -> Option<&'static Model> {
        MODELS.iter().find(|model| model.id == id)
    }

    /// Where the model's file is kept in `dir`.
    pub fn path(&self, dir: &Path) -> PathBuf {
        dir.join(self.file)
    }

    /// Whether the model's file is in `dir` at its full size. Its sum was
    /// checked before it was put there.
    pub fn installed(&self, dir: &Path) -> bool {
        fs::metadata(self.path(dir)).is_ok_and(|meta| meta.is_file() && meta.len() == self.bytes)
    }

    /// The megabytes it weighs, as a person reads them.
    pub fn megabytes(&self) -> u64 {
        self.bytes.div_ceil(1_000_000)
    }
}

/// The code Whisper knows a language tag by: its language subtag, with
/// Norwegian and Filipino as Whisper names them. `None` when Whisper does
/// not know the language.
pub fn whisper_language(tag: &str) -> Option<&'static str> {
    let primary = tag.split(['-', '_']).next()?.to_ascii_lowercase();
    let primary = match primary.as_str() {
        "nb" | "nn" => "no",
        "fil" => "tl",
        "iw" => "he",
        "jv" => "jw",
        other => other,
    };
    // whisper.cpp also takes names; only codes are wanted here.
    if !(2..=3).contains(&primary.len()) || !primary.bytes().all(|b| b.is_ascii_lowercase()) {
        return None;
    }
    quiet();
    let code = whisper_rs::get_lang_str(whisper_rs::get_lang_id(primary)?)?;
    (code == primary).then_some(code)
}

/// whisper.cpp writes to the terminal unless it is given somewhere else.
fn quiet() {
    static QUIET: Once = Once::new();
    QUIET.call_once(whisper_rs::install_logging_hooks);
}

/// What a model file says it is in its header, which picks the attention
/// heads that time its words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelShape {
    pub vocabulary: i32,
    pub audio_layers: i32,
    pub text_layers: i32,
    pub text_state: i32,
    pub mels: i32,
}

impl ModelShape {
    /// Reads the header of a whisper.cpp model file. A file that is not one
    /// is refused before whisper.cpp opens it.
    pub fn read(path: &Path) -> Result<Self, MediaError> {
        let mut header = [0_u8; 48];
        fs::File::open(path)
            .and_then(|mut file| file.read_exact(&mut header))
            .map_err(|error| io("Cannot read the speech model", error))?;
        let field = |index: usize| {
            let at = 4 * index;
            i32::from_le_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
        };
        if field(0) as u32 != 0x6767_6d6c {
            return Err(MediaError::Invalid(
                "This file is not a whisper.cpp model".into(),
            ));
        }
        let shape = Self {
            vocabulary: field(1),
            audio_layers: field(5),
            text_layers: field(9),
            text_state: field(7),
            mels: field(10),
        };
        if !(1..=64).contains(&shape.text_layers) || !(1..=64).contains(&shape.audio_layers) {
            return Err(MediaError::Invalid(
                "This file is not a whisper.cpp model".into(),
            ));
        }
        Ok(shape)
    }

    /// Whether it hears more languages than English.
    pub fn multilingual(&self) -> bool {
        self.vocabulary >= 51_865
    }

    /// The attention heads its word times come from: the ones whisper.cpp
    /// names for its kind, or its top layers for a kind it does not know.
    fn alignment(&self) -> DtwMode<'static> {
        let english = !self.multilingual();
        let preset = match (self.audio_layers, self.text_layers, self.text_state) {
            (4, 4, 384) if english => DtwModelPreset::TinyEn,
            (4, 4, 384) => DtwModelPreset::Tiny,
            (6, 6, 512) if english => DtwModelPreset::BaseEn,
            (6, 6, 512) => DtwModelPreset::Base,
            (12, 12, 768) if english => DtwModelPreset::SmallEn,
            (12, 12, 768) => DtwModelPreset::Small,
            (24, 24, 1024) if english => DtwModelPreset::MediumEn,
            (24, 24, 1024) => DtwModelPreset::Medium,
            (32, 4, 1280) => DtwModelPreset::LargeV3Turbo,
            (32, 32, 1280) if self.mels == 128 => DtwModelPreset::LargeV3,
            (32, 32, 1280) => DtwModelPreset::LargeV2,
            _ => {
                return DtwMode::TopMost {
                    n_top: self.text_layers.min(2),
                };
            }
        };
        DtwMode::ModelPreset {
            model_preset: preset,
        }
    }
}

/// A stretch of the timeline to hear.
#[derive(Clone)]
pub struct Transcription {
    pub project: Arc<Project>,
    pub start: Time,
    pub end: Time,
    /// The Whisper model file.
    pub model: PathBuf,
    /// The speech detector's file. Without it, or while it is not there,
    /// all of the stretch is heard.
    pub detector: Option<PathBuf>,
    /// A code from [`whisper_language`], or `None` to tell it from the sound.
    pub language: Option<String>,
    /// Write English whatever is spoken.
    pub translate: bool,
}

pub enum SpeechJob {
    /// Downloads a model and the speech detector into `dir`, unless they
    /// are there.
    Download {
        model: &'static Model,
        dir: PathBuf,
    },
    Transcribe(Transcription),
}

/// What a job is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Downloading the model with this name.
    Downloading(&'static str),
    /// Checking a downloaded file against its sum.
    Checking,
    /// Opening the model.
    Loading,
    /// Mixing the timeline's sound.
    Listening,
    /// Whisper is writing what it heard.
    Recognizing,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeechProgress {
    pub request: u64,
    pub stage: Stage,
    /// How much of the job is done, from 0 to 1.
    pub done: f32,
}

#[derive(Debug)]
pub enum SpeechEvent {
    Downloaded {
        request: u64,
        model: &'static Model,
    },
    /// The words heard, in order, with timeline times inside the stretch,
    /// and the language they are in when it is known.
    Transcribed {
        request: u64,
        words: Vec<SpokenWord>,
        language: Option<String>,
    },
    Failed {
        request: u64,
        message: String,
    },
    Cancelled {
        request: u64,
    },
}

impl SpeechEvent {
    pub fn request(&self) -> u64 {
        match self {
            Self::Downloaded { request, .. }
            | Self::Transcribed { request, .. }
            | Self::Failed { request, .. }
            | Self::Cancelled { request } => *request,
        }
    }
}

enum Outcome {
    Downloaded(&'static Model),
    Transcribed(Vec<SpokenWord>, Option<String>),
}

type Runner = Arc<dyn Fn(SpeechJob, &Control) -> Result<Outcome, MediaError> + Send + Sync>;

/// Events kept for the caller; each job gives one.
const MAX_EVENTS: usize = 16;
/// The least time between two wakes for progress.
const REPORT_EVERY: Duration = Duration::from_millis(100);

#[derive(Default)]
struct Lane {
    next: u64,
    waiting: Option<(u64, SpeechJob)>,
    running: Option<u64>,
    progress: Option<SpeechProgress>,
    reported: Option<Instant>,
    events: VecDeque<SpeechEvent>,
}

struct Shared {
    lane: Mutex<Lane>,
    ready: Condvar,
    /// The request a cancel was asked for.
    cancelled: AtomicU64,
    stopped: AtomicBool,
    wake: Wake,
}

impl Shared {
    fn cancelled(&self, request: u64) -> bool {
        self.stopped.load(Ordering::Acquire) || self.cancelled.load(Ordering::Acquire) == request
    }

    fn report(&self, request: u64, stage: Stage, done: f32) {
        let Ok(mut lane) = self.lane.lock() else {
            return;
        };
        if lane.running != Some(request) {
            return;
        }
        let changed = lane.progress.is_none_or(|progress| progress.stage != stage);
        lane.progress = Some(SpeechProgress {
            request,
            stage,
            done: done.clamp(0.0, 1.0),
        });
        let now = Instant::now();
        if changed || lane.reported.is_none_or(|at| now - at >= REPORT_EVERY) {
            lane.reported = Some(now);
            drop(lane);
            (self.wake)();
        }
    }
}

/// What a running job asks of its lane: whether to stop, and where to say
/// how far it is.
pub(crate) struct Control {
    shared: Arc<Shared>,
    request: u64,
}

impl Control {
    fn cancelled(&self) -> bool {
        self.shared.cancelled(self.request)
    }

    fn report(&self, stage: Stage, done: f32) {
        self.shared.report(self.request, stage, done);
    }
}

/// The speech worker. Dropping it stops the job under way without waiting.
pub struct SpeechService {
    shared: Arc<Shared>,
    processes: Processes,
}

impl SpeechService {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Result<Self, MediaError> {
        let processes = Processes::default();
        let children = processes.clone();
        Self::with_runner(
            Arc::new(wake),
            processes,
            Arc::new(move |job, control| match job {
                SpeechJob::Download { model, dir } => download(model, &dir, control, &curl()),
                SpeechJob::Transcribe(job) => transcribe(job, control, &children),
            }),
        )
    }

    fn with_runner(wake: Wake, processes: Processes, run: Runner) -> Result<Self, MediaError> {
        let shared = Arc::new(Shared {
            lane: Mutex::new(Lane::default()),
            ready: Condvar::new(),
            cancelled: AtomicU64::new(0),
            stopped: AtomicBool::new(false),
            wake,
        });
        let state = Arc::clone(&shared);
        thread::Builder::new()
            .name("neptune-video-speech".into())
            .spawn(move || worker(state, run))
            .map_err(|error| io("Cannot start the speech worker", error))?;
        Ok(Self { shared, processes })
    }

    /// Starts a job and names its request. `Busy` while another waits or
    /// runs: there is one at a time.
    pub fn start(&self, job: SpeechJob) -> Result<u64, MediaError> {
        let mut lane = self.shared.lane.lock().map_err(|_| MediaError::Closed)?;
        if lane.waiting.is_some() || lane.running.is_some() {
            return Err(MediaError::Busy);
        }
        lane.next += 1;
        let request = lane.next;
        lane.waiting = Some((request, job));
        drop(lane);
        self.shared.ready.notify_one();
        Ok(request)
    }

    /// Stops a request. One that has not started answers `Cancelled` at
    /// once; one under way answers it when it stops: a download or the
    /// timeline's decoders at once, whisper.cpp when a pass of its encoder
    /// or decoder ends.
    pub fn cancel(&self, request: u64) {
        let Ok(mut lane) = self.shared.lane.lock() else {
            return;
        };
        if lane
            .waiting
            .as_ref()
            .is_some_and(|(waiting, _)| *waiting == request)
        {
            lane.waiting = None;
            lane.events.push_back(SpeechEvent::Cancelled { request });
            drop(lane);
            (self.shared.wake)();
        } else if lane.running == Some(request) {
            self.shared.cancelled.store(request, Ordering::Release);
            drop(lane);
            // The timeline's decoders are this lane's own children.
            self.processes.kill_all();
        }
    }

    pub fn poll(&self) -> Option<SpeechEvent> {
        self.shared.lane.lock().ok()?.events.pop_front()
    }

    /// How far the job under way is.
    pub fn progress(&self) -> Option<SpeechProgress> {
        self.shared.lane.lock().ok()?.progress
    }

    /// Whether a job waits or runs.
    pub fn busy(&self) -> bool {
        self.shared
            .lane
            .lock()
            .is_ok_and(|lane| lane.waiting.is_some() || lane.running.is_some())
    }
}

impl Drop for SpeechService {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::Release);
        self.processes.kill_all();
        self.shared.ready.notify_all();
    }
}

fn worker(shared: Arc<Shared>, run: Runner) {
    loop {
        let (request, job) = {
            let Ok(mut lane) = shared.lane.lock() else {
                return;
            };
            loop {
                if shared.stopped.load(Ordering::Acquire) {
                    return;
                }
                if let Some(next) = lane.waiting.take() {
                    lane.running = Some(next.0);
                    lane.progress = None;
                    break next;
                }
                lane = match shared.ready.wait(lane) {
                    Ok(lane) => lane,
                    Err(_) => return,
                };
            }
        };
        let control = Control {
            shared: Arc::clone(&shared),
            request,
        };
        // A job that panics still answers, and the lane goes on.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(job, &control)))
            .unwrap_or_else(|_| {
                Err(MediaError::Invalid(
                    "Speech recognition failed unexpectedly".into(),
                ))
            });
        // A cancelled job never answers with what it made.
        let event = match result {
            _ if control.cancelled() => SpeechEvent::Cancelled { request },
            Ok(Outcome::Downloaded(model)) => SpeechEvent::Downloaded { request, model },
            Ok(Outcome::Transcribed(words, language)) => SpeechEvent::Transcribed {
                request,
                words,
                language,
            },
            Err(MediaError::Cancelled) => SpeechEvent::Cancelled { request },
            Err(error) => SpeechEvent::Failed {
                request,
                message: error.to_string(),
            },
        };
        if let Ok(mut lane) = shared.lane.lock() {
            lane.running = None;
            lane.progress = None;
            if lane.events.len() >= MAX_EVENTS {
                lane.events.pop_front();
            }
            lane.events.push_back(event);
        }
        (shared.wake)();
    }
}

// ---- Downloads -------------------------------------------------------------

/// The curl program: `NEPTUNE_VIDEO_CURL`, or the one on the path.
fn curl() -> PathBuf {
    std::env::var_os("NEPTUNE_VIDEO_CURL").map_or_else(|| PathBuf::from("curl"), PathBuf::from)
}

/// Downloads `model` and the speech detector into `dir`, each unless it is
/// there, through `program` (curl). A file is kept only once its size and
/// sum are right.
fn download(
    model: &'static Model,
    dir: &Path,
    control: &Control,
    program: &Path,
) -> Result<Outcome, MediaError> {
    fs::create_dir_all(dir).map_err(|error| io("Cannot make the models folder", error))?;
    let wanted: Vec<&Model> = [model, &DETECTOR]
        .into_iter()
        .filter(|file| !file.installed(dir))
        .collect();
    let total: u64 = wanted.iter().map(|file| file.bytes).sum();
    let mut before = 0;
    for file in wanted {
        let span = (before as f32 / total as f32)..((before + file.bytes) as f32 / total as f32);
        fetch(file, dir, control, program, model.name, span)?;
        before += file.bytes;
    }
    Ok(Outcome::Downloaded(model))
}

/// One file into `dir`: downloaded beside it as `<file>.part`, carried on
/// from a part an earlier attempt left, checked, then renamed into place.
/// `span` is the part of the job's progress it is.
fn fetch(
    file: &Model,
    dir: &Path,
    control: &Control,
    program: &Path,
    label: &'static str,
    span: Range<f32>,
) -> Result<(), MediaError> {
    let part = dir.join(format!("{}.part", file.file));
    let length = |path: &Path| fs::metadata(path).map_or(0, |meta| meta.len());
    // A part longer than the file is not the start of it.
    if length(&part) > file.bytes {
        let _ = fs::remove_file(&part);
    }
    if length(&part) < file.bytes {
        let mut command = Command::new(program);
        command
            .args([
                "--fail",
                "--location",
                "--silent",
                "--show-error",
                "--proto",
                "=https",
                "--retry",
                "3",
                "--connect-timeout",
                "30",
                "--continue-at",
                "-",
                "--output",
            ])
            .arg(&part)
            .arg(file.url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let mut child = command.spawn().map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                MediaError::Invalid(CURL_MISSING.into())
            } else {
                io("Cannot start curl", error)
            }
        })?;
        let status = loop {
            if control.cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(MediaError::Cancelled);
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(io("Cannot follow the download", error));
                }
            }
            let got = length(&part).min(file.bytes) as f32 / file.bytes as f32;
            control.report(
                Stage::Downloading(label),
                span.start + (span.end - span.start) * got,
            );
            thread::sleep(Duration::from_millis(100));
        };
        if !status.success() {
            let mut said = String::new();
            if let Some(stderr) = child.stderr.take() {
                let _ = stderr.take(2048).read_to_string(&mut said);
            }
            let said = said.trim().trim_start_matches("curl: ");
            return Err(MediaError::Invalid(if said.is_empty() {
                format!("Cannot download {}: curl stopped ({status})", file.name)
            } else {
                format!("Cannot download {}: {said}", file.name)
            }));
        }
    }
    control.report(Stage::Checking, span.end);
    if length(&part) != file.bytes || !sum_matches(&part, file.sha256, control)? {
        let _ = fs::remove_file(&part);
        return Err(MediaError::Invalid(format!(
            "The download of {} arrived damaged and was removed; try again.",
            file.name
        )));
    }
    fs::rename(&part, file.path(dir)).map_err(|error| io("Cannot keep the downloaded model", error))
}

/// Whether the file's SHA-256 is `expected`, read a megabyte at a time.
fn sum_matches(path: &Path, expected: &str, control: &Control) -> Result<bool, MediaError> {
    let mut file = fs::File::open(path).map_err(|error| io("Cannot check the download", error))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1 << 20];
    loop {
        if control.cancelled() {
            return Err(MediaError::Cancelled);
        }
        let read = match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(io("Cannot check the download", error)),
        };
        hasher.update(&buffer[..read]);
    }
    let sum = hasher.finalize();
    let hex: String = sum.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(hex == expected)
}

// ---- Transcription ---------------------------------------------------------

/// The threads whisper.cpp takes: all but one, and at most eight.
fn threads() -> i32 {
    let available = thread::available_parallelism().map_or(4, |count| count.get());
    available.saturating_sub(1).clamp(1, 8) as i32
}

fn transcribe(
    job: Transcription,
    control: &Control,
    processes: &Processes,
) -> Result<Outcome, MediaError> {
    if job.end <= job.start {
        return Err(MediaError::Invalid(
            "There is nothing on the timeline to hear.".into(),
        ));
    }
    if job.end - job.start > MAX_SPEECH {
        return Err(MediaError::Invalid(
            "Captions are made for at most six hours at a time.".into(),
        ));
    }
    quiet();
    control.report(Stage::Loading, 0.0);
    let shape = ModelShape::read(&job.model)?;
    let mut parameters = WhisperContextParameters::default();
    parameters.dtw_parameters(DtwParameters {
        mode: shape.alignment(),
        ..DtwParameters::default()
    });
    let context = WhisperContext::new_with_params(&job.model, parameters)
        .map_err(|error| MediaError::Invalid(format!("Cannot open the speech model: {error}")))?;
    let mut state = context.create_state().map_err(|error| {
        MediaError::Invalid(format!("Cannot start speech recognition: {error}"))
    })?;
    // A detector that is not there yet hears all of the stretch.
    let mut detector = match job.detector.as_ref().filter(|path| path.is_file()) {
        Some(path) => {
            let path = path.to_str().ok_or_else(|| {
                MediaError::Invalid("The speech detector's path cannot be opened".into())
            })?;
            let mut parameters = WhisperVadContextParams::new();
            parameters.set_n_threads(threads());
            Some(WhisperVadContext::new(path, parameters).map_err(|error| {
                MediaError::Invalid(format!("Cannot open the speech detector: {error}"))
            })?)
        }
        None => None,
    };
    if control.cancelled() {
        return Err(MediaError::Cancelled);
    }
    let english = !shape.multilingual();
    let mut language = match (&job.language, english) {
        (_, true) => Some("en".to_owned()),
        (language, false) => language.clone(),
    };
    let eot = context.token_eot();
    let total = ((job.end - job.start).as_seconds() * f64::from(SPEECH_RATE)).round() as usize;
    let mut mix = OfflineMix::new(
        Arc::clone(&job.project),
        job.start,
        SPEECH_RATE,
        processes.clone(),
    );
    let mut words = Vec::new();
    let mut taken = 0;
    let mut carry: Vec<f32> = Vec::new();
    let mut piece_start = 0;
    loop {
        let mut piece = std::mem::take(&mut carry);
        while piece.len() < PIECE && taken < total {
            if control.cancelled() {
                return Err(MediaError::Cancelled);
            }
            let chunk = mix.next()?;
            let wanted = (total - taken).min(chunk.len());
            piece.extend_from_slice(&chunk[..wanted]);
            taken += wanted;
            control.report(Stage::Listening, piece_start as f32 / total as f32);
        }
        if piece.is_empty() {
            break;
        }
        if taken < total {
            let cut = timing::quiet_cut(&piece, PIECE, CUT_SEARCH, CUT_WINDOW);
            carry = piece.split_off(cut);
        }
        let spans = speech_spans(detector.as_mut(), &piece)?;
        if !spans.is_empty() {
            let (heard, map) = timing::Condensed::build(&piece, &spans, GAP);
            let progress = (
                piece_start as f32 / total as f32,
                piece.len() as f32 / total as f32,
            );
            let pieces = recognize(
                &mut state,
                &heard,
                &mut language,
                job.translate,
                eot,
                control,
                progress,
            )?;
            let offset =
                job.start + Time::from_seconds(piece_start as f64 / f64::from(SPEECH_RATE));
            words.extend(timing::spoken(
                &pieces,
                &map,
                offset,
                SPEECH_RATE,
                job.start..job.end,
            ));
        }
        piece_start += piece.len();
        if taken >= total && carry.is_empty() {
            break;
        }
    }
    let language = if job.translate {
        Some("en".to_owned())
    } else {
        language
    };
    Ok(Outcome::Transcribed(words, language))
}

/// Where people speak in `samples`, in samples, from the speech detector;
/// all of it without one. Stretches less than a third of a second apart
/// are one.
fn speech_spans(
    detector: Option<&mut WhisperVadContext>,
    samples: &[f32],
) -> Result<Vec<Range<usize>>, MediaError> {
    let Some(detector) = detector else {
        return Ok(std::iter::once(0..samples.len()).collect());
    };
    let mut parameters = WhisperVadParams::new();
    parameters.set_min_speech_duration(150);
    parameters.set_min_silence_duration(300);
    parameters.set_speech_pad(200);
    let segments = detector
        .segments_from_samples(parameters, samples)
        .map_err(|error| MediaError::Invalid(format!("The speech detector failed: {error}")))?;
    let sample = |hundredths: f32| -> usize {
        ((f64::from(hundredths.max(0.0)) * f64::from(SPEECH_RATE) / 100.0).round() as usize)
            .min(samples.len())
    };
    let mut spans: Vec<Range<usize>> = Vec::new();
    for segment in segments {
        let span = sample(segment.start)..sample(segment.end);
        if span.is_empty() {
            continue;
        }
        match spans.last_mut() {
            Some(last) if span.start <= last.end + GAP => last.end = last.end.max(span.end),
            _ => spans.push(span),
        }
    }
    Ok(spans)
}

/// Whisper's text tokens for `samples`, with their times in hundredths of
/// a second of `samples`. `language` is filled with the one it heard when
/// it was not given. `progress` is where this run starts in the job and the
/// part of the job it is.
fn recognize(
    state: &mut WhisperState,
    samples: &[f32],
    language: &mut Option<String>,
    translate: bool,
    eot: i32,
    control: &Control,
    progress: (f32, f32),
) -> Result<Vec<timing::Piece>, MediaError> {
    // Beam search as whisper.cpp's own tool does: on a test recording it
    // took 15% longer than greedy decoding and wrote fewer wrong words.
    let mut parameters = FullParams::new(SamplingStrategy::BeamSearch {
        beam_size: 5,
        patience: -1.0,
    });
    parameters.set_n_threads(threads());
    parameters.set_translate(translate);
    parameters.set_language(Some(language.as_deref().unwrap_or("auto")));
    parameters.set_print_special(false);
    parameters.set_print_progress(false);
    parameters.set_print_realtime(false);
    parameters.set_print_timestamps(false);
    parameters.set_token_timestamps(true);
    parameters.set_suppress_nst(true);
    let shared = Arc::clone(&control.shared);
    let request = control.request;
    parameters.set_progress_callback_safe(move |percent: i32| {
        let done = progress.0 + progress.1 * percent.clamp(0, 100) as f32 / 100.0;
        shared.report(request, Stage::Recognizing, done);
    });
    // whisper-rs calls an abort callback as the closure type it is given
    // while it keeps the closure boxed, so the closure handed over is the
    // box itself.
    let shared = Arc::clone(&control.shared);
    let abort: Box<dyn FnMut() -> bool> = Box::new(move || shared.cancelled(request));
    parameters.set_abort_callback_safe::<_, Box<dyn FnMut() -> bool>>(abort);
    control.report(Stage::Recognizing, progress.0);
    state.full(parameters, samples).map_err(|error| {
        if control.cancelled() {
            MediaError::Cancelled
        } else {
            MediaError::Invalid(format!("Speech recognition failed: {error}"))
        }
    })?;
    if language.is_none() {
        *language = whisper_rs::get_lang_str(state.full_lang_id_from_state()).map(str::to_owned);
    }
    let mut pieces = Vec::new();
    for (index, segment) in state.as_iter().enumerate() {
        if segment
            .to_str_lossy()
            .is_ok_and(|text| timing::annotation(&text))
        {
            continue;
        }
        let segment_end = segment.end_timestamp();
        for token in 0..segment.n_tokens() {
            let Some(token) = segment.get_token(token) else {
                continue;
            };
            if token.token_id() >= eot {
                continue;
            }
            let Ok(bytes) = token.to_bytes() else {
                continue;
            };
            let data = token.token_data();
            pieces.push(timing::Piece {
                bytes: bytes.to_vec(),
                at: if data.t_dtw >= 0 { data.t_dtw } else { data.t0 },
                segment: index,
                segment_end,
            });
        }
    }
    Ok(pieces)
}

#[cfg(test)]
mod tests;
