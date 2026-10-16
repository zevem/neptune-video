//! The timeline's sound for an export, mixed offline by the playback mixer
//! into a raw file the encoder reads beside the pictures.
use crate::{MediaError, audio::OfflineMix, io, process::Processes};
use editor_core::{Project, export::SampleSpan};
use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// Renders an export's sound. Its decoders are its own processes; the
/// canceller, made before `render` starts, stops it from any thread.
pub struct AudioRender {
    processes: Processes,
    cancelled: Arc<AtomicBool>,
}

impl Default for AudioRender {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioRender {
    pub fn new() -> Self {
        Self {
            processes: Processes::default(),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    #[cfg(test)]
    pub(crate) fn processes(&self) -> &Processes {
        &self.processes
    }

    /// Stops a render in progress, or the next one, and kills its decoders.
    pub fn canceller(&self) -> impl Fn() + Send + Sync + 'static {
        let processes = self.processes.clone();
        let cancelled = Arc::clone(&self.cancelled);
        move || {
            cancelled.store(true, Ordering::Release);
            processes.kill_all();
        }
    }

    /// Measures the same post-limiter PCM that an export writes, without a
    /// temporary audio file. This blocks on the caller's worker thread and
    /// uses this render's cancellation and decoder ownership.
    pub fn measure_loudness(
        &self,
        project: Arc<Project>,
        span: SampleSpan,
        progress: &dyn Fn(f32),
    ) -> Result<(crate::Loudness, Vec<String>), MediaError> {
        let cancelled = || self.cancelled.load(Ordering::Acquire);
        let result = super::loudness::measure(project, span, &self.processes, &cancelled, progress);
        match result {
            Err(_) if cancelled() => Err(MediaError::Cancelled),
            result => result,
        }
    }

    /// Writes exactly `span.frames` interleaved little-endian `f32` sample
    /// frames of `project`'s sound from `span.start` to `out`, as playback
    /// would mix them at `span.rate` with `span.channels` channels. Silence
    /// fills where nothing sounds. `progress` is told 0..=1 as it goes. On
    /// failure or cancellation `out` is removed and every decoder reaped.
    /// Answers what the mix left out: audio effects this FFmpeg cannot run,
    /// which playback leaves out too.
    pub fn render(
        &self,
        project: Arc<Project>,
        span: SampleSpan,
        out: &Path,
        progress: &dyn Fn(f32),
    ) -> Result<Vec<String>, MediaError> {
        let cancelled = || self.cancelled.load(Ordering::Acquire);
        let result = write(project, span, out, &self.processes, &cancelled, progress);
        if result.is_err() {
            let _ = fs::remove_file(out);
        }
        match result {
            Err(_) if cancelled() => Err(MediaError::Cancelled),
            result => result,
        }
    }
}

/// Renders `span` of `project`'s sound to `out` with a render of its own:
/// see [`AudioRender::render`]. `cancel` is checked between chunks of about
/// 20 ms; use [`AudioRender`] to also stop a decoder that is blocked.
pub fn render_audio(
    project: Arc<Project>,
    span: SampleSpan,
    out: &Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(f32),
) -> Result<Vec<String>, MediaError> {
    let render = AudioRender::new();
    let check = || cancel.load(Ordering::Acquire) || render.cancelled.load(Ordering::Acquire);
    let result = write(project, span, out, &render.processes, &check, progress);
    if result.is_err() {
        let _ = fs::remove_file(out);
    }
    match result {
        Err(_) if check() => Err(MediaError::Cancelled),
        result => result,
    }
}

fn write(
    project: Arc<Project>,
    span: SampleSpan,
    out: &Path,
    processes: &Processes,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(f32),
) -> Result<Vec<String>, MediaError> {
    if !(1..=8).contains(&span.channels) {
        return Err(MediaError::Invalid(
            "Export sound must have between 1 and 8 channels".into(),
        ));
    }
    if !(8_000..=384_000).contains(&span.rate) {
        return Err(MediaError::Invalid(format!(
            "{} Hz is not a sample rate the export can render",
            span.rate
        )));
    }
    let channels = usize::from(span.channels);
    let file =
        File::create(out).map_err(|error| io("Cannot create the export's sound file", error))?;
    let mut file = BufWriter::with_capacity(1 << 20, file);
    // The mixer and its decoders drop (kill and reap) when this returns.
    let mut mix =
        OfflineMix::with_layout(project, span.start, span.rate, channels, processes.clone());
    let mut written = 0_u64;
    let mut bytes = Vec::new();
    let mut left_out = Vec::new();
    progress(0.0);
    while written < span.frames {
        if cancelled() {
            return Err(MediaError::Cancelled);
        }
        let chunk = mix
            .next_up_to(span.frames - written)
            .map_err(export_limit)?;
        let frames = ((chunk.len() / channels) as u64).min(span.frames - written);
        if frames == 0 {
            return Err(MediaError::Invalid(
                "The sound mixer produced nothing".into(),
            ));
        }
        bytes.clear();
        bytes.extend(
            chunk[..frames as usize * channels]
                .iter()
                .flat_map(|sample| sample.to_le_bytes()),
        );
        file.write_all(&bytes)
            .map_err(|error| io("Cannot write the export's sound", error))?;
        written += frames;
        progress((written as f64 / span.frames as f64) as f32);
        left_out.extend(mix.left_out());
    }
    file.flush()
        .map_err(|error| io("Cannot write the export's sound", error))?;
    drop(mix);
    if cancelled() {
        return Err(MediaError::Cancelled);
    }
    progress(1.0);
    Ok(left_out)
}

/// The mixer's voice limit, said of the export rather than the preview.
fn export_limit(error: MediaError) -> MediaError {
    match error {
        MediaError::Invalid(message) if message.starts_with("Preview audio supports at most") => {
            MediaError::Invalid(
                "Export sound supports at most 16 simultaneous audible clips; mute extra tracks"
                    .into(),
            )
        }
        error => error,
    }
}
