//! Meter the export mix a chunk at a time. PCM, decoder pipes and the meter's
//! error tail have the same finite bounds for a minute or a long timeline.
use crate::{
    Loudness, MediaError,
    audio::OfflineMix,
    process::{Process, Processes, tool_command},
};
use editor_core::{Project, export::SampleSpan};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

pub(super) fn measure(
    project: Arc<Project>,
    span: SampleSpan,
    processes: &Processes,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(f32),
) -> Result<(Loudness, Vec<String>), MediaError> {
    if !span.start.is_valid()
        || span.frames == 0
        || !(8_000..=384_000).contains(&span.rate)
        || !(1..=8).contains(&span.channels)
    {
        return Err(MediaError::Invalid(
            "Loudness needs a nonempty sample span, 8000..384000 Hz and 1..8 channels".into(),
        ));
    }
    if cancelled() {
        return Err(MediaError::Cancelled);
    }
    let mut command = tool_command("ffmpeg");
    command
        .args([
            "-hide_banner",
            "-nostdin",
            "-loglevel",
            "info",
            "-nostats",
            "-threads",
            "1",
            "-f",
            "f32le",
            "-ar",
        ])
        .arg(span.rate.to_string())
        .arg("-ac")
        .arg(span.channels.to_string())
        .args([
            "-i",
            "pipe:0",
            "-af",
            "ebur128=peak=true:framelog=verbose",
            "-f",
            "null",
            "-",
        ]);
    let mut meter = Process::spawn_writer("ffmpeg", &mut command, processes)?;
    let channels = usize::from(span.channels);
    let mut mix =
        OfflineMix::with_layout(project, span.start, span.rate, channels, processes.clone());
    let mut written = 0;
    let mut bytes = Vec::with_capacity(1024 * channels * 4);
    let mut notices = Vec::new();
    while written < span.frames {
        if cancelled() {
            return Err(MediaError::Cancelled);
        }
        let chunk = mix.next_up_to(span.frames - written)?;
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
        meter.write_until(&bytes, Instant::now() + Duration::from_secs(10))?;
        notices.extend(mix.left_out());
        written += frames;
        progress(written as f32 / span.frames as f32);
    }
    drop(mix);
    meter.close_input();
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        if cancelled() {
            return Err(MediaError::Cancelled);
        }
        let now = Instant::now();
        if now >= until {
            return Err(meter.failure("the loudness meter did not finish"));
        }
        if let Some(status) = meter.wait_until((now + Duration::from_millis(50)).min(until))? {
            if !status.success() {
                return Err(meter.failure("could not measure the mix"));
            }
            break;
        }
    }
    let loudness =
        crate::sound::loudness_summary(&meter.stderr_tail()).map_err(MediaError::Decode)?;
    progress(1.0);
    Ok((loudness, notices))
}
