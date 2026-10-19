//! Exports through the installed FFmpeg: the capability probe, the encoder
//! for each container, the sound render and the output check. Fixtures are
//! generated in temporary directories; every test skips, saying so, when
//! FFmpeg is unavailable.
use super::{
    AudioInput, AudioRender, EncodeInputs, Encoder, Expectation, SubtitleInput, VideoInput,
    args_tests::{audio_plan, plan, video_plan},
    probe_capabilities, render_audio, verify_output,
};
use crate::{
    MediaError,
    process::{Processes, capture, tool_command},
};
use editor_core::{
    Asset, Clip, FrameRate, MediaKind, Project, Time, Track, TrackKind,
    export::{
        AudioCodec, Capabilities, Container, EncodeSpeed, ExportPlan, QualityLevel, RateControl,
        SampleSpan, VIDEO_ENCODERS,
    },
};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, OnceLock, atomic::AtomicBool},
    thread,
    time::{Duration, Instant},
};

const NTSC: FrameRate = FrameRate {
    numerator: 30_000,
    denominator: 1_001,
};
const RECOMMENDED: RateControl = RateControl::Quality {
    level: QualityLevel::Recommended,
};
const WRITE: Duration = Duration::from_secs(20);
const FINISH: Duration = Duration::from_secs(60);

