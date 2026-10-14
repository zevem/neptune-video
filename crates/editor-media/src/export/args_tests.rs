//! The FFmpeg arguments for each encoder family and container, without
//! running FFmpeg.
use super::{
    AudioInput, EncodeInputs, SubtitleInput, VideoInput,
    args::{
        Family, audio_args, device_args, encode_args, environment, hardware_quality, language_code,
        video_args, x264_threads,
    },
};
use editor_core::{
    FrameRate, Time,
    export::{
        AudioCodec, AudioEncoder, AudioPlan, AudioSettings, Container, EncodeSpeed, ExportPlan,
        ExportSettings, Fit, FrameSpan, Geometry, KnownVideoEncoder, QualityLevel, RateControl,
        SampleSpan, VIDEO_ENCODERS, VideoCodec, VideoEncoder, VideoPlan, VideoSettings,
        known_video_encoder, suggested_kbps,
    },
};
use std::{ffi::OsString, path::Path};

const NTSC: FrameRate = FrameRate {
    numerator: 30_000,
    denominator: 1_001,
};
const SIZE: [u32; 2] = [1920, 1080];

pub(super) fn encoder(known: &KnownVideoEncoder) -> VideoEncoder {
    VideoEncoder {
        name: known.name.into(),
        codec: known.codec,
        hardware: known.hardware,
        max_side: known.max_side,
        min_size: known.min_size,
        device: None,
    }
}

pub(super) fn video_plan(
    name: &str,
    rate: FrameRate,
    size: [u32; 2],
    rate_control: RateControl,
    speed: EncodeSpeed,
) -> VideoPlan {
    let known = known_video_encoder(name).expect("a known encoder");
    VideoPlan {
        frames: FrameSpan {
            rate,
            first: 0,
            count: 24,
        },
        geometry: Geometry::new(size, size, Fit::Letterbox),
        encoder: encoder(known),
        settings: VideoSettings {
            codec: known.codec,
            rate_control,
            speed,
            ..VideoSettings::default()
        },
    }
}

pub(super) fn audio_plan(
    name: &str,
    codec: AudioCodec,
    kbps: u32,
    rate: u32,
    channels: u8,
) -> AudioPlan {
    AudioPlan {
        samples: SampleSpan {
            start: Time::ZERO,
            frames: u64::from(rate),
            rate,
            channels,
        },
        encoder: AudioEncoder {
            name: name.into(),
            codec,
        },
        settings: AudioSettings {
            codec,
            kbps,
            sample_rate: rate,
            channels,
        },
    }
}

pub(super) fn plan(
    container: Container,
    video: Option<VideoPlan>,
    audio: Option<AudioPlan>,
) -> ExportPlan {
    let end = video
        .as_ref()
        .map(|video| video.frames.end())
        .or_else(|| {
            audio.as_ref().map(|audio| {
                Time((audio.samples.frames as i64 * 1_000_000) / i64::from(audio.samples.rate))
            })
        })
        .unwrap_or(Time::ZERO);
    ExportPlan {
        settings: ExportSettings {
            container,
            video: video.as_ref().map(|video| video.settings),
            audio: audio.as_ref().map(|audio| audio.settings),
            ..ExportSettings::default()
        },
        start: Time::ZERO,
        end,
        video,
        audio,
        cues: None,
    }
}

fn quality(level: QualityLevel) -> RateControl {
    RateControl::Quality { level }
}

const RECOMMENDED: RateControl = RateControl::Quality {
    level: QualityLevel::Recommended,
};

fn strings(args: &[OsString]) -> Vec<String> {
    args.iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

/// Whether `args` holds `run` as consecutive arguments.
fn has(args: &[OsString], run: &[&str]) -> bool {
    strings(args)
        .windows(run.len())
        .any(|window| window.iter().zip(run).all(|(a, b)| a == b))
}

fn value<'a>(args: &'a [String], option: &str) -> Option<&'a str> {
    let index = args.iter().position(|arg| arg == option)?;
    args.get(index + 1).map(String::as_str)
}

fn video(name: &str, rate_control: RateControl, speed: EncodeSpeed) -> Vec<OsString> {
    video_args(
        &video_plan(name, NTSC, SIZE, rate_control, speed),
        Some(Container::Mp4),
    )
}

