//! What an export makes, as data: containers, codecs, quality, ranges,
//! presets and the exact frames and samples a render covers.
//!
//! Pure like the rest of the core. Probing encoders, decoding, composing and
//! writing files belong to `editor-media` and the desktop; they describe what
//! they found with [`Capabilities`] and report jobs with [`RenderJobInfo`].
//! An export is never an edit: nothing here changes a project.
use crate::{
    FrameRate, Project, Time, Track, TrackId, TrackKind, WordTime,
    captions::{self, Cue},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
use uuid::Uuid;

/// The longest side an exported picture may have: the compositor's texture
/// limit on every supported device.
pub const MAX_OUTPUT_SIDE: u32 = 8192;
/// The shortest side an exported picture may have.
pub const MIN_OUTPUT_SIDE: u32 = 16;
/// The most pixels an asked-for picture may have, as for a project canvas.
pub const MAX_OUTPUT_PIXELS: u64 = 134_217_728;
/// Lowest and highest video bitrates a person may ask for, in kbit/s.
pub const MIN_VIDEO_KBPS: u32 = 100;
pub const MAX_VIDEO_KBPS: u32 = 800_000;
/// Lowest and highest audio bitrates, in kbit/s.
pub const MIN_AUDIO_KBPS: u32 = 32;
pub const MAX_AUDIO_KBPS: u32 = 512;
/// Output sample rates offered.
pub const SAMPLE_RATES: [u32; 3] = [44_100, 48_000, 96_000];

// ---------------------------------------------------------------------------
// Formats
// ---------------------------------------------------------------------------

/// The file an export writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Container {
    Mp4,
    Mov,
    Webm,
    /// Audio only: AAC in an MPEG-4 audio file.
    M4a,
    Mp3,
    Wav,
    Flac,
}

impl Container {
    pub const ALL: [Self; 7] = [
        Self::Mp4,
        Self::Mov,
        Self::Webm,
        Self::M4a,
        Self::Mp3,
        Self::Wav,
        Self::Flac,
    ];
    pub const VIDEO: [Self; 3] = [Self::Mp4, Self::Mov, Self::Webm];
    pub const AUDIO: [Self; 4] = [Self::M4a, Self::Mp3, Self::Wav, Self::Flac];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mov => "mov",
            Self::Webm => "webm",
            Self::M4a => "m4a",
            Self::Mp3 => "mp3",
            Self::Wav => "wav",
            Self::Flac => "flac",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Mp4 => "MP4",
            Self::Mov => "MOV",
            Self::Webm => "WebM",
            Self::M4a => "M4A",
            Self::Mp3 => "MP3",
            Self::Wav => "WAV",
            Self::Flac => "FLAC",
        }
    }

    /// The container an extension names.
    pub fn from_extension(extension: &str) -> Option<Self> {
        let extension = extension.to_ascii_lowercase();
        Self::ALL.into_iter().find(|c| c.extension() == extension)
    }

    /// FFmpeg's muxer for this container.
    pub fn muxer(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mov => "mov",
            Self::Webm => "webm",
            Self::M4a => "ipod",
            Self::Mp3 => "mp3",
            Self::Wav => "wav",
            Self::Flac => "flac",
        }
    }

    pub fn has_video(self) -> bool {
        Self::VIDEO.contains(&self)
    }

    /// Video codecs this container carries, most usual first.
    pub fn video_codecs(self) -> &'static [VideoCodec] {
        match self {
            Self::Mp4 => &[VideoCodec::H264, VideoCodec::H265, VideoCodec::Av1],
            Self::Mov => &[VideoCodec::H264, VideoCodec::H265, VideoCodec::ProRes],
            Self::Webm => &[VideoCodec::Vp9, VideoCodec::Av1],
            _ => &[],
        }
    }

    /// Audio codecs this container carries, most usual first.
    pub fn audio_codecs(self) -> &'static [AudioCodec] {
        match self {
            Self::Mp4 => &[AudioCodec::Aac],
            Self::Mov => &[AudioCodec::Aac, AudioCodec::Pcm16, AudioCodec::Pcm24],
            Self::Webm => &[AudioCodec::Opus],
            Self::M4a => &[AudioCodec::Aac],
            Self::Mp3 => &[AudioCodec::Mp3],
            Self::Wav => &[AudioCodec::Pcm16, AudioCodec::Pcm24],
            Self::Flac => &[AudioCodec::Flac],
        }
    }

    /// The subtitle stream a container can carry inside the file.
    pub fn embedded_subtitles(self) -> Option<&'static str> {
        match self {
            Self::Mp4 | Self::Mov => Some("mov_text"),
            Self::Webm => Some("webvtt"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VideoCodec {
    #[default]
    H264,
    H265,
    Av1,
    Vp9,
    ProRes,
}

impl VideoCodec {
    pub const ALL: [Self; 5] = [Self::H264, Self::H265, Self::Av1, Self::Vp9, Self::ProRes];

    pub fn label(self) -> &'static str {
        match self {
            Self::H264 => "H.264",
            Self::H265 => "H.265 (HEVC)",
            Self::Av1 => "AV1",
            Self::Vp9 => "VP9",
            Self::ProRes => "Apple ProRes",
        }
    }

    /// Bits per pixel and frame at the recommended level, used to suggest a
    /// bitrate and estimate a file's size. ProRes is near-constant by
    /// profile; the figure is for 422.
    ///
    /// H.264 and H.265 are what libx264 at CRF 21 and libx265 at CRF 24
    /// (medium) wrote, and NVENC at its matched quality, on testsrc2 with and
    /// without temporal noise at 1280x720 and 1920x1080, 30 fps: 0.125 to
    /// 0.166 and 0.098 to 0.122. Detailed or noisy footage comes out larger
    /// and plain footage smaller. AV1 and VP9 were not measured.
    fn bits_per_pixel(self) -> f64 {
        match self {
            Self::H264 => 0.14,
            Self::H265 => 0.11,
            Self::Av1 => 0.055,
            Self::Vp9 => 0.07,
            Self::ProRes => 1.9,
        }
    }

    /// Whether the codec's pictures must have even sides (4:2:0 or 4:2:2).
    pub fn even_sides(self) -> bool {
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AudioCodec {
    Aac,
    Opus,
    Mp3,
    Flac,
    /// 16-bit little-endian PCM.
    Pcm16,
    /// 24-bit little-endian PCM.
    Pcm24,
}

impl AudioCodec {
    pub fn label(self) -> &'static str {
        match self {
            Self::Aac => "AAC",
            Self::Opus => "Opus",
            Self::Mp3 => "MP3",
            Self::Flac => "FLAC (lossless)",
            Self::Pcm16 => "PCM 16-bit",
            Self::Pcm24 => "PCM 24-bit",
        }
    }

    /// Whether a bitrate means anything for this codec.
    pub fn has_bitrate(self) -> bool {
        matches!(self, Self::Aac | Self::Opus | Self::Mp3)
    }

    /// Sample rates the codec accepts from [`SAMPLE_RATES`].
    pub fn accepts_rate(self, rate: u32) -> bool {
        match self {
            // Opus runs at 48 kHz; FFmpeg would resample anything else.
            Self::Opus => rate == 48_000,
            Self::Mp3 => rate <= 48_000,
            _ => SAMPLE_RATES.contains(&rate),
        }
    }

    /// The highest bitrate the encoder honours for `channels`, in kbit/s;
    /// above it FFmpeg fails (Opus) or silently caps (MP3, AAC).
    pub fn max_kbps(self, channels: u8) -> u32 {
        let channels = u32::from(channels.max(1));
        match self {
            Self::Aac => (256 * channels).min(MAX_AUDIO_KBPS),
            Self::Opus => (256 * channels).min(MAX_AUDIO_KBPS),
            Self::Mp3 => 320,
            Self::Flac | Self::Pcm16 | Self::Pcm24 => MAX_AUDIO_KBPS,
        }
    }

    pub fn default_kbps(self) -> u32 {
        match self {
            Self::Aac => 256,
            Self::Opus => 160,
            Self::Mp3 => 256,
            Self::Flac | Self::Pcm16 | Self::Pcm24 => 0,
        }
    }
}

/// A family of hardware video encoders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HardwareKind {
    /// NVIDIA NVENC.
    Nvenc,
    /// AMD AMF.
    Amf,
    /// Intel Quick Sync Video.
    Qsv,
    /// Apple VideoToolbox.
    VideoToolbox,
    /// VA-API (Linux; Intel and AMD).
    Vaapi,
    /// Windows Media Foundation.
    MediaFoundation,
}

impl HardwareKind {
    pub const ALL: [Self; 6] = [
        Self::Nvenc,
        Self::Amf,
        Self::Qsv,
        Self::VideoToolbox,
        Self::Vaapi,
        Self::MediaFoundation,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Nvenc => "NVIDIA NVENC",
            Self::Amf => "AMD AMF",
            Self::Qsv => "Intel Quick Sync",
            Self::VideoToolbox => "Apple VideoToolbox",
            Self::Vaapi => "VA-API",
            Self::MediaFoundation => "Media Foundation",
        }
    }

    /// The order `Auto` tries families in: dedicated encoders before
    /// generic driver layers.
    fn rank(self) -> usize {
        match self {
            Self::VideoToolbox => 0,
            Self::Nvenc => 1,
            Self::Amf => 2,
            Self::Qsv => 3,
            Self::Vaapi => 4,
            Self::MediaFoundation => 5,
        }
    }
}

