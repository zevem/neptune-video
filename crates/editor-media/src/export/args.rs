//! FFmpeg arguments for an export, built from a plan alone. The capability
//! probe encodes its test picture through the same video and audio pieces,
//! so an encoder offered is one these arguments have driven on this host.
use super::EncodeInputs;
use editor_core::export::{
    AudioPlan, Container, EncodeSpeed, ExportPlan, HardwareKind, QualityLevel, RateControl,
    VideoCodec, VideoEncoder, VideoPlan, suggested_kbps,
};
use std::{ffi::OsString, num::NonZeroUsize, path::Path};

/// RGB to limited-range BT.709 Y'CbCr with the export's scaler.
const SCALE: &str =
    "scale=out_color_matrix=bt709:out_range=tv:flags=bicubic+accurate_rnd+full_chroma_int";
/// Encoders take colour tags from the frames they are given; a raw RGBA pipe
/// has none, so they are set in the chain after the conversion.
const TAGS: &str = "setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv";
/// The VA-API node used when the probe recorded none.
const DEFAULT_VAAPI_DEVICE: &str = "/dev/dri/renderD128";

/// How an encoder is driven. Every entry of `VIDEO_ENCODERS` has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Family {
    X264,
    X265,
    SvtAv1,
    Aom,
    Vpx,
    ProRes,
    Nvenc,
    Amf,
    Qsv,
    VideoToolbox,
    Vaapi,
    MediaFoundation,
}

impl Family {
    pub(crate) fn of(encoder: &VideoEncoder) -> Option<Self> {
        Some(match encoder.hardware {
            Some(HardwareKind::Nvenc) => Self::Nvenc,
            Some(HardwareKind::Amf) => Self::Amf,
            Some(HardwareKind::Qsv) => Self::Qsv,
            Some(HardwareKind::VideoToolbox) => Self::VideoToolbox,
            Some(HardwareKind::Vaapi) => Self::Vaapi,
            Some(HardwareKind::MediaFoundation) => Self::MediaFoundation,
            None => match encoder.name.as_str() {
                "libx264" => Self::X264,
                "libx265" => Self::X265,
                "libsvtav1" => Self::SvtAv1,
                "libaom-av1" => Self::Aom,
                "libvpx-vp9" => Self::Vpx,
                "prores_ks" => Self::ProRes,
                _ => return None,
            },
        })
    }
}

/// An environment variable the encoder's process needs: SVT-AV1 prints its
/// banner and settings to the error output unless told not to.
pub(crate) fn environment(encoder: &VideoEncoder) -> Option<(&'static str, &'static str)> {
    (Family::of(encoder) == Some(Family::SvtAv1)).then_some(("SVT_LOG", "1"))
}

/// The whole command line after the program name. Pure: the inputs a plan
/// has but `inputs` lacks (or the reverse) are left out, and `Encoder::start`
/// refuses such a pair before running anything.
pub(crate) fn encode_args(
    plan: &ExportPlan,
    inputs: &EncodeInputs,
    output: &Path,
) -> Vec<OsString> {
    let container = plan.settings.container;
    let mut args = Args::default();
    args.push_all(prefix());
    if inputs.video.is_none() {
        // No picture pipe: standard input is nothing FFmpeg should read.
        args.push("-nostdin");
    }
    if let Some(video) = &plan.video {
        args.push_all(device_args(&video.encoder));
    }
    let mut next = 0;
    let mut maps = Vec::new();
    if let Some(input) = &inputs.video {
        args.push_all(["-f", "rawvideo", "-pix_fmt", "rgba", "-s"]);
        args.push(format!("{}x{}", input.size[0], input.size[1]));
        args.push("-framerate");
        args.push(format!(
            "{}/{}",
            input.rate.numerator, input.rate.denominator
        ));
        args.push_all(["-i", "pipe:0"]);
        maps.push(format!("{next}:v:0"));
        next += 1;
    }
    if let Some(input) = &inputs.audio {
        args.push_all(["-f", "f32le", "-ar"]);
        args.push(input.rate.to_string());
        args.push("-ac");
        args.push(input.channels.to_string());
        args.push("-i");
        args.push(input.path);
        maps.push(format!("{next}:a:0"));
        next += 1;
    }
    let subtitles = inputs
        .subtitles
        .filter(|_| container.embedded_subtitles().is_some());
    if let Some(input) = &subtitles {
        args.push("-i");
        args.push(input.path);
        maps.push(format!("{next}:s:0"));
    }
    for map in maps {
        args.push("-map");
        args.push(map);
    }
    if let (Some(video), Some(_)) = (&plan.video, &inputs.video) {
        args.push_all(video_args(video, Some(container)));
    }
    if let (Some(audio), Some(_)) = (&plan.audio, &inputs.audio) {
        args.push_all(audio_args(audio));
    }
    if let (Some(input), Some(codec)) = (&subtitles, container.embedded_subtitles()) {
        args.push_all(["-c:s", codec]);
        if container == Container::Mov {
            // QuickTime's own `text` fourcc otherwise; players expect tx3g.
            args.push_all(["-tag:s", "tx3g"]);
        }
        if let Some(language) = input
            .language
            .and_then(|tag| language_code(tag, container == Container::Webm))
        {
            args.push("-metadata:s:s:0");
            args.push(format!("language={language}"));
        }
    }
    args.push_all(container_args(plan, inputs.title));
    args.push(output);
    args.0
}

