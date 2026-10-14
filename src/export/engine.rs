//! One export, start to finish, on the render thread.
//!
//! The plan is made again from the job's snapshot with the capabilities of
//! the moment. The sound is rendered first, by the playback mixer, to a raw
//! file in the job's working folder, hidden beside the destination (the
//! system's temporary folder only when that folder takes none); captions
//! carried in the file are written there too. FFmpeg is then started on a hidden part file beside
//! the destination, and every output frame is drawn by the
//! [`FrameRenderer`] from the job's render project (rule 15) and handed to a
//! writer thread through a channel of two recycled buffers, so drawing the
//! next frame overlaps encoding the last and at most three output frames
//! exist at once. When FFmpeg has exited cleanly and `ffprobe` finds the
//! streams and length expected, the part file is renamed over the
//! destination and a caption file, when asked for, is put beside it the same
//! way. Whatever happens, the working folder and part files go.
//!
//! A hardware encode that fails under `EncoderChoice::Auto` is run again
//! once, from the first frame, with the software encoder; the sound already
//! rendered is reused and the job says what happened (decision 6). A failure
//! of the disk or the folder (no room, no permission, a name too long) and a
//! finish that stalls fail at once: encoding again would meet them again.
//!
//! FFmpeg is waited for while it finishes the file (MP4 and MOV rewrite it
//! to put their index first) for at least [`FINISH_DEADLINE`] plus time for
//! the file's size, and beyond that for as long as the file keeps changing.
use super::{
    frame::{FrameRenderer, Pictures, READ_DEADLINE, headless_device},
    queue::{Handle, Report},
};
use editor_core::{
    Asset, ClipId, Project, Time,
    captions::{format_srt, format_vtt},
    export::{
        Capabilities, Container, EncoderChoice, ExportPlan, ExportSettings, RenderState,
        SidecarFormat, VideoEncoder, VideoPlan,
    },
};
use editor_media::{
    Frame, MediaError,
    export::{
        AudioInput, AudioRender, EncodeInputs, Encoder, Expectation, SourceReader, SubtitleInput,
        VideoInput, verify_output,
    },
};
use eframe::wgpu;
use std::{
    cell::RefCell,
    collections::{HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime},
};
use uuid::Uuid;

/// How long FFmpeg may take to accept one picture.
pub(crate) const WRITE_DEADLINE: Duration = Duration::from_secs(60);
/// The least time FFmpeg is given to finish the file once every picture is
/// in, whatever it does meanwhile.
pub(crate) const FINISH_DEADLINE: Duration = Duration::from_secs(600);
/// Bytes a second the file's end is at worst written at: a file of `n`
/// bytes is given `n / FINISH_PACE` seconds beyond [`FINISH_DEADLINE`],
/// since MP4 and MOV move their index to the front by rewriting the file.
pub(crate) const FINISH_PACE: u64 = 8 * 1024 * 1024;
/// Past those, how long FFmpeg may go without changing its file before the
/// finish is taken to have stalled.
pub(crate) const FINISH_STALL: Duration = Duration::from_secs(120);
/// Left-over part files and working folders older than this, not of a job
/// listed, are a stopped export's and are removed.
pub(crate) const STALE_AFTER: Duration = Duration::from_secs(60 * 60);
/// The longest file name most file systems take, in bytes.
const NAME_MAX: usize = 255;
/// Output frames handed to the writer at once, besides the one being drawn.
pub(crate) const IN_FLIGHT: usize = 2;
/// The span the rendering speed is measured over.
const SPEED_WINDOW: Duration = Duration::from_secs(2);

/// One job as its run receives it.
#[derive(Debug, Clone)]
pub(crate) struct Ticket {
    pub id: Uuid,
    pub project: Arc<Project>,
    pub settings: ExportSettings,
    pub destination: PathBuf,
    pub caps: Arc<Capabilities>,
    /// The caption file written beside the destination, when asked for.
    pub sidecar: Option<PathBuf>,
    /// The project's file, when it has one.
    pub project_path: Option<PathBuf>,
}

/// How a job's run ended.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Finish {
    Done {
        path: PathBuf,
        bytes: Option<u64>,
        note: Option<String>,
    },
    Failed(String),
    Cancelled,
}

/// Runs jobs: the device they draw on, and where stabilization is kept.
pub(crate) struct Engine {
    /// The window's device, when there is one.
    given: Option<(wgpu::Device, wgpu::Queue)>,
    /// A device of the export's own, opened when first needed.
    made: Option<(wgpu::Device, wgpu::Queue)>,
    steady: Option<PathBuf>,
}

impl Engine {
    pub fn new(gpu: Option<(wgpu::Device, wgpu::Queue)>, steady: Option<PathBuf>) -> Self {
        Self {
            given: gpu,
            made: None,
            steady,
        }
    }