#[test]
fn every_known_encoder_is_driven_with_bt709_and_a_two_second_gop() {
    for known in VIDEO_ENCODERS {
        let encoder = encoder(known);
        assert!(
            Family::of(&encoder).is_some(),
            "{} has no family",
            known.name
        );
        for rate_control in [RECOMMENDED, RateControl::Bitrate { kbps: 8_000 }] {
            let plan = video_plan(known.name, NTSC, SIZE, rate_control, EncodeSpeed::Balanced);
            let args = strings(&video_args(&plan, Some(Container::Mov)));
            assert_eq!(value(&args, "-c:v"), Some(known.name));
            let chain = value(&args, "-vf").unwrap();
            assert!(
                chain.starts_with("scale=out_color_matrix=bt709:out_range=tv:flags=bicubic+accurate_rnd+full_chroma_int,format="),
                "{chain}"
            );
            assert!(chain.contains(
                ",setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv"
            ));
            // Tags are in the chain; output colour options would be ignored.
            assert!(!args.iter().any(|arg| arg == "-color_primaries"));
            if known.codec == VideoCodec::ProRes {
                assert_eq!(value(&args, "-g"), None, "{}", known.name);
            } else {
                assert_eq!(value(&args, "-g"), Some("60"), "{}", known.name);
            }
        }
    }
}

#[test]
fn x264_and_x265_use_crf_or_a_capped_bitrate_and_presets() {
    let args = video("libx264", RECOMMENDED, EncodeSpeed::Balanced);
    assert!(has(&args, &["-crf", "21", "-preset", "medium"]));
    assert!(
        strings(&args)
            .iter()
            .any(|arg| arg.contains("format=yuv420p,"))
    );
    let args = video(
        "libx264",
        RateControl::Bitrate { kbps: 8_000 },
        EncodeSpeed::Faster,
    );
    assert!(has(
        &args,
        &[
            "-b:v", "8000k", "-maxrate", "12000k", "-bufsize", "16000k", "-preset", "veryfast"
        ]
    ));
    assert!(!has(&args, &["-crf"]));
    let args = video(
        "libx265",
        quality(QualityLevel::Higher),
        EncodeSpeed::Smaller,
    );
    assert!(has(
        &args,
        &[
            "-crf",
            "20",
            "-preset",
            "slow",
            "-x265-params",
            "log-level=error"
        ]
    ));
    assert!(has(&args, &["-tag:v", "hvc1"]));
    // hvc1 is for MP4 and MOV only; the probe has no container.
    let plan = video_plan("libx265", NTSC, SIZE, RECOMMENDED, EncodeSpeed::Balanced);
    assert!(!has(&video_args(&plan, None), &["-tag:v"]));
}

#[test]
fn nvenc_uses_presets_vbr_and_constant_quality_under_a_ceiling() {
    // H.264 at five above x264's factor, with room to peak at four times the
    // suggested rate: without a ceiling NVENC holds the rate to its level's.
    let args = video("h264_nvenc", RECOMMENDED, EncodeSpeed::Balanced);
    let kbps = suggested_kbps(
        VideoCodec::H264,
        SIZE,
        NTSC.fps(),
        QualityLevel::Recommended,
    );
    assert!(has(
        &args,
        &[
            "-preset",
            "p5",
            "-tune",
            "hq",
            "-rc",
            "vbr",
            "-cq",
            "26",
            "-b:v",
            "0",
            "-maxrate",
            &format!("{}k", kbps * 4),
            "-bufsize",
            &format!("{}k", kbps * 8),
        ]
    ));
    // HEVC at three above x265's.
    let args = video(
        "hevc_nvenc",
        quality(QualityLevel::Maximum),
        EncodeSpeed::Balanced,
    );
    assert!(has(&args, &["-cq", "17", "-b:v", "0", "-maxrate"]));
    // AV1 keeps its own scale.
    let args = video(
        "av1_nvenc",
        quality(QualityLevel::Lower),
        EncodeSpeed::Faster,
    );
    assert!(has(
        &args,
        &["-preset", "p3", "-tune", "hq", "-rc", "vbr", "-cq", "40"]
    ));
    let args = video(
        "hevc_nvenc",
        RateControl::Bitrate { kbps: 20_000 },
        EncodeSpeed::Smaller,
    );
    assert!(has(&args, &["-preset", "p7"]));
    assert!(has(
        &args,
        &["-b:v", "20000k", "-maxrate", "30000k", "-bufsize", "40000k"]
    ));
    assert!(!has(&args, &["-cq"]));
}