/// Quiet, and overwriting the part file the job owns. Standard input is the
/// picture pipe, so `-nostdin` is never given.
pub(crate) fn prefix() -> [&'static str; 5] {
    ["-hide_banner", "-nostats", "-loglevel", "error", "-y"]
}

/// Options that must precede the inputs: the VA-API device.
pub(crate) fn device_args(encoder: &VideoEncoder) -> Vec<OsString> {
    if Family::of(encoder) == Some(Family::Vaapi) {
        let device = encoder.device.as_deref().unwrap_or(DEFAULT_VAAPI_DEVICE);
        vec!["-vaapi_device".into(), device.into()]
    } else {
        Vec::new()
    }
}

/// The video stream's options: codec, conversion and tags, rate control,
/// effort and keyframe interval. `container` is `None` for the probe.
pub(crate) fn video_args(video: &VideoPlan, container: Option<Container>) -> Vec<OsString> {
    let encoder = &video.encoder;
    let family = Family::of(encoder);
    let codec = encoder.codec;
    let settings = &video.settings;
    let fps = video.frames.rate.fps();
    let mut args = Args::default();
    args.push("-c:v");
    args.push(encoder.name.as_str());
    args.push("-vf");
    args.push(filter(family, codec, settings.rate_control));

    let quality = match settings.rate_control {
        RateControl::Quality { level } => Some(level),
        RateControl::Bitrate { .. } => None,
    };
    let kbps = match settings.rate_control {
        RateControl::Bitrate { kbps } => kbps,
        RateControl::Quality { level } => suggested_kbps(codec, video.geometry.output, fps, level),
    };
    let crf = |level: QualityLevel| level.crf(codec).to_string();
    // The same level on a hardware family's own scale.
    let hardware = |level: QualityLevel| match family {
        Some(family) => hardware_quality(family, codec, level).to_string(),
        None => crf(level),
    };
    // AV1 and VP9 hardware quality scales run 0..255; the 0..63 factor is
    // stretched onto them.
    let wide = |level: QualityLevel| (u32::from(level.crf(codec)) * 4).clamp(1, 255).to_string();
    let speed = settings.speed;
    let pick = |faster: &'static str, balanced: &'static str, smaller: &'static str| match speed {
        EncodeSpeed::Faster => faster,
        EncodeSpeed::Balanced => balanced,
        EncodeSpeed::Smaller => smaller,
    };
    let peak = |args: &mut Args| {
        args.push("-b:v");
        args.push(format!("{kbps}k"));
        args.push("-maxrate");
        args.push(format!("{}k", kbps.saturating_mul(3) / 2));
        args.push("-bufsize");
        args.push(format!("{}k", kbps.saturating_mul(2)));
    };
    let bitrate = |args: &mut Args| {
        args.push("-b:v");
        args.push(format!("{kbps}k"));
    };

    match family {
        Some(Family::X264 | Family::X265) => {
            match quality {
                Some(level) => args.push_all(["-crf".into(), crf(level)]),
                None => peak(&mut args),
            }
            args.push_all(["-preset", pick("veryfast", "medium", "slow")]);
            if family == Some(Family::X265) {
                // No -threads: libx265 takes it as frame threads, above the
                // at most six it picks itself, and its memory grew with them.
                args.push_all(["-x265-params", "log-level=error"]);
            } else {
                args.push("-threads:v");
                args.push(x264_threads(parallelism()).to_string());
            }
        }
        Some(Family::Nvenc) => {
            args.push_all([
                "-preset",
                pick("p3", "p5", "p7"),
                "-tune",
                "hq",
                "-rc",
                "vbr",
            ]);
            match quality {
                Some(level) => {
                    args.push_all(["-cq".into(), hardware(level), "-b:v".into(), "0".into()]);
                    // Without a ceiling NVENC holds the bitrate to what the
                    // level it signals for the picture size allows (about
                    // 16 Mbit/s for H.264 and 10 for HEVC at 1080p), which
                    // caps Higher and Maximum. The measured fixtures
                    // averaged at most 1.3 times the suggested rate; four
                    // times it leaves room for peaks and keeps a level
                    // players decode (5.0 for H.264 1080p30 Maximum, where
                    // the largest ceiling FFmpeg takes signals 6.2).
                    let ceiling = kbps.saturating_mul(NVENC_CEILING);
                    args.push("-maxrate");
                    args.push(format!("{ceiling}k"));
                    args.push("-bufsize");
                    args.push(format!("{}k", ceiling.saturating_mul(2)));
                }
                None => peak(&mut args),
            }
        }
        Some(Family::Amf) => {
            args.push_all(["-quality", pick("speed", "balanced", "quality")]);
            match quality {
                Some(level) => {
                    let qp = if codec == VideoCodec::Av1 {
                        wide(level)
                    } else {
                        hardware(level)
                    };
                    args.push_all(["-rc", "cqp", "-qp_i"]);
                    args.push(qp.as_str());
                    args.push("-qp_p");
                    args.push(qp.as_str());
                    if codec != VideoCodec::Av1 {
                        args.push("-qp_b");
                        args.push(qp.as_str());
                    }
                }
                None => {
                    args.push_all(["-rc", "vbr_peak"]);
                    bitrate(&mut args);
                    args.push("-maxrate");
                    args.push(format!("{}k", kbps.saturating_mul(3) / 2));
                }
            }
        }
        Some(Family::Qsv) => {
            args.push_all(["-preset", pick("veryfast", "medium", "veryslow")]);
            match quality {
                Some(level) => {
                    let value = match codec {
                        VideoCodec::Av1 | VideoCodec::Vp9 => wide(level),
                        _ => crf(level),
                    };
                    args.push_all(["-global_quality".into(), value]);
                }
                None => {
                    bitrate(&mut args);
                    args.push("-maxrate");
                    args.push(format!("{}k", kbps.saturating_mul(3) / 2));
                }
            }
        }
        Some(Family::VideoToolbox) => {
            if codec == VideoCodec::ProRes {
                let profile = quality.unwrap_or_default().prores_profile();
                args.push_all(["-profile:v", videotoolbox_prores(profile)]);
            } else {
                match quality {
                    Some(level) => args.push_all(["-q:v", videotoolbox_quality(level)]),
                    None => bitrate(&mut args),
                }
            }
            args.push_all(["-allow_sw", "0"]);
        }
        Some(Family::Vaapi) => match quality {
            Some(level) => {
                args.push_all(["-rc_mode", "CQP"]);
                // av1_vaapi and vp9_vaapi have no -qp; it would be ignored.
                match codec {
                    VideoCodec::Av1 | VideoCodec::Vp9 => {
                        args.push_all(["-global_quality".into(), wide(level)])
                    }
                    _ => args.push_all(["-qp".into(), hardware(level)]),
                }
            }
            None => {
                args.push_all(["-rc_mode", "VBR"]);
                bitrate(&mut args);
                args.push("-maxrate");
                args.push(format!("{}k", kbps.saturating_mul(3) / 2));
            }
        },
        Some(Family::MediaFoundation) => {
            // Quality mode holds the suggested bitrate: MF has no portable
            // constant-quality control.
            args.push_all(["-hw_encoding", "1", "-rate_control", "u_vbr"]);
            bitrate(&mut args);
        }
        Some(Family::Vpx) => {
            match quality {
                Some(level) => {
                    args.push_all(["-crf".into(), crf(level), "-b:v".into(), "0".into()])
                }
                None => bitrate(&mut args),
            }
            args.push_all(["-row-mt", "1", "-deadline", "good", "-cpu-used"]);
            args.push(pick("4", "2", "1"));
        }
        Some(Family::SvtAv1) => {
            match quality {
                Some(level) => args.push_all(["-crf".into(), crf(level)]),
                None => bitrate(&mut args),
            }
            args.push_all(["-preset", pick("10", "8", "5")]);
            // SVT-AV1 ignores -threads; see `SVT_AV1_PARALLELISM`.
            args.push_all(["-svtav1-params", SVT_AV1_PARALLELISM]);
        }
        Some(Family::Aom) => {
            match quality {
                Some(level) => {
                    args.push_all(["-crf".into(), crf(level), "-b:v".into(), "0".into()])
                }
                None => bitrate(&mut args),
            }
            args.push_all(["-cpu-used", pick("6", "4", "3"), "-row-mt", "1"]);
        }
        Some(Family::ProRes) => {
            // `plan` refuses a bitrate for ProRes; its profile sets quality.
            let profile = quality.unwrap_or_default().prores_profile();
            args.push("-profile:v");
            args.push(profile.to_string());
            args.push_all(["-vendor", "apl0"]);
        }
        // An encoder the export does not know: its own defaults at the
        // suggested bitrate.
        None => bitrate(&mut args),
    }

    if codec != VideoCodec::ProRes {
        args.push("-g");
        args.push(((2.0 * fps).round() as u32).max(1).to_string());
    }
    if codec == VideoCodec::H265 && matches!(container, Some(Container::Mp4 | Container::Mov)) {
        // Apple players refuse HEVC tagged hev1.
        args.push_all(["-tag:v", "hvc1"]);
    }
    args.0
}