    /// Runs one job to its end. Its working folder and part files are gone
    /// when this returns, however it ended.
    pub fn run(&mut self, ticket: &Ticket, handle: &Handle) -> Finish {
        let mut scratch = Scratch::new(ticket.id, &ticket.destination);
        let result = self.export(ticket, handle, &mut scratch);
        drop(scratch);
        match result {
            Ok(finish) => finish,
            Err(_) if handle.cancelled() => Finish::Cancelled,
            Err(Failure::Cancelled) => Finish::Cancelled,
            Err(Failure::Picture(why) | Failure::Encoder(why) | Failure::Other(why)) => {
                Finish::Failed(why)
            }
        }
    }

    fn device(&mut self) -> Result<(wgpu::Device, wgpu::Queue), String> {
        if let Some(gpu) = self.given.as_ref().or(self.made.as_ref()) {
            return Ok(gpu.clone());
        }
        let made = headless_device()?;
        let gpu = (made.device, made.queue);
        self.made = Some(gpu.clone());
        Ok(gpu)
    }

    fn export(
        &mut self,
        ticket: &Ticket,
        handle: &Handle,
        scratch: &mut Scratch,
    ) -> Result<Finish, Failure> {
        let plan = ticket
            .settings
            .plan(&ticket.project, &ticket.caps)
            .map_err(|error| Failure::Other(error.to_string()))?;
        handle.encoder(encoder_name(&plan));
        let folder = ticket.destination.parent().unwrap_or(Path::new(""));
        if !folder.is_dir() {
            return Err(Failure::Other(format!(
                "The folder {} no longer exists",
                folder.display()
            )));
        }
        let frames_total = plan.video.as_ref().map_or(0, |video| video.frames.count);
        let meter = RefCell::new(Meter::new(handle, &plan, frames_total));
        meter.borrow_mut().report(0.0, 0);

        // The picture first: a device that cannot draw it fails the job
        // before anything is rendered.
        let mut picture = match &plan.video {
            Some(video) => Some(self.picture(ticket, handle, video)?),
            None => None,
        };

        scratch.make_temp().map_err(|error| {
            Failure::Other(format!("Cannot make the export's working folder: {error}"))
        })?;
        let mut sound_notes = Vec::new();
        let sound = match &plan.audio {
            Some(audio) => {
                let path = scratch.temp.join("sound.f32");
                let render = AudioRender::new();
                handle.on_cancel(render.canceller());
                sound_notes = render
                    .render(
                        Arc::clone(&ticket.project),
                        audio.samples,
                        &path,
                        &|fraction| meter.borrow_mut().audio(fraction),
                    )
                    .map_err(|error| media_failure(error, Failure::Other))?;
                Some(path)
            }
            None => None,
        };
        if handle.cancelled() {
            return Err(Failure::Cancelled);
        }
        let embedded = match (&plan.cues, ticket.settings.subtitles.embed) {
            (Some(cues), true) => {
                let (text, extension) = match ticket.settings.container {
                    Container::Webm => (format_vtt(&cues.cues, cues.language.as_deref()), "vtt"),
                    _ => (format_srt(&cues.cues), "srt"),
                };
                let path = scratch.temp.join(format!("captions.{extension}"));
                fs::write(&path, text).map_err(|error| {
                    Failure::Other(format!("Cannot write the captions to embed: {error}"))
                })?;
                Some(path)
            }
            _ => None,
        };
        let inputs = Inputs {
            sound: sound.as_deref(),
            subtitles: embedded
                .as_deref()
                .map(|path| (path, plan.cues.as_ref().and_then(|c| c.language.as_deref()))),
            title: &ticket.project.name,
        };

        // Encode, falling back once from hardware to software under Auto.
        let mut current = plan.clone();
        let mut notes = sound_notes;
        // The hardware encoder's failure, once the software one runs.
        let mut first: Option<(String, String)> = None;
        loop {
            let fail_at = forced_failure(&ticket.destination).map(|why| (3, why));
            match encode(
                &current,
                &inputs,
                picture.as_mut(),
                &scratch.part,
                handle,
                &meter,
                fail_at.as_ref(),
            ) {
                Ok(()) => break,
                Err(Failure::Encoder(why)) if !handle.cancelled() => {
                    let _ = fs::remove_file(&scratch.part);
                    // The disk's or the folder's trouble: encoding again
                    // in software would meet it again.
                    if !folder.is_dir() {
                        return Err(Failure::Other(format!(
                            "The folder {} no longer exists",
                            folder.display()
                        )));
                    }
                    if file_problem(&why, &scratch.part) {
                        return Err(Failure::Other(why));
                    }
                    let Some(software) = fallback(&current, &ticket.caps) else {
                        return Err(Failure::Encoder(match first {
                            Some((hardware, reason)) => format!(
                                "{hardware} failed ({reason}), and so did the software \
                                 encoder: {why}"
                            ),
                            None => why,
                        }));
                    };
                    let video = current.video.as_mut().expect("a fallback has a picture");
                    let hardware = video
                        .encoder
                        .hardware
                        .map_or("The encoder", |kind| kind.label());
                    first = Some((hardware.to_owned(), short(&why)));
                    notes.push(format!(
                        "{hardware} failed ({}); finished with {} software encoding",
                        short(&why),
                        software.name
                    ));
                    video.encoder = software;
                    handle.encoder(encoder_name(&current));
                    meter.borrow_mut().restart();
                }
                Err(failure) => return Err(failure),
            }
        }
        if handle.cancelled() {
            return Err(Failure::Cancelled);
        }

        // Into place: the caption file is ready before anything is replaced.
        let sidecar = match (&plan.cues, ticket.settings.subtitles.sidecar) {
            (Some(cues), Some(format)) => {
                let path = sidecar_path(&ticket.destination, cues.language.as_deref(), format);
                let text = match format {
                    SidecarFormat::Srt => format_srt(&cues.cues),
                    SidecarFormat::Vtt => format_vtt(&cues.cues, cues.language.as_deref()),
                };
                let part = hidden_part(&path, ticket.id);
                scratch.sidecar = Some(part.clone());
                fs::write(&part, text).map_err(|error| {
                    Failure::Other(format!("Cannot write the caption file: {error}"))
                })?;
                Some((part, path))
            }
            _ => None,
        };
        settle(&scratch.part);
        fs::rename(&scratch.part, &ticket.destination).map_err(|error| {
            Failure::Other(format!("Cannot move the export into place: {error}"))
        })?;
        scratch.moved = true;
        if let Some((part, path)) = sidecar {
            settle(&part);
            if let Err(error) = fs::rename(&part, &path) {
                notes.push(format!("The caption file could not be written: {error}"));
            }
        }
        settle_folder(folder);
        if let Some(picture) = &picture {
            notes.extend(picture.renderer.warnings().iter().cloned());
        }
        let bytes = fs::metadata(&ticket.destination)
            .ok()
            .map(|meta| meta.len());
        Ok(Finish::Done {
            path: ticket.destination.clone(),
            bytes,
            note: (!notes.is_empty()).then(|| notes.join(" ")),
        })
    }

