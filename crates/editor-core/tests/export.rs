use std::{path::PathBuf, sync::Arc};

use editor_core::{
    Asset, CanvasSettings, Clip, ClipContent, FrameRate, MediaKind, Project, Time, Track,
    TrackKind, WordTime,
    export::{
        self, AUDIO_ENCODERS, AudioCodec, AudioEncoder, AudioSettings, Capabilities, Container,
        Destination, EncodeSpeed, EncoderChoice, ExportError, ExportPreset, ExportRange,
        ExportSettings, Fit, FrameSpan, Geometry, HardwareKind, KnownVideoEncoder, MAX_OUTPUT_SIDE,
        QualityLevel, RateControl, RenderJobInfo, RenderState, Resolution, SidecarFormat,
        VIDEO_ENCODERS, VideoCodec, VideoEncoder, VideoSettings,
    },
};
use uuid::Uuid;

fn ms(value: i64) -> Time {
    Time(value * 1000)
}

fn rate(numerator: u32, denominator: u32) -> FrameRate {
    FrameRate {
        numerator,
        denominator,
    }
}

fn encoder(known: &KnownVideoEncoder) -> VideoEncoder {
    VideoEncoder {
        name: known.name.into(),
        codec: known.codec,
        hardware: known.hardware,
        max_side: known.max_side,
        min_size: known.min_size,
        device: None,
    }
}

/// Every encoder and container the export drives.
fn full_caps() -> Capabilities {
    caps_with(|_| true)
}

fn caps_with(keep: impl Fn(&KnownVideoEncoder) -> bool) -> Capabilities {
    Capabilities {
        ffmpeg: "ffmpeg version 8.0.1".into(),
        video: VIDEO_ENCODERS
            .iter()
            .filter(|e| keep(e))
            .map(encoder)
            .collect(),
        audio: AUDIO_ENCODERS
            .iter()
            .map(|(name, codec)| AudioEncoder {
                name: (*name).into(),
                codec: *codec,
            })
            .collect(),
        containers: Container::ALL.to_vec(),
        unavailable: Vec::new(),
    }
}

/// What this Linux host has: software, NVENC and VA-API.
fn host_caps() -> Capabilities {
    caps_with(|e| {
        matches!(
            e.hardware,
            None | Some(HardwareKind::Nvenc) | Some(HardwareKind::Vaapi)
        )
    })
}

/// A 1920x1080 project at `frame_rate` with one video clip of `seconds`.
fn project(frame_rate: FrameRate, seconds: i64) -> Project {
    let mut project = Project::new("Export");
    project.settings = CanvasSettings {
        frame_rate,
        ..CanvasSettings::default()
    };
    let mut asset = Asset::new(
        PathBuf::from("/media/clip.mp4"),
        MediaKind::Video,
        ms(seconds * 1000),
    );
    asset.width = 1920;
    asset.height = 1080;
    asset.frame_rate = Some(frame_rate);
    asset.has_audio = true;
    let clip = Clip::from_asset(&asset, Time::ZERO);
    Arc::make_mut(&mut project.assets).insert(asset.id, asset);
    Arc::make_mut(&mut project.tracks[0]).clips.push(clip);
    project
}

fn caption(text: &str, start: Time, end: Time, words: &[(i64, i64)]) -> Clip {
    let mut clip = Clip::text(text, start, end - start);
    if let ClipContent::Text { words: times, .. } = &mut clip.content {
        // Word times are clip-local.
        *times = words
            .iter()
            .map(|&(at, end)| WordTime {
                at: ms(at) - start,
                end: ms(end) - start,
            })
            .collect();
    }
    clip
}

/// A caption track `en` with cues at 0–2 s, 3–5 s (timed words) and 6–8 s.
fn captions() -> Track {
    let mut track = Track::new("Captions", TrackKind::Text);
    track.language = Some("en".into());
    track.clips = vec![
        caption("Before", ms(0), ms(2000), &[]),
        caption(
            "one two three",
            ms(3000),
            ms(5000),
            &[(3000, 3500), (3500, 4200), (4200, 5000)],
        ),
        caption("After", ms(6000), ms(8000), &[]),
    ];
    track
}

fn with_captions(mut project: Project) -> Project {
    project.tracks.push(Arc::new(captions()));
    project
}

fn settings(change: impl FnOnce(&mut ExportSettings)) -> ExportSettings {
    let mut settings = ExportSettings::default();
    change(&mut settings);
    settings
}

fn video(settings: &mut ExportSettings) -> &mut VideoSettings {
    settings.video.as_mut().unwrap()
}

fn audio(settings: &mut ExportSettings) -> &mut AudioSettings {
    settings.audio.as_mut().unwrap()
}

