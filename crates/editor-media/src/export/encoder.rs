//! The FFmpeg process that writes an export: raw RGBA pictures on its input
//! pipe, the rendered sound and subtitles from files, one output file.
use super::{
    EncodeInputs,
    args::{encode_args, environment},
};
use crate::{
    MediaError,
    process::{Process, Processes, tool_command},
};
use editor_core::export::ExportPlan;
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// How long a failed write waits for FFmpeg to exit and say why.
const FAILURE_GRACE: Duration = Duration::from_secs(2);

/// One running encode. Pictures go in with `write_frame`; `finish` ends the
/// streams and waits for a clean exit. Dropping it unfinished, or calling its
/// canceller, kills FFmpeg: the output is then incomplete and the caller
/// removes it. Pausing is not writing: FFmpeg waits on its pipe.
pub struct Encoder {
    process: Process,
    processes: Processes,
    cancelled: Arc<AtomicBool>,
    frame_bytes: Option<usize>,
    frames: u64,
}

impl Encoder {
    /// Starts FFmpeg for `plan`, writing `output` (a part file: the container
    /// is named explicitly, not taken from its extension). The inputs must
    /// match the plan: pictures of its output size and rate when it has
    /// video, sound at its rate and channels when it has audio.
    pub fn start(
        plan: &ExportPlan,
        inputs: &EncodeInputs,
        output: &Path,
    ) -> Result<Self, MediaError> {
        check(plan, inputs)?;
        let args = encode_args(plan, inputs, output);
        let mut command = tool_command("ffmpeg");
        command.args(&args);
        if let Some((key, value)) = plan
            .video
            .as_ref()
            .and_then(|video| environment(&video.encoder))
        {
            command.env(key, value);
        }
        let processes = Processes::default();
        let frame_bytes = inputs
            .video
            .map(|video| video.size[0] as usize * video.size[1] as usize * 4);
        // Sound-only exports read files alone and get no input pipe.
        let process = if frame_bytes.is_some() {
            Process::spawn_writer("ffmpeg", &mut command, &processes)?
        } else {
            Process::spawn("ffmpeg", &mut command, &processes)?
        };
        Ok(Self {
            process,
            processes,
            cancelled: Arc::new(AtomicBool::new(false)),
            frame_bytes,
            frames: 0,
        })
    }

    /// Writes the next output picture, tightly packed RGBA of the output
    /// size. FFmpeg is killed if it has not taken the picture within
    /// `deadline`.
    pub fn write_frame(&mut self, rgba: &[u8], deadline: Duration) -> Result<(), MediaError> {
        let Some(bytes) = self.frame_bytes else {
            return Err(MediaError::Invalid(
                "This export has no picture to write".into(),
            ));
        };
        if rgba.len() != bytes {
            return Err(MediaError::Invalid(format!(
                "An export picture must be {bytes} bytes, not {}",
                rgba.len()
            )));
        }
        if self.cancelled.load(Ordering::Acquire) {
            return Err(MediaError::Cancelled);
        }
        match self.process.write_until(rgba, Instant::now() + deadline) {
            Ok(()) => {
                self.frames += 1;
                Ok(())
            }
            Err(_) if self.cancelled.load(Ordering::Acquire) => Err(MediaError::Cancelled),
            Err(error) => {
                // FFmpeg usually stopped reading because it failed; its own
                // words say more than the broken pipe.
                let until = Instant::now() + FAILURE_GRACE;
                let _ = self.process.wait_until(until);
                let stderr = self.process.stderr_until(until);
                if stderr.trim().is_empty() {
                    Err(error)
                } else {
                    Err(self
                        .process
                        .failure(&format!("Encoding stopped after {} frames", self.frames)))
                }
            }
        }
    }

    /// Pictures written so far.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Ends the picture stream and waits up to `deadline` for FFmpeg to
    /// write the file's end and exit. Anything but a clean exit is an error
    /// carrying the end of FFmpeg's error output.
    pub fn finish(mut self, deadline: Duration) -> Result<(), MediaError> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(MediaError::Cancelled);
        }
        self.process.close_input();
        let until = Instant::now() + deadline;
        let status = self.process.wait_until(until)?;
        if self.cancelled.load(Ordering::Acquire) {
            return Err(MediaError::Cancelled);
        }
        match status {
            Some(status) if status.success() => Ok(()),
            Some(status) => {
                let _ = self.process.stderr_until(Instant::now() + FAILURE_GRACE);
                Err(self.process.failure(&format!("Encoding failed ({status})")))
            }
            None => Err(self.process.failure(&format!(
                "Encoding did not finish within {} seconds",
                deadline.as_secs()
            ))),
        }
    }

    /// Kills the encode from any thread. The writer then sees `Cancelled`.
    pub fn canceller(&self) -> impl Fn() + Send + Sync + 'static {
        let processes = self.processes.clone();
        let cancelled = Arc::clone(&self.cancelled);
        move || {
            cancelled.store(true, Ordering::Release);
            processes.kill_all();
        }
    }
}

/// The inputs are the plan's: present together, of its size, rate and layout.
fn check(plan: &ExportPlan, inputs: &EncodeInputs) -> Result<(), MediaError> {
    match (&plan.video, &inputs.video) {
        (Some(video), Some(input)) => {
            if input.size != video.geometry.output || input.rate != video.frames.rate {
                return Err(MediaError::Invalid(
                    "The pictures given differ from the export's size or frame rate".into(),
                ));
            }
        }
        (None, None) => {}
        _ => {
            return Err(MediaError::Invalid(
                "The export's pictures and its plan disagree".into(),
            ));
        }
    }
    match (&plan.audio, &inputs.audio) {
        (Some(audio), Some(input)) => {
            if input.rate != audio.settings.sample_rate || input.channels != audio.settings.channels
            {
                return Err(MediaError::Invalid(
                    "The sound given differs from the export's rate or channels".into(),
                ));
            }
        }
        (None, None) => {}
        _ => {
            return Err(MediaError::Invalid(
                "The export's sound and its plan disagree".into(),
            ));
        }
    }
    if inputs.subtitles.is_some()
        && (plan.settings.container.embedded_subtitles().is_none() || plan.video.is_none())
    {
        return Err(MediaError::Invalid(format!(
            "{} files cannot carry a subtitle stream here",
            plan.settings.container.label()
        )));
    }
    if plan.video.is_none() && plan.audio.is_none() {
        return Err(MediaError::Invalid(
            "The export has neither pictures nor sound".into(),
        ));
    }
    Ok(())
}