    fn picture(
        &mut self,
        ticket: &Ticket,
        handle: &Handle,
        video: &VideoPlan,
    ) -> Result<Picture, Failure> {
        let render = ticket.settings.render_project(&ticket.project);
        let (device, queue) = self.device().map_err(Failure::Other)?;
        let mut renderer =
            FrameRenderer::new(device, queue, video.geometry, &render, self.steady.clone())
                .map_err(Failure::Other)?;
        renderer.set_stop(handle.stop_flag());
        renderer.set_deadline(READ_DEADLINE);
        let reader = SourceReader::new();
        handle.on_cancel(reader.canceller());
        Ok(Picture {
            render,
            renderer,
            reader,
            live: HashSet::new(),
        })
    }
}

/// What an encode reads besides its pictures.
struct Inputs<'a> {
    sound: Option<&'a Path>,
    subtitles: Option<(&'a Path, Option<&'a str>)>,
    title: &'a str,
}

/// How a job stopped short.
#[derive(Debug)]
enum Failure {
    Cancelled,
    /// Drawing or decoding a picture failed.
    Picture(String),
    /// FFmpeg failed to start, take pictures, finish, or make the file
    /// expected: what falls back to software.
    Encoder(String),
    Other(String),
}

fn media_failure(error: MediaError, kind: fn(String) -> Failure) -> Failure {
    match error {
        MediaError::Cancelled => Failure::Cancelled,
        error => kind(error.to_string()),
    }
}

/// The software encoder a failed hardware encode is run again with: only
/// under `Auto`, only once.
fn fallback(plan: &ExportPlan, caps: &Capabilities) -> Option<VideoEncoder> {
    let video = plan.video.as_ref()?;
    if video.encoder.hardware.is_none() || video.settings.encoder != EncoderChoice::Auto {
        return None;
    }
    caps.software_encoder(video.settings.codec, video.geometry.output)
        .cloned()
}

/// The encoder a plan names: the picture's, or the sound's alone.
pub(crate) fn encoder_name(plan: &ExportPlan) -> Option<String> {
    (plan.video.as_ref().map(|video| video.encoder.name.clone()))
        .or_else(|| plan.audio.as_ref().map(|audio| audio.encoder.name.clone()))
}

/// The first line of a reason, at most about a hundred characters.
fn short(why: &str) -> String {
    let line = why
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(why);
    let line = line.trim().trim_end_matches('.');
    if line.chars().count() <= 100 {
        return line.to_owned();
    }
    let cut: String = line.chars().take(99).collect();
    format!("{}…", cut.trim_end())
}