fn invalid(result: Result<export::ExportPlan, ExportError>) -> String {
    match result {
        Err(ExportError::Invalid(message)) => message,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

fn unsupported(result: Result<export::ExportPlan, ExportError>) -> String {
    match result {
        Err(ExportError::Unsupported(message)) => message,
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

fn empty(result: Result<export::ExportPlan, ExportError>) -> String {
    match result {
        Err(ExportError::Empty(message)) => message,
        other => panic!("expected Empty, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Frames and samples
// ---------------------------------------------------------------------------

#[test]
fn frame_span_covers_whole_frames_at_common_rates() {
    for frame_rate in [rate(30_000, 1001), rate(24, 1), rate(60, 1)] {
        let span = FrameSpan::covering(Time::ZERO, ms(10_000), frame_rate);
        assert_eq!(span.first, 0);
        // The last frame starts before the end; the next one at or after it.
        assert!(span.time(span.count - 1) < ms(10_000), "{frame_rate:?}");
        assert!(span.end() >= ms(10_000), "{frame_rate:?}");
        for index in [0, 1, 7, span.count - 1, span.count] {
            assert_eq!(span.time(index), frame_rate.frame_time(index as i64));
        }
        assert_eq!(span.duration(), span.end() - span.start());
    }
    // 299.7 frames of 30000/1001 in ten seconds round up to 300.
    let ntsc = FrameSpan::covering(Time::ZERO, ms(10_000), rate(30_000, 1001));
    assert_eq!(ntsc.count, 300);
    assert_eq!(ntsc.end(), Time(10_010_000));
    assert_eq!(
        FrameSpan::covering(Time::ZERO, ms(10_000), rate(24, 1)).count,
        240
    );
    assert_eq!(
        FrameSpan::covering(Time::ZERO, ms(10_000), rate(60, 1)).count,
        600
    );
}

#[test]
fn frame_span_snaps_a_range_outwards() {
    let ntsc = rate(30_000, 1001);
    // Frame starts at 30000/1001 are not whole microseconds; frame_time floors.
    assert_eq!(ntsc.frame_time(1), Time(33_366));
    assert_eq!(ntsc.frame_time(2), Time(66_733));

    // Exactly on frame boundaries: no extra frame either side.
    let span = FrameSpan::covering(Time(33_366), Time(66_733), ntsc);
    assert_eq!((span.first, span.count), (1, 1));
    // One microsecond past the end needs the next frame.
    let span = FrameSpan::covering(Time(33_366), Time(66_734), ntsc);
    assert_eq!((span.first, span.count), (1, 2));
    // One microsecond before a frame starts is still the previous frame.
    let span = FrameSpan::covering(Time(33_365), Time(66_733), ntsc);
    assert_eq!((span.first, span.count), (0, 2));
    assert_eq!(span.start(), Time::ZERO);

    // 24 fps: 1.01 s is in frame 24; 2.5 s is exactly frame 60.
    let span = FrameSpan::covering(ms(1010), ms(2500), rate(24, 1));
    assert_eq!((span.first, span.count), (24, 36));
    assert_eq!((span.start(), span.end()), (ms(1000), ms(2500)));

    // 60 fps: mid-frame on both sides.
    let span = FrameSpan::covering(Time(10_000), Time(40_000), rate(60, 1));
    assert_eq!((span.first, span.count), (0, 3));
    assert_eq!(span.end(), Time(50_000));

    // Negative starts are clamped to zero; reversed ranges cover nothing.
    assert_eq!(
        FrameSpan::covering(Time(-5), Time(10), rate(24, 1)).first,
        0
    );
    assert_eq!(FrameSpan::covering(ms(500), ms(100), rate(24, 1)).count, 0);
}

#[test]
fn sample_count_is_exact_over_ntsc_frames() {
    let ntsc = rate(30_000, 1001);
    // 100 frames at 30000/1001 are exactly 160160 samples at 48 kHz,
    // wherever they start.
    for first in [0, 1, 7, 1001, 29_999, 1_000_003] {
        let span = FrameSpan {
            rate: ntsc,
            first,
            count: 100,
        };
        assert_eq!(
            export::sample_count(span.start(), span.end(), 48_000),
            160_160,
            "{first}"
        );
    }
    assert_eq!(export::sample_count(ms(1000), ms(1000), 48_000), 0);
    assert_eq!(export::sample_count(ms(1000), ms(500), 48_000), 0);
    assert_eq!(export::sample_count(Time::ZERO, ms(1000), 44_100), 44_100);
}

#[test]
fn sample_count_does_not_drift_over_long_spans() {
    let ntsc = rate(30_000, 1001);
    // Each NTSC frame is 1601.6 samples at 48 kHz; every count from zero
    // stays within half a sample of the exact value, over 9 hours.
    for frame in (0..1_000_000_i64).step_by(997).chain([1_000_000]) {
        let count = export::sample_count(Time::ZERO, ntsc.frame_time(frame), 48_000) as i64;
        let exact_tenths = frame * 16_016;
        assert!(
            (count * 10 - exact_tenths).abs() <= 5,
            "frame {frame}: {count} samples"
        );
    }
    assert_eq!(
        export::sample_count(Time::ZERO, ntsc.frame_time(1_000_000), 48_000),
        1_601_600_000
    );
    // 24000 frames of 24000/1001 are exactly 1001 s.
    let film = rate(24_000, 1001);
    assert_eq!(
        export::sample_count(Time::ZERO, film.frame_time(24_000), 44_100),
        44_144_100
    );
}

// ---------------------------------------------------------------------------
// Sizes and placement
// ---------------------------------------------------------------------------

#[test]
fn resolution_sizes_follow_the_canvas() {
    let wide = [1920, 1080];
    let tall = [1080, 1920];
    let square = [1080, 1080];
    assert_eq!(Resolution::Project.size(wide), [1920, 1080]);
    assert_eq!(Resolution::P720.size(wide), [1280, 720]);
    assert_eq!(Resolution::P1080.size(wide), [1920, 1080]);
    assert_eq!(Resolution::P1440.size(wide), [2560, 1440]);
    assert_eq!(Resolution::P2160.size(wide), [3840, 2160]);
    assert_eq!(Resolution::P1080.size(tall), [1080, 1920]);
    assert_eq!(Resolution::P2160.size(tall), [2160, 3840]);
    assert_eq!(Resolution::P720.size(square), [720, 720]);
    assert_eq!(
        Resolution::Custom {
            width: 1080,
            height: 1350
        }
        .size(wide),
        [1080, 1350]
    );
    // Odd sides round up to even.
    assert_eq!(Resolution::Project.size([1081, 607]), [1082, 608]);
    assert_eq!(Resolution::P720.size([1081, 607]), [1282, 720]);
    assert_eq!(
        Resolution::Custom {
            width: 1081,
            height: 1921
        }
        .size(wide),
        [1082, 1922]
    );
}

#[test]
fn huge_sizes_scale_down_whole() {
    for (resolution, canvas) in [
        (Resolution::Project, [16_384, 9216]),
        (Resolution::Project, [9216, 16_384]),
        (Resolution::Project, [16_384, 8192]),
        (Resolution::Project, [11_000, 11_000]),
        (Resolution::Short { pixels: 8192 }, [1920, 1080]),
        (Resolution::Short { pixels: 4320 }, [3000, 1000]),
    ] {
        let size = resolution.size(canvas);
        assert!(size.iter().all(|side| side % 2 == 0), "{size:?}");
        assert_eq!(size[0].max(size[1]), MAX_OUTPUT_SIDE, "{canvas:?}");
        let asked = canvas[0] as f64 / canvas[1] as f64;
        let made = size[0] as f64 / size[1] as f64;
        assert!(
            (asked - made).abs() / asked < 0.002,
            "{canvas:?} → {size:?}"
        );
    }
    assert_eq!(Resolution::Project.size([16_384, 9216]), [8192, 4608]);
    assert_eq!(
        Resolution::Short { pixels: 4320 }.size([1920, 1080]),
        [7680, 4320]
    );
}

#[test]
fn geometry_letterboxes_and_fills() {
    let exact = Geometry::new([1920, 1080], [1280, 720], Fit::Letterbox);
    assert!(exact.is_exact());
    assert_eq!(exact.drawn, [1280, 720]);

    // A wide canvas in a tall picture: bars above and below.
    let boxed = Geometry::new([1920, 1080], [1080, 1920], Fit::Letterbox);
    assert_eq!(boxed.drawn, [1080, 608]);
    assert_eq!(boxed.offset, [0, 656]);
    assert!(!boxed.is_exact());

    // Filled: the canvas overflows the sides evenly.
    let filled = Geometry::new([1920, 1080], [1080, 1920], Fit::Fill);
    assert_eq!(filled.drawn, [3413, 1920]);
    assert_eq!(filled.offset, [-1166, 0]);

    // A tall canvas in a wide picture.
    let pillar = Geometry::new([1080, 1920], [1920, 1080], Fit::Letterbox);
    assert_eq!(pillar.drawn, [608, 1080]);
    assert_eq!(pillar.offset, [656, 0]);
    let cropped = Geometry::new([1080, 1920], [1920, 1080], Fit::Fill);
    assert_eq!(cropped.drawn, [1920, 3413]);
    assert_eq!(cropped.offset, [0, -1166]);

    // Odd canvases rounded up to even still cover the output exactly.
    for fit in [Fit::Letterbox, Fit::Fill] {
        assert!(Geometry::new([1081, 1081], [1082, 1082], fit).is_exact());
        assert!(Geometry::new([1081, 607], [1082, 608], fit).is_exact());
    }
}

// ---------------------------------------------------------------------------
// Encoders
// ---------------------------------------------------------------------------

#[test]
fn auto_takes_fitting_hardware_then_software() {
    let caps = host_caps();
    let name = |codec, choice, size| {
        caps.video_encoder(codec, choice, size)
            .map(|e| e.name.as_str())
    };
    let auto = EncoderChoice::Auto;
    assert_eq!(caps.hardware(), [HardwareKind::Nvenc, HardwareKind::Vaapi]);

    assert_eq!(
        name(VideoCodec::H264, auto, [1920, 1080]),
        Some("h264_nvenc")
    );
    assert_eq!(
        name(VideoCodec::H265, auto, [3840, 2160]),
        Some("hevc_nvenc")
    );
    // H.264 hardware stops at 4096 a side; H.265 goes on.
    assert_eq!(name(VideoCodec::H264, auto, [7680, 4320]), Some("libx264"));
    assert_eq!(
        name(VideoCodec::H265, auto, [7680, 4320]),
        Some("hevc_nvenc")
    );
    // NVENC refuses tiny pictures.
    assert_eq!(name(VideoCodec::H264, auto, [144, 48]), Some("libx264"));
    assert_eq!(name(VideoCodec::H264, auto, [146, 50]), Some("h264_nvenc"));
    assert_eq!(name(VideoCodec::H265, auto, [128, 34]), Some("libx265"));
    // Codecs with no hardware here use software.
    assert_eq!(
        name(VideoCodec::ProRes, auto, [1920, 1080]),
        Some("prores_ks")
    );
    assert_eq!(name(VideoCodec::Vp9, auto, [1920, 1080]), Some("vp9_vaapi"));

    let software = EncoderChoice::Software;
    assert_eq!(
        name(VideoCodec::H264, software, [1920, 1080]),
        Some("libx264")
    );
    assert_eq!(
        name(VideoCodec::Av1, software, [1920, 1080]),
        Some("libsvtav1")
    );
    assert_eq!(
        caps.software_encoder(VideoCodec::H265, [1920, 1080])
            .map(|e| e.name.as_str()),
        Some("libx265")
    );

    let vaapi = EncoderChoice::Hardware(HardwareKind::Vaapi);
    assert_eq!(
        name(VideoCodec::H264, vaapi, [1920, 1080]),
        Some("h264_vaapi")
    );
    assert_eq!(name(VideoCodec::H264, vaapi, [7680, 4320]), None);
    assert_eq!(name(VideoCodec::H264, vaapi, [100, 100]), None);
    let amf = EncoderChoice::Hardware(HardwareKind::Amf);
    assert_eq!(name(VideoCodec::H264, amf, [1920, 1080]), None);

    // VideoToolbox ranks first when present.
    let all = full_caps();
    assert_eq!(
        all.video_encoder(VideoCodec::H264, auto, [1920, 1080])
            .map(|e| e.name.as_str()),
        Some("h264_videotoolbox")
    );
    assert_eq!(all.hardware()[0], HardwareKind::VideoToolbox);
    assert_eq!(all.hardware().len(), HardwareKind::ALL.len());
}

// ---------------------------------------------------------------------------
// Planning: refusals
// ---------------------------------------------------------------------------

#[test]
fn nothing_to_export_is_empty() {
    let caps = full_caps();
    let blank = Project::new("Blank");
    assert_eq!(
        empty(ExportSettings::default().plan(&blank, &caps)),
        "The timeline is empty"
    );
    let project = project(rate(30, 1), 10);
    for (start, end) in [(20_000, 30_000), (5000, 5000), (6000, 5000), (-3000, 0)] {
        let range = settings(|s| {
            s.range = ExportRange::Span {
                start: ms(start),
                end: ms(end),
            }
        });
        empty(range.plan(&project, &caps));
    }
}

#[test]
fn formats_must_match_the_container_and_host() {
    let project = project(rate(30, 1), 10);
    let caps = full_caps();

    let mut no_webm = full_caps();
    no_webm.containers.retain(|c| *c != Container::Webm);
    let webm = builtin("webm").settings;
    assert_eq!(
        unsupported(webm.plan(&project, &no_webm)),
        "WebM files cannot be written by this FFmpeg"
    );

    // Sound-only containers take no picture; picture containers need one.
    let wav_with_video = settings(|s| {
        s.container = Container::Wav;
        s.audio = Some(AudioSettings {
            codec: AudioCodec::Pcm24,
            ..AudioSettings::default()
        });
    });
    invalid(wav_with_video.plan(&project, &caps));
    let mp4_without_video = settings(|s| s.video = None);
    invalid(mp4_without_video.plan(&project, &caps));
    let silent_wav = ExportSettings {
        container: Container::Wav,
        video: None,
        audio: None,
        ..ExportSettings::default()
    };
    invalid(silent_wav.plan(&project, &caps));

    let vp9_in_mp4 = settings(|s| video(s).codec = VideoCodec::Vp9);
    assert_eq!(
        invalid(vp9_in_mp4.plan(&project, &caps)),
        "MP4 files cannot hold VP9 video; choose WebM"
    );
    let aac_in_webm = settings(|s| {
        s.container = Container::Webm;
        video(s).codec = VideoCodec::Vp9;
    });
    assert_eq!(
        invalid(aac_in_webm.plan(&project, &caps)),
        "WebM files cannot hold AAC sound; choose Opus"
    );

    let fast = settings(|s| video(s).frame_rate = Some(rate(300, 1)));
    invalid(fast.plan(&project, &caps));
    let zero = settings(|s| video(s).frame_rate = Some(rate(30, 0)));
    invalid(zero.plan(&project, &caps));
}

#[test]
fn missing_encoders_are_named() {
    let project = project(rate(30, 1), 10);

    let no_hevc = caps_with(|e| e.codec != VideoCodec::H265);
    let hevc = settings(|s| video(s).codec = VideoCodec::H265);
    let message = unsupported(hevc.plan(&project, &no_hevc));
    assert!(message.contains("H.265"), "{message}");

    let amf = settings(|s| video(s).encoder = EncoderChoice::Hardware(HardwareKind::Amf));
    let message = unsupported(amf.plan(&project, &host_caps()));
    assert!(
        message.contains("AMD AMF") && message.contains("H.264"),
        "{message}"
    );

    let no_software = caps_with(|e| e.hardware.is_some());
    let software = settings(|s| video(s).encoder = EncoderChoice::Software);
    let message = unsupported(software.plan(&project, &no_software));
    assert!(message.contains("software H.264"), "{message}");

    // Only encoders smaller than the picture: the size is named.
    let nvenc_only = caps_with(|e| e.hardware == Some(HardwareKind::Nvenc));
    let uhd = settings(|s| video(s).resolution = Resolution::Short { pixels: 2400 });
    let message = unsupported(uhd.plan(&project, &nvenc_only));
    assert!(
        message.contains("4,096") && message.contains("4268 × 2400"),
        "{message}"
    );
    let nvenc = settings(|s| {
        let video = video(s);
        video.resolution = Resolution::Custom {
            width: 64,
            height: 36,
        };
        video.encoder = EncoderChoice::Hardware(HardwareKind::Nvenc);
    });
    let message = unsupported(nvenc.plan(&project, &host_caps()));
    assert!(
        message.contains("NVIDIA NVENC") && message.contains("64 × 36"),
        "{message}"
    );

    let no_opus = Capabilities {
        audio: Vec::new(),
        ..full_caps()
    };
    let message = unsupported(builtin("webm").settings.plan(&project, &no_opus));
    assert!(message.contains("Opus"), "{message}");
}

#[test]
fn asked_sizes_outside_the_limits_are_refused() {
    let project = project(rate(30, 1), 10);
    let caps = full_caps();
    for resolution in [
        Resolution::Custom {
            width: 8,
            height: 100,
        },
        Resolution::Custom {
            width: 9000,
            height: 100,
        },
        Resolution::Custom {
            width: 100_000,
            height: 5,
        },
        Resolution::Short { pixels: 0 },
        Resolution::Short { pixels: 15 },
        Resolution::Short { pixels: 10_000 },
    ] {
        let asked = settings(|s| video(s).resolution = resolution);
        invalid(asked.plan(&project, &caps));
    }
    let largest = settings(|s| {
        video(s).resolution = Resolution::Custom {
            width: MAX_OUTPUT_SIDE,
            height: 16,
        }
    });
    assert!(largest.plan(&project, &caps).is_ok());

    // A fill whose drawn canvas passes the compositor's limit.
    let tall_fill = settings(|s| {
        let video = video(s);
        video.resolution = Resolution::Custom {
            width: 4320,
            height: 7680,
        };
        video.fit = Fit::Fill;
    });
    assert_eq!(
        invalid(tall_fill.plan(&project, &caps)),
        "Fill would need a picture larger than the compositor can draw; use Letterbox"
    );
    let tall_box = settings(|s| {
        video(s).resolution = Resolution::Custom {
            width: 4320,
            height: 7680,
        }
    });
    let plan = tall_box.plan(&project, &caps).unwrap();
    assert_eq!(plan.video.unwrap().geometry.drawn, [4320, 2430]);
}

#[test]
fn bitrates_are_checked_per_codec() {
    let project = project(rate(30, 1), 10);
    let caps = full_caps();
    for kbps in [0, 99, 800_001] {
        let low = settings(|s| video(s).rate_control = RateControl::Bitrate { kbps });
        invalid(low.plan(&project, &caps));
    }
    for kbps in [100, 800_000] {
        let fine = settings(|s| video(s).rate_control = RateControl::Bitrate { kbps });
        assert!(fine.plan(&project, &caps).is_ok());
    }

    let prores = settings(|s| {
        s.container = Container::Mov;
        let video = video(s);
        video.codec = VideoCodec::ProRes;
        video.rate_control = RateControl::Bitrate { kbps: 100_000 };
    });
    assert_eq!(
        invalid(prores.plan(&project, &caps)),
        "ProRes quality is set by its profile"
    );

    for kbps in [16, 513] {
        let aac = settings(|s| audio(s).kbps = kbps);
        invalid(aac.plan(&project, &caps));
    }
    // AAC mono stops at 256.
    let mono = settings(|s| {
        let audio = audio(s);
        audio.channels = 1;
        audio.kbps = 320;
    });
    invalid(mono.plan(&project, &caps));

    let webm = builtin("webm").settings;
    let opus = |channels, kbps| {
        let mut settings = webm.clone();
        let audio = settings.audio.as_mut().unwrap();
        audio.channels = channels;
        audio.kbps = kbps;
        settings
    };
    let message = invalid(opus(1, 300).plan(&project, &caps));
    assert!(message.contains("256"), "{message}");
    assert!(opus(1, 256).plan(&project, &caps).is_ok());
    assert!(opus(2, 512).plan(&project, &caps).is_ok());

    let mp3 = |kbps| {
        let mut settings = builtin("audio-mp3").settings;
        settings.audio.as_mut().unwrap().kbps = kbps;
        settings
    };
    invalid(mp3(384).plan(&project, &caps));
    assert!(mp3(320).plan(&project, &caps).is_ok());

    // Lossless sound ignores the bitrate.
    let mut wav = builtin("audio-wav").settings;
    wav.audio.as_mut().unwrap().kbps = 100_000;
    assert!(wav.plan(&project, &caps).is_ok());
}

#[test]
fn sound_layout_and_rate_are_checked() {
    let project = project(rate(30, 1), 10);
    let caps = full_caps();
    for channels in [0, 3, 6] {
        let surround = settings(|s| audio(s).channels = channels);
        assert_eq!(
            invalid(surround.plan(&project, &caps)),
            "Sound must be mono or stereo"
        );
    }
    let odd_rate = settings(|s| audio(s).sample_rate = 22_050);
    invalid(odd_rate.plan(&project, &caps));

    let mut mp3 = builtin("audio-mp3").settings;
    mp3.audio.as_mut().unwrap().sample_rate = 96_000;
    assert_eq!(
        invalid(mp3.plan(&project, &caps)),
        "MP3 sound cannot be 96 kHz; choose 48 kHz"
    );
    let mut opus = builtin("webm").settings;
    opus.audio.as_mut().unwrap().sample_rate = 44_100;
    assert_eq!(
        invalid(opus.plan(&project, &caps)),
        "Opus sound cannot be 44.1 kHz; choose 48 kHz"
    );
}

#[test]
fn caption_files_and_streams_need_a_caption_track() {
    let project = project(rate(30, 1), 10);
    let caps = full_caps();
    let sidecar = settings(|s| s.subtitles.sidecar = Some(SidecarFormat::Srt));
    assert_eq!(
        invalid(sidecar.plan(&project, &caps)),
        "There is no caption track to write"
    );
    let embed = settings(|s| s.subtitles.embed = true);
    assert_eq!(
        invalid(embed.plan(&project, &caps)),
        "There is no caption track to write"
    );

    // A text track without a language is titles, not captions.
    let mut titles = project.clone();
    Arc::make_mut(&mut titles.tracks[2])
        .clips
        .push(Clip::text("Title", Time::ZERO, ms(1000)));
    invalid(sidecar.plan(&titles, &caps));

    let captioned = with_captions(project.clone());
    let wrong = settings(|s| {
        s.subtitles.sidecar = Some(SidecarFormat::Vtt);
        s.subtitles.track = Some(Uuid::new_v4());
    });
    invalid(wrong.plan(&captioned, &caps));

    // Audio files carry no caption stream.
    let mut wav = builtin("audio-wav").settings;
    wav.subtitles.embed = true;
    assert_eq!(
        invalid(wav.plan(&captioned, &caps)),
        "WAV files cannot carry captions; write a caption file instead"
    );

    // A range with no captions in it.
    let quiet = settings(|s| {
        s.subtitles.sidecar = Some(SidecarFormat::Srt);
        s.range = ExportRange::Span {
            start: ms(2000),
            end: ms(3000),
        };
    });
    assert_eq!(
        empty(quiet.plan(&captioned, &caps)),
        "The caption track has no captions in the range"
    );
}

// ---------------------------------------------------------------------------
// Planning: what a valid export covers
// ---------------------------------------------------------------------------

#[test]
fn sound_spans_exactly_the_pictures() {
    let ntsc = rate(30_000, 1001);
    let project = project(ntsc, 10);
    let plan = ExportSettings::default()
        .plan(&project, &host_caps())
        .unwrap();
    let video = plan.video.as_ref().unwrap();
    let audio = plan.audio.as_ref().unwrap();
    assert_eq!(video.frames.rate, ntsc);
    assert_eq!((video.frames.first, video.frames.count), (0, 300));
    assert_eq!((plan.start, plan.end), (Time::ZERO, Time(10_010_000)));
    assert_eq!(video.encoder.name, "h264_nvenc");
    assert!(video.geometry.is_exact());
    assert_eq!(video.geometry.output, [1920, 1080]);
    assert_eq!(audio.encoder.name, "aac_at");
    assert_eq!(audio.samples.start, video.frames.start());
    assert_eq!(audio.samples.rate, 48_000);
    assert_eq!(audio.samples.channels, 2);
    assert_eq!(
        audio.samples.frames,
        export::sample_count(video.frames.start(), video.frames.end(), 48_000)
    );
    assert_eq!(audio.samples.frames, 480_480);
    assert!(plan.cues.is_none());
    assert_eq!(plan.settings, ExportSettings::default());
}

#[test]
fn a_range_snaps_to_output_frames() {
    let project = project(rate(30, 1), 10);
    let range = settings(|s| {
        s.range = ExportRange::Span {
            start: ms(1010),
            end: ms(2500),
        };
        video(s).frame_rate = Some(rate(24, 1));
    });
    let plan = range.plan(&project, &full_caps()).unwrap();
    let video = plan.video.unwrap();
    assert_eq!(video.frames.rate, rate(24, 1));
    assert_eq!((video.frames.first, video.frames.count), (24, 36));
    assert_eq!((plan.start, plan.end), (ms(1000), ms(2500)));
    let samples = plan.audio.unwrap().samples;
    assert_eq!((samples.start, samples.frames), (ms(1000), 72_000));

    // A range past the end stops at the timeline's end.
    let tail = settings(|s| {
        s.range = ExportRange::Span {
            start: ms(9000),
            end: ms(60_000),
        }
    });
    let plan = tail.plan(&project, &full_caps()).unwrap();
    assert_eq!((plan.start, plan.end), (ms(9000), ms(10_000)));
}

#[test]
fn sound_only_covers_the_range_unsnapped() {
    let project = project(rate(30, 1), 10);
    let mut wav = builtin("audio-wav").settings;
    wav.range = ExportRange::Span {
        start: ms(1010),
        end: ms(2500),
    };
    let plan = wav.plan(&project, &full_caps()).unwrap();
    assert!(plan.video.is_none());
    assert_eq!((plan.start, plan.end), (ms(1010), ms(2500)));
    let audio = plan.audio.unwrap();
    assert_eq!(audio.encoder.name, "pcm_s24le");
    assert_eq!(
        (audio.samples.start, audio.samples.frames),
        (ms(1010), 71_520)
    );
}

#[test]
fn caption_plans_carry_the_cut_track() {
    let project = with_captions(project(rate(30, 1), 10));
    let track = project.tracks.last().unwrap().id;
    let both = settings(|s| {
        s.subtitles.sidecar = Some(SidecarFormat::Vtt);
        s.subtitles.embed = true;
        s.range = ExportRange::Span {
            start: ms(4000),
            end: ms(7000),
        };
    });
    let plan = both.plan(&project, &full_caps()).unwrap();
    let cues = plan.cues.unwrap();
    assert_eq!(cues.track, track);
    assert_eq!(cues.language.as_deref(), Some("en"));
    assert_eq!(cues.cues.len(), 2);
    assert_eq!(
        (cues.cues[0].start, cues.cues[0].end),
        (Time::ZERO, ms(1000))
    );
    assert_eq!((cues.cues[1].start, cues.cues[1].end), (ms(2000), ms(3000)));

    // The chosen track by id; a WebM stream works too.
    let mut webm = builtin("webm").settings;
    webm.subtitles.embed = true;
    webm.subtitles.track = Some(track);
    let plan = webm.plan(&project, &full_caps()).unwrap();
    assert_eq!(plan.cues.unwrap().cues.len(), 3);
}

#[test]
fn estimates_follow_the_bitrate() {
    let project = project(rate(30, 1), 10);
    let caps = full_caps();
    let mut x = builtin("x").settings;
    x.audio = None;
    let plan = x.plan(&project, &caps).unwrap();
    // 12,000 kbit/s for ten seconds, plus 1 %.
    assert_eq!(ExportSettings::estimate_bytes(&plan), 15_150_000);

    let wav = builtin("audio-wav").settings.plan(&project, &caps).unwrap();
    // 480,000 stereo 24-bit samples, plus 1 %.
    assert_eq!(ExportSettings::estimate_bytes(&wav), 2_908_800);
    let flac = builtin("audio-flac")
        .settings
        .plan(&project, &caps)
        .unwrap();
    assert_eq!(ExportSettings::estimate_bytes(&flac), 1_163_520);
    let aac = builtin("audio-aac").settings.plan(&project, &caps).unwrap();
    assert_eq!(ExportSettings::estimate_bytes(&aac), 323_200);

    // Quality mode assumes the suggested bitrate; higher means larger.
    let estimate = |level| {
        let quality = settings(|s| video(s).rate_control = RateControl::Quality { level });
        ExportSettings::estimate_bytes(&quality.plan(&project, &caps).unwrap())
    };
    assert!(estimate(QualityLevel::Lower) < estimate(QualityLevel::Recommended));
    assert!(estimate(QualityLevel::Recommended) < estimate(QualityLevel::Higher));
    assert!(estimate(QualityLevel::Higher) < estimate(QualityLevel::Maximum));
    let suggested = export::suggested_kbps(
        VideoCodec::H264,
        [1920, 1080],
        30.0,
        QualityLevel::Recommended,
    );
    let sound = 256.0 * 125.0 * 10.0;
    let expected = ((suggested as f64 * 125.0 * 10.0 + sound) * 1.01).round() as u64;
    assert_eq!(estimate(QualityLevel::Recommended), expected);
}

#[test]
fn quality_estimates_follow_measured_encodes_on_the_encoder_that_runs() {
    // Calibrated against libx264 at CRF 21 and libx265 at CRF 24 (medium),
    // and NVENC at the values the export gives it: 0.14 and 0.11 bits a
    // pixel and frame at Recommended.
    let full_hd =
        |codec| export::suggested_kbps(codec, [1920, 1080], 30.0, QualityLevel::Recommended);
    assert_eq!(full_hd(VideoCodec::H264), 8_709);
    assert_eq!(full_hd(VideoCodec::H265), 6_843);

    let project = project(rate(30, 1), 10);
    let estimate = |codec, choice, speed| {
        let quality = settings(|s| {
            s.audio = None;
            let video = video(s);
            video.codec = codec;
            video.encoder = choice;
            video.speed = speed;
        });
        let plan = quality.plan(&project, &full_caps()).unwrap();
        (
            plan.video.as_ref().unwrap().encoder.name.clone(),
            ExportSettings::estimate_bytes(&plan),
        )
    };
    let software = estimate(
        VideoCodec::H264,
        EncoderChoice::Software,
        EncodeSpeed::Balanced,
    );
    assert_eq!(software.0, "libx264");
    // 8,709 kbit/s for ten seconds, plus 1 %.
    assert_eq!(software.1, 10_995_113);
    // NVENC is given the value whose files match x264's: the same estimate.
    let nvenc = estimate(
        VideoCodec::H264,
        EncoderChoice::Hardware(HardwareKind::Nvenc),
        EncodeSpeed::Balanced,
    );
    assert_eq!(nvenc, ("h264_nvenc".to_owned(), software.1));
    // x264's fastest preset writes less at the same factor; its slowest
    // about the same.
    let faster = estimate(
        VideoCodec::H264,
        EncoderChoice::Software,
        EncodeSpeed::Faster,
    );
    assert_eq!(faster.1, (software.1 as f64 * 0.85).round() as u64);
    let smaller = estimate(
        VideoCodec::H264,
        EncoderChoice::Software,
        EncodeSpeed::Smaller,
    );
    assert_eq!(smaller.1, software.1);
    // Other encoders' presets do not move it.
    let x265 = estimate(
        VideoCodec::H265,
        EncoderChoice::Software,
        EncodeSpeed::Faster,
    );
    assert_eq!(x265.0, "libx265");
    assert_eq!(
        x265.1,
        estimate(
            VideoCodec::H265,
            EncoderChoice::Software,
            EncodeSpeed::Balanced
        )
        .1
    );
    // A bitrate is held whatever runs it.
    let held = |speed| {
        let bitrate = settings(|s| {
            s.audio = None;
            let video = video(s);
            video.encoder = EncoderChoice::Software;
            video.speed = speed;
            video.rate_control = RateControl::Bitrate { kbps: 8_000 };
        });
        ExportSettings::estimate_bytes(&bitrate.plan(&project, &full_caps()).unwrap())
    };
    assert_eq!(held(EncodeSpeed::Faster), 10_100_000);
    assert_eq!(held(EncodeSpeed::Balanced), 10_100_000);
}

// ---------------------------------------------------------------------------
// The project as drawn, and captions cut to a range
// ---------------------------------------------------------------------------

#[test]
fn render_project_sets_the_rate_and_hides_unburned_captions() {
    let mut project = with_captions(project(rate(30, 1), 10));
    Arc::make_mut(&mut project.tracks[2])
        .clips
        .push(Clip::text("Title", Time::ZERO, ms(1000)));
    let original = project.clone();

    let kept = ExportSettings::default().render_project(&project);
    assert_eq!(kept, project);

    let plain = settings(|s| {
        s.subtitles.burn_in = false;
        video(s).frame_rate = Some(rate(60, 1));
    });
    let drawn = plain.render_project(&project);
    assert_eq!(drawn.settings.frame_rate, rate(60, 1));
    let hidden: Vec<_> = drawn.tracks.iter().map(|t| t.hidden).collect();
    // Video, audio and the title track stay; the caption track is hidden.
    assert_eq!(hidden, [false, false, false, true]);
    assert_eq!(drawn.tracks[3].clips, project.tracks[3].clips);
    // The snapshot itself is never changed.
    assert_eq!(project, original);
    assert!(!project.tracks[3].hidden);
}

#[test]
fn range_cues_are_cut_and_moved() {
    let track = captions();
    let all = export::range_cues(&track, Time::ZERO, ms(10_000));
    assert_eq!(all.len(), 3);
    assert_eq!(all[1].words.len(), 3);

    let cues = export::range_cues(&track, ms(4000), ms(7000));
    assert_eq!(cues.len(), 2);
    let cut = &cues[0];
    assert_eq!(cut.text, "one two three");
    assert_eq!((cut.start, cut.end), (Time::ZERO, ms(1000)));
    // One word time per word, clamped into the cut cue.
    let words: Vec<_> = cut.words.iter().map(|w| (w.at, w.end)).collect();
    assert_eq!(
        words,
        [
            (Time::ZERO, Time::ZERO),
            (Time::ZERO, ms(200)),
            (ms(200), ms(1000))
        ]
    );
    assert_eq!((cues[1].start, cues[1].end), (ms(2000), ms(3000)));
    assert_eq!(cues[1].text, "After");

    // Touching an edge is not overlap.
    let between = export::range_cues(&track, ms(2000), ms(3000));
    assert!(between.is_empty());
    assert!(export::range_cues(&track, ms(5000), ms(5000)).is_empty());
    let first = export::range_cues(&track, ms(1000), ms(3000));
    assert_eq!(first.len(), 1);
    assert_eq!((first[0].start, first[0].end), (Time::ZERO, ms(1000)));

    // Written out, the cut track starts at zero.
    let srt = editor_core::captions::format_srt(&cues);
    assert!(
        srt.starts_with("1\n00:00:00,000 --> 00:00:01,000\none two three\n"),
        "{srt}"
    );
}

// ---------------------------------------------------------------------------
// Presets and the wire
// ---------------------------------------------------------------------------

fn builtin(id: &str) -> ExportPreset {
    export::builtin_presets()
        .into_iter()
        .find(|preset| preset.id == id)
        .unwrap_or_else(|| panic!("no preset {id}"))
}

#[test]
fn builtin_presets_are_stable_and_plan_on_a_full_host() {
    let presets = export::builtin_presets();
    let ids: Vec<_> = presets.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "recommended",
            "high-quality",
            "small-file",
            "youtube-1080",
            "youtube-4k",
            "youtube-shorts",
            "tiktok",
            "instagram-reels",
            "instagram-feed",
            "facebook",
            "x",
            "vimeo",
            "prores-master",
            "webm",
            "audio-aac",
            "audio-wav",
            "audio-mp3",
            "audio-flac",
        ]
    );
    let project = project(rate(30_000, 1001), 10);
    for caps in [
        full_caps(),
        host_caps(),
        caps_with(|e| e.hardware.is_none()),
    ] {
        for preset in &presets {
            let plan = preset
                .settings
                .plan(&project, &caps)
                .unwrap_or_else(|error| panic!("{}: {error}", preset.id));
            assert!(ExportSettings::estimate_bytes(&plan) > 0, "{}", preset.id);
        }
    }
    for preset in &presets {
        let settings = &preset.settings;
        assert!(!preset.name.is_empty() && !preset.note.is_empty());
        assert_eq!(settings.range, ExportRange::Timeline, "{}", preset.id);
        assert_eq!(settings.subtitles, Default::default(), "{}", preset.id);
        if let Some(video) = &settings.video {
            assert_eq!(video.frame_rate, None);
            assert_eq!(video.encoder, EncoderChoice::Auto);
            assert_eq!(video.fit, Fit::Letterbox);
        }
        assert_eq!(
            settings.video.is_some(),
            preset.destination != Destination::Audio,
            "{}",
            preset.id
        );
    }
    let note = |id| builtin(id).note;
    assert_eq!(note("youtube-shorts"), "1080 × 1920 · H.264 · Higher");
    assert_eq!(note("recommended"), "Project size · H.264 · Recommended");
    assert_eq!(note("youtube-4k"), "4K · H.265 · Higher");
    assert_eq!(note("x"), "1080p · H.264 · 12,000 kbit/s");
    assert_eq!(note("prores-master"), "Project size · ProRes 422 HQ");
    assert_eq!(note("audio-mp3"), "MP3 · 320 kbit/s · 44.1 kHz");
    assert_eq!(note("audio-flac"), "FLAC · 48 kHz");

    let shorts = builtin("youtube-shorts")
        .settings
        .plan(&project, &full_caps())
        .unwrap();
    assert_eq!(shorts.video.unwrap().geometry.output, [1080, 1920]);
    let small = builtin("small-file").settings;
    assert_eq!(small.video.unwrap().codec, VideoCodec::H265);
    assert_eq!(small.audio.unwrap().kbps, 160);
}

#[test]
fn applying_a_preset_keeps_the_range_and_track() {
    let track = Uuid::new_v4();
    let current = settings(|s| {
        s.range = ExportRange::Span {
            start: ms(1000),
            end: ms(2000),
        };
        s.subtitles.track = Some(track);
        s.subtitles.burn_in = false;
    });
    let applied = export::apply_preset(&current, &builtin("webm"));
    assert_eq!(applied.container, Container::Webm);
    assert_eq!(applied.range, current.range);
    assert_eq!(applied.subtitles.track, Some(track));
    assert!(applied.subtitles.burn_in);
}

#[test]
fn settings_and_jobs_round_trip_as_json() {
    let custom = settings(|s| {
        let video = video(s);
        video.codec = VideoCodec::H265;
        video.resolution = Resolution::Custom {
            width: 1080,
            height: 1350,
        };
        video.frame_rate = Some(rate(30_000, 1001));
        video.rate_control = RateControl::Bitrate { kbps: 20_000 };
        video.encoder = EncoderChoice::Hardware(HardwareKind::Nvenc);
        video.fit = Fit::Fill;
        s.range = ExportRange::Span {
            start: ms(1000),
            end: ms(2000),
        };
        s.subtitles.sidecar = Some(SidecarFormat::Vtt);
        s.subtitles.track = Some(Uuid::new_v4());
    });
    let json = serde_json::to_value(&custom).unwrap();
    assert_eq!(
        json["video"]["encoder"],
        serde_json::json!({"kind": "hardware", "family": "nvenc"})
    );
    assert_eq!(json["video"]["rate_control"]["mode"], "bitrate");
    assert_eq!(json["video"]["resolution"]["kind"], "custom");
    assert_eq!(json["range"]["kind"], "span");
    let back: ExportSettings = serde_json::from_value(json).unwrap();
    assert_eq!(back, custom);
    for preset in export::builtin_presets() {
        let text = serde_json::to_string(&preset).unwrap();
        assert_eq!(serde_json::from_str::<ExportPreset>(&text).unwrap(), preset);
    }

    // Defaults fill what a short agent request leaves out.
    let short: ExportSettings = serde_json::from_value(serde_json::json!({
        "container": "mp4",
        "video": {"codec": "h264"},
        "audio": null,
    }))
    .unwrap();
    assert_eq!(short.video, Some(VideoSettings::default()));
    assert_eq!(short.range, ExportRange::Timeline);
    assert!(short.subtitles.burn_in);

    let job = RenderJobInfo {
        id: Uuid::new_v4(),
        name: "Export.mp4".into(),
        destination: PathBuf::from("/home/someone/Videos/Export.mp4"),
        settings: custom,
        project_id: Uuid::new_v4(),
        revision: 42,
        state: RenderState::Rendering,
        progress: 0.25,
        frames_done: 75,
        frames_total: 300,
        speed_fps: 120.5,
        elapsed_seconds: 0.625,
        eta_seconds: Some(1.875),
        encoder: Some("hevc_nvenc".into()),
        output_bytes: None,
        context_id: Uuid::new_v4(),
        message: None,
        attempts: 1,
    };
    let text = serde_json::to_string(&job).unwrap();
    assert!(text.contains("\"state\":\"rendering\""), "{text}");
    assert_eq!(serde_json::from_str::<RenderJobInfo>(&text).unwrap(), job);

    let error = ExportError::Empty("The timeline is empty".into());
    let json = serde_json::to_value(&error).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"kind": "empty", "message": "The timeline is empty"})
    );
}

#[test]
fn settings_schema_describes_the_wire() {
    let schema = serde_json::to_value(schemars::schema_for!(ExportSettings)).unwrap();
    let text = schema.to_string();
    for name in [
        "container",
        "video",
        "audio",
        "range",
        "subtitles",
        "nvenc",
        "letterbox",
    ] {
        assert!(text.contains(name), "schema lacks {name}");
    }
    let required = schema["required"].as_array().unwrap();
    assert!(required.iter().any(|r| r == "container"));
    assert!(!required.iter().any(|r| r == "range"));
    // The other wire types describe themselves too.
    serde_json::to_value(schemars::schema_for!(RenderJobInfo)).unwrap();
    serde_json::to_value(schemars::schema_for!(Capabilities)).unwrap();
}
