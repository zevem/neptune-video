//! The check an export passes before it replaces anything: the file has the
//! streams asked for and lasts as long as it should.
use crate::{
    MediaError,
    process::{Processes, capture, tool_command},
};
use editor_core::{Time, export::AudioCodec};
use serde_json::Value;
use std::path::Path;

/// What a finished export must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Expectation {
    pub video: bool,
    pub audio: bool,
    pub subtitles: bool,
    /// The length rendered: the frame span's, or the samples' alone.
    pub duration: Time,
    /// One output frame; zero without video.
    pub frame: Time,
    /// Lossy codecs pad by a frame and a delay of their own.
    pub audio_codec: Option<AudioCodec>,
    pub sample_rate: u32,
}

impl Expectation {
    /// How far the file's duration may be from `duration`: one output frame,
    /// one codec frame and its delay, and 10 ms.
    pub fn tolerance(&self) -> Time {
        let padding = match self.audio_codec {
            Some(AudioCodec::Aac) => 2048,
            Some(AudioCodec::Opus) => 960 + 312,
            Some(AudioCodec::Mp3) => 1152 + 1105,
            _ => 0,
        };
        let padding = if self.sample_rate == 0 {
            Time::ZERO
        } else {
            Time(
                (padding * 1_000_000 + i64::from(self.sample_rate) - 1)
                    / i64::from(self.sample_rate),
            )
        };
        self.frame + padding + Time(10_000)
    }
}

/// Checks a finished export with ffprobe: as many video, audio and subtitle
/// streams as expected (one or none of each, counted by `codec_type`) and a
/// container duration within [`Expectation::tolerance`] of the expected
/// one. Stream durations and start times are not compared: Matroska has
/// none, and Opus and MP3 start off zero. Blocking.
pub fn verify_output(path: &Path, expect: &Expectation) -> Result<(), MediaError> {
    let mut command = tool_command("ffprobe");
    command
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration:stream=codec_type",
            "-of",
            "json",
        ])
        .arg(path);
    let bytes = capture("ffprobe", &mut command, &Processes::default(), 1024 * 1024)?;
    check(&bytes, expect)
}

pub(crate) fn check(probe: &[u8], expect: &Expectation) -> Result<(), MediaError> {
    let probe: Value = serde_json::from_slice(probe)
        .map_err(|error| MediaError::Decode(format!("ffprobe gave unreadable output: {error}")))?;
    let count = |kind: &str| {
        probe["streams"].as_array().map_or(0, |streams| {
            streams
                .iter()
                .filter(|stream| stream["codec_type"] == kind)
                .count()
        })
    };
    for (kind, label, wanted) in [
        ("video", "picture", expect.video),
        ("audio", "sound", expect.audio),
        ("subtitle", "subtitle", expect.subtitles),
    ] {
        let found = count(kind);
        if found != usize::from(wanted) {
            return Err(MediaError::Decode(format!(
                "The exported file has {found} {label} streams; {} expected",
                usize::from(wanted)
            )));
        }
    }
    let duration = probe["format"]["duration"]
        .as_str()
        .and_then(|value| value.parse::<f64>().ok())
        .or_else(|| probe["format"]["duration"].as_f64())
        .filter(|value| value.is_finite())
        .ok_or_else(|| MediaError::Decode("The exported file has no duration".into()))?;
    let duration = Time::from_seconds(duration);
    let off = Time((duration.0 - expect.duration.0).abs());
    if off > expect.tolerance() {
        return Err(MediaError::Decode(format!(
            "The exported file lasts {:.3}s; {:.3}s expected",
            duration.as_seconds(),
            expect.duration.as_seconds()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expectation() -> Expectation {
        Expectation {
            video: true,
            audio: true,
            subtitles: false,
            duration: Time(3_336_667),
            frame: Time(33_367),
            audio_codec: Some(AudioCodec::Opus),
            sample_rate: 48_000,
        }
    }

    #[test]
    fn streams_are_counted_and_the_duration_is_held_to_its_tolerance() {
        // Measured: WebM with VP9 and Opus for 100 frames at 30000/1001.
        let webm = br#"{"streams":[{"codec_type":"video"},{"codec_type":"audio"}],"format":{"duration":"3.344000"}}"#;
        check(webm, &expectation()).unwrap();
        let long = br#"{"streams":[{"codec_type":"video"},{"codec_type":"audio"}],"format":{"duration":"3.500000"}}"#;
        assert!(check(long, &expectation()).is_err());
        let silent = br#"{"streams":[{"codec_type":"video"}],"format":{"duration":"3.336667"}}"#;
        assert!(check(silent, &expectation()).is_err());
        let extra = br#"{"streams":[{"codec_type":"video"},{"codec_type":"audio"},{"codec_type":"subtitle"}],"format":{"duration":"3.336667"}}"#;
        assert!(check(extra, &expectation()).is_err());
        let undated = br#"{"streams":[{"codec_type":"video"},{"codec_type":"audio"}],"format":{}}"#;
        assert!(check(undated, &expectation()).is_err());
        assert!(check(b"not json", &expectation()).is_err());
        // One frame, 1272 samples (26.5 ms) and 10 ms.
        assert_eq!(expectation().tolerance(), Time(33_367 + 26_500 + 10_000));
    }
}