/// What the finished file must be, from the plan.
fn expectation(plan: &ExportPlan, subtitles: bool) -> Expectation {
    let (duration, frame) = match (&plan.video, &plan.audio) {
        (Some(video), _) => (video.frames.duration(), video.frames.rate.frame_time(1)),
        (None, Some(audio)) => (
            Time(
                (i128::from(audio.samples.frames) * 1_000_000 / i128::from(audio.samples.rate))
                    as i64,
            ),
            Time::ZERO,
        ),
        (None, None) => (Time::ZERO, Time::ZERO),
    };
    Expectation {
        video: plan.video.is_some(),
        audio: plan.audio.is_some(),
        subtitles,
        duration,
        frame,
        audio_codec: plan.audio.as_ref().map(|audio| audio.settings.codec),
        sample_rate: plan
            .audio
            .as_ref()
            .map_or(0, |audio| audio.settings.sample_rate),
    }
}

/// One encode attempt: FFmpeg started, every picture written, the file
/// finished and checked.
fn encode(
    plan: &ExportPlan,
    inputs: &Inputs,
    picture: Option<&mut Picture>,
    part: &Path,
    handle: &Handle,
    meter: &RefCell<Meter>,
    fail_at: Option<&(u64, String)>,
) -> Result<(), Failure> {
    let encode_inputs = EncodeInputs {
        video: plan.video.as_ref().map(|video| VideoInput {
            size: video.geometry.output,
            rate: video.frames.rate,
        }),
        audio: plan
            .audio
            .as_ref()
            .zip(inputs.sound)
            .map(|(audio, path)| AudioInput {
                path,
                rate: audio.settings.sample_rate,
                channels: audio.settings.channels,
            }),
        subtitles: inputs
            .subtitles
            .map(|(path, language)| SubtitleInput { path, language }),
        title: inputs.title,
    };
    // A pause asked for while the sound rendered takes effect here, before
    // FFmpeg starts; later ones between frames.
    handle.hold().ok_or(Failure::Cancelled)?;
    let encoder = Encoder::start(plan, &encode_inputs, part)
        .map_err(|error| media_failure(error, Failure::Encoder))?;
    handle.on_cancel(encoder.canceller());
    handle.phase(RenderState::Rendering);
    let encoder = match (&plan.video, picture) {
        (Some(video), Some(picture)) => frames(video, picture, encoder, handle, meter, fail_at)?,
        _ => encoder,
    };
    if let Some((_, why)) = fail_at {
        // A test asked for this encoder to fail; one without pictures fails
        // as it finishes.
        return Err(Failure::Encoder(why.clone()));
    }
    handle.phase(RenderState::Finalizing);
    meter.borrow_mut().finishing();
    finish(encoder, part)?;
    if handle.cancelled() {
        return Err(Failure::Cancelled);
    }
    verify_output(part, &expectation(plan, inputs.subtitles.is_some()))
        .map_err(|error| media_failure(error, Failure::Encoder))?;
    Ok(())
}

/// Ends FFmpeg's input and waits for it to finish the file for as long as it
/// is working on it: at least [`FINISH_DEADLINE`] plus time for the file's
/// size, and beyond that while the file keeps changing. A finish that stops
/// changing the file for [`FINISH_STALL`] after that is killed and fails;
/// it is no fault of the encoder's, so it does not fall back.
fn finish(encoder: Encoder, part: &Path) -> Result<(), Failure> {
    /// What the wait on FFmpeg itself is given: the watch decides first.
    const NO_LIMIT: Duration = Duration::from_secs(60 * 60 * 24 * 30);
    let kill = encoder.canceller();
    let stalled = AtomicBool::new(false);
    let seen = || {
        fs::metadata(part)
            .ok()
            .map(|meta| (meta.len(), meta.modified().ok()))
    };
    let result = thread::scope(|scope| {
        let finishing = thread::Builder::new()
            .name("aster-render-finish".into())
            .spawn_scoped(scope, move || encoder.finish(NO_LIMIT))
            .map_err(|error| {
                Failure::Other(format!("The export could not be finished: {error}"))
            })?;
        let mut watch = FinishWatch::new(Instant::now(), seen());
        let mut pause = Duration::from_millis(2);
        while !finishing.is_finished() {
            thread::sleep(pause);
            pause = (pause * 2).min(Duration::from_millis(250));
            if !watch.patient(Instant::now(), seen()) {
                stalled.store(true, Ordering::Release);
                kill();
                break;
            }
        }
        match finishing.join() {
            Ok(result) => Ok(result),
            Err(panic) => std::panic::resume_unwind(panic),
        }
    })?;
    if stalled.load(Ordering::Acquire) {
        return Err(Failure::Other(format!(
            "FFmpeg stopped finishing the file: nothing was written to it for {} seconds",
            FINISH_STALL.as_secs()
        )));
    }
    result.map_err(|error| media_failure(error, Failure::Encoder))
}

/// What the part file looked like: its length and when it last changed.
type Seen = Option<(u64, Option<SystemTime>)>;