/// One FFmpeg encoder the export knows how to drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownVideoEncoder {
    pub name: &'static str,
    pub codec: VideoCodec,
    pub hardware: Option<HardwareKind>,
    /// The longest picture side it accepts.
    pub max_side: u32,
    /// The smallest picture it accepts, width and height.
    pub min_size: [u32; 2],
}

const fn known(
    name: &'static str,
    codec: VideoCodec,
    hardware: Option<HardwareKind>,
    max_side: u32,
) -> KnownVideoEncoder {
    // NVENC refuses small pictures (measured: h264 needs 146x50, hevc and
    // av1 130x34); other hardware families are given the same margin.
    let min_size = match (hardware, codec) {
        (None, _) => [MIN_OUTPUT_SIDE, MIN_OUTPUT_SIDE],
        (Some(_), VideoCodec::H264) => [146, 50],
        (Some(_), _) => [130, 34],
    };
    KnownVideoEncoder {
        name,
        codec,
        hardware,
        max_side,
        min_size,
    }
}

/// Every video encoder the export drives, software first within a codec.
/// Probing keeps those that encode a test picture on this host.
pub const VIDEO_ENCODERS: &[KnownVideoEncoder] = {
    use HardwareKind::*;
    use VideoCodec::*;
    &[
        known("libx264", H264, None, 8192),
        known("libx265", H265, None, 8192),
        known("libsvtav1", Av1, None, 8192),
        known("libaom-av1", Av1, None, 8192),
        known("libvpx-vp9", Vp9, None, 8192),
        known("prores_ks", ProRes, None, 8192),
        known("h264_nvenc", H264, Some(Nvenc), 4096),
        known("hevc_nvenc", H265, Some(Nvenc), 8192),
        known("av1_nvenc", Av1, Some(Nvenc), 8192),
        known("h264_amf", H264, Some(Amf), 4096),
        known("hevc_amf", H265, Some(Amf), 8192),
        known("av1_amf", Av1, Some(Amf), 8192),
        known("h264_qsv", H264, Some(Qsv), 4096),
        known("hevc_qsv", H265, Some(Qsv), 8192),
        known("av1_qsv", Av1, Some(Qsv), 8192),
        known("vp9_qsv", Vp9, Some(Qsv), 8192),
        known("h264_videotoolbox", H264, Some(VideoToolbox), 4096),
        known("hevc_videotoolbox", H265, Some(VideoToolbox), 8192),
        known("prores_videotoolbox", ProRes, Some(VideoToolbox), 8192),
        known("h264_vaapi", H264, Some(Vaapi), 4096),
        known("hevc_vaapi", H265, Some(Vaapi), 8192),
        known("av1_vaapi", Av1, Some(Vaapi), 8192),
        known("vp9_vaapi", Vp9, Some(Vaapi), 8192),
        known("h264_mf", H264, Some(MediaFoundation), 4096),
        known("hevc_mf", H265, Some(MediaFoundation), 8192),
    ]
};

/// Every audio encoder the export drives, preferred first within a codec.
pub const AUDIO_ENCODERS: &[(&str, AudioCodec)] = &[
    ("aac_at", AudioCodec::Aac),
    ("aac", AudioCodec::Aac),
    // FFmpeg's own `opus` encoder is experimental and refused without
    // `-strict -2`; only libopus is driven.
    ("libopus", AudioCodec::Opus),
    ("libmp3lame", AudioCodec::Mp3),
    ("flac", AudioCodec::Flac),
    ("pcm_s16le", AudioCodec::Pcm16),
    ("pcm_s24le", AudioCodec::Pcm24),
];

pub fn known_video_encoder(name: &str) -> Option<&'static KnownVideoEncoder> {
    VIDEO_ENCODERS.iter().find(|e| e.name == name)
}

// ---------------------------------------------------------------------------
// Capabilities: what this host's FFmpeg can actually do
// ---------------------------------------------------------------------------

/// A video encoder that encoded a test picture on this host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VideoEncoder {
    /// FFmpeg's encoder name, e.g. `h264_nvenc`.
    pub name: String,
    pub codec: VideoCodec,
    pub hardware: Option<HardwareKind>,
    pub max_side: u32,
    pub min_size: [u32; 2],
    /// The device node the probe encoded on, for families that are given
    /// one (VA-API: `/dev/dri/renderD129`).
    #[serde(default)]
    pub device: Option<String>,
}

