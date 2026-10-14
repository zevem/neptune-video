//! A span of a media file's sound measured for loudness by FFmpeg's
//! `ebur128` filter, behind the clip's own audio effects. Nothing is written
//! and nothing is kept: the figures are for the one who asked.
//!
//! The filter writes its summary to the error pipe as FFmpeg ends, at the
//! `info` level, followed by more lines; `framelog=verbose` keeps its ten
//! lines a second out of it at that level. Progress comes on stdout from
//! `-progress`.
use super::Control;
use crate::{
    MediaError,
    audio::chain::{self, Filters},
    process::{Process, Processes, tool_command},
};
use editor_core::{Asset, Effect, MediaKind, Time, look::MAX_EFFECTS};
use std::io::Read;
use uuid::Uuid;

/// The rate the effects run at in a measurement. A voice's chain runs at the
/// device's rate, and a filter's frequency is held under a part of it; this
/// is the common one.
const MEASURE_RATE: u32 = 48_000;
/// The channels the effects are written for and the meter hears, as on a
/// two-channel output.
const MEASURE_CHANNELS: usize = 2;
/// A voice is summed to the output's channels after its chain (`-ac`); the
/// measurement sums to two the same way, by FFmpeg's default matrix, before
/// the meter. Without it a 5.1 or mono source is metered in its own layout,
/// and its loudness and true peak are not those a stereo output plays.
const MEASURE_LAYOUT: &str = "aformat=channel_layouts=stereo";
/// The meter's absolute gate: an integrated loudness at or under it is the
/// floor, not a measurement.
pub const LOUDNESS_FLOOR: f32 = -70.0;
/// The longest progress line kept while it is read; FFmpeg's are short.
pub(super) const MAX_LINE: usize = 256;

/// A span of one media file's sound to be measured, with the effects of the
/// clip that plays it.
#[derive(Clone, Debug)]
pub struct LoudnessJob {
    /// The project that asks, handed back with the answer and never read.
    pub project: Uuid,
    pub asset: Asset,
    /// Where the span starts, in source time.
    pub from: Time,
    /// How long it is, in source time.
    pub span: Time,
    /// The clip's effects. Its audio effects run as in the clip's voice on a
    /// two-channel output, less `Pitch` and the clip's speed; the rest are
    /// passed over.
    pub effects: Vec<Effect>,
}

/// What a span measured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Loudness {
    /// Integrated loudness in LUFS and true peak in dBFS.
    Measured { integrated: f32, true_peak: f32 },
    /// No sample was other than nought: the true peak is minus infinity.
    Silent,
    /// Sound with no 0.4 s block above the meter's gate: a span too short
    /// or too quiet, which reads as the floor.
    TooShort,
}

// Refused here, before any process.
pub(super) fn check(job: &LoudnessJob) -> Result<(), MediaError> {
    let invalid = |why: &str| Err(MediaError::Invalid(why.into()));
    let asset = &job.asset;
    if !asset.has_audio && asset.kind != MediaKind::Audio {
        return invalid("Media has no sound to measure");
    }
    if !asset.duration.is_valid() || asset.duration <= Time::ZERO {
        return invalid("Media has no length");
    }
    if !job.from.is_valid() || !job.span.is_valid() || job.span <= Time::ZERO {
        return invalid("The span to measure is empty");
    }
    if job.from >= asset.duration {
        return invalid("The span to measure starts after the media ends");
    }
    if job.effects.len() > MAX_EFFECTS {
        return invalid("A clip has at most 16 effects");
    }
    Ok(())
}

/// The `-af` of a measurement: the clip's audio effects that this FFmpeg
/// runs, at [`MEASURE_RATE`], then the sum to two channels and the meter.
pub(super) fn graph(effects: &[Effect], filters: &Filters) -> String {
    let effects = chain::stack_effects(effects, MEASURE_RATE, MEASURE_CHANNELS, filters);
    let meter = format!("{MEASURE_LAYOUT},ebur128=peak=true:framelog=verbose");
    if effects.is_empty() {
        meter
    } else {
        format!("aresample={MEASURE_RATE},{effects},{meter}")
    }
}