/// The most threads libx264 is given. On its own it takes one and a half a
/// logical processor, every frame thread holding pictures of its own, so
/// its memory grew with the host: 48 threads held 5.3-6.2 GiB at 4K here.
///
/// Measured with FFmpeg 8.0.1 on an i9-13900HX (32 logical processors) at
/// the medium preset and CRF 21, RGBA piped in. 24 threads held 3.6-4.4 GiB
/// at 4K and 1.0-1.1 GiB at 1080p, against 5.3-6.2 and 1.2-1.3. Behind the
/// source reader the median rate was 69.6 fps at 1080p and 17.7 at 4K,
/// against 72.4 and 17.8; the encoder alone ran 3 % slower at 1080p and
/// 15 % at 4K. 16 threads held about 0.4 GiB less at 4K and lost about 16 % of
/// the encoder's own 1080p speed. One thread still held 2.3 GiB at 4K: the
/// 40-frame lookahead and FFmpeg's queues of 4K pictures do not follow it.
const X264_MAX_THREADS: usize = 24;

/// x264's own thread count for `parallelism` logical processors, capped.
pub(crate) fn x264_threads(parallelism: usize) -> usize {
    (parallelism.saturating_mul(3) / 2).clamp(1, X264_MAX_THREADS)
}

/// This host's logical processors, or four when it cannot say.
fn parallelism() -> usize {
    std::thread::available_parallelism().map_or(4, NonZeroUsize::get)
}