impl VideoEncoder {
    /// Whether it accepts a picture of `size`.
    pub fn fits(&self, size: [u32; 2]) -> bool {
        size[0].max(size[1]) <= self.max_side
            && size[0] >= self.min_size[0]
            && size[1] >= self.min_size[1]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AudioEncoder {
    pub name: String,
    pub codec: AudioCodec,
}

/// An encoder FFmpeg lists but that could not be used here, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Unavailable {
    pub name: String,
    pub reason: String,
}

/// What the installed FFmpeg can encode on this host. Only encoders that
/// produced a test picture are listed, so nothing offered is known broken.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Capabilities {
    /// FFmpeg's first version line.
    pub ffmpeg: String,
    pub video: Vec<VideoEncoder>,
    pub audio: Vec<AudioEncoder>,
    /// Containers whose muxer FFmpeg has.
    pub containers: Vec<Container>,
    /// Hardware or software encoders listed by FFmpeg that failed their test.
    pub unavailable: Vec<Unavailable>,
}

impl Capabilities {
    pub fn has_container(&self, container: Container) -> bool {
        self.containers.contains(&container)
    }

    pub fn has_video_codec(&self, codec: VideoCodec) -> bool {
        self.video.iter().any(|e| e.codec == codec)
    }

    pub fn has_audio_codec(&self, codec: AudioCodec) -> bool {
        self.audio.iter().any(|e| e.codec == codec)
    }

    /// Hardware families with at least one working encoder.
    pub fn hardware(&self) -> Vec<HardwareKind> {
        let mut kinds: Vec<_> = self.video.iter().filter_map(|e| e.hardware).collect();
        kinds.sort_by_key(|k| k.rank());
        kinds.dedup();
        kinds
    }

    /// The encoder a choice resolves to for `codec` at `size`, or `None`
    /// when this host has none. `Auto` takes the best hardware encoder that
    /// fits the size and falls back to software.
    pub fn video_encoder(
        &self,
        codec: VideoCodec,
        choice: EncoderChoice,
        size: [u32; 2],
    ) -> Option<&VideoEncoder> {
        let fits = |e: &&VideoEncoder| e.codec == codec && e.fits(size);
        let software = || {
            self.video
                .iter()
                .filter(fits)
                .find(|e| e.hardware.is_none())
        };
        match choice {
            EncoderChoice::Software => software(),
            EncoderChoice::Hardware(kind) => self
                .video
                .iter()
                .filter(fits)
                .find(|e| e.hardware == Some(kind)),
            EncoderChoice::Auto => self
                .video
                .iter()
                .filter(fits)
                .filter(|e| e.hardware.is_some())
                .min_by_key(|e| e.hardware.map_or(usize::MAX, HardwareKind::rank))
                .or_else(software),
        }
    }

    /// The software encoder for `codec` at `size`: what a failed hardware
    /// encode is retried with.
    pub fn software_encoder(&self, codec: VideoCodec, size: [u32; 2]) -> Option<&VideoEncoder> {
        self.video_encoder(codec, EncoderChoice::Software, size)
    }

    pub fn audio_encoder(&self, codec: AudioCodec) -> Option<&AudioEncoder> {
        self.audio.iter().find(|e| e.codec == codec)
    }
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// How the encoder is chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind", content = "family")]
pub enum EncoderChoice {
    /// The best working hardware encoder, otherwise software.
    #[default]
    Auto,
    Software,
    Hardware(HardwareKind),
}

/// A quality target the encoder holds while the bitrate varies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QualityLevel {
    /// Smaller files with visible loss in detailed scenes.
    Lower,
    /// Visually clean for viewing and uploading.
    #[default]
    Recommended,
    /// For further editing or demanding platforms.
    Higher,
    /// Near-lossless; very large files.
    Maximum,
}

impl QualityLevel {
    pub const ALL: [Self; 4] = [Self::Lower, Self::Recommended, Self::Higher, Self::Maximum];

    pub fn label(self) -> &'static str {
        match self {
            Self::Lower => "Lower",
            Self::Recommended => "Recommended",
            Self::Higher => "Higher",
            Self::Maximum => "Maximum",
        }
    }

    /// Multiplies [`VideoCodec`]'s bits per pixel for estimates.
    fn factor(self) -> f64 {
        match self {
            Self::Lower => 0.55,
            Self::Recommended => 1.0,
            Self::Higher => 1.8,
            Self::Maximum => 4.0,
        }
    }

    /// The constant rate factor for x264/x265-like scales (0..51).
    pub fn crf(self, codec: VideoCodec) -> u8 {
        match (codec, self) {
            (VideoCodec::H264, Self::Lower) => 26,
            (VideoCodec::H264, Self::Recommended) => 21,
            (VideoCodec::H264, Self::Higher) => 17,
            (VideoCodec::H264, Self::Maximum) => 12,
            (VideoCodec::H265, Self::Lower) => 28,
            (VideoCodec::H265, Self::Recommended) => 24,
            (VideoCodec::H265, Self::Higher) => 20,
            (VideoCodec::H265, Self::Maximum) => 14,
            // libvpx-vp9 and the AV1 encoders use 0..63.
            (VideoCodec::Vp9 | VideoCodec::Av1, Self::Lower) => 40,
            (VideoCodec::Vp9 | VideoCodec::Av1, Self::Recommended) => 32,
            (VideoCodec::Vp9 | VideoCodec::Av1, Self::Higher) => 26,
            (VideoCodec::Vp9 | VideoCodec::Av1, Self::Maximum) => 18,
            // ProRes has profiles, not a factor; see `prores_profile`.
            (VideoCodec::ProRes, _) => 0,
        }
    }

    /// The ProRes profile a level stands for: LT, 422, HQ, 4444.
    pub fn prores_profile(self) -> u8 {
        match self {
            Self::Lower => 1,
            Self::Recommended => 2,
            Self::Higher => 3,
            Self::Maximum => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "mode")]
pub enum RateControl {
    /// Hold a quality; the size follows the picture.
    Quality { level: QualityLevel },
    /// Hold an average bitrate in kbit/s; the quality follows the picture.
    Bitrate { kbps: u32 },
}

impl Default for RateControl {
    fn default() -> Self {
        Self::Quality {
            level: QualityLevel::Recommended,
        }
    }
}

/// Encoder effort: faster encodes or smaller files at the same quality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EncodeSpeed {
    Faster,
    #[default]
    Balanced,
    Smaller,
}

/// The output picture size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Resolution {
    /// The project canvas, sides rounded up to even.
    #[default]
    Project,
    /// Named by the short side, at the project's aspect: 1080 is 1920x1080
    /// for a wide project and 1080x1920 for a tall one.
    Short { pixels: u32 },
    /// Exactly this size; a different aspect is fitted per [`Fit`].
    Custom { width: u32, height: u32 },
}