/// Decides how long a finishing FFmpeg is waited for.
#[derive(Debug)]
pub(crate) struct FinishWatch {
    started: Instant,
    /// Waited for whatever the file does.
    floor: Duration,
    last: Seen,
    changed: Instant,
}

impl FinishWatch {
    pub fn new(now: Instant, seen: Seen) -> Self {
        let bytes = seen.map_or(0, |(bytes, _)| bytes);
        Self {
            started: now,
            floor: FINISH_DEADLINE + Duration::from_secs(bytes / FINISH_PACE),
            last: seen,
            changed: now,
        }
    }

    /// Whether to go on waiting, the file looking like `seen` at `now`.
    pub fn patient(&mut self, now: Instant, seen: Seen) -> bool {
        if seen != self.last {
            self.last = seen;
            self.changed = now;
        }
        now.saturating_duration_since(self.started) < self.floor
            || now.saturating_duration_since(self.changed) < FINISH_STALL
    }
}

/// Whether an encoder's failure is the file's: no room, no permission, a
/// name too long, a folder gone. Encoding again would meet it again.
/// "Permission denied" and "No such file or directory" count only where
/// they speak of the output, since a hardware device can say them too.
pub(crate) fn file_problem(why: &str, part: &Path) -> bool {
    const ANYWHERE: [&str; 6] = [
        "No space left on device",
        "Disk quota exceeded",
        "Read-only file system",
        "File name too long",
        "Input/output error",
        "File too large",
    ];
    const OF_THE_OUTPUT: [&str; 2] = ["Permission denied", "No such file or directory"];
    let name = part
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    why.lines().any(|line| {
        ANYWHERE.iter().any(|cause| line.contains(cause))
            || (OF_THE_OUTPUT.iter().any(|cause| line.contains(cause))
                && ((!name.is_empty() && line.contains(&name))
                    || line.contains("Error opening output")))
    })
}

/// Draws every output frame and hands it to a writer thread that feeds
/// FFmpeg. Gives the encoder back once every picture is written.
fn frames(
    video: &VideoPlan,
    picture: &mut Picture,
    encoder: Encoder,
    handle: &Handle,
    meter: &RefCell<Meter>,
    fail_at: Option<&(u64, String)>,
) -> Result<Encoder, Failure> {
    let count = video.frames.count;
    let (filled, written) = mpsc::sync_channel::<Vec<u8>>(IN_FLIGHT);
    let (emptied, empty) = mpsc::sync_channel::<Vec<u8>>(IN_FLIGHT);
    for _ in 0..IN_FLIGHT {
        let _ = emptied.send(Vec::new());
    }
    thread::scope(|scope| {
        let writer = thread::Builder::new()
            .name("aster-render-writer".into())
            .spawn_scoped(scope, move || -> Result<Encoder, MediaError> {
                let mut encoder = encoder;
                for buffer in written {
                    if let Some((at, why)) = fail_at
                        && *at == encoder.frames()
                    {
                        return Err(MediaError::Decode(why.clone()));
                    }
                    encoder.write_frame(&buffer, WRITE_DEADLINE)?;
                    // The renderer may have stopped; the buffer then goes.
                    let _ = emptied.send(buffer);
                }
                Ok(encoder)
            })
            .map_err(|error| {
                Failure::Other(format!("The export's writer could not start: {error}"))
            })?;
        let drawn = draw(video, picture, &filled, &empty, handle, meter, count);
        drop(filled);
        let written = match writer.join() {
            Ok(written) => written,
            Err(panic) => std::panic::resume_unwind(panic),
        };
        match (drawn, written) {
            (Err(Failure::Cancelled), _) | (_, Err(MediaError::Cancelled)) => {
                Err(Failure::Cancelled)
            }
            (Err(failure), _) => Err(failure),
            (Ok(()), Err(error)) => Err(Failure::Encoder(error.to_string())),
            (Ok(()), Ok(encoder)) if encoder.frames() != count => Err(Failure::Encoder(format!(
                "FFmpeg took {} of {count} pictures",
                encoder.frames()
            ))),
            (Ok(()), Ok(encoder)) => Ok(encoder),
        }
    })
}