#[test]
fn a_quality_level_is_moved_onto_each_hardware_familys_scale() {
    use QualityLevel::*;
    use VideoCodec::*;
    let levels = [Lower, Recommended, Higher, Maximum];
    let on = |family, codec| levels.map(|level| hardware_quality(family, codec, level));
    // x264's 26/21/17/12 and x265's 28/24/20/14, moved.
    assert_eq!(on(Family::Nvenc, H264), [31, 26, 22, 17]);
    assert_eq!(on(Family::Nvenc, H265), [31, 27, 23, 17]);
    for family in [Family::Amf, Family::Vaapi] {
        assert_eq!(on(family, H264), [28, 23, 19, 14]);
        assert_eq!(on(family, H265), [29, 25, 21, 15]);
    }
    // Unmeasured: QSV's ICQ, and AV1 and VP9 everywhere.
    assert_eq!(on(Family::Qsv, H264), [26, 21, 17, 12]);
    assert_eq!(on(Family::Qsv, H265), [28, 24, 20, 14]);
    for family in [Family::Nvenc, Family::Amf, Family::Qsv, Family::Vaapi] {
        for codec in [Av1, Vp9] {
            assert_eq!(on(family, codec), levels.map(|level| level.crf(codec)));
        }
    }
    // H.264 and HEVC never leave the 0..51 scale.
    for family in [Family::Nvenc, Family::Amf, Family::Qsv, Family::Vaapi] {
        for codec in [H264, H265] {
            assert!(on(family, codec).iter().all(|value| *value <= 51));
        }
    }
}

#[test]
fn amf_uses_constant_qp_or_peak_constrained_vbr() {
    let args = video("h264_amf", RECOMMENDED, EncodeSpeed::Balanced);
    assert!(has(
        &args,
        &[
            "-quality", "balanced", "-rc", "cqp", "-qp_i", "23", "-qp_p", "23", "-qp_b", "23"
        ]
    ));
    let args = video("hevc_amf", RECOMMENDED, EncodeSpeed::Balanced);
    assert!(has(&args, &["-qp_i", "25", "-qp_p", "25", "-qp_b", "25"]));
    let args = video("av1_amf", RECOMMENDED, EncodeSpeed::Faster);
    assert!(has(
        &args,
        &[
            "-quality", "speed", "-rc", "cqp", "-qp_i", "128", "-qp_p", "128"
        ]
    ));
    assert!(!has(&args, &["-qp_b"]));
    let args = video(
        "hevc_amf",
        RateControl::Bitrate { kbps: 9_000 },
        EncodeSpeed::Smaller,
    );
    assert!(has(
        &args,
        &[
            "-quality", "quality", "-rc", "vbr_peak", "-b:v", "9000k", "-maxrate", "13500k"
        ]
    ));
}

#[test]
fn qsv_takes_nv12_and_global_quality() {
    let args = video("h264_qsv", RECOMMENDED, EncodeSpeed::Smaller);
    assert!(has(
        &args,
        &["-preset", "veryslow", "-global_quality", "21"]
    ));
    assert!(
        strings(&args)
            .iter()
            .any(|arg| arg.contains("format=nv12,"))
    );
    let args = video("vp9_qsv", RECOMMENDED, EncodeSpeed::Balanced);
    assert!(has(&args, &["-preset", "medium", "-global_quality", "128"]));
    let args = video(
        "av1_qsv",
        RateControl::Bitrate { kbps: 6_000 },
        EncodeSpeed::Faster,
    );
    assert!(has(
        &args,
        &["-preset", "veryfast", "-b:v", "6000k", "-maxrate", "9000k"]
    ));
}

#[test]
fn videotoolbox_uses_its_quality_scale_and_no_software_fallback() {
    let args = video(
        "h264_videotoolbox",
        quality(QualityLevel::Maximum),
        EncodeSpeed::Balanced,
    );
    assert!(has(&args, &["-q:v", "90", "-allow_sw", "0"]));
    let args = video(
        "hevc_videotoolbox",
        RateControl::Bitrate { kbps: 7_000 },
        EncodeSpeed::Balanced,
    );
    assert!(has(&args, &["-b:v", "7000k", "-allow_sw", "0"]));
    let args = video(
        "prores_videotoolbox",
        quality(QualityLevel::Higher),
        EncodeSpeed::Balanced,
    );
    assert!(has(&args, &["-profile:v", "hq", "-allow_sw", "0"]));
    assert!(
        strings(&args)
            .iter()
            .any(|arg| arg.contains("format=p210le,"))
    );
}