impl Resolution {
    pub const P720: Self = Self::Short { pixels: 720 };
    pub const P1080: Self = Self::Short { pixels: 1080 };
    pub const P1440: Self = Self::Short { pixels: 1440 };
    pub const P2160: Self = Self::Short { pixels: 2160 };
    pub const NAMED: [(Self, &'static str); 4] = [
        (Self::P720, "720p"),
        (Self::P1080, "1080p"),
        (Self::P1440, "1440p"),
        (Self::P2160, "4K"),
    ];

    /// The output size for a canvas, sides rounded up to even. A size whose
    /// long side passes [`MAX_OUTPUT_SIDE`] is scaled down whole, keeping
    /// its aspect; [`ExportSettings::plan`] refuses asked-for sizes outside
    /// the limits before this is reached.
    pub fn size(self, canvas: [u32; 2]) -> [u32; 2] {
        let [w, h] = canvas.map(|s| s.max(1) as f64);
        let [w, h] = match self {
            Self::Project => [w, h],
            Self::Short { pixels } => {
                let short = pixels as f64;
                if w >= h {
                    [short * w / h, short]
                } else {
                    [short, short * h / w]
                }
            }
            Self::Custom { width, height } => [width as f64, height as f64],
        };
        let k = (MAX_OUTPUT_SIDE as f64 / w.max(h)).min(1.0);
        let even = |side: f64| {
            let side = (side * k).round().max(MIN_OUTPUT_SIDE as f64) as u32;
            ((side + 1) & !1).min(MAX_OUTPUT_SIDE)
        };
        [even(w), even(h)]
    }
}

/// How a canvas of another aspect fills the output picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    /// The whole canvas, with black bars.
    #[default]
    Letterbox,
    /// The output filled, the canvas's overflow cut evenly.
    Fill,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct VideoSettings {
    pub codec: VideoCodec,
    #[serde(default)]
    pub resolution: Resolution,
    /// `None` keeps the project's rate.
    #[serde(default)]
    pub frame_rate: Option<FrameRate>,
    #[serde(default)]
    pub rate_control: RateControl,
    #[serde(default)]
    pub speed: EncodeSpeed,
    #[serde(default)]
    pub encoder: EncoderChoice,
    #[serde(default)]
    pub fit: Fit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AudioSettings {
    pub codec: AudioCodec,
    /// Ignored by lossless codecs.
    pub kbps: u32,
    pub sample_rate: u32,
    /// 1 (mono) or 2 (stereo).
    pub channels: u8,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            codec: AudioCodec::Aac,
            kbps: AudioCodec::Aac.default_kbps(),
            sample_rate: 48_000,
            channels: 2,
        }
    }
}

/// The part of the timeline exported, as half-open timeline times.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ExportRange {
    /// From zero to the end of the last clip.
    #[default]
    Timeline,
    /// `[start, end)`, snapped outwards to whole output frames.
    Span { start: Time, end: Time },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SidecarFormat {
    Srt,
    Vtt,
}

impl SidecarFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Srt => "srt",
            Self::Vtt => "vtt",
        }
    }
}

/// What happens to caption tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SubtitleSettings {
    /// Draw visible caption tracks into the picture, as the viewer shows
    /// them. Off hides every caption track from the render.
    pub burn_in: bool,
    /// Also write the caption track as a file beside the export.
    pub sidecar: Option<SidecarFormat>,
    /// Also carry the caption track as a subtitle stream in the file.
    pub embed: bool,
    /// The caption track for the sidecar and the stream; `None` takes the
    /// first caption track.
    pub track: Option<TrackId>,
}

impl Default for SubtitleSettings {
    fn default() -> Self {
        Self {
            burn_in: true,
            sidecar: None,
            embed: false,
            track: None,
        }
    }
}

/// Everything an export is asked to make. Serializable and schema-described
/// so presets, the agent protocol and the settings file share it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExportSettings {
    pub container: Container,
    /// `None` writes sound only; the container must be an audio one.
    pub video: Option<VideoSettings>,
    /// `None` writes no sound track.
    pub audio: Option<AudioSettings>,
    #[serde(default)]
    pub range: ExportRange,
    #[serde(default)]
    pub subtitles: SubtitleSettings,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            container: Container::Mp4,
            video: Some(VideoSettings::default()),
            audio: Some(AudioSettings::default()),
            range: ExportRange::Timeline,
            subtitles: SubtitleSettings::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Validation: settings, a project and a host's capabilities
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind", content = "message")]
pub enum ExportError {
    /// The settings contradict themselves or the limits.
    #[error("{0}")]
    Invalid(String),
    /// This host's FFmpeg cannot make what was asked.
    #[error("{0}")]
    Unsupported(String),
    /// There is nothing in the range to export.
    #[error("{0}")]
    Empty(String),
}

/// Output frames `first..first + count` at `rate`; frame `n` shows the
/// timeline at `rate.frame_time(n)` and lasts until `frame_time(n + 1)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FrameSpan {
    pub rate: FrameRate,
    pub first: i64,
    pub count: u64,
}

impl FrameSpan {
    /// The frames covering `[start, end)`: from the frame `start` is in to
    /// the first frame starting at or after `end`.
    pub fn covering(start: Time, end: Time, rate: FrameRate) -> Self {
        let first = rate.frame_at(start.max(Time::ZERO));
        let mut last = rate.frame_at(end);
        if rate.frame_time(last) < end {
            last += 1;
        }
        Self {
            rate,
            first,
            count: last.saturating_sub(first).max(0) as u64,
        }
    }

    /// The timeline time output frame `index` (from zero) shows.
    pub fn time(&self, index: u64) -> Time {
        self.rate.frame_time(self.first + index as i64)
    }

    pub fn start(&self) -> Time {
        self.time(0)
    }

    pub fn end(&self) -> Time {
        self.time(self.count)
    }

    pub fn duration(&self) -> Time {
        self.end() - self.start()
    }
}

/// The exact number of sample frames that span `[start, end)` at `rate`:
/// rounded from the whole span, never summed per video frame, so sound and
/// picture end together.
pub fn sample_count(start: Time, end: Time, rate: u32) -> u64 {
    let span = (end.0 - start.0).max(0) as i128;
    ((span * rate as i128 + 500_000) / 1_000_000) as u64
}

/// The sound an export renders: `frames` sample frames from `start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SampleSpan {
    pub start: Time,
    pub frames: u64,
    pub rate: u32,
    pub channels: u8,
}

/// Where the canvas lands in the output picture.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Geometry {
    pub output: [u32; 2],
    /// The size the canvas is drawn at; larger than `output` on one side
    /// when filled, smaller when letterboxed.
    pub drawn: [u32; 2],
    /// The drawn canvas's left top in output pixels; negative when filled.
    pub offset: [i32; 2],
}

