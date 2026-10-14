//! What this host's FFmpeg can encode: every known encoder it lists is asked
//! to encode a test picture (or a tenth of a second of silence) with the
//! export's own arguments, so only encoders that worked here are offered.
use super::args::{audio_args, device_args, environment, prefix, video_args};
use crate::{
    MediaError,
    process::{Process, Processes, capture, tool_command},
};
use editor_core::{
    FrameRate, Time,
    export::{
        AUDIO_ENCODERS, AudioEncoder, AudioPlan, AudioSettings, Capabilities, Container, Fit,
        FrameSpan, Geometry, KnownVideoEncoder, SampleSpan, Unavailable, VIDEO_ENCODERS,
        VideoEncoder, VideoPlan, VideoSettings,
    },
};
use std::{
    collections::HashSet,
    ffi::OsString,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

/// Test encodes running at once.
const PROBES_AT_ONCE: usize = 4;
/// How long one test encode may take before it counts as failed.
const PROBE_DEADLINE: Duration = Duration::from_secs(10);
/// The test picture: small enough for any encoder, above NVENC's minimum.
const PROBE_PICTURE: &str = "color=black:s=256x144:r=30,format=rgba";
const LISTING_BYTES: usize = 4 * 1024 * 1024;

/// Probes the installed FFmpeg. Blocking: run it on a worker. A missing
/// FFmpeg is an error; an encoder that fails its test is listed in
/// `unavailable` with the reason FFmpeg gave.
pub fn probe_capabilities() -> Result<Capabilities, MediaError> {
    let processes = Processes::default();
    let listing = |args: &[&str]| -> Result<String, MediaError> {
        let mut command = tool_command("ffmpeg");
        command.args(args);
        let bytes = capture("ffmpeg", &mut command, &processes, LISTING_BYTES)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    };
    let version = listing(&["-hide_banner", "-version"])?;
    let encoders = names(&listing(&["-hide_banner", "-encoders"])?);
    let muxers = names(&listing(&["-hide_banner", "-muxers"])?);

    let trials: Vec<Trial> = VIDEO_ENCODERS
        .iter()
        .filter(|known| encoders.contains(known.name))
        .map(Trial::Video)
        .chain(
            AUDIO_ENCODERS
                .iter()
                .filter(|(name, _)| encoders.contains(*name))
                .map(|&(name, codec)| {
                    Trial::Audio(AudioEncoder {
                        name: name.into(),
                        codec,
                    })
                }),
        )
        .collect();
    let devices = render_nodes();
    let outcomes = run_all(&trials, &devices);

    let mut capabilities = Capabilities {
        ffmpeg: version.lines().next().unwrap_or_default().trim().to_owned(),
        containers: Container::ALL
            .into_iter()
            .filter(|container| muxers.contains(container.muxer()))
            .collect(),
        ..Capabilities::default()
    };
    for (trial, outcome) in trials.into_iter().zip(outcomes) {
        match (trial, outcome) {
            (Trial::Video(known), Ok(device)) => {
                capabilities.video.push(VideoEncoder {
                    device,
                    ..video_encoder(known, None)
                });
            }
            (Trial::Audio(encoder), Ok(_)) => capabilities.audio.push(encoder),
            (trial, Err(reason)) => capabilities.unavailable.push(Unavailable {
                name: trial.name().into(),
                reason,
            }),
        }
    }
    Ok(capabilities)
}

enum Trial {
    Video(&'static KnownVideoEncoder),
    Audio(AudioEncoder),
}

impl Trial {
    fn name(&self) -> &str {
        match self {
            Self::Video(known) => known.name,
            Self::Audio(encoder) => &encoder.name,
        }
    }
}

/// The device a working encoder used, or why it did not work.
type Outcome = Result<Option<String>, String>;

/// Runs every trial, at most `PROBES_AT_ONCE` at a time, in their order.
fn run_all(trials: &[Trial], devices: &[PathBuf]) -> Vec<Outcome> {
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<Outcome>>> = Mutex::new(trials.iter().map(|_| None).collect());
    thread::scope(|scope| {
        for _ in 0..PROBES_AT_ONCE.min(trials.len()) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::AcqRel);
                    let Some(trial) = trials.get(index) else {
                        return;
                    };
                    let outcome = run(trial, devices);
                    if let Ok(mut results) = results.lock() {
                        results[index] = Some(outcome);
                    }
                }
            });
        }
    });
    results
        .into_inner()
        .unwrap_or_default()
        .into_iter()
        .map(|outcome| outcome.unwrap_or_else(|| Err("The test encode did not run".into())))
        .collect()
}