#[test]
fn vaapi_names_its_device_uploads_and_uses_the_right_quality_option() {
    let mut plan = video_plan("h264_vaapi", NTSC, SIZE, RECOMMENDED, EncodeSpeed::Balanced);
    plan.encoder.device = Some("/dev/dri/renderD129".into());
    assert_eq!(
        strings(&device_args(&plan.encoder)),
        ["-vaapi_device", "/dev/dri/renderD129"]
    );
    let args = video_args(&plan, Some(Container::Mp4));
    assert!(has(&args, &["-rc_mode", "CQP", "-qp", "23"]));
    let chain = strings(&args)
        .into_iter()
        .find(|arg| arg.starts_with("scale="))
        .unwrap();
    assert!(chain.ends_with(",format=nv12,setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv,hwupload"));
    // av1_vaapi and vp9_vaapi have no -qp.
    for name in ["av1_vaapi", "vp9_vaapi"] {
        let args = video(name, RECOMMENDED, EncodeSpeed::Balanced);
        assert!(
            has(&args, &["-rc_mode", "CQP", "-global_quality", "128"]),
            "{name}"
        );
        assert!(!has(&args, &["-qp"]), "{name}");
    }
    let args = video(
        "hevc_vaapi",
        RateControl::Bitrate { kbps: 5_000 },
        EncodeSpeed::Balanced,
    );
    assert!(has(
        &args,
        &["-rc_mode", "VBR", "-b:v", "5000k", "-maxrate", "7500k"]
    ));
    // Families other than VA-API name no device.
    let software = video_plan("libx264", NTSC, SIZE, RECOMMENDED, EncodeSpeed::Balanced);
    assert!(device_args(&software.encoder).is_empty());
}

#[test]
fn media_foundation_holds_a_bitrate_even_in_quality_mode() {
    let args = video("h264_mf", RECOMMENDED, EncodeSpeed::Balanced);
    let kbps = suggested_kbps(
        VideoCodec::H264,
        SIZE,
        NTSC.fps(),
        QualityLevel::Recommended,
    );
    assert!(has(
        &args,
        &[
            "-hw_encoding",
            "1",
            "-rate_control",
            "u_vbr",
            "-b:v",
            &format!("{kbps}k")
        ]
    ));
}

#[test]
fn vp9_svt_and_aom_use_their_own_effort_and_rate_options() {
    let args = video("libvpx-vp9", RECOMMENDED, EncodeSpeed::Balanced);
    assert!(has(
        &args,
        &[
            "-crf",
            "32",
            "-b:v",
            "0",
            "-row-mt",
            "1",
            "-deadline",
            "good",
            "-cpu-used",
            "2"
        ]
    ));
    let args = video(
        "libvpx-vp9",
        RateControl::Bitrate { kbps: 4_000 },
        EncodeSpeed::Faster,
    );
    assert!(has(
        &args,
        &[
            "-b:v",
            "4000k",
            "-row-mt",
            "1",
            "-deadline",
            "good",
            "-cpu-used",
            "4"
        ]
    ));
    assert!(!has(&args, &["-crf"]));
    let args = video("libsvtav1", RECOMMENDED, EncodeSpeed::Smaller);
    assert!(has(&args, &["-crf", "32", "-preset", "5"]));
    let args = video(
        "libsvtav1",
        RateControl::Bitrate { kbps: 3_000 },
        EncodeSpeed::Faster,
    );
    assert!(has(&args, &["-b:v", "3000k", "-preset", "10"]));
    let plan = video_plan("libsvtav1", NTSC, SIZE, RECOMMENDED, EncodeSpeed::Faster);
    assert_eq!(environment(&plan.encoder), Some(("SVT_LOG", "1")));
    assert_eq!(
        environment(&video_plan("libx264", NTSC, SIZE, RECOMMENDED, EncodeSpeed::Faster).encoder),
        None
    );
    let args = video("libaom-av1", RECOMMENDED, EncodeSpeed::Balanced);
    assert!(has(
        &args,
        &["-crf", "32", "-b:v", "0", "-cpu-used", "4", "-row-mt", "1"]
    ));
    let args = video(
        "libaom-av1",
        RateControl::Bitrate { kbps: 2_000 },
        EncodeSpeed::Smaller,
    );
    assert!(has(
        &args,
        &["-b:v", "2000k", "-cpu-used", "3", "-row-mt", "1"]
    ));
}