/// SVT-AV1's level of parallelism, 1 to 6: more threads and more frames in
/// flight. It chose 6 on this host whatever processors it was allowed, and
/// held 6.8-7.9 GiB at 4K and 2.3-2.4 GiB at 1080p at preset 8; level 4
/// held 3.9-4.5 and 1.4-1.5 GiB and encoded within 1 % or faster at presets
/// 8 and 10, and 9 % slower at preset 5 in 1080p. Level 3 lost 13-18 % at
/// 1080p. SVT-AV1 before 2.0 read `lp` as a processor count (not measured).
const SVT_AV1_PARALLELISM: &str = "lp=4";

/// How many times the suggested bitrate NVENC may peak at in quality mode.
const NVENC_CEILING: u32 = 4;

/// The quality value `family` is given for `level` on its own 0..51 scale:
/// the x264/x265 factor plus what makes the family's output match theirs.
///
/// Measured with FFmpeg 8.0.1 on an RTX 4060 (driver 595.99) against
/// libx264/libx265 at the medium preset, on testsrc2 with and without
/// temporal noise at 1280x720 and 1920x1080, 30 fps. NVENC's VBR `-cq`
/// matched x264's SSIM and PSNR at about `crf + 5` for H.264 and x265's at
/// about `crf + 3` for HEVC, with files of about the same size; its
/// constant QP matched at about `crf + 2` and `crf + 1`. AMF and VA-API run
/// constant QP and are given those offsets; they were not measured on their
/// own hardware. QSV's `-global_quality` is ICQ, scaled like CRF by design,
/// and the AV1 and VP9 scales are unchanged: neither was measured.
pub(crate) fn hardware_quality(family: Family, codec: VideoCodec, level: QualityLevel) -> u8 {
    let offset = match (family, codec) {
        (Family::Nvenc, VideoCodec::H264) => 5,
        (Family::Nvenc, VideoCodec::H265) => 3,
        (Family::Amf | Family::Vaapi, VideoCodec::H264) => 2,
        (Family::Amf | Family::Vaapi, VideoCodec::H265) => 1,
        _ => 0,
    };
    (level.crf(codec) + offset).min(51)
}