impl Geometry {
    pub fn new(canvas: [u32; 2], output: [u32; 2], fit: Fit) -> Self {
        let [cw, ch] = canvas.map(|s| s.max(1) as f64);
        let [ow, oh] = output.map(f64::from);
        let scale = match fit {
            Fit::Letterbox => (ow / cw).min(oh / ch),
            Fit::Fill => (ow / cw).max(oh / ch),
        };
        let drawn = [
            ((cw * scale).round() as u32).max(1),
            ((ch * scale).round() as u32).max(1),
        ];
        // An aspect within half a pixel of the output's is the output's.
        let drawn = [
            if drawn[0].abs_diff(output[0]) <= 1 {
                output[0]
            } else {
                drawn[0]
            },
            if drawn[1].abs_diff(output[1]) <= 1 {
                output[1]
            } else {
                drawn[1]
            },
        ];
        let offset = [
            (output[0] as i32 - drawn[0] as i32) / 2,
            (output[1] as i32 - drawn[1] as i32) / 2,
        ];
        Self {
            output,
            drawn,
            offset,
        }
    }

    /// Whether the canvas exactly covers the output.
    pub fn is_exact(&self) -> bool {
        self.drawn == self.output && self.offset == [0, 0]
    }
}

/// Settings checked against a project and a host: everything a renderer
/// needs, with no choice left open.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportPlan {
    pub settings: ExportSettings,
    /// The timeline span covered, whole output frames.
    pub start: Time,
    pub end: Time,
    pub video: Option<VideoPlan>,
    pub audio: Option<AudioPlan>,
    /// Cues of the chosen caption track for a sidecar or stream, shifted to
    /// start at zero and cut to the range.
    pub cues: Option<CuePlan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoPlan {
    pub frames: FrameSpan,
    pub geometry: Geometry,
    pub encoder: VideoEncoder,
    pub settings: VideoSettings,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioPlan {
    pub samples: SampleSpan,
    pub encoder: AudioEncoder,
    pub settings: AudioSettings,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CuePlan {
    pub track: TrackId,
    pub language: Option<String>,
    pub cues: Vec<Cue>,
}

impl ExportSettings {
    /// The project's timeline span this export covers, before snapping.
    pub fn span(&self, project: &Project) -> Result<(Time, Time), ExportError> {
        let duration = project.duration();
        let (start, end) = match self.range {
            ExportRange::Timeline => (Time::ZERO, duration),
            ExportRange::Span { start, end } => (start.max(Time::ZERO), end.min(duration)),
        };
        if duration <= Time::ZERO {
            return Err(ExportError::Empty("The timeline is empty".into()));
        }
        if end <= start {
            return Err(ExportError::Empty(
                "The range has no length inside the timeline".into(),
            ));
        }
        Ok((start, end))
    }

    /// Checks the settings against the project and this host and fixes every
    /// remaining choice. A failure names what to change.
    pub fn plan(&self, project: &Project, caps: &Capabilities) -> Result<ExportPlan, ExportError> {
        let (start, end) = self.span(project)?;
        let container = self.container;
        let label = container.label();
        if !caps.has_container(container) {
            return Err(ExportError::Unsupported(format!(
                "{label} files cannot be written by this FFmpeg"
            )));
        }
        match (container.has_video(), &self.video, &self.audio) {
            (false, Some(_), _) => {
                return Err(ExportError::Invalid(format!(
                    "{label} files hold sound only; turn the picture off or choose MP4"
                )));
            }
            (false, None, None) => {
                return Err(ExportError::Invalid(format!(
                    "{label} files need sound; turn the sound on"
                )));
            }
            (true, None, _) => {
                return Err(ExportError::Invalid(format!(
                    "{label} files need a picture; choose an audio format for sound only"
                )));
            }
            _ => {}
        }
        let video = self
            .video
            .as_ref()
            .map(|video| plan_video(video, container, project, caps, start, end))
            .transpose()?;
        // With a picture, everything covers its whole frames.
        let (start, end) = video.as_ref().map_or((start, end), |video| {
            (video.frames.start(), video.frames.end())
        });
        let audio = self
            .audio
            .as_ref()
            .map(|audio| plan_audio(audio, container, caps, start, end))
            .transpose()?;
        let cues = self.plan_cues(project, start, end)?;
        Ok(ExportPlan {
            settings: self.clone(),
            start,
            end,
            video,
            audio,
            cues,
        })
    }

    // The caption track written beside or into the file, cut to the range.
    fn plan_cues(
        &self,
        project: &Project,
        start: Time,
        end: Time,
    ) -> Result<Option<CuePlan>, ExportError> {
        let subtitles = &self.subtitles;
        if subtitles.sidecar.is_none() && !subtitles.embed {
            return Ok(None);
        }
        if subtitles.embed
            && (self.container.embedded_subtitles().is_none() || self.video.is_none())
        {
            return Err(ExportError::Invalid(format!(
                "{} files cannot carry captions; write a caption file instead",
                self.container.label()
            )));
        }
        let track = match subtitles.track {
            Some(id) => caption_tracks(project)
                .find(|track| track.id == id)
                .ok_or_else(|| {
                    ExportError::Invalid("The chosen caption track is not in the project".into())
                })?,
            None => caption_tracks(project)
                .next()
                .ok_or_else(|| ExportError::Invalid("There is no caption track to write".into()))?,
        };
        let cues = range_cues(track, start, end);
        if cues.is_empty() {
            return Err(ExportError::Empty(
                "The caption track has no captions in the range".into(),
            ));
        }
        Ok(Some(CuePlan {
            track: track.id,
            language: track.language.clone(),
            cues,
        }))
    }

    /// The project as this export draws it: the output frame rate, and
    /// caption tracks hidden when they are not burned in. Never committed.
    ///
    /// The renderer is given this project for `resolve` and `source_frame`
    /// alike, so titles, noise and reversed clips step at the output rate.
    /// It is never validated (a transition shorter than one output frame is
    /// still drawn), serialized or committed.
    pub fn render_project(&self, project: &Project) -> Project {
        let mut project = project.clone();
        if let Some(rate) = self.video.as_ref().and_then(|video| video.frame_rate) {
            project.settings.frame_rate = rate;
        }
        if !self.subtitles.burn_in {
            for track in &mut project.tracks {
                if is_caption_track(track) && !track.hidden {
                    Arc::make_mut(track).hidden = true;
                }
            }
        }
        project
    }

    /// A rough file size in bytes for the plan, for the dialog. A bitrate is
    /// held, so its size is close; a quality's size follows the picture, and
    /// the figure is what the encoder the plan runs writes for typical
    /// footage at that level.
    pub fn estimate_bytes(plan: &ExportPlan) -> u64 {
        let mut bytes = 0.0;
        if let Some(video) = &plan.video {
            let kbps = match video.settings.rate_control {
                RateControl::Bitrate { kbps } => f64::from(kbps),
                RateControl::Quality { level } => {
                    f64::from(suggested_kbps(
                        video.settings.codec,
                        video.geometry.output,
                        video.frames.rate.fps(),
                        level,
                    )) * quality_size_factor(&video.encoder, video.settings.speed)
                }
            };
            bytes += kbps * 125.0 * video.frames.duration().as_seconds();
        }
        if let Some(audio) = &plan.audio {
            let samples = audio.samples;
            let pcm = |sample_bytes: f64| {
                samples.frames as f64 * f64::from(samples.channels) * sample_bytes
            };
            bytes += match audio.settings.codec {
                AudioCodec::Pcm16 => pcm(2.0),
                AudioCodec::Pcm24 => pcm(3.0),
                AudioCodec::Flac => 0.6 * pcm(2.0),
                AudioCodec::Aac | AudioCodec::Opus | AudioCodec::Mp3 => {
                    f64::from(audio.settings.kbps) * 125.0 * samples.frames as f64
                        / f64::from(samples.rate.max(1))
                }
            };
        }
        // Container headers and indexes.
        (bytes * 1.01).round() as u64
    }
}

fn plan_video(
    settings: &VideoSettings,
    container: Container,
    project: &Project,
    caps: &Capabilities,
    start: Time,
    end: Time,
) -> Result<VideoPlan, ExportError> {
    let codec = settings.codec;
    if !container.video_codecs().contains(&codec) {
        let holders: Vec<_> = Container::VIDEO
            .into_iter()
            .filter(|c| c.video_codecs().contains(&codec))
            .map(Container::label)
            .collect();
        return Err(ExportError::Invalid(format!(
            "{} files cannot hold {} video; choose {}",
            container.label(),
            codec.label(),
            holders.join(" or ")
        )));
    }
    check_asked_size(settings.resolution)?;
    let rate = settings.frame_rate.unwrap_or(project.settings.frame_rate);
    if !rate.validate() {
        return Err(ExportError::Invalid(
            "The frame rate must be between 1 and 240 frames per second".into(),
        ));
    }
    match settings.rate_control {
        RateControl::Bitrate { .. } if codec == VideoCodec::ProRes => {
            return Err(ExportError::Invalid(
                "ProRes quality is set by its profile".into(),
            ));
        }
        RateControl::Bitrate { kbps } if !(MIN_VIDEO_KBPS..=MAX_VIDEO_KBPS).contains(&kbps) => {
            return Err(ExportError::Invalid(format!(
                "The video bitrate must be between {} and {} kbit/s",
                thousands(MIN_VIDEO_KBPS),
                thousands(MAX_VIDEO_KBPS)
            )));
        }
        _ => {}
    }
    let canvas = [project.settings.width, project.settings.height];
    let size = settings.resolution.size(canvas);
    let geometry = Geometry::new(canvas, size, settings.fit);
    if geometry.drawn[0].max(geometry.drawn[1]) > MAX_OUTPUT_SIDE {
        return Err(ExportError::Invalid(
            "Fill would need a picture larger than the compositor can draw; use Letterbox".into(),
        ));
    }
    let encoder = caps
        .video_encoder(codec, settings.encoder, size)
        .cloned()
        .ok_or_else(|| missing_video_encoder(caps, codec, settings.encoder, size))?;
    Ok(VideoPlan {
        frames: FrameSpan::covering(start, end, rate),
        geometry,
        encoder,
        settings: *settings,
    })
}

// Sizes a person typed are refused rather than quietly changed.
fn check_asked_size(resolution: Resolution) -> Result<(), ExportError> {
    let sides = match resolution {
        Resolution::Project => return Ok(()),
        Resolution::Short { pixels } => [pixels, pixels],
        Resolution::Custom { width, height } => [width, height],
    };
    if !sides
        .iter()
        .all(|side| (MIN_OUTPUT_SIDE..=MAX_OUTPUT_SIDE).contains(side))
    {
        return Err(ExportError::Invalid(format!(
            "Each side of the picture must be between {MIN_OUTPUT_SIDE} and {} pixels",
            thousands(MAX_OUTPUT_SIDE)
        )));
    }
    if u64::from(sides[0]) * u64::from(sides[1]) > MAX_OUTPUT_PIXELS {
        return Err(ExportError::Invalid(
            "The picture is too large; choose a smaller size".into(),
        ));
    }
    Ok(())
}

// Why no encoder of a choice takes `codec` at `size` here.
fn missing_video_encoder(
    caps: &Capabilities,
    codec: VideoCodec,
    choice: EncoderChoice,
    size: [u32; 2],
) -> ExportError {
    let candidates: Vec<_> = caps
        .video
        .iter()
        .filter(|e| e.codec == codec)
        .filter(|e| match choice {
            EncoderChoice::Auto => true,
            EncoderChoice::Software => e.hardware.is_none(),
            EncoderChoice::Hardware(kind) => e.hardware == Some(kind),
        })
        .collect();
    let codec_label = codec.label();
    let Some(largest) = candidates.iter().map(|e| e.max_side).max() else {
        return ExportError::Unsupported(match choice {
            EncoderChoice::Auto => format!("This FFmpeg cannot encode {codec_label}"),
            EncoderChoice::Software => format!(
                "This FFmpeg has no software {codec_label} encoder; choose a hardware encoder"
            ),
            EncoderChoice::Hardware(kind) => format!(
                "{} cannot encode {codec_label} on this computer; choose Auto or Software",
                kind.label()
            ),
        });
    };
    let who = match choice {
        EncoderChoice::Hardware(kind) => kind.label(),
        EncoderChoice::Software => "The software encoder",
        EncoderChoice::Auto => "This computer",
    };
    let [width, height] = size;
    if width.max(height) > largest {
        ExportError::Unsupported(format!(
            "{who} encodes {codec_label} only up to {} pixels a side, not {width} × {height}; \
             choose a smaller size",
            thousands(largest)
        ))
    } else {
        ExportError::Unsupported(format!(
            "{who} cannot encode {codec_label} pictures as small as {width} × {height}; \
             choose a larger size"
        ))
    }
}

fn plan_audio(
    settings: &AudioSettings,
    container: Container,
    caps: &Capabilities,
    start: Time,
    end: Time,
) -> Result<AudioPlan, ExportError> {
    let codec = settings.codec;
    let codecs = container.audio_codecs();
    if !codecs.contains(&codec) {
        let options: Vec<_> = codecs.iter().map(|c| c.label()).collect();
        return Err(ExportError::Invalid(format!(
            "{} files cannot hold {} sound; choose {}",
            container.label(),
            codec.label(),
            options.join(" or ")
        )));
    }
    let encoder = caps.audio_encoder(codec).cloned().ok_or_else(|| {
        ExportError::Unsupported(format!("This FFmpeg cannot encode {} sound", codec.label()))
    })?;
    if !matches!(settings.channels, 1 | 2) {
        return Err(ExportError::Invalid("Sound must be mono or stereo".into()));
    }
    let rate = settings.sample_rate;
    if !SAMPLE_RATES.contains(&rate) {
        return Err(ExportError::Invalid(
            "Choose a sample rate of 44.1, 48 or 96 kHz".into(),
        ));
    }
    if !codec.accepts_rate(rate) {
        return Err(ExportError::Invalid(format!(
            "{} sound cannot be {}; choose 48 kHz",
            codec.label(),
            kilohertz(rate)
        )));
    }
    if codec.has_bitrate() {
        let max = codec.max_kbps(settings.channels);
        if !(MIN_AUDIO_KBPS..=max).contains(&settings.kbps) {
            let layout = if settings.channels == 1 {
                "mono"
            } else {
                "stereo"
            };
            return Err(ExportError::Invalid(format!(
                "The {} bitrate for {layout} must be between {MIN_AUDIO_KBPS} and {max} kbit/s",
                codec.label()
            )));
        }
    }
    Ok(AudioPlan {
        samples: SampleSpan {
            start,
            frames: sample_count(start, end, rate),
            rate,
            channels: settings.channels,
        },
        encoder,
        settings: *settings,
    })
}

// 12000 as "12,000".
fn thousands(value: u32) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

// 44100 as "44.1 kHz".
fn kilohertz(rate: u32) -> String {
    let khz = f64::from(rate) / 1000.0;
    format!("{} kHz", (khz * 10.0).round() / 10.0)
}

fn is_caption_track(track: &Track) -> bool {
    track.kind == TrackKind::Text && track.language.is_some()
}

/// A bitrate in kbit/s that suits `codec` at `size` and `fps` and `level`:
/// what bitrate mode starts from and what estimates assume.
pub fn suggested_kbps(codec: VideoCodec, size: [u32; 2], fps: f64, level: QualityLevel) -> u32 {
    let pixels = size[0] as f64 * size[1] as f64 * fps.clamp(1.0, 240.0);
    let kbps = pixels * codec.bits_per_pixel() * level.factor() / 1000.0;
    (kbps.round() as u32).clamp(MIN_VIDEO_KBPS, MAX_VIDEO_KBPS)
}

/// How a quality-mode encode's size compares with [`suggested_kbps`] for the
/// encoder that runs it at `speed`. NVENC is given the quality value whose
/// files match libx264's and libx265's (see `editor-media`'s export
/// arguments), and Media Foundation holds the suggested bitrate, so they
/// share the figure; the other hardware families are assumed to land near
/// it, unmeasured. libx264's faster preset wrote 0.82 to 0.89 times what its
/// medium one did at the same factor and its slower one about the same;
/// libx265's and NVENC's presets moved the size by a few per cent.
fn quality_size_factor(encoder: &VideoEncoder, speed: EncodeSpeed) -> f64 {
    match (encoder.name.as_str(), speed) {
        ("libx264", EncodeSpeed::Faster) => 0.85,
        _ => 1.0,
    }
}

/// Caption tracks: text tracks with a language.
pub fn caption_tracks(project: &Project) -> impl Iterator<Item = &Track> {
    project
        .tracks
        .iter()
        .map(AsRef::as_ref)
        .filter(|track| is_caption_track(track))
}

/// The cues of `track` inside `[start, end)`, cut to it and moved so the
/// range starts at zero. Word times are moved with their cue and clamped
/// into it, so a cue keeps one word time per word or none.
pub fn range_cues(track: &Track, start: Time, end: Time) -> Vec<Cue> {
    if end <= start {
        return Vec::new();
    }
    let mut cues: Vec<Cue> = captions::track_cues(track)
        .into_iter()
        .filter(|cue| cue.start < end && cue.end > start)
        .map(|cue| {
            let from = cue.start.max(start);
            let to = cue.end.min(end);
            let local = |time: Time| time.clamp(from, to) - start;
            Cue {
                start: from - start,
                end: to - start,
                words: cue
                    .words
                    .iter()
                    .map(|word| WordTime {
                        at: local(word.at),
                        end: local(word.end),
                    })
                    .collect(),
                text: cue.text,
            }
        })
        .filter(|cue| cue.end > cue.start)
        .collect();
    captions::settle(&mut cues);
    cues
}

// ---------------------------------------------------------------------------
// Presets
// ---------------------------------------------------------------------------

/// Where a preset is meant to go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Destination {
    General,
    YouTube,
    TikTok,
    Instagram,
    Facebook,
    X,
    Vimeo,
    Master,
    Audio,
    /// Saved by the person.
    Custom,
}

impl Destination {
    pub fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::YouTube => "YouTube",
            Self::TikTok => "TikTok",
            Self::Instagram => "Instagram",
            Self::Facebook => "Facebook",
            Self::X => "X",
            Self::Vimeo => "Vimeo",
            Self::Master => "Master",
            Self::Audio => "Audio",
            Self::Custom => "Saved",
        }
    }
}