#[test]
fn prores_is_set_by_its_profile_and_ten_bit_chroma() {
    let args = strings(&video("prores_ks", RECOMMENDED, EncodeSpeed::Balanced));
    assert!(
        args.windows(4)
            .any(|w| w == ["-profile:v", "2", "-vendor", "apl0"])
    );
    assert!(value(&args, "-vf").unwrap().contains("format=yuv422p10le,"));
    let args = strings(&video(
        "prores_ks",
        quality(QualityLevel::Maximum),
        EncodeSpeed::Balanced,
    ));
    assert!(args.windows(2).any(|w| w == ["-profile:v", "4"]));
    assert!(value(&args, "-vf").unwrap().contains("format=yuv444p10le,"));
}

#[test]
fn a_gop_is_two_seconds_of_the_output_rate() {
    let rate = FrameRate {
        numerator: 24_000,
        denominator: 1_001,
    };
    let plan = video_plan("libx264", rate, SIZE, RECOMMENDED, EncodeSpeed::Balanced);
    assert!(has(&video_args(&plan, None), &["-g", "48"]));
}

#[test]
fn an_mp4_maps_the_picture_pipe_and_sound_file_with_exact_timing() {
    let plan = plan(
        Container::Mp4,
        Some(video_plan(
            "libx264",
            NTSC,
            SIZE,
            RECOMMENDED,
            EncodeSpeed::Balanced,
        )),
        Some(audio_plan("aac", AudioCodec::Aac, 256, 48_000, 2)),
    );
    let inputs = EncodeInputs {
        video: Some(VideoInput {
            size: SIZE,
            rate: NTSC,
        }),
        audio: Some(AudioInput {
            path: Path::new("/tmp/job/sound.f32"),
            rate: 48_000,
            channels: 2,
        }),
        subtitles: None,
        title: "Film",
    };
    let args = encode_args(&plan, &inputs, Path::new("/out/.film.mp4.part"));
    let text = strings(&args);
    assert_eq!(
        &text[..5],
        ["-hide_banner", "-nostats", "-loglevel", "error", "-y"]
    );
    // Standard input is the picture pipe.
    assert!(!text.iter().any(|arg| arg == "-nostdin"));
    assert!(has(
        &args,
        &[
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            "1920x1080",
            "-framerate",
            "30000/1001",
            "-i",
            "pipe:0"
        ]
    ));
    assert!(has(
        &args,
        &[
            "-f",
            "f32le",
            "-ar",
            "48000",
            "-ac",
            "2",
            "-i",
            "/tmp/job/sound.f32"
        ]
    ));
    assert!(has(&args, &["-map", "0:v:0", "-map", "1:a:0"]));
    assert!(has(
        &args,
        &["-c:a", "aac", "-b:a", "256k", "-ar", "48000", "-ac", "2"]
    ));
    assert!(has(
        &args,
        &[
            "-f",
            "mp4",
            "-metadata",
            "title=Film",
            "-movflags",
            "+faststart",
            "-movie_timescale",
            "48000"
        ]
    ));
    assert_eq!(text.last().unwrap(), "/out/.film.mp4.part");
    assert!(!text.iter().any(|arg| arg == "-shortest"));
}

#[test]
fn a_silent_video_uses_its_frame_rate_as_the_movie_timescale_and_no_title_is_written_empty() {
    let plan = plan(
        Container::Mov,
        Some(video_plan(
            "libx264",
            NTSC,
            SIZE,
            RECOMMENDED,
            EncodeSpeed::Balanced,
        )),
        None,
    );
    let inputs = EncodeInputs {
        video: Some(VideoInput {
            size: SIZE,
            rate: NTSC,
        }),
        audio: None,
        subtitles: None,
        title: "",
    };
    let args = encode_args(&plan, &inputs, Path::new("out.part"));
    assert!(has(&args, &["-movie_timescale", "30000"]));
    assert!(has(&args, &["-f", "mov"]));
    assert!(!has(&args, &["-metadata"]));
    assert!(!has(&args, &["-c:a"]));
}