fn run(trial: &Trial, devices: &[PathBuf]) -> Outcome {
    match trial {
        Trial::Audio(encoder) => attempt(&audio_probe(encoder), None, &encoder.name).map(|_| None),
        Trial::Video(known) if known.hardware == Some(editor_core::export::HardwareKind::Vaapi) => {
            // The default node may belong to another GPU (here the first is
            // NVIDIA's); the node that encoded is kept for the export.
            if devices.is_empty() {
                return Err("No /dev/dri/renderD* device to encode on".into());
            }
            let mut reasons = Vec::new();
            for device in devices {
                let device = device.to_string_lossy().into_owned();
                let encoder = video_encoder(known, Some(device.clone()));
                match attempt(&video_probe(&encoder), environment(&encoder), known.name) {
                    Ok(()) => return Ok(Some(device)),
                    Err(reason) => reasons.push(format!("{device}: {reason}")),
                }
            }
            Err(reasons.join("; "))
        }
        Trial::Video(known) => {
            let encoder = video_encoder(known, None);
            attempt(&video_probe(&encoder), environment(&encoder), known.name).map(|_| None)
        }
    }
}

fn video_encoder(known: &KnownVideoEncoder, device: Option<String>) -> VideoEncoder {
    VideoEncoder {
        name: known.name.into(),
        codec: known.codec,
        hardware: known.hardware,
        max_side: known.max_side,
        min_size: known.min_size,
        device,
    }
}

/// Two frames of the test picture through the export's video arguments.
fn video_probe(encoder: &VideoEncoder) -> Vec<OsString> {
    let rate = FrameRate {
        numerator: 30,
        denominator: 1,
    };
    let size = [256, 144];
    let video = VideoPlan {
        frames: FrameSpan {
            rate,
            first: 0,
            count: 2,
        },
        geometry: Geometry::new(size, size, Fit::Letterbox),
        encoder: encoder.clone(),
        settings: VideoSettings {
            codec: encoder.codec,
            ..VideoSettings::default()
        },
    };
    let mut args: Vec<OsString> = prefix().into_iter().map(Into::into).collect();
    args.extend(device_args(encoder));
    args.extend(["-f", "lavfi", "-i", PROBE_PICTURE, "-frames:v", "2"].map(Into::into));
    args.extend(video_args(&video, None));
    args.extend(["-f", "null", "-"].map(Into::into));
    args
}

/// A tenth of a second of stereo silence through the export's audio
/// arguments. A video-only test would never open an audio encoder.
fn audio_probe(encoder: &AudioEncoder) -> Vec<OsString> {
    let rate = 48_000;
    let audio = AudioPlan {
        samples: SampleSpan {
            start: Time::ZERO,
            frames: 4_800,
            rate,
            channels: 2,
        },
        encoder: encoder.clone(),
        settings: AudioSettings {
            codec: encoder.codec,
            kbps: encoder.codec.default_kbps(),
            sample_rate: rate,
            channels: 2,
        },
    };
    let mut args: Vec<OsString> = prefix().into_iter().map(Into::into).collect();
    args.extend(
        [
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=48000:cl=stereo",
            "-t",
            "0.1",
            "-map",
            "0:a",
        ]
        .map(Into::into),
    );
    args.extend(audio_args(&audio));
    args.extend(["-f", "null", "-"].map(Into::into));
    args
}