/// A named starting point for the export settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExportPreset {
    /// Stable: built-ins use a fixed slug, saved presets a UUID string.
    pub id: String,
    pub name: String,
    pub destination: Destination,
    /// One line on what it is for, e.g. "1080p H.264 · up to 60 fps".
    pub note: String,
    pub settings: ExportSettings,
}

/// The presets that come with the editor. A preset never carries a range or
/// a caption track: those stay as the dialog has them.
pub fn builtin_presets() -> Vec<ExportPreset> {
    use AudioCodec::{Aac, Flac, Mp3, Opus, Pcm24};
    use Destination::*;
    use QualityLevel::{Higher, Recommended};
    use VideoCodec::{H264, H265, ProRes, Vp9};
    const PROJECT: Resolution = Resolution::Project;
    const TALL: Resolution = Resolution::Custom {
        width: 1080,
        height: 1920,
    };
    const FEED: Resolution = Resolution::Custom {
        width: 1080,
        height: 1350,
    };
    let quality = |level| RateControl::Quality { level };
    let video = |codec, resolution, rate_control| VideoSettings {
        codec,
        resolution,
        rate_control,
        ..VideoSettings::default()
    };
    let sound = |codec: AudioCodec, kbps, sample_rate| AudioSettings {
        codec,
        kbps,
        sample_rate,
        channels: 2,
    };
    let film = |container, video, audio| ExportSettings {
        container,
        video: Some(video),
        audio: Some(audio),
        ..ExportSettings::default()
    };
    let mp4 = |codec, resolution, level, kbps| {
        film(
            Container::Mp4,
            video(codec, resolution, quality(level)),
            sound(Aac, kbps, 48_000),
        )
    };
    let only = |container, audio| ExportSettings {
        container,
        video: None,
        audio: Some(audio),
        ..ExportSettings::default()
    };
    let small = ExportSettings {
        video: Some(VideoSettings {
            speed: EncodeSpeed::Smaller,
            ..video(H265, PROJECT, quality(Recommended))
        }),
        ..mp4(H265, PROJECT, Recommended, 160)
    };
    let x = film(
        Container::Mp4,
        video(
            H264,
            Resolution::P1080,
            RateControl::Bitrate { kbps: 12_000 },
        ),
        sound(Aac, 256, 48_000),
    );
    [
        (
            "recommended",
            "Recommended",
            General,
            mp4(H264, PROJECT, Recommended, 256),
        ),
        (
            "high-quality",
            "High quality",
            General,
            mp4(H264, PROJECT, Higher, 320),
        ),
        ("small-file", "Small file", General, small),
        (
            "youtube-1080",
            "YouTube 1080p",
            YouTube,
            mp4(H264, Resolution::P1080, Higher, 384),
        ),
        (
            "youtube-4k",
            "YouTube 4K",
            YouTube,
            mp4(H265, Resolution::P2160, Higher, 384),
        ),
        (
            "youtube-shorts",
            "YouTube Shorts",
            YouTube,
            mp4(H264, TALL, Higher, 256),
        ),
        ("tiktok", "TikTok", TikTok, mp4(H264, TALL, Higher, 256)),
        (
            "instagram-reels",
            "Instagram Reels",
            Instagram,
            mp4(H264, TALL, Recommended, 256),
        ),
        (
            "instagram-feed",
            "Instagram feed (4:5)",
            Instagram,
            mp4(H264, FEED, Recommended, 256),
        ),
        (
            "facebook",
            "Facebook",
            Facebook,
            mp4(H264, Resolution::P1080, Recommended, 256),
        ),
        ("x", "X", X, x),
        (
            "vimeo",
            "Vimeo",
            Vimeo,
            mp4(H264, Resolution::P1080, Higher, 320),
        ),
        (
            "prores-master",
            "ProRes 422 HQ master",
            Master,
            film(
                Container::Mov,
                video(ProRes, PROJECT, quality(Higher)),
                sound(Pcm24, 0, 48_000),
            ),
        ),
        (
            "webm",
            "Web (VP9)",
            General,
            film(
                Container::Webm,
                video(Vp9, PROJECT, quality(Recommended)),
                sound(Opus, 160, 48_000),
            ),
        ),
        (
            "audio-aac",
            "Audio (AAC)",
            Audio,
            only(Container::M4a, sound(Aac, 256, 48_000)),
        ),
        (
            "audio-wav",
            "Audio (WAV)",
            Audio,
            only(Container::Wav, sound(Pcm24, 0, 48_000)),
        ),
        (
            "audio-mp3",
            "Audio (MP3)",
            Audio,
            only(Container::Mp3, sound(Mp3, 320, 44_100)),
        ),
        (
            "audio-flac",
            "Audio (FLAC)",
            Audio,
            only(Container::Flac, sound(Flac, 0, 48_000)),
        ),
    ]
    .into_iter()
    .map(|(id, name, destination, settings)| ExportPreset {
        id: id.into(),
        name: name.into(),
        destination,
        note: preset_note(&settings),
        settings,
    })
    .collect()
}