#[test]
fn embedded_subtitles_get_their_codec_tag_and_three_letter_language() {
    let mov = plan(
        Container::Mov,
        Some(video_plan(
            "prores_ks",
            NTSC,
            SIZE,
            RECOMMENDED,
            EncodeSpeed::Balanced,
        )),
        Some(audio_plan("pcm_s24le", AudioCodec::Pcm24, 0, 48_000, 2)),
    );
    let inputs = EncodeInputs {
        video: Some(VideoInput {
            size: SIZE,
            rate: NTSC,
        }),
        audio: Some(AudioInput {
            path: Path::new("a.f32"),
            rate: 48_000,
            channels: 2,
        }),
        subtitles: Some(SubtitleInput {
            path: Path::new("c.srt"),
            language: Some("pt-BR"),
        }),
        title: "",
    };
    let args = encode_args(&mov, &inputs, Path::new("out.part"));
    assert!(has(&args, &["-i", "c.srt"]));
    assert!(has(
        &args,
        &["-map", "0:v:0", "-map", "1:a:0", "-map", "2:s:0"]
    ));
    assert!(has(
        &args,
        &[
            "-c:s",
            "mov_text",
            "-tag:s",
            "tx3g",
            "-metadata:s:s:0",
            "language=por"
        ]
    ));
    // PCM has no bitrate.
    assert!(has(&args, &["-c:a", "pcm_s24le", "-ar", "48000"]));
    assert!(!has(&args, &["-b:a"]));

    let webm = plan(
        Container::Webm,
        Some(video_plan(
            "libvpx-vp9",
            NTSC,
            SIZE,
            RECOMMENDED,
            EncodeSpeed::Balanced,
        )),
        Some(audio_plan("libopus", AudioCodec::Opus, 160, 48_000, 2)),
    );
    let inputs = EncodeInputs {
        subtitles: Some(SubtitleInput {
            path: Path::new("c.vtt"),
            language: Some("de-AT"),
        }),
        ..inputs
    };
    let args = encode_args(&webm, &inputs, Path::new("out.part"));
    assert!(has(
        &args,
        &["-c:s", "webvtt", "-metadata:s:s:0", "language=ger"]
    ));
    assert!(!has(&args, &["-tag:s"]));
    assert!(!has(&args, &["-movflags"]));
    assert!(has(&args, &["-c:a", "libopus", "-b:a", "160k"]));
    // A tag without a known code is left off.
    let inputs = EncodeInputs {
        subtitles: Some(SubtitleInput {
            path: Path::new("c.vtt"),
            language: Some("x-klingon"),
        }),
        ..inputs
    };
    assert!(!has(
        &encode_args(&webm, &inputs, Path::new("o")),
        &["-metadata:s:s:0"]
    ));
}

#[test]
fn sound_only_containers_read_no_pipe_and_keep_their_own_rules() {
    let input = AudioInput {
        path: Path::new("a.f32"),
        rate: 44_100,
        channels: 2,
    };
    let inputs = EncodeInputs {
        video: None,
        audio: Some(input),
        subtitles: None,
        title: "Song",
    };
    let m4a = plan(
        Container::M4a,
        None,
        Some(audio_plan("aac", AudioCodec::Aac, 256, 44_100, 2)),
    );
    let args = encode_args(&m4a, &inputs, Path::new("o"));
    assert!(has(&args, &["-nostdin"]));
    assert!(has(&args, &["-map", "0:a:0", "-c:a", "aac"]));
    assert!(has(
        &args,
        &[
            "-f",
            "ipod",
            "-metadata",
            "title=Song",
            "-movflags",
            "+faststart",
            "-movie_timescale",
            "44100"
        ]
    ));
    assert!(!has(&args, &["pipe:0"]));

    let wav = plan(
        Container::Wav,
        None,
        Some(audio_plan("pcm_s16le", AudioCodec::Pcm16, 0, 44_100, 2)),
    );
    let args = encode_args(&wav, &inputs, Path::new("o"));
    assert!(has(&args, &["-f", "wav"]));
    assert!(has(&args, &["-rf64", "auto"]));

    let flac = plan(
        Container::Flac,
        None,
        Some(audio_plan("flac", AudioCodec::Flac, 0, 44_100, 2)),
    );
    assert!(has(
        &encode_args(&flac, &inputs, Path::new("o")),
        &[
            "-c:a",
            "flac",
            "-sample_fmt",
            "s16",
            "-ar",
            "44100",
            "-ac",
            "2",
            "-f",
            "flac"
        ]
    ));

    let mp3 = plan(
        Container::Mp3,
        None,
        Some(audio_plan("libmp3lame", AudioCodec::Mp3, 320, 44_100, 2)),
    );
    let args = encode_args(&mp3, &inputs, Path::new("o"));
    assert!(has(
        &args,
        &[
            "-c:a",
            "libmp3lame",
            "-b:a",
            "320k",
            "-ar",
            "44100",
            "-ac",
            "2",
            "-f",
            "mp3"
        ]
    ));
    assert!(!has(&args, &["-movflags"]));
}