/// Runs one test encode with its deadline.
fn attempt(
    args: &[OsString],
    environment: Option<(&str, &str)>,
    encoder: &str,
) -> Result<(), String> {
    let processes = Processes::default();
    let mut command = tool_command("ffmpeg");
    command.args(args);
    if let Some((key, value)) = environment {
        command.env(key, value);
    }
    let process =
        Process::spawn("ffmpeg", &mut command, &processes).map_err(|error| error.to_string())?;
    let status = process
        .wait_until(Instant::now() + PROBE_DEADLINE)
        .map_err(|error| error.to_string())?;
    match status {
        Some(status) if status.success() => Ok(()),
        Some(status) => {
            let stderr = process.stderr_until(Instant::now() + Duration::from_secs(1));
            Err(reason(&stderr, encoder).unwrap_or_else(|| format!("FFmpeg exited with {status}")))
        }
        None => Err(format!(
            "The test encode did not finish within {} seconds",
            PROBE_DEADLINE.as_secs()
        )),
    }
}

/// The line of FFmpeg's error output that says why an encoder failed: the
/// encoder's own complaint or a device failure before the generic lines that
/// follow it, otherwise the first line. Log prefixes are removed.
pub(crate) fn reason(stderr: &str, encoder: &str) -> Option<String> {
    let own = format!("[{encoder} @");
    let lines = || {
        stderr
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
    };
    let line = lines()
        .find(|line| {
            line.contains(&own)
                || line.contains("[enc:")
                || line.contains("Device creation failed")
                || line.contains("Failed to initialise")
        })
        .or_else(|| lines().next())?;
    let mut text = line;
    // `[h264_qsv @ 0x…] [enc:h264_qsv @ 0x…] Error …` → `Error …`.
    while let Some(rest) = text.strip_prefix('[') {
        match rest.find("] ") {
            Some(end) => text = rest[end + 2..].trim_start(),
            None => break,
        }
    }
    let text = if text.is_empty() { line } else { text };
    Some(text.chars().take(300).collect())
}

/// Names from `-encoders` or `-muxers`: the second column after the rule.
fn names(listing: &str) -> HashSet<String> {
    listing
        .lines()
        .skip_while(|line| !line.trim_start().starts_with("--"))
        .skip(1)
        .filter_map(|line| line.split_whitespace().nth(1))
        .flat_map(|names| names.split(','))
        .map(str::to_owned)
        .collect()
}

/// VA-API render nodes, in order.
fn render_nodes() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir("/dev/dri") else {
        return Vec::new();
    };
    let mut nodes: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("renderD"))
        .map(|entry| entry.path())
        .collect();
    nodes.sort();
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listings_give_their_names() {
        let encoders = "Encoders:\n V..... = Video\n ------\n V....D libx264              libx264 H.264\n A....D aac                  AAC\n";
        let names = names(encoders);
        assert!(names.contains("libx264") && names.contains("aac"));
        assert!(!names.contains("V....."));
        let muxers = "Formats:\n D.. = Demuxing\n ---\n  E  mp4             MP4\n DE  mov,mp4m         QuickTime\n";
        let names = super::names(muxers);
        assert!(names.contains("mp4") && names.contains("mov") && names.contains("mp4m"));
    }

    #[test]
    fn a_failed_probe_names_the_encoder_or_device_complaint() {
        let qsv = "[h264_qsv @ 0x59b34ec6d780] Error creating a MFX session: -9.\n[vost#0:0/h264_qsv @ 0x1] [enc:h264_qsv @ 0x2] Error while opening encoder\nNothing was written into output file, because at least one of its streams received no packets.\n";
        assert_eq!(
            reason(qsv, "h264_qsv").as_deref(),
            Some("Error creating a MFX session: -9.")
        );
        let vaapi = "[VAAPI @ 0x5e6] Failed to initialise VAAPI connection: -1 (unknown libva error).\nDevice creation failed: -5.\nError parsing global options: Input/output error\n";
        assert_eq!(
            reason(vaapi, "h264_vaapi").as_deref(),
            Some("Failed to initialise VAAPI connection: -1 (unknown libva error).")
        );
        assert_eq!(
            reason("\n\nUnknown encoder 'x'\n", "x").as_deref(),
            Some("Unknown encoder 'x'")
        );
        assert_eq!(reason("  \n", "x"), None);
    }
}