/// The conversion to the encoder's input format, with BT.709 tags.
fn filter(family: Option<Family>, codec: VideoCodec, rate_control: RateControl) -> String {
    let pixels = match family {
        Some(Family::Qsv | Family::Vaapi) => "nv12",
        // 10-bit 4:2:2, semi-planar: what VideoToolbox's ProRes takes.
        Some(Family::VideoToolbox) if codec == VideoCodec::ProRes => "p210le",
        Some(Family::ProRes) => match rate_control {
            RateControl::Quality {
                level: QualityLevel::Maximum,
            } => "yuv444p10le",
            _ => "yuv422p10le",
        },
        _ => "yuv420p",
    };
    let mut chain = format!("{SCALE},format={pixels},{TAGS}");
    if family == Some(Family::Vaapi) {
        chain.push_str(",hwupload");
    }
    chain
}

/// The sound stream's options.
pub(crate) fn audio_args(audio: &AudioPlan) -> Vec<OsString> {
    let settings = &audio.settings;
    let mut args = Args::default();
    args.push("-c:a");
    args.push(audio.encoder.name.as_str());
    if settings.codec.has_bitrate() {
        args.push("-b:a");
        args.push(format!("{}k", settings.kbps));
    }
    if audio.encoder.name == "flac" {
        // 16-bit, as estimated; FFmpeg would keep 24 from float input.
        args.push_all(["-sample_fmt", "s16"]);
    }
    args.push("-ar");
    args.push(settings.sample_rate.to_string());
    args.push("-ac");
    args.push(settings.channels.to_string());
    args.0
}

/// Muxer, title and the container's own options.
fn container_args(plan: &ExportPlan, title: &str) -> Vec<OsString> {
    let container = plan.settings.container;
    let mut args = Args::default();
    args.push_all(["-f", container.muxer()]);
    if !title.is_empty() {
        args.push("-metadata");
        args.push(format!("title={title}"));
    }
    match container {
        Container::Mp4 | Container::Mov | Container::M4a => {
            args.push_all(["-movflags", "+faststart", "-movie_timescale"]);
            // The default of 1000 cuts AAC's edit list to whole milliseconds.
            let timescale = match (&plan.audio, &plan.video) {
                (Some(audio), _) => audio.settings.sample_rate,
                (None, Some(video)) => video.frames.rate.numerator,
                (None, None) => 1000,
            };
            args.push(timescale.to_string());
        }
        // Past 4 GiB a RIFF header cannot hold the size.
        Container::Wav => args.push_all(["-rf64", "auto"]),
        Container::Webm | Container::Mp3 | Container::Flac => {}
    }
    args.0
}

fn videotoolbox_quality(level: QualityLevel) -> &'static str {
    match level {
        QualityLevel::Lower => "45",
        QualityLevel::Recommended => "60",
        QualityLevel::Higher => "75",
        QualityLevel::Maximum => "90",
    }
}