/// The render thread's half of [`frames`]: `Ok` also when the writer
/// stopped, whose own result then says why.
fn draw(
    video: &VideoPlan,
    picture: &mut Picture,
    filled: &mpsc::SyncSender<Vec<u8>>,
    empty: &mpsc::Receiver<Vec<u8>>,
    handle: &Handle,
    meter: &RefCell<Meter>,
    count: u64,
) -> Result<(), Failure> {
    let Picture {
        render,
        renderer,
        reader,
        live,
    } = picture;
    for index in 0..count {
        let waited = handle.hold().ok_or(Failure::Cancelled)?;
        if !waited.is_zero() {
            meter.borrow_mut().resumed();
        }
        let at = video.frames.time(index);
        let rgba = renderer
            .render(render, at, &mut Sources(reader))
            .map_err(|why| {
                if handle.cancelled() {
                    Failure::Cancelled
                } else {
                    Failure::Picture(format!("Frame {} could not be drawn: {why}", index + 1))
                }
            })?;
        let Ok(mut buffer) = empty.recv() else {
            return Ok(());
        };
        buffer.clear();
        buffer.extend_from_slice(rgba);
        if filled.send(buffer).is_err() {
            return Ok(());
        }
        // Pipes of clips no longer drawn close; those about to be stay.
        live_lanes(render, at, video.frames.time(index + 1), live);
        reader.retain(&|lane| live.contains(&lane));
        meter.borrow_mut().frame(index + 1);
    }
    Ok(())
}

/// The clips drawn between `from` and `to`, with what their transitions
/// hold before and after them.
fn live_lanes(project: &Project, from: Time, to: Time, live: &mut HashSet<ClipId>) {
    live.clear();
    for track in &project.tracks {
        for clip in &track.clips {
            let before = clip
                .transition_in
                .as_ref()
                .map_or(Time::ZERO, |t| t.duration);
            let after = clip
                .transition_out
                .as_ref()
                .map_or(Time::ZERO, |t| t.duration);
            if clip.start - before <= to && clip.end() + after >= from {
                live.insert(clip.id);
            }
        }
    }
}

/// What draws a job's pictures.
struct Picture {
    render: Project,
    renderer: FrameRenderer,
    reader: SourceReader,
    live: HashSet<ClipId>,
}

/// The frame renderer's pictures, decoded from the originals.
struct Sources<'a>(&'a mut SourceReader);

impl Pictures for Sources<'_> {
    fn picture(
        &mut self,
        lane: ClipId,
        asset: &Asset,
        time: Time,
        size: [u32; 2],
    ) -> Result<Frame, String> {
        self.0
            .picture(lane, asset, time, size)
            .map_err(|error| format!("{} could not be read: {error}", asset.name))
    }
}

// ---------------------------------------------------------------------------
// Progress
// ---------------------------------------------------------------------------

/// The share of the job each stage takes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Weights {
    pub audio: f32,
    pub video: f32,
    pub finish: f32,
}

impl Weights {
    pub fn of(plan: &ExportPlan) -> Self {
        match (plan.video.is_some(), plan.audio.is_some()) {
            (true, true) => Self {
                audio: 0.05,
                video: 0.93,
                finish: 0.02,
            },
            (true, false) => Self {
                audio: 0.0,
                video: 0.98,
                finish: 0.02,
            },
            _ => Self {
                audio: 0.95,
                video: 0.0,
                finish: 0.05,
            },
        }
    }
}

/// Turns stages and frames into the job's progress, speed and time left.
struct Meter<'h> {
    handle: &'h Handle,
    weights: Weights,
    total: u64,
    /// Frames done at moments of the last [`SPEED_WINDOW`].
    window: VecDeque<(Instant, u64)>,
    speed: f32,
    audio_done: f32,
}

impl<'h> Meter<'h> {
    fn new(handle: &'h Handle, plan: &ExportPlan, total: u64) -> Self {
        Self {
            handle,
            weights: Weights::of(plan),
            total,
            window: VecDeque::new(),
            speed: 0.0,
            audio_done: 0.0,
        }
    }

    fn report(&self, progress: f32, done: u64) {
        let eta = if self.total > 0 && done > 0 {
            (self.speed > 0.0)
                .then(|| (self.total - done.min(self.total)) as f64 / f64::from(self.speed))
        } else if self.total == 0 && progress >= 0.05 {
            // Sound only: from the pace so far.
            let elapsed = self.handle.elapsed().as_secs_f64();
            Some(elapsed * f64::from(1.0 - progress) / f64::from(progress))
        } else {
            None
        };
        self.handle.report(Report {
            progress,
            frames_done: done,
            frames_total: self.total,
            speed_fps: self.speed,
            eta_seconds: eta,
        });
    }

    fn audio(&mut self, fraction: f32) {
        self.audio_done = fraction.clamp(0.0, 1.0);
        self.report(self.weights.audio * self.audio_done, 0);
    }

    fn frame(&mut self, done: u64) {
        let now = Instant::now();
        self.window.push_back((now, done));
        while self.window.len() > 2
            && self
                .window
                .get(1)
                .is_some_and(|(at, _)| now.duration_since(*at) >= SPEED_WINDOW)
        {
            self.window.pop_front();
        }
        if let Some((from, first)) = self.window.front() {
            let span = now.duration_since(*from).as_secs_f64();
            if span >= 0.2 {
                self.speed = ((done - first) as f64 / span) as f32;
            }
        }
        let share = if self.total == 0 {
            1.0
        } else {
            done as f32 / self.total as f32
        };
        self.report(self.weights.audio + self.weights.video * share, done);
    }