pub(super) fn ffmpeg_available() -> bool {
    ["ffmpeg", "ffprobe"].into_iter().all(|tool| {
        tool_command(tool)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

/// This host's capabilities, probed once for every test.
fn capabilities() -> Option<&'static Capabilities> {
    static PROBED: OnceLock<Option<Capabilities>> = OnceLock::new();
    PROBED
        .get_or_init(|| {
            if !ffmpeg_available() {
                return None;
            }
            Some(probe_capabilities().expect("FFmpeg answers the capability probe"))
        })
        .as_ref()
}

macro_rules! require_ffmpeg {
    ($fixture:expr) => {
        if !ffmpeg_available() {
            eprintln!("FFmpeg is unavailable; {} skipped", $fixture);
            return;
        }
    };
}

/// `count` RGBA pictures of FFmpeg's moving test pattern.
fn pictures(size: [u32; 2], count: usize) -> Vec<u8> {
    let bytes = size[0] as usize * size[1] as usize * 4 * count;
    let mut command = tool_command("ffmpeg");
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg(format!(
            "testsrc2=size={}x{}:rate=30,format=rgba",
            size[0], size[1]
        ))
        .args(["-frames:v", &count.to_string(), "-f", "rawvideo", "pipe:1"]);
    let raw = capture("ffmpeg", &mut command, &Processes::default(), bytes).unwrap();
    assert_eq!(raw.len(), bytes);
    raw
}

/// A 440 Hz tone at half level, interleaved `f32` samples.
fn tone(path: &Path, rate: u32, channels: u8, frames: u64) {
    let mut bytes = Vec::with_capacity(frames as usize * usize::from(channels) * 4);
    for frame in 0..frames {
        let sample =
            0.5 * (frame as f64 * 440.0 * std::f64::consts::TAU / f64::from(rate)).sin() as f32;
        for _ in 0..channels {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
    }
    fs::write(path, bytes).unwrap();
}

fn ffprobe(path: &Path, entries: &str) -> Value {
    let mut command = tool_command("ffprobe");
    command
        .args(["-v", "error", "-show_entries", entries, "-of", "json"])
        .arg(path);
    let bytes = capture("ffprobe", &mut command, &Processes::default(), 1 << 20).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn stream<'a>(probe: &'a Value, kind: &str) -> &'a Value {
    probe["streams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stream| stream["codec_type"] == kind)
        .unwrap_or_else(|| panic!("no {kind} stream in {probe}"))
}

/// The sample frames of the file's sound, decoded.
fn decoded_samples(path: &Path) -> u64 {
    let mut command = tool_command("ffmpeg");
    command
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-map", "0:a:0", "-ac", "1", "-f", "f32le", "pipe:1"]);
    let bytes = capture("ffmpeg", &mut command, &Processes::default(), 64 << 20).unwrap();
    bytes.len() as u64 / 4
}

fn has_encoder(name: &str) -> bool {
    capabilities().is_some_and(|caps| {
        caps.video.iter().any(|e| e.name == name) || caps.audio.iter().any(|e| e.name == name)
    })
}

/// Encodes `plan` with test pictures and a tone, checks the output and
/// returns its path.
fn encode(dir: &Path, name: &str, plan: &ExportPlan, subtitles: Option<SubtitleInput>) -> PathBuf {
    let output = dir.join(format!(".{name}.part"));
    let sound = dir.join(format!("{name}.f32"));
    let video = plan.video.as_ref().map(|video| VideoInput {
        size: video.geometry.output,
        rate: video.frames.rate,
    });
    let audio = plan.audio.as_ref().map(|audio| {
        tone(
            &sound,
            audio.settings.sample_rate,
            audio.settings.channels,
            audio.samples.frames,
        );
        AudioInput {
            path: &sound,
            rate: audio.settings.sample_rate,
            channels: audio.settings.channels,
        }
    });
    let inputs = EncodeInputs {
        video,
        audio,
        subtitles,
        title: "Neptune Video export test",
    };
    let mut encoder = Encoder::start(plan, &inputs, &output).unwrap();
    if let Some(video) = &plan.video {
        let size = video.geometry.output;
        let frame = size[0] as usize * size[1] as usize * 4;
        let raw = pictures(size, video.frames.count as usize);
        for picture in raw.chunks_exact(frame) {
            encoder.write_frame(picture, WRITE).unwrap();
        }
        assert_eq!(encoder.frames(), video.frames.count);
    }
    encoder.finish(FINISH).unwrap();
    verify_output(&output, &expectation(plan, subtitles.is_some())).unwrap();
    output
}

fn expectation(plan: &ExportPlan, subtitles: bool) -> Expectation {
    let (duration, frame) = match (&plan.video, &plan.audio) {
        (Some(video), _) => (video.frames.duration(), video.frames.rate.frame_time(1)),
        (None, Some(audio)) => (
            Time((audio.samples.frames as i64 * 1_000_000) / i64::from(audio.samples.rate)),
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

/// A video plan of `count` frames at `size`.
fn video_of(name: &str, size: [u32; 2], count: u64) -> editor_core::export::VideoPlan {
    let mut video = video_plan(name, NTSC, size, RECOMMENDED, EncodeSpeed::Faster);
    video.frames.count = count;
    video
}

/// Sound covering exactly the plan's frames.
fn sound_for(
    video: &editor_core::export::VideoPlan,
    name: &str,
    codec: AudioCodec,
    kbps: u32,
) -> editor_core::export::AudioPlan {
    let mut audio = audio_plan(name, codec, kbps, 48_000, 2);
    audio.samples.frames =
        editor_core::export::sample_count(video.frames.start(), video.frames.end(), 48_000);
    audio
}

fn assert_bt709(probe: &Value) {
    let video = stream(probe, "video");
    assert_eq!(video["color_space"], "bt709", "{video}");
    assert_eq!(video["color_primaries"], "bt709", "{video}");
    assert_eq!(video["color_transfer"], "bt709", "{video}");
    // ProRes is limited range by definition and records no range.
    if video["codec_name"] != "prores" {
        assert_eq!(video["color_range"], "tv", "{video}");
    }
}

const VIDEO_ENTRIES: &str = "stream=codec_type,codec_name,codec_tag_string,pix_fmt,color_space,color_primaries,color_transfer,color_range,nb_read_frames:stream_tags=language:format_tags=title";

#[test]
fn the_probe_offers_only_encoders_that_encoded_here() {
    require_ffmpeg!("capability probe");
    let started = Instant::now();
    let caps = capabilities().unwrap();
    eprintln!(
        "PROBE: {} ({:.1}s incl. other tests' wait)",
        caps.ffmpeg,
        started.elapsed().as_secs_f64()
    );
    for encoder in &caps.video {
        eprintln!(
            "PROBE: video {} {:?} device={:?}",
            encoder.name, encoder.hardware, encoder.device
        );
    }
    for encoder in &caps.audio {
        eprintln!("PROBE: audio {}", encoder.name);
    }
    for unavailable in &caps.unavailable {
        eprintln!(
            "PROBE: unavailable {}: {}",
            unavailable.name, unavailable.reason
        );
    }
    assert!(caps.ffmpeg.starts_with("ffmpeg version"), "{}", caps.ffmpeg);
    let mut command = tool_command("ffmpeg");
    command.args(["-hide_banner", "-encoders"]);
    let listing =
        String::from_utf8(capture("ffmpeg", &mut command, &Processes::default(), 4 << 20).unwrap())
            .unwrap();
    let listed = |name: &str| {
        listing
            .lines()
            .any(|line| line.split_whitespace().nth(1) == Some(name))
    };
    for known in VIDEO_ENCODERS {
        let offered = caps.video.iter().filter(|e| e.name == known.name).count();
        let refused = caps
            .unavailable
            .iter()
            .filter(|u| u.name == known.name)
            .count();
        if listed(known.name) {
            assert_eq!(offered + refused, 1, "{} is listed once", known.name);
        } else {
            assert_eq!(
                offered + refused,
                0,
                "{} is not listed by FFmpeg",
                known.name
            );
        }
        if known.hardware.is_none() && listed(known.name) {
            // Software encoders do not depend on a device.
            assert_eq!(offered, 1, "{} must encode on any host", known.name);
        }
    }
    for unavailable in &caps.unavailable {
        assert!(
            !unavailable.reason.trim().is_empty(),
            "{}",
            unavailable.name
        );
        assert!(
            !unavailable.reason.starts_with("Nothing was written"),
            "{}: the reason must be the encoder's own: {}",
            unavailable.name,
            unavailable.reason
        );
    }
    for container in Container::ALL {
        assert!(caps.has_container(container), "{container:?}");
    }
    for name in [
        "aac",
        "libopus",
        "libmp3lame",
        "flac",
        "pcm_s16le",
        "pcm_s24le",
    ] {
        if listed(name) {
            assert!(has_encoder(name), "{name}");
        }
    }
    // The probe takes the device a VA-API encoder used; a failure says why.
    for encoder in caps.video.iter().filter(|e| e.name.ends_with("_vaapi")) {
        assert!(
            encoder
                .device
                .as_deref()
                .is_some_and(|d| d.starts_with("/dev/dri/renderD"))
        );
    }
    for unavailable in caps
        .unavailable
        .iter()
        .filter(|u| u.name.ends_with("_vaapi"))
    {
        assert!(
            unavailable.reason.contains("/dev/dri/") || unavailable.reason.contains("device"),
            "{}",
            unavailable.reason
        );
    }
}

#[test]
fn h264_and_aac_in_mp4_carry_every_frame_bt709_tags_title_and_subtitles() {
    require_ffmpeg!("x264 MP4 export");
    let dir = tempfile::tempdir().unwrap();
    let video = video_of("libx264", [320, 180], 30);
    let audio = sound_for(&video, "aac", AudioCodec::Aac, 192);
    let mut plan = plan(Container::Mp4, Some(video), Some(audio));
    plan.settings.subtitles.embed = true;
    let captions = dir.path().join("captions.srt");
    fs::write(
        &captions,
        "1\n00:00:00,000 --> 00:00:00,500\nHello\n\n2\n00:00:00,500 --> 00:00:00,900\nworld\n",
    )
    .unwrap();
    let output = encode(
        dir.path(),
        "h264",
        &plan,
        Some(SubtitleInput {
            path: &captions,
            language: Some("en-GB"),
        }),
    );
    let mut command = tool_command("ffprobe");
    command
        .args([
            "-v",
            "error",
            "-count_frames",
            "-show_entries",
            VIDEO_ENTRIES,
            "-of",
            "json",
        ])
        .arg(&output);
    let probe: Value = serde_json::from_slice(
        &capture("ffprobe", &mut command, &Processes::default(), 1 << 20).unwrap(),
    )
    .unwrap();
    let picture = stream(&probe, "video");
    assert_eq!(picture["codec_name"], "h264");
    assert_eq!(picture["pix_fmt"], "yuv420p");
    assert_eq!(picture["nb_read_frames"], "30");
    assert_bt709(&probe);
    assert_eq!(stream(&probe, "subtitle")["codec_name"], "mov_text");
    assert_eq!(stream(&probe, "subtitle")["tags"]["language"], "eng");
    assert_eq!(
        probe["format"]["tags"]["title"],
        "Neptune Video export test"
    );
    // The edit list is in samples, not milliseconds.
    let timing = ffprobe(&output, "stream=codec_type,time_base,duration_ts");
    assert_eq!(stream(&timing, "audio")["time_base"], "1/48000");
}

#[test]
fn nvenc_encodes_h264_in_mp4_when_this_host_has_it() {
    require_ffmpeg!("NVENC MP4 export");
    if !has_encoder("h264_nvenc") {
        eprintln!("h264_nvenc did not pass the probe here; NVENC export skipped");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let video = video_of("h264_nvenc", [320, 180], 30);
    let audio = sound_for(&video, "aac", AudioCodec::Aac, 256);
    let output = encode(
        dir.path(),
        "nvenc",
        &plan(Container::Mp4, Some(video), Some(audio)),
        None,
    );
    let probe = ffprobe(&output, VIDEO_ENTRIES);
    assert_eq!(stream(&probe, "video")["codec_name"], "h264");
    assert_bt709(&probe);
    for name in ["hevc_nvenc", "av1_nvenc"] {
        if has_encoder(name) {
            let video = video_of(name, [320, 180], 12);
            let output = encode(
                dir.path(),
                name,
                &plan(Container::Mp4, Some(video), None),
                None,
            );
            assert_bt709(&ffprobe(&output, VIDEO_ENTRIES));
        }
    }
}

#[test]
fn h265_in_mp4_is_tagged_hvc1() {
    require_ffmpeg!("x265 MP4 export");
    if !has_encoder("libx265") {
        eprintln!("libx265 is not offered here; H.265 export skipped");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let video = video_of("libx265", [320, 180], 20);
    let output = encode(
        dir.path(),
        "h265",
        &plan(Container::Mp4, Some(video), None),
        None,
    );
    let probe = ffprobe(&output, VIDEO_ENTRIES);
    assert_eq!(stream(&probe, "video")["codec_tag_string"], "hvc1");
    assert_bt709(&probe);
}

#[test]
fn prores_and_pcm_in_mov_keep_ten_bit_chroma_and_every_sample() {
    require_ffmpeg!("ProRes MOV export");
    if !has_encoder("prores_ks") {
        eprintln!("prores_ks is not offered here; ProRes export skipped");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let video = video_of("prores_ks", [320, 180], 15);
    let audio = sound_for(&video, "pcm_s24le", AudioCodec::Pcm24, 0);
    let samples = audio.samples.frames;
    let output = encode(
        dir.path(),
        "prores",
        &plan(Container::Mov, Some(video), Some(audio)),
        None,
    );
    let probe = ffprobe(&output, VIDEO_ENTRIES);
    assert_eq!(stream(&probe, "video")["codec_name"], "prores");
    assert_eq!(stream(&probe, "video")["pix_fmt"], "yuv422p10le");
    assert_eq!(stream(&probe, "audio")["codec_name"], "pcm_s24le");
    assert_bt709(&probe);
    assert_eq!(decoded_samples(&output), samples);
}

#[test]
fn vp9_and_opus_in_webm_pass_the_check_with_subtitles() {
    require_ffmpeg!("VP9 WebM export");
    if !has_encoder("libvpx-vp9") || !has_encoder("libopus") {
        eprintln!("libvpx-vp9 or libopus is not offered here; WebM export skipped");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let video = video_of("libvpx-vp9", [320, 180], 20);
    let audio = sound_for(&video, "libopus", AudioCodec::Opus, 128);
    let samples = audio.samples.frames;
    let captions = dir.path().join("captions.vtt");
    fs::write(&captions, "WEBVTT\n\n00:00.000 --> 00:00.400\nHallo\n").unwrap();
    let output = encode(
        dir.path(),
        "vp9",
        &plan(Container::Webm, Some(video), Some(audio)),
        Some(SubtitleInput {
            path: &captions,
            language: Some("de"),
        }),
    );
    let probe = ffprobe(&output, VIDEO_ENTRIES);
    assert_eq!(stream(&probe, "video")["codec_name"], "vp9");
    assert_eq!(stream(&probe, "audio")["codec_name"], "opus");
    assert_eq!(stream(&probe, "subtitle")["tags"]["language"], "ger");
    assert_bt709(&probe);
    // Opus trims its own padding: decoded, it is exact.
    assert_eq!(decoded_samples(&output), samples);
}

#[test]
fn sound_only_exports_last_as_long_as_their_samples() {
    require_ffmpeg!("audio-only exports");
    let dir = tempfile::tempdir().unwrap();
    // An odd count, not a multiple of any codec frame.
    let frames = 44_100 + 777;
    for (container, name, codec, kbps, exact) in [
        (Container::M4a, "aac", AudioCodec::Aac, 256, false),
        (Container::Wav, "pcm_s16le", AudioCodec::Pcm16, 0, true),
        (Container::Wav, "pcm_s24le", AudioCodec::Pcm24, 0, true),
        (Container::Flac, "flac", AudioCodec::Flac, 0, true),
        (Container::Mp3, "libmp3lame", AudioCodec::Mp3, 320, false),
    ] {
        if !has_encoder(name) {
            eprintln!("{name} is not offered here; its {container:?} export skipped");
            continue;
        }
        let mut audio = audio_plan(name, codec, kbps, 44_100, 2);
        audio.samples.frames = frames;
        let output = encode(
            dir.path(),
            &format!("{name}-{container:?}"),
            &plan(container, None, Some(audio)),
            None,
        );
        let decoded = decoded_samples(&output);
        if exact {
            assert_eq!(decoded, frames, "{container:?} {name}");
        } else {
            // Lossy codecs pad by under a frame and their delay.
            assert!(
                decoded >= frames && decoded <= frames + 2 * 2257,
                "{container:?} {name}: {decoded}"
            );
        }
        if container == Container::Flac {
            let probe = ffprobe(&output, "stream=sample_fmt,bits_per_raw_sample");
            assert_eq!(probe["streams"][0]["sample_fmt"], "s16");
        }
    }
}

#[test]
fn a_failing_encode_reports_ffmpegs_reason_and_mismatched_inputs_are_refused() {
    require_ffmpeg!("encoder failure");
    let dir = tempfile::tempdir().unwrap();
    // x264 needs even sides for 4:2:0.
    let video = video_of("libx264", [33, 17], 4);
    let plan = plan(Container::Mp4, Some(video), None);
    let output = dir.path().join(".odd.part");
    let inputs = EncodeInputs {
        video: Some(VideoInput {
            size: [33, 17],
            rate: NTSC,
        }),
        audio: None,
        subtitles: None,
        title: "",
    };
    let mut encoder = Encoder::start(&plan, &inputs, &output).unwrap();
    let picture = vec![0; 33 * 17 * 4];
    let written = (0..4).try_for_each(|_| encoder.write_frame(&picture, WRITE));
    let error = match written {
        Err(error) => error,
        Ok(()) => encoder.finish(FINISH).unwrap_err(),
    };
    let MediaError::Decode(message) = error else {
        panic!("expected FFmpeg's failure, got {error:?}");
    };
    assert!(message.starts_with("ffmpeg:"), "{message}");
    assert!(
        message.len() > "ffmpeg: Encoding failed".len() + 10,
        "{message}"
    );

    // A wrong size, missing sound or wrong picture length runs nothing.
    let other = EncodeInputs {
        video: Some(VideoInput {
            size: [32, 16],
            rate: NTSC,
        }),
        ..inputs
    };
    assert!(matches!(
        Encoder::start(&plan, &other, &output),
        Err(MediaError::Invalid(_))
    ));
    let silent = EncodeInputs {
        video: None,
        ..inputs
    };
    assert!(matches!(
        Encoder::start(&plan, &silent, &output),
        Err(MediaError::Invalid(_))
    ));
    let good = video_of("libx264", [32, 16], 2);
    let mut encoder = Encoder::start(
        &super::args_tests::plan(Container::Mp4, Some(good), None),
        &other,
        &output,
    )
    .unwrap();
    assert!(matches!(
        encoder.write_frame(&[0; 5], WRITE),
        Err(MediaError::Invalid(_))
    ));
}

#[test]
fn a_cancelled_encode_stops_its_writer_and_its_process() {
    require_ffmpeg!("encoder cancellation");
    let dir = tempfile::tempdir().unwrap();
    let video = video_of("libx264", [320, 180], 1_000);
    let plan = plan(Container::Mp4, Some(video), None);
    let inputs = EncodeInputs {
        video: Some(VideoInput {
            size: [320, 180],
            rate: NTSC,
        }),
        audio: None,
        subtitles: None,
        title: "",
    };
    let mut encoder = Encoder::start(&plan, &inputs, &dir.path().join(".c.part")).unwrap();
    let picture = vec![128; 320 * 180 * 4];
    encoder.write_frame(&picture, WRITE).unwrap();
    let cancel = encoder.canceller();
    thread::spawn(cancel).join().unwrap();
    let until = Instant::now() + Duration::from_secs(3);
    let result = loop {
        // Writes may land in the pipe until FFmpeg is gone.
        match encoder.write_frame(&picture, WRITE) {
            Ok(()) => assert!(
                Instant::now() < until,
                "cancelled encoder kept taking frames"
            ),
            result => break result,
        }
    };
    assert!(matches!(result, Err(MediaError::Cancelled)), "{result:?}");
    assert!(matches!(encoder.finish(FINISH), Err(MediaError::Cancelled)));
}

/// A 48 kHz stereo WAV of a constant level.
fn level_wav(path: &Path, seconds: u32, level: i16) {
    let frames = 48_000 * seconds;
    let data = frames * 4;
    let mut wave = Vec::with_capacity(44 + data as usize);
    wave.extend_from_slice(b"RIFF");
    wave.extend_from_slice(&(36 + data).to_le_bytes());
    wave.extend_from_slice(b"WAVEfmt ");
    wave.extend_from_slice(&16_u32.to_le_bytes());
    wave.extend_from_slice(&1_u16.to_le_bytes());
    wave.extend_from_slice(&2_u16.to_le_bytes());
    wave.extend_from_slice(&48_000_u32.to_le_bytes());
    wave.extend_from_slice(&(48_000_u32 * 4).to_le_bytes());
    wave.extend_from_slice(&4_u16.to_le_bytes());
    wave.extend_from_slice(&16_u16.to_le_bytes());
    wave.extend_from_slice(b"data");
    wave.extend_from_slice(&data.to_le_bytes());
    for _ in 0..frames * 2 {
        wave.extend_from_slice(&level.to_le_bytes());
    }
    fs::write(path, wave).unwrap();
}

fn timeline(asset: &Asset, at: Time) -> Arc<Project> {
    let mut project = Project::new("Export sound fixture");
    Arc::make_mut(&mut project.assets).insert(asset.id, asset.clone());
    let mut track = Track::new("Sound", TrackKind::Audio);
    track.clips.push(Clip::from_asset(asset, at));
    project.tracks = vec![Arc::new(track)];
    project.validate().unwrap();
    Arc::new(project)
}

#[test]
fn the_sound_render_writes_exactly_the_span_and_encodes_sample_exact() {
    require_ffmpeg!("export sound render");
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("level.wav");
    level_wav(&source, 1, 8192);
    let asset = Asset::new(source, MediaKind::Audio, Time::from_seconds(1.0));
    let project = timeline(&asset, Time::from_seconds(0.5));
    let span = SampleSpan {
        start: Time::from_seconds(0.25),
        frames: 48_000 + 12_345,
        rate: 48_000,
        channels: 2,
    };
    let out = dir.path().join("sound.f32");
    let reported = std::sync::Mutex::new(Vec::new());
    render_audio(
        Arc::clone(&project),
        span,
        &out,
        &AtomicBool::new(false),
        &|done| reported.lock().unwrap().push(done),
    )
    .unwrap();
    let bytes = fs::read(&out).unwrap();
    assert_eq!(bytes.len() as u64, span.frames * 2 * 4);
    let samples: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    let at = |seconds: f64| samples[((seconds - 0.25) * 48_000.0) as usize * 2];
    assert_eq!(at(0.3), 0.0);
    assert!((at(0.75) - 0.25).abs() < 0.01, "{}", at(0.75));
    assert!((at(1.49) - 0.25).abs() < 0.01, "{}", at(1.49));
    // The clip ends at 1.5 s; the span runs on in silence to its last sample.
    assert_eq!(samples[samples.len() - 1], 0.0);
    let reported = reported.into_inner().unwrap();
    assert_eq!(reported.first(), Some(&0.0));
    assert_eq!(reported.last(), Some(&1.0));
    assert!(reported.windows(2).all(|w| w[0] <= w[1]));

    // The same file as a WAV export keeps every sample.
    let mut audio = audio_plan("pcm_s16le", AudioCodec::Pcm16, 0, 48_000, 2);
    audio.samples = span;
    let plan = plan(Container::Wav, None, Some(audio));
    let output = dir.path().join(".sound.wav.part");
    let inputs = EncodeInputs {
        video: None,
        audio: Some(AudioInput {
            path: &out,
            rate: 48_000,
            channels: 2,
        }),
        subtitles: None,
        title: "",
    };
    Encoder::start(&plan, &inputs, &output)
        .unwrap()
        .finish(FINISH)
        .unwrap();
    verify_output(&output, &expectation(&plan, false)).unwrap();
    assert_eq!(decoded_samples(&output), span.frames);
}

#[test]
fn cancelling_the_sound_render_removes_its_file_and_reaps_its_decoders() {
    require_ffmpeg!("export sound cancellation");
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("long.wav");
    level_wav(&source, 20, 4096);
    let asset = Asset::new(source, MediaKind::Audio, Time::from_seconds(20.0));
    let project = timeline(&asset, Time::ZERO);
    let span = SampleSpan {
        start: Time::ZERO,
        frames: 48_000 * 20,
        rate: 48_000,
        channels: 2,
    };
    let out = dir.path().join("sound.f32");
    let render = AudioRender::new();
    let cancel = render.canceller();
    let result = render.render(project.clone(), span, &out, &|done| {
        if done > 0.1 {
            cancel();
        }
    });
    assert!(matches!(result, Err(MediaError::Cancelled)), "{result:?}");
    assert!(!out.exists(), "a cancelled render leaves no file");
    assert!(render.processes().started_tools() >= 1);
    let until = Instant::now() + Duration::from_secs(3);
    while render.processes().retained_monitors() != 0 {
        assert!(
            Instant::now() < until,
            "a decoder outlived its cancelled render"
        );
        thread::sleep(Duration::from_millis(5));
    }

    // A flag already set stops the render before it writes anything.
    let result = render_audio(project, span, &out, &AtomicBool::new(true), &|_| {});
    assert!(matches!(result, Err(MediaError::Cancelled)));
    assert!(!out.exists());
}

#[test]
fn a_plan_from_the_core_renders_its_sound_and_encodes_through_auto() {
    require_ffmpeg!("planned export");
    let caps = capabilities().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("level.wav");
    level_wav(&source, 1, 8192);
    let asset = Asset::new(source, MediaKind::Audio, Time::from_seconds(1.0));
    let project = timeline(&asset, Time::ZERO);
    let settings = editor_core::export::ExportSettings {
        video: Some(editor_core::export::VideoSettings {
            resolution: editor_core::export::Resolution::Custom {
                width: 320,
                height: 180,
            },
            ..editor_core::export::VideoSettings::default()
        }),
        ..editor_core::export::ExportSettings::default()
    };
    let plan = settings.plan(&project, caps).unwrap();
    let video = plan.video.as_ref().unwrap();
    let audio = plan.audio.as_ref().unwrap();
    eprintln!(
        "PLANNED: {} frames with {}",
        video.frames.count, video.encoder.name
    );
    let sound = dir.path().join("sound.f32");
    render_audio(
        Arc::clone(&project),
        audio.samples,
        &sound,
        &AtomicBool::new(false),
        &|_| {},
    )
    .unwrap();
    let output = dir.path().join(".planned.mp4.part");
    let inputs = EncodeInputs {
        video: Some(VideoInput {
            size: video.geometry.output,
            rate: video.frames.rate,
        }),
        audio: Some(AudioInput {
            path: &sound,
            rate: audio.samples.rate,
            channels: audio.samples.channels,
        }),
        subtitles: None,
        title: &project.name,
    };
    let mut encoder = Encoder::start(&plan, &inputs, &output).unwrap();
    let black = vec![0; 320 * 180 * 4];
    for _ in 0..video.frames.count {
        encoder.write_frame(&black, WRITE).unwrap();
    }
    encoder.finish(FINISH).unwrap();
    verify_output(&output, &expectation(&plan, false)).unwrap();
    assert!(decoded_samples(&output) >= audio.samples.frames);
}

/// The first sample frame louder than a quarter, or `None`.
fn onset(samples: &[f32], channels: usize) -> Option<usize> {
    samples
        .chunks_exact(channels)
        .position(|frame| frame[0].abs() > 0.25)
}

/// Writes a file of three seconds of 160x90 picture and a sound that steps
/// up one second into itself. `sound_input` is FFmpeg's input options for
/// the sound (`-itsoffset`, say) and `codecs` its output ones.
fn stepped(path: &Path, sound_input: &[&str], codecs: &[&str]) {
    let made = tool_command("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
        .args(["-f", "lavfi", "-i", "testsrc2=size=160x90:rate=30:d=3"])
        .args(sound_input)
        .args(["-f", "lavfi", "-i"])
        .arg("aevalsrc=if(gte(t\\,1)\\,0.5\\,0):s=48000:d=2.9")
        .args(["-map", "0", "-map", "1"])
        .args(codecs)
        .arg(path)
        .status()
        .unwrap();
    assert!(made.success());
}

/// The start of a file's streams of `kind`, or of the file itself.
fn start_of(path: &Path, kind: Option<&str>) -> f64 {
    let probe = ffprobe(path, "stream=codec_type,start_time:format=start_time");
    let value = match kind {
        Some(kind) => &stream(&probe, kind)["start_time"],
        None => &probe["format"]["start_time"],
    };
    value.as_str().unwrap().parse().unwrap()
}

/// Renders two seconds of a timeline holding only `asset` from each source
/// point and asserts the step sounds where expected, within `slack` sample
/// frames. Times are the file's: from the start of its earliest stream, as
/// the picture is read.
fn assert_step_onsets(asset: &Asset, cases: &[(f64, f64)], slack: usize) {
    let dir = tempfile::tempdir().unwrap();
    for &(source_in, expected) in cases {
        let mut project = Project::new("Stepped sound fixture");
        Arc::make_mut(&mut project.assets).insert(asset.id, asset.clone());
        let mut track = Track::new("Picture", TrackKind::Video);
        let mut clip = Clip::from_asset(asset, Time::ZERO);
        clip.source_in = Time::from_seconds(source_in);
        clip.duration = Time::from_seconds(2.0);
        track.clips.push(clip);
        project.tracks = vec![Arc::new(track)];
        project.validate().unwrap();
        let span = SampleSpan {
            start: Time::ZERO,
            frames: 96_000,
            rate: 48_000,
            channels: 2,
        };
        let out = dir.path().join("sound.f32");
        render_audio(
            Arc::new(project),
            span,
            &out,
            &AtomicBool::new(false),
            &|_| {},
        )
        .unwrap();
        let samples: Vec<f32> = fs::read(&out)
            .unwrap()
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        let found = onset(&samples, 2).expect("the step sounds");
        let wanted = (expected * 48_000.0_f64).round() as usize;
        assert!(
            found.abs_diff(wanted) <= slack,
            "{}: from {source_in} s the step sounds at sample {found}, not {wanted}",
            asset.path.display()
        );
    }
}

fn stepped_asset(path: PathBuf) -> Asset {
    let mut asset = Asset::new(path, MediaKind::Video, Time::from_seconds(3.0));
    asset.has_audio = true;
    asset.width = 160;
    asset.height = 90;
    asset
}

#[test]
fn a_late_audio_stream_keeps_its_place_against_the_picture() {
    require_ffmpeg!("late audio stream");
    let dir = tempfile::tempdir().unwrap();
    // The sound starts 80 ms after the picture and steps up one second into
    // itself: at 1.08 s of the file.
    let source = dir.path().join("late.mov");
    stepped(
        &source,
        &["-itsoffset", "0.08"],
        &["-c:v", "mjpeg", "-c:a", "pcm_s16le"],
    );
    let late = start_of(&source, Some("audio"));
    assert!((late - 0.08).abs() < 0.002, "audio starts at {late}");
    // From the file's start (before the sound's), from a point still before
    // it, and from a point inside it.
    assert_step_onsets(
        &stepped_asset(source),
        &[(0.0, 1.08), (0.04, 1.04), (0.5, 0.58)],
        4,
    );
}

#[test]
fn a_seek_into_a_matroska_cluster_keeps_the_sound_in_place() {
    require_ffmpeg!("Matroska range");
    let dir = tempfile::tempdir().unwrap();
    // PCM in 1024-sample packets with millisecond timestamps, and a picture
    // whose clusters start every 0.5 s: a seek to 0.5 s lands on a cluster
    // whose first sound is 12 ms later, and one to 0.7 s between packets.
    // Read as they come, the sound played early by that much.
    let source = dir.path().join("range.mkv");
    stepped(&source, &[], &["-c:v", "mjpeg", "-c:a", "pcm_s16le"]);
    assert_step_onsets(
        &stepped_asset(source),
        &[(0.0, 1.0), (0.5, 0.5), (0.51, 0.49), (0.7, 0.3)],
        4,
    );
}

#[test]
fn opus_with_its_lead_in_keeps_the_files_time_from_any_point() {
    require_ffmpeg!("WebM Opus lead-in");
    if !has_encoder("libopus") {
        eprintln!("SKIP: libopus is unavailable; the WebM Opus fixture was not run");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // Opus starts its file a few milliseconds early, with samples its
    // decoder drops: the step sounds that much after the file's start plus
    // one second, as the picture's frames are, whether the clip starts at
    // the file's start or inside it.
    let source = dir.path().join("lead.webm");
    stepped(
        &source,
        &[],
        &[
            "-c:v",
            "libvpx-vp9",
            "-deadline",
            "realtime",
            "-c:a",
            "libopus",
        ],
    );
    let lead = start_of(&source, None);
    // FFmpeg 8.0's ffprobe starts this file before zero; 8.1's starts it after
    // the dropped samples, at zero. The sound keeps the file's time either way.
    assert!(lead <= 0.0, "the file starts at {lead}");
    // Matroska keeps milliseconds: allow 2 ms.
    assert_step_onsets(
        &stepped_asset(source),
        &[(0.0, 1.0 - lead), (0.5, 0.5 - lead), (0.75, 0.25 - lead)],
        96,
    );
}

/// Encodes the plan's frames of the lavfi `source` (a source and a filter
/// chain after it, its size and rate added) with `plan`'s video alone, reading them from FFmpeg as they
/// come, and returns the file's size in bytes.
fn encoded_size(dir: &Path, name: &str, plan: &ExportPlan, source: &str) -> u64 {
    use std::io::Read;
    let video = plan.video.as_ref().unwrap();
    let size = video.geometry.output;
    let (head, chain) = source.split_once(',').unwrap_or((source, "null"));
    let mut reader = tool_command("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg(format!(
            "{head}=size={}x{}:rate=30,{chain},format=rgba",
            size[0], size[1]
        ))
        .args(["-frames:v", &video.frames.count.to_string()])
        .args(["-f", "rawvideo", "pipe:1"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut pictures = reader.stdout.take().unwrap();
    let output = dir.join(format!(".{name}.mp4.part"));
    let inputs = EncodeInputs {
        video: Some(VideoInput {
            size,
            rate: video.frames.rate,
        }),
        audio: None,
        subtitles: None,
        title: "",
    };
    let mut encoder = Encoder::start(plan, &inputs, &output).unwrap();
    let mut picture = vec![0; size[0] as usize * size[1] as usize * 4];
    for _ in 0..video.frames.count {
        pictures.read_exact(&mut picture).unwrap();
        encoder.write_frame(&picture, WRITE).unwrap();
    }
    assert!(reader.wait().unwrap().success());
    encoder.finish(FINISH).unwrap();
    let bytes = fs::metadata(&output).unwrap().len();
    fs::remove_file(&output).unwrap();
    bytes
}

#[test]
fn a_quality_level_writes_about_the_estimate_on_software_and_nvenc_alike() {
    require_ffmpeg!("quality calibration");
    let dir = tempfile::tempdir().unwrap();
    // Grainy test pattern at 1280x720: the fixture the estimate and the
    // hardware quality values were measured on.
    let size = [1280, 720];
    let grain = "testsrc2,noise=alls=4:allf=t";
    let mut said = Vec::new();
    for (software, hardware) in [("libx264", "h264_nvenc"), ("libx265", "hevc_nvenc")] {
        let at = |name: &str, level, seconds: u64| {
            let mut video = video_plan(
                name,
                FrameRate {
                    numerator: 30,
                    denominator: 1,
                },
                size,
                RateControl::Quality { level },
                EncodeSpeed::Balanced,
            );
            video.frames.count = 30 * seconds;
            plan(Container::Mp4, Some(video), None)
        };
        let reference = at(software, QualityLevel::Recommended, 1);
        let estimate = editor_core::export::ExportSettings::estimate_bytes(&reference) as f64;
        let written = encoded_size(dir.path(), software, &reference, grain) as f64;
        said.push(format!("{software} {written} against {estimate}"));
        assert!(
            (0.7..=1.5).contains(&(written / estimate)),
            "{software} wrote {written} bytes where {estimate} were estimated"
        );
        if !has_encoder(hardware) {
            eprintln!("{hardware} did not pass the probe here; its calibration skipped");
            continue;
        }
        // The same level on NVENC writes about what the software encoder did.
        let same = encoded_size(
            dir.path(),
            hardware,
            &at(hardware, QualityLevel::Recommended, 1),
            grain,
        ) as f64;
        said.push(format!("{hardware} {same}"));
        assert!(
            (0.7..=1.4).contains(&(same / written)),
            "{hardware} wrote {same} bytes where {software} wrote {written}"
        );
        // Heavy grain at Maximum wants more than the level NVENC picks for
        // 720p allows. Held to it, an RTX 4060 wrote 13.6 Mbit/s with H.264
        // and 9.5 with HEVC; with the export's ceiling 16.7 and 16.5.
        let most = encoded_size(
            dir.path(),
            hardware,
            &at(hardware, QualityLevel::Maximum, 3),
            "testsrc2,noise=alls=10:allf=t",
        ) as f64;
        let rate = most * 8.0 / 3.0;
        said.push(format!("{hardware} at Maximum {rate:.0} bit/s"));
        assert!(
            rate > 15e6,
            "{hardware} wrote {rate:.0} bit/s at Maximum: held to its level's rate"
        );
    }
    eprintln!("CALIBRATION: {}", said.join("; "));
}