// "1080 × 1920 · H.264 · Higher", "MP3 · 320 kbit/s · 44.1 kHz".
fn preset_note(settings: &ExportSettings) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(video) = &settings.video {
        parts.push(match video.resolution {
            Resolution::Project => "Project size".into(),
            Resolution::Short { pixels } => Resolution::NAMED
                .iter()
                .find(|(named, _)| *named == video.resolution)
                .map_or_else(|| format!("{pixels}p"), |(_, name)| (*name).into()),
            Resolution::Custom { width, height } => format!("{width} × {height}"),
        });
        match (video.codec, video.rate_control) {
            (VideoCodec::ProRes, RateControl::Quality { level }) => {
                let profile = match level {
                    QualityLevel::Lower => "422 LT",
                    QualityLevel::Recommended => "422",
                    QualityLevel::Higher => "422 HQ",
                    QualityLevel::Maximum => "4444",
                };
                parts.push(format!("ProRes {profile}"));
            }
            (codec, rate_control) => {
                parts.push(short_codec(codec).into());
                parts.push(match rate_control {
                    RateControl::Quality { level } => level.label().into(),
                    RateControl::Bitrate { kbps } => format!("{} kbit/s", thousands(kbps)),
                });
            }
        }
    } else if let Some(audio) = &settings.audio {
        parts.push(
            match audio.codec {
                AudioCodec::Flac => "FLAC",
                codec => codec.label(),
            }
            .into(),
        );
        if audio.codec.has_bitrate() {
            parts.push(format!("{} kbit/s", audio.kbps));
        }
        parts.push(kilohertz(audio.sample_rate));
    }
    parts.join(" · ")
}