#[test]
fn audio_arguments_name_encoder_rate_and_layout() {
    let args = strings(&audio_args(&audio_plan(
        "libopus",
        AudioCodec::Opus,
        96,
        48_000,
        1,
    )));
    assert_eq!(
        args,
        ["-c:a", "libopus", "-b:a", "96k", "-ar", "48000", "-ac", "1"]
    );
}

#[test]
fn bcp47_tags_map_to_iso_639_2() {
    assert_eq!(language_code("en", false).as_deref(), Some("eng"));
    assert_eq!(language_code("en-US", false).as_deref(), Some("eng"));
    assert_eq!(language_code("fr_CA", false).as_deref(), Some("fra"));
    assert_eq!(language_code("fr", true).as_deref(), Some("fre"));
    assert_eq!(language_code("zh-Hans-CN", false).as_deref(), Some("zho"));
    assert_eq!(language_code("zh", true).as_deref(), Some("chi"));
    assert_eq!(language_code("ES", false).as_deref(), Some("spa"));
    assert_eq!(language_code("ja", true).as_deref(), Some("jpn"));
    assert_eq!(language_code("yue", false).as_deref(), Some("yue"));
    assert_eq!(language_code("", false), None);
    assert_eq!(language_code("qq", false), None);
    assert_eq!(language_code("e1", false), None);
}

/// How many thread options `args` has, for any stream.
fn threads(args: &[String]) -> usize {
    args.iter()
        .filter(|arg| *arg == "-threads" || arg.starts_with("-threads:"))
        .count()
}

const SPEEDS: [EncodeSpeed; 3] = [
    EncodeSpeed::Faster,
    EncodeSpeed::Balanced,
    EncodeSpeed::Smaller,
];

#[test]
fn x264_takes_its_own_thread_count_up_to_a_cap() {
    // x264 alone takes one and a half threads a logical processor, each
    // frame thread holding pictures: 48 here, and a 4K encoder of 5.4 GiB.
    assert_eq!(x264_threads(1), 1);
    assert_eq!(x264_threads(2), 3);
    assert_eq!(x264_threads(8), 12);
    assert_eq!(x264_threads(16), 24);
    assert_eq!(x264_threads(32), 24);
    assert_eq!(x264_threads(256), 24);
    let host = std::thread::available_parallelism().map_or(4, |count| count.get());
    let expected = x264_threads(host).to_string();
    for speed in SPEEDS {
        for rate_control in [RECOMMENDED, RateControl::Bitrate { kbps: 8_000 }] {
            let args = strings(&video("libx264", rate_control, speed));
            assert_eq!(value(&args, "-threads:v"), Some(expected.as_str()));
            assert_eq!(threads(&args), 1);
        }
    }
    // The probe drives the same arguments.
    let plan = video_plan("libx264", NTSC, SIZE, RECOMMENDED, EncodeSpeed::Balanced);
    assert!(has(&video_args(&plan, None), &["-threads:v", &expected]));
}

#[test]
fn svt_av1_runs_at_a_bounded_level_of_parallelism() {
    // SVT-AV1 ignores -threads; its automatic level (6) held 7.2 GiB at
    // 4K Balanced, level 4 held 4.0 GiB and encoded no slower.
    for speed in SPEEDS {
        let args = video("libsvtav1", RECOMMENDED, speed);
        assert!(has(&args, &["-svtav1-params", "lp=4"]));
        assert_eq!(threads(&strings(&args)), 0);
    }
}

#[test]
fn encoders_that_bound_their_own_threads_are_left_alone() {
    // libx265 maps -threads onto frame threads, past its own bounded
    // choice; libvpx-vp9 and ProRes stop at 16 on their own; libaom's
    // memory did not follow its thread count. Hardware encoders run on
    // the device.
    for known in VIDEO_ENCODERS {
        if matches!(known.name, "libx264") {
            continue;
        }
        for speed in SPEEDS {
            let args = strings(&video(known.name, RECOMMENDED, speed));
            assert_eq!(threads(&args), 0, "{} has a thread count", known.name);
        }
    }
}