fn videotoolbox_prores(profile: u8) -> &'static str {
    match profile {
        0 => "proxy",
        1 => "lt",
        2 => "standard",
        3 => "hq",
        _ => "4444",
    }
}

/// The three-letter ISO 639-2 code containers carry for a BCP 47 tag:
/// terminological codes for MP4 and MOV, bibliographic ones for Matroska
/// and WebM. A tag whose primary subtag is already three letters is kept.
pub(crate) fn language_code(tag: &str, bibliographic: bool) -> Option<String> {
    let primary = tag
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if primary.len() == 3 && primary.bytes().all(|b| b.is_ascii_lowercase()) {
        return Some(primary);
    }
    let (terminological, bibliographic_code) = match primary.as_str() {
        "af" => ("afr", None),
        "am" => ("amh", None),
        "ar" => ("ara", None),
        "az" => ("aze", None),
        "be" => ("bel", None),
        "bg" => ("bul", None),
        "bn" => ("ben", None),
        "bs" => ("bos", None),
        "ca" => ("cat", None),
        "cs" => ("ces", Some("cze")),
        "cy" => ("cym", Some("wel")),
        "da" => ("dan", None),
        "de" => ("deu", Some("ger")),
        "el" => ("ell", Some("gre")),
        "en" => ("eng", None),
        "eo" => ("epo", None),
        "es" => ("spa", None),
        "et" => ("est", None),
        "eu" => ("eus", Some("baq")),
        "fa" => ("fas", Some("per")),
        "fi" => ("fin", None),
        "fil" => ("fil", None),
        "fr" => ("fra", Some("fre")),
        "ga" => ("gle", None),
        "gl" => ("glg", None),
        "gu" => ("guj", None),
        "he" | "iw" => ("heb", None),
        "hi" => ("hin", None),
        "hr" => ("hrv", None),
        "hu" => ("hun", None),
        "hy" => ("hye", Some("arm")),
        "id" | "in" => ("ind", None),
        "is" => ("isl", Some("ice")),
        "it" => ("ita", None),
        "ja" => ("jpn", None),
        "ka" => ("kat", Some("geo")),
        "kk" => ("kaz", None),
        "km" => ("khm", None),
        "kn" => ("kan", None),
        "ko" => ("kor", None),
        "lt" => ("lit", None),
        "lv" => ("lav", None),
        "mk" => ("mkd", Some("mac")),
        "ml" => ("mal", None),
        "mn" => ("mon", None),
        "mr" => ("mar", None),
        "ms" => ("msa", Some("may")),
        "my" => ("mya", Some("bur")),
        "nb" => ("nob", None),
        "ne" => ("nep", None),
        "nl" => ("nld", Some("dut")),
        "nn" => ("nno", None),
        "no" => ("nor", None),
        "pa" => ("pan", None),
        "pl" => ("pol", None),
        "pt" => ("por", None),
        "ro" => ("ron", Some("rum")),
        "ru" => ("rus", None),
        "si" => ("sin", None),
        "sk" => ("slk", Some("slo")),
        "sl" => ("slv", None),
        "sq" => ("sqi", Some("alb")),
        "sr" => ("srp", None),
        "sv" => ("swe", None),
        "sw" => ("swa", None),
        "ta" => ("tam", None),
        "te" => ("tel", None),
        "th" => ("tha", None),
        "tl" => ("tgl", None),
        "tr" => ("tur", None),
        "uk" => ("ukr", None),
        "ur" => ("urd", None),
        "uz" => ("uzb", None),
        "vi" => ("vie", None),
        "zh" => ("zho", Some("chi")),
        "zu" => ("zul", None),
        _ => return None,
    };
    let code = if bibliographic {
        bibliographic_code.unwrap_or(terminological)
    } else {
        terminological
    };
    Some(code.into())
}

/// An argument list that takes strings and paths alike.
#[derive(Default)]
struct Args(Vec<OsString>);

impl Args {
    fn push(&mut self, arg: impl Into<OsString>) {
        self.0.push(arg.into());
    }

    fn push_all<T: Into<OsString>>(&mut self, args: impl IntoIterator<Item = T>) {
        self.0.extend(args.into_iter().map(Into::into));
    }
}