/// Measures the job's span through one FFmpeg process, which writes nothing.
pub(super) fn measure(
    job: &LoudnessJob,
    filters: &Filters,
    processes: &Processes,
    control: &Control,
) -> Result<Loudness, MediaError> {
    // Given up while the session's probe ran: no process is started.
    if control.cancelled() {
        return Err(MediaError::Cancelled);
    }
    let asset = &job.asset;
    // What is read is the span, or what of it the file has.
    let length = job.span.min(Time(asset.duration.0 - job.from.0)).0.max(1) as f64;
    let mut command = tool_command("ffmpeg");
    command
        .args(["-hide_banner", "-nostdin", "-loglevel", "info", "-nostats"])
        .args(["-progress", "pipe:1", "-threads", "1", "-ss"])
        .arg(format!("{:.6}", job.from.as_seconds()))
        .arg("-t")
        .arg(format!("{:.6}", job.span.as_seconds()))
        .arg("-i")
        .arg(&asset.path)
        .args(["-map", "0:a:0", "-vn", "-sn", "-dn", "-af"])
        .arg(graph(&job.effects, filters))
        .args(["-f", "null", "-"]);
    let mut process = Process::spawn("ffmpeg", &mut command, processes)?;
    let mut bytes = [0u8; 4096];
    let mut line = Vec::with_capacity(MAX_LINE);
    let mut said = 0.0;
    loop {
        if control.cancelled() {
            return Err(MediaError::Cancelled);
        }
        // FFmpeg says how far it is twice a second, so a read that takes
        // the whole deadline is a process that is stuck.
        process.begin_read();
        let outcome = process.stdout.read(&mut bytes);
        let in_time = process.end_read_checked();
        if control.cancelled() {
            return Err(MediaError::Cancelled);
        }
        let count = match outcome {
            Ok(_) if !in_time => return Err(process.failure("the measurement took too long")),
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(process.failure(&format!("could not read its progress ({error})")));
            }
        };
        for &byte in &bytes[..count] {
            if byte != b'\n' {
                if line.len() < MAX_LINE {
                    line.push(byte);
                }
                continue;
            }
            if let Some(micros) = out_time(&line) {
                let done = (micros as f64 / length).clamp(0.0, 1.0) as f32;
                if done - said >= 0.01 {
                    said = done;
                    control.progress(done);
                }
            }
            line.clear();
        }
    }
    let status = super::await_exit(&process, control)?;
    if !status.success() {
        return Err(process.failure("could not measure the sound"));
    }
    summary(&process.stderr_tail()).map_err(|why| MediaError::Decode(format!("ffmpeg: {why}")))
}

/// The microseconds of a `-progress` line that says how far the output is.
pub(super) fn out_time(line: &[u8]) -> Option<i64> {
    let value = line.strip_prefix(b"out_time_us=")?;
    std::str::from_utf8(value).ok()?.trim().parse().ok()
}

/// The figures of the last `ebur128` summary in FFmpeg's report: the number
/// after `I:` and after `Peak:`, which may be `-inf`. Lines follow the
/// summary and others come before it; neither is read.
pub(crate) fn summary(report: &str) -> Result<Loudness, String> {
    let Some(at) = report.rfind("Summary:") else {
        return Err("its report has no loudness summary".into());
    };
    let (mut integrated, mut peak) = (None, None);
    for line in report[at..].lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("I:") {
            integrated = integrated.or(Some(figure(rest, "LUFS")));
        } else if let Some(rest) = line.strip_prefix("Peak:") {
            peak = peak.or(Some(figure(rest, "dBFS")));
        }
    }
    let integrated = (integrated.flatten())
        .filter(|value| value.is_finite())
        .ok_or("its loudness summary has no integrated loudness")?;
    let peak = (peak.flatten())
        .filter(|value| *value < f32::INFINITY)
        .ok_or("its loudness summary has no true peak")?;
    Ok(if peak == f32::NEG_INFINITY {
        Loudness::Silent
    } else if integrated <= LOUDNESS_FLOOR {
        Loudness::TooShort
    } else {
        Loudness::Measured {
            integrated,
            true_peak: peak,
        }
    })
}

/// A number and its unit, as the summary writes them: `-23.4 LUFS`.
fn figure(text: &str, unit: &str) -> Option<f32> {
    let mut words = text.split_whitespace();
    let value: f32 = words.next()?.parse().ok()?;
    (words.next() == Some(unit) && !value.is_nan()).then_some(value)
}