fn short_codec(codec: VideoCodec) -> &'static str {
    match codec {
        VideoCodec::H265 => "H.265",
        VideoCodec::ProRes => "ProRes",
        codec => codec.label(),
    }
}

/// A preset's settings applied over current ones: format, picture, sound
/// and caption handling change; the range and caption track stay.
pub fn apply_preset(current: &ExportSettings, preset: &ExportPreset) -> ExportSettings {
    ExportSettings {
        range: current.range,
        subtitles: SubtitleSettings {
            track: current.subtitles.track,
            ..preset.settings.subtitles
        },
        ..preset.settings.clone()
    }
}

// ---------------------------------------------------------------------------
// Render jobs as reported to the person and to agents
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RenderState {
    Queued,
    /// Rendering the sound and starting the encoder.
    Preparing,
    Rendering,
    Paused,
    /// Closing the file and moving it into place.
    Finalizing,
    Done,
    Failed,
    Cancelled,
}

impl RenderState {
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }

    pub fn is_active(self) -> bool {
        matches!(self, Self::Preparing | Self::Rendering | Self::Finalizing)
    }
}

/// One job of the render queue as it stands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RenderJobInfo {
    pub id: Uuid,
    /// The output's file name.
    pub name: String,
    pub destination: PathBuf,
    pub settings: ExportSettings,
    pub project_id: Uuid,
    /// The project revision the job was queued from.
    pub revision: u64,
    pub state: RenderState,
    /// 0..=1 over the whole job.
    pub progress: f32,
    pub frames_done: u64,
    pub frames_total: u64,
    /// Output frames per second over the recent past.
    pub speed_fps: f32,
    pub elapsed_seconds: f64,
    pub eta_seconds: Option<f64>,
    /// The encoder in use or used, e.g. `h264_nvenc`.
    pub encoder: Option<String>,
    /// The output's size once done.
    pub output_bytes: Option<u64>,
    /// The editing context (`Editor::instance_id`) the job was queued from.
    pub context_id: Uuid,
    /// Why it failed, or a note such as a fallback to software.
    pub message: Option<String>,
    /// How many times it has been started.
    pub attempts: u32,
}