    /// A pause leaves a gap the speed is not measured over.
    fn resumed(&mut self) {
        self.window.clear();
    }

    /// The pictures start again from the first.
    fn restart(&mut self) {
        self.window.clear();
        self.speed = 0.0;
        self.report(self.weights.audio, 0);
    }

    fn finishing(&mut self) {
        self.report(1.0 - self.weights.finish, self.total);
    }
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

/// The hidden file an export of `destination` is written to first, beside
/// it: `.<name>.<job>.part`, the name shortened so the whole stays within
/// [`NAME_MAX`] bytes.
pub(crate) fn part_path(destination: &Path, id: Uuid) -> PathBuf {
    hidden_part(destination, id)
}

fn hidden_part(path: &Path, id: Uuid) -> PathBuf {
    let name = path
        .file_name()
        .map_or_else(|| "export".into(), |name| name.to_string_lossy());
    let ending = format!(".{id}.part");
    let room = NAME_MAX - 1 - ending.len();
    let mut cut = name.len().min(room);
    while !name.is_char_boundary(cut) {
        cut -= 1;
    }
    path.with_file_name(format!(".{}{ending}", &name[..cut]))
}

/// The job's working folder, for its sound and captions, when the
/// destination's folder takes none: in the system's temporary folder.
pub(crate) fn temp_path(id: Uuid) -> PathBuf {
    std::env::temp_dir().join(format!("{TEMP_PREFIX}{id}"))
}

/// The job's working folder beside its destination, hidden: on the same
/// disk as the file it makes, and not in a temporary folder that may be
/// held in memory.
pub(crate) fn work_path(destination: &Path, id: Uuid) -> PathBuf {
    destination.with_file_name(format!(".{TEMP_PREFIX}{id}"))
}

/// How working folders are named, before the job's id.
const TEMP_PREFIX: &str = "aster-render-";

/// The caption file beside an export: `<stem>.<language>.<ext>`, or
/// `<stem>.<ext>` when the track has no usable language.
pub(crate) fn sidecar_path(
    destination: &Path,
    language: Option<&str>,
    format: SidecarFormat,
) -> PathBuf {
    let stem = destination
        .file_stem()
        .map_or_else(|| "export".into(), |stem| stem.to_string_lossy());
    let language = language.filter(|tag| {
        !tag.is_empty()
            && tag.len() <= 35
            && tag
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    });
    let name = match language {
        Some(language) => format!("{stem}.{language}.{}", format.extension()),
        None => format!("{stem}.{}", format.extension()),
    };
    destination.with_file_name(name)
}

/// The caption file an export of `project` with `settings` writes beside
/// `destination`, when it asks for one: the language is the caption track's
/// the plan takes. `None` also when there is no such track; the plan then
/// refuses the export.
pub(crate) fn sidecar_for(
    project: &Project,
    settings: &ExportSettings,
    destination: &Path,
) -> Option<PathBuf> {
    let format = settings.subtitles.sidecar?;
    let mut tracks = editor_core::export::caption_tracks(project);
    let track = match settings.subtitles.track {
        Some(id) => tracks.find(|track| track.id == id),
        None => tracks.next(),
    }?;
    Some(sidecar_path(destination, track.language.as_deref(), format))
}

/// Removes what a job leaves while it runs: its part files beside the
/// destination and its working folders. For a job that stopped without
/// tidying up.
pub(crate) fn clean(id: Uuid, destination: &Path) {
    let _ = fs::remove_file(part_path(destination, id));
    let _ = fs::remove_dir_all(work_path(destination, id));
    let _ = fs::remove_dir_all(temp_path(id));
    let ending = format!(".{id}.part");
    if let Some(folder) = destination.parent()
        && let Ok(entries) = fs::read_dir(folder)
    {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') && name.ends_with(&ending) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

/// Removes the working folders in `folder` (the system's temporary folder)
/// that exports stopped by a crash or a kill left: `aster-render-<id>`,
/// unchanged for [`STALE_AFTER`]. Returns how many went.
pub(crate) fn sweep_temp(folder: &Path, now: SystemTime) -> usize {
    sweep(folder, now, &HashSet::new(), |name| {
        name.strip_prefix(TEMP_PREFIX).map(|id| (id, true))
    })
}

/// Removes, from the folder exports are written to, what stopped exports
/// left there: hidden part files `.<name>.<id>.part` and working folders
/// `.aster-render-<id>`, unchanged for [`STALE_AFTER`] and of no job in
/// `live`. Returns how many went.
pub(crate) fn sweep_folder(folder: &Path, live: &HashSet<Uuid>, now: SystemTime) -> usize {
    sweep(folder, now, live, |name| {
        let hidden = name.strip_prefix('.')?;
        if let Some(id) = hidden.strip_prefix(TEMP_PREFIX) {
            return Some((id, true));
        }
        let rest = hidden.strip_suffix(".part")?;
        let (_, id) = rest.rsplit_once('.')?;
        Some((id, false))
    })
}

/// Removes the entries of `folder` whose name `left_over` reads as a job's
/// file (its id, and whether it is a folder), when that job is not in `live`
/// and nothing in the entry changed for [`STALE_AFTER`].
fn sweep(
    folder: &Path,
    now: SystemTime,
    live: &HashSet<Uuid>,
    left_over: impl Fn(&str) -> Option<(&str, bool)>,
) -> usize {
    let Ok(entries) = fs::read_dir(folder) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some((id, is_folder)) = left_over(name) else {
            continue;
        };
        let Ok(id) = Uuid::parse_str(id) else {
            continue;
        };
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if live.contains(&id) || kind.is_symlink() || kind.is_dir() != is_folder {
            continue;
        }
        let path = entry.path();
        if !stale(&path, is_folder, now) {
            continue;
        }
        let gone = if is_folder {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        removed += usize::from(gone.is_ok());
    }
    removed
}

/// Nothing at `path`, nor directly in it when it is a folder, changed for
/// [`STALE_AFTER`] before `now`.
fn stale(path: &Path, is_folder: bool, now: SystemTime) -> bool {
    let old = |meta: &fs::Metadata| {
        meta.modified()
            .ok()
            .and_then(|at| now.duration_since(at).ok())
            .is_some_and(|age| age >= STALE_AFTER)
    };
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if !old(&meta) {
        return false;
    }
    if !is_folder {
        return true;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return false;
    };
    entries
        .flatten()
        .all(|entry| entry.metadata().is_ok_and(|meta| old(&meta)))
}

/// A job's files, removed when it ends unless moved into place.
struct Scratch {
    /// The working folder; beside the destination unless that folder took
    /// none.
    temp: PathBuf,
    /// Where the working folder goes when the destination's folder takes
    /// none.
    elsewhere: PathBuf,
    part: PathBuf,
    sidecar: Option<PathBuf>,
    moved: bool,
}

impl Scratch {
    fn new(id: Uuid, destination: &Path) -> Self {
        Self {
            temp: work_path(destination, id),
            elsewhere: temp_path(id),
            part: part_path(destination, id),
            sidecar: None,
            moved: false,
        }
    }

    /// Makes the working folder: beside the destination, or else in the
    /// system's temporary folder.
    fn make_temp(&mut self) -> std::io::Result<()> {
        match fs::create_dir(&self.temp) {
            Ok(()) => return Ok(()),
            Err(error)
                if error.kind() == std::io::ErrorKind::AlreadyExists && self.temp.is_dir() =>
            {
                return Ok(());
            }
            Err(_) => {}
        }
        fs::create_dir_all(&self.elsewhere)?;
        self.temp = self.elsewhere.clone();
        Ok(())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if !self.moved {
            let _ = fs::remove_file(&self.part);
        }
        if let Some(sidecar) = &self.sidecar {
            let _ = fs::remove_file(sidecar);
        }
        let _ = fs::remove_dir_all(&self.temp);
        let _ = fs::remove_dir_all(&self.elsewhere);
    }
}

/// Puts a finished file's bytes on disk before it replaces anything.
fn settle(path: &Path) {
    if let Ok(file) = fs::OpenOptions::new().write(true).open(path) {
        let _ = file.sync_all();
    }
}

/// Makes a rename in `folder` last, where the system allows it.
fn settle_folder(folder: &Path) {
    #[cfg(unix)]
    if let Ok(folder) = fs::File::open(folder) {
        let _ = folder.sync_all();
    }
    #[cfg(not(unix))]
    let _ = folder;
}

// ---------------------------------------------------------------------------
// Test hook: a first encoder that fails
// ---------------------------------------------------------------------------

#[cfg(test)]
static FAIL_FIRST_ENCODER: std::sync::Mutex<Vec<(PathBuf, String)>> =
    std::sync::Mutex::new(Vec::new());

/// Makes the next encode of the job writing `destination` fail after a
/// few pictures, as a broken hardware encoder would. Asked twice, the two
/// next encodes fail.
#[cfg(test)]
pub(crate) fn fail_first_encoder(destination: &Path) {
    fail_encoder_with(destination, "the encoder was made to fail for a test");
}

/// Like [`fail_first_encoder`], FFmpeg saying `why`.
#[cfg(test)]
pub(crate) fn fail_encoder_with(destination: &Path, why: &str) {
    FAIL_FIRST_ENCODER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push((destination.to_path_buf(), why.to_owned()));
}

/// Why the next encode of `destination` is to fail, when a test asked.
fn forced_failure(destination: &Path) -> Option<String> {
    #[cfg(test)]
    {
        let mut forced = FAIL_FIRST_ENCODER
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(index) = forced.iter().position(|(path, _)| path == destination) {
            return Some(forced.remove(index).1);
        }
    }
    let _ = destination;
    None
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
