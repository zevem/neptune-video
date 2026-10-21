//! The export dialog: which file to make, from which part of the timeline,
//! and how. It edits only its own copy of the settings and answers what the
//! person asked for; the application queues the job, asks for a folder and
//! keeps presets. Nothing offered is something this host cannot encode:
//! choices come from the probed [`Capabilities`], and the rest are absent or
//! disabled with the reason on hover.
//!
//! It is drawn on Terminal's sheet: a 52-point header, a body of cards that
//! scrolls under a fade, and a fixed 60-point footer whose Export is the
//! sheet's one accent.
use super::{
    controls::{self, ButtonKind, Group},
    kit, widgets, window,
};
use crate::{
    icons::{self, Icon},
    theme::{self, Accent, Palette, metrics},
    timefmt,
};
use editor_core::{
    FrameRate, Project, Time, TrackId,
    export::{
        self, AudioCodec, AudioSettings, Capabilities, Container, Destination, EncodeSpeed,
        EncoderChoice, ExportError, ExportPlan, ExportPreset, ExportRange, ExportSettings, Fit,
        Geometry, HardwareKind, MAX_AUDIO_KBPS, MAX_VIDEO_KBPS, MIN_AUDIO_KBPS, MIN_VIDEO_KBPS,
        QualityLevel, RateControl, Resolution, SAMPLE_RATES, SidecarFormat, VideoCodec,
        VideoSettings,
    },
};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, Id, Layout, Rect, Sense, Stroke, Ui,
    UiBuilder, Vec2, WidgetInfo, WidgetType, pos2, vec2,
};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// The most jobs the render queue lists; the dialog queues no more.
pub const QUEUE_LIMIT: usize = crate::export::queue::MAX_JOBS;

/// Frame rates offered besides the project's, as the canvas settings offer
/// them.
const RATES: [(u32, u32, &str); 8] = [
    (24000, 1001, "23.976"),
    (24, 1, "24"),
    (25, 1, "25"),
    (30000, 1001, "29.97"),
    (30, 1, "30"),
    (50, 1, "50"),
    (60000, 1001, "59.94"),
    (60, 1, "60"),
];

/// Sound bitrates offered, in kbit/s; each codec takes those up to its own
/// limit.
const AUDIO_KBPS: [u32; 8] = [96, 128, 160, 192, 256, 320, 384, 512];

/// The width of the column with the picture and the summary.
const LEFT: f32 = 260.0;
/// Presets offered as chips beside the preset list, the most used first;
/// as many as fit on one row are shown.
const QUICK: [&str; 5] = [
    "recommended",
    "youtube-1080",
    "tiktok",
    "instagram-feed",
    "audio-wav",
];
/// The sheet below which the columns stack.
const TWO_COLUMNS: f32 = 640.0;

/// What the host's encoders are known to be, as the queue reports it.
#[derive(Clone, Copy, Debug)]
pub enum CapabilityView<'a> {
    /// FFmpeg's encoders are being tried.
    Probing,
    Ready(&'a Capabilities),
    /// They could not be tried; why.
    Failed(&'a str),
}

/// What the sheet reads each frame. Nothing here is changed by it.
pub struct ExportSheetInput<'a> {
    pub project: &'a Project,
    pub caps: CapabilityView<'a>,
    /// Built-in presets, then the person's: theirs have
    /// [`Destination::Custom`].
    pub presets: &'a [ExportPreset],
    /// The span of the selected clips, when any are selected.
    pub selection: Option<(Time, Time)>,
    pub playhead: Time,
    /// The output picture at the playhead as the export draws it, bars and
    /// crop included, and its size: the poster. `None` until one is drawn.
    pub poster: Option<(egui::TextureId, Vec2)>,
    /// Why the poster could not be drawn, when it could not.
    pub poster_problem: Option<&'a str>,
    /// What the application knows will not come out as previewed, such as
    /// offline media.
    pub warnings: &'a [String],
    /// Jobs listed in the render queue.
    pub queue_len: usize,
}

/// What the person asked of the sheet.
#[derive(Clone, Debug, PartialEq)]
pub enum ExportAnswer {
    Close,
    /// Queue an export of the project as it is now to `destination`. Both
    /// buttons queue: `start_now` is Export, whose job renders as soon as the
    /// queue reaches it and whose progress the queue panel then shows; Add to
    /// queue (`false`) waits, paused, until the person starts it there.
    Queue {
        settings: ExportSettings,
        destination: PathBuf,
        start_now: bool,
    },
    /// Ask for the folder to export into, then give it to
    /// [`ExportSheetState::folder_chosen`]. Export pressed before a folder
    /// was chosen asks this first and queues once a folder is given.
    ChooseFolder,
    /// Keep these settings as a preset under `name`. The range and caption
    /// track are not part of a preset.
    SavePreset {
        name: String,
        settings: ExportSettings,
    },
    /// Forget the person's preset with this id.
    DeletePreset(String),
    /// Try FFmpeg's encoders again.
    Reprobe,
    /// Show the render queue.
    OpenQueue,
}

/// Which part of the timeline is exported, as the person picked it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeChoice {
    Timeline,
    /// The span of the clips selected while the sheet is open.
    Selection,
    /// Typed start and end times.
    Custom,
}

/// The output size as the segmented control picks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SizeChoice {
    Short(u32),
    Project,
    Custom,
}

/// How the bitrate is chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Quality,
    Bitrate,
}

/// The dialog's own state: the settings it edits and what is being typed.
pub struct ExportSheetState {
    settings: ExportSettings,
    /// The file name without its extension.
    name: String,
    folder: Option<PathBuf>,
    /// The person's home folder, which the folder is shown from as `~`.
    home: Option<PathBuf>,
    /// The preset last applied, kept while the settings still match it.
    preset: Option<String>,
    range: RangeChoice,
    start_text: String,
    end_text: String,
    width_text: String,
    height_text: String,
    kbps_text: String,
    /// The container and picture settings put aside while Video is off, so
    /// that turning it back on restores them.
    parked_video: Option<(Container, VideoSettings)>,
    parked_audio: Option<AudioSettings>,
    /// Encoder and speed rows are shown.
    encoding_open: bool,
    /// A preset name being typed.
    naming: Option<String>,
    focus_naming: bool,
    /// Export was pressed before a folder was chosen: whether to start it
    /// once one is.
    pending: Option<bool>,
    /// The settings were moved onto what this host can encode.
    fitted: bool,
    /// The editing context (`Editor::instance_id`) the sheet was opened
    /// for: a sheet closed unfinished is opened again as it was only for
    /// the same project.
    owner: Option<Uuid>,
    /// Why the queue refused what the sheet last asked of it, said in the
    /// footer until the person changes something or asks again. A toast
    /// would wait under the sheet in a short window, out of sight.
    refusal: Option<String>,
}

impl ExportSheetState {
    /// A sheet starting from `settings` (the last used, or the defaults),
    /// writing `name` into `folder`.
    pub fn new(settings: ExportSettings, name: String, folder: Option<PathBuf>) -> Self {
        let range = match settings.range {
            ExportRange::Timeline => RangeChoice::Timeline,
            ExportRange::Span { .. } => RangeChoice::Custom,
        };
        let mut state = Self {
            settings,
            name: stem_of(&name),
            folder,
            home: directories::UserDirs::new().map(|dirs| dirs.home_dir().to_path_buf()),
            preset: None,
            range,
            start_text: String::new(),
            end_text: String::new(),
            width_text: String::new(),
            height_text: String::new(),
            kbps_text: String::new(),
            parked_video: None,
            parked_audio: None,
            encoding_open: false,
            naming: None,
            focus_naming: false,
            pending: None,
            fitted: false,
            owner: None,
            refusal: None,
        };
        state.refresh_texts();
        state
    }

    /// Shows the folder from `home` rather than the person's home folder.
    #[cfg(test)]
    pub fn with_home(mut self, home: Option<PathBuf>) -> Self {
        self.home = home;
        self
    }

    /// The sheet of the editing context `owner`.
    pub fn with_owner(mut self, owner: Uuid) -> Self {
        self.owner = Some(owner);
        self
    }

    pub fn owner(&self) -> Option<Uuid> {
        self.owner
    }

    /// Starts from the preset `id` the settings were last set from: it is
    /// shown chosen while the settings are still that preset's, even where
    /// another preset has the same settings.
    pub fn with_preset(mut self, id: Option<String>) -> Self {
        self.preset = id;
        self
    }

    /// The sheet shown again after it was closed unfinished: what was being
    /// typed and kept is kept, an Export that waited for a folder is not.
    pub fn reopened(&mut self) {
        self.pending = None;
        self.naming = None;
        self.focus_naming = false;
        self.refusal = None;
    }

    /// Says in the sheet why what it asked for was refused, while the sheet
    /// is open and the person is looking at it.
    pub fn refuse(&mut self, why: String) {
        self.refusal = Some(why);
    }

    #[cfg(test)]
    pub fn refusal(&self) -> Option<&str> {
        self.refusal.as_deref()
    }

    pub fn settings(&self) -> &ExportSettings {
        &self.settings
    }

    pub fn folder(&self) -> Option<&Path> {
        self.folder.as_deref()
    }

    /// The id of the preset the settings were last set from, while they
    /// still match it.
    pub fn preset(&self) -> Option<&str> {
        self.preset.as_deref()
    }

    #[cfg(test)]
    pub fn range(&self) -> RangeChoice {
        self.range
    }

    /// The answer to [`ExportAnswer::ChooseFolder`]: the folder picked, or
    /// `None` when the person cancelled, which also gives up an Export that
    /// waited for it.
    pub fn folder_chosen(&mut self, folder: Option<PathBuf>) {
        match folder {
            Some(folder) => self.folder = Some(folder),
            None => self.pending = None,
        }
    }

    /// Where the file goes: the folder, the name and the container's
    /// extension.
    pub fn destination(&self) -> Option<PathBuf> {
        let folder = self.folder.as_ref()?;
        let stem = stem_of(&self.name);
        (!stem.is_empty())
            .then(|| folder.join(format!("{stem}.{}", self.settings.container.extension())))
    }

    /// Fills the typed fields from the settings.
    fn refresh_texts(&mut self) {
        if let Some(video) = &self.settings.video {
            if let Resolution::Custom { width, height } = video.resolution {
                self.width_text = width.to_string();
                self.height_text = height.to_string();
            }
            if let RateControl::Bitrate { kbps } = video.rate_control {
                self.kbps_text = kbps.to_string();
            }
        }
    }

    fn apply(&mut self, preset: &ExportPreset) {
        self.settings = export::apply_preset(&self.settings, preset);
        self.preset = Some(preset.id.clone());
        self.refresh_texts();
    }

    /// The range the settings carry, from the choice and what is typed or
    /// selected. A problem is returned rather than kept.
    fn follow_range(&mut self, input: &ExportSheetInput) -> Option<String> {
        let rate = input.project.settings.frame_rate;
        match self.range {
            RangeChoice::Timeline => {
                self.settings.range = ExportRange::Timeline;
                None
            }
            RangeChoice::Selection => match input.selection {
                Some((start, end)) => {
                    self.settings.range = ExportRange::Span { start, end };
                    None
                }
                None => Some("No clips are selected".to_owned()),
            },
            RangeChoice::Custom => {
                match (
                    timefmt::parse(&self.start_text, rate),
                    timefmt::parse(&self.end_text, rate),
                ) {
                    (Some(start), Some(end)) if end > start => {
                        self.settings.range = ExportRange::Span { start, end };
                        None
                    }
                    (Some(_), Some(_)) => Some("The range ends before it starts".to_owned()),
                    _ => Some(
                        "Type the start and end as times, such as 0:12 or 00:00:12:15".to_owned(),
                    ),
                }
            }
        }
    }

    /// What is typed that cannot be used, first problem first.
    fn typed_problem(&self) -> Option<String> {
        if let Some(problem) = name_problem(&self.name) {
            return Some(problem.to_owned());
        }
        let video = self.settings.video.as_ref()?;
        if matches!(video.resolution, Resolution::Custom { .. })
            && (self.width_text.trim().parse::<u32>().is_err()
                || self.height_text.trim().parse::<u32>().is_err())
        {
            return Some("Width and height are whole numbers of pixels".to_owned());
        }
        if matches!(video.rate_control, RateControl::Bitrate { .. })
            && self.kbps_text.trim().parse::<u32>().is_err()
        {
            return Some("The bitrate is a whole number of kbit/s".to_owned());
        }
        None
    }

    fn set_video(&mut self, on: bool, caps: &Capabilities) {
        if on == self.settings.video.is_some() {
            return;
        }
        if on {
            let (container, video) = self.parked_video.take().unwrap_or_else(|| {
                let container = (Container::VIDEO.into_iter())
                    .find(|c| caps.has_container(*c))
                    .unwrap_or(Container::Mp4);
                (container, VideoSettings::default())
            });
            self.settings.container = container;
            self.settings.video = Some(video);
        } else {
            let video = self.settings.video.take().unwrap_or_default();
            self.parked_video = Some((self.settings.container, video));
            if self.settings.audio.is_none() {
                self.settings.audio = Some(self.parked_audio.take().unwrap_or_default());
            }
            let codec = self.settings.audio.map(|audio| audio.codec);
            // The sound-only file that carries the sound as it is, else the
            // first this host writes.
            let usable = |c: &Container| caps.has_container(*c);
            self.settings.container = (Container::AUDIO.into_iter())
                .filter(usable)
                .find(|c| codec.is_some_and(|codec| c.audio_codecs().contains(&codec)))
                .or_else(|| Container::AUDIO.into_iter().find(usable))
                .unwrap_or(Container::M4a);
        }
        settle(&mut self.settings, caps);
        self.refresh_texts();
    }

    fn set_audio(&mut self, on: bool, caps: &Capabilities) {
        if on == self.settings.audio.is_some() {
            return;
        }
        if on {
            self.settings.audio = Some(self.parked_audio.take().unwrap_or_default());
        } else {
            self.parked_audio = self.settings.audio.take();
        }
        settle(&mut self.settings, caps);
    }
}

/// Moves what the container cannot carry, or this host cannot encode, onto
/// the nearest choice it can; what it can is kept.
fn settle(settings: &mut ExportSettings, caps: &Capabilities) {
    let container = settings.container;
    if let Some(video) = settings.video.as_mut() {
        let carried = container.video_codecs();
        if !carried.contains(&video.codec) || !caps.has_video_codec(video.codec) {
            video.codec = (carried.iter().copied())
                .find(|c| caps.has_video_codec(*c))
                .or_else(|| carried.first().copied())
                .unwrap_or_default();
        }
        if video.codec == VideoCodec::ProRes
            && matches!(video.rate_control, RateControl::Bitrate { .. })
        {
            video.rate_control = RateControl::default();
        }
    }
    if let Some(audio) = settings.audio.as_mut() {
        let carried = container.audio_codecs();
        if !carried.contains(&audio.codec) || !caps.has_audio_codec(audio.codec) {
            let codec = (carried.iter().copied())
                .find(|c| caps.has_audio_codec(*c))
                .or_else(|| carried.first().copied())
                .unwrap_or(AudioCodec::Aac);
            if codec != audio.codec {
                audio.codec = codec;
                audio.kbps = codec.default_kbps();
            }
        }
        if !audio.codec.accepts_rate(audio.sample_rate) {
            audio.sample_rate = (SAMPLE_RATES.into_iter())
                .rev()
                .filter(|rate| *rate <= 48_000)
                .find(|rate| audio.codec.accepts_rate(*rate))
                .unwrap_or(48_000);
        }
        audio.channels = audio.channels.clamp(1, 2);
        if audio.codec.has_bitrate() {
            audio.kbps = (audio.kbps).clamp(MIN_AUDIO_KBPS, audio.codec.max_kbps(audio.channels));
        }
    }
    if container.embedded_subtitles().is_none() || settings.video.is_none() {
        settings.subtitles.embed = false;
    }
}

/// Moves settings this host cannot make onto ones it can: a container it
/// has no muxer for, or one of the wrong kind, then the codecs.
fn fit_to(settings: &mut ExportSettings, caps: &Capabilities) {
    let video = settings.video.is_some();
    if !caps.has_container(settings.container) || settings.container.has_video() != video {
        let pool: &[Container] = if video {
            &Container::VIDEO
        } else {
            &Container::AUDIO
        };
        if let Some(container) = pool.iter().copied().find(|c| caps.has_container(*c)) {
            settings.container = container;
        }
    }
    settle(settings, caps);
}

/// The file name without an extension a container would add.
fn stem_of(name: &str) -> String {
    let name = name.trim();
    if let Some((stem, extension)) = name.rsplit_once('.')
        && !stem.is_empty()
        && Container::from_extension(extension).is_some()
    {
        return stem.trim_end().to_owned();
    }
    name.to_owned()
}

/// Why a typed name cannot name a file on every system the editor runs on.
fn name_problem(name: &str) -> Option<&'static str> {
    let stem = stem_of(name);
    if stem.is_empty() || stem == "." || stem == ".." {
        return Some("Name the file");
    }
    if stem.chars().any(|c| {
        c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
    }) {
        return Some("A file name cannot contain / \\ : * ? \" < > |");
    }
    None
}

/// Why this host cannot make a preset, if it cannot.
fn preset_problem(preset: &ExportPreset, caps: &Capabilities) -> Option<String> {
    let settings = &preset.settings;
    if !caps.has_container(settings.container) {
        return Some(format!(
            "This FFmpeg cannot write {} files",
            settings.container.label()
        ));
    }
    if let Some(video) = &settings.video
        && !caps.has_video_codec(video.codec)
    {
        return Some(format!("No {} encoder works here", video.codec.label()));
    }
    if let Some(audio) = &settings.audio
        && !caps.has_audio_codec(audio.codec)
    {
        return Some(format!("No {} encoder works here", audio.codec.label()));
    }
    None
}

/// The preset the settings are: the one last applied while it still
/// matches, else the first that matches.
fn matched<'a>(
    settings: &ExportSettings,
    chosen: Option<&str>,
    presets: &'a [ExportPreset],
) -> Option<&'a ExportPreset> {
    let same = |preset: &&ExportPreset| export::apply_preset(settings, preset) == *settings;
    chosen
        .and_then(|id| presets.iter().find(|preset| preset.id == id))
        .filter(same)
        .or_else(|| presets.iter().find(same))
}

fn rate_label(rate: FrameRate) -> String {
    RATES
        .iter()
        .find(|(n, d, _)| rate.numerator == *n && rate.denominator == *d)
        .map_or_else(
            || {
                let fps = format!("{:.3}", rate.fps());
                fps.trim_end_matches('0').trim_end_matches('.').to_owned()
            },
            |(_, _, label)| (*label).to_owned(),
        )
}

/// `16:9`, or `1.85:1` when the reduced sides are not small numbers.
fn aspect(size: [u32; 2]) -> String {
    fn gcd(a: u32, b: u32) -> u32 {
        if b == 0 { a } else { gcd(b, a % b) }
    }
    let [w, h] = size.map(|s| s.max(1));
    let g = gcd(w, h);
    let (a, b) = (w / g, h / g);
    if a <= 32 && b <= 32 {
        format!("{a}:{b}")
    } else if w >= h {
        format!("{:.2}:1", w as f64 / h as f64)
    } else {
        format!("1:{:.2}", h as f64 / w as f64)
    }
}

fn hardware_of(name: &str) -> Option<HardwareKind> {
    export::known_video_encoder(name).and_then(|known| known.hardware)
}

/// What an encoder is called in the summary: "NVIDIA NVENC · h264_nvenc".
fn encoder_label(hardware: Option<HardwareKind>, name: &str) -> String {
    format!(
        "{} · {name}",
        hardware.map_or("Software", HardwareKind::label)
    )
}

/// Why a codec is not offered: the failed test of its encoder when there
/// was one.
fn codec_reason(caps: &Capabilities, codec: VideoCodec) -> String {
    let failed = (caps.unavailable.iter())
        .find(|u| export::known_video_encoder(&u.name).is_some_and(|k| k.codec == codec));
    match failed {
        Some(failed) => format!("{} failed its test here: {}", failed.name, failed.reason),
        None => format!("This FFmpeg has no {} encoder", codec.label()),
    }
}

/// Whether an encoder choice can encode `codec` at `size`, and why not.
fn choice_reason(
    caps: &Capabilities,
    choice: EncoderChoice,
    codec: VideoCodec,
    size: [u32; 2],
) -> Option<String> {
    if caps.video_encoder(codec, choice, size).is_some() {
        return None;
    }
    let family = |e: &&export::VideoEncoder| match choice {
        EncoderChoice::Hardware(kind) => e.hardware == Some(kind),
        EncoderChoice::Software => e.hardware.is_none(),
        EncoderChoice::Auto => true,
    };
    let name = match choice {
        EncoderChoice::Hardware(kind) => kind.label(),
        _ => "Software",
    };
    if caps.video.iter().filter(family).any(|e| e.codec == codec) {
        return Some(format!(
            "{name} cannot encode {} × {} {}",
            size[0],
            size[1],
            codec.label()
        ));
    }
    let failed = (caps.unavailable.iter()).find(|u| {
        export::known_video_encoder(&u.name).is_some_and(|k| {
            k.codec == codec
                && match choice {
                    EncoderChoice::Hardware(kind) => k.hardware == Some(kind),
                    _ => k.hardware.is_none(),
                }
        })
    });
    Some(match failed {
        Some(failed) => format!("{} failed its test here: {}", failed.name, failed.reason),
        None => format!("{name} has no {} encoder here", codec.label()),
    })
}

// ---- The sheet ----------------------------------------------------------------

/// The sheet at its largest; a smaller window keeps 12 points clear of it
/// on every side.
const WIDTH: f32 = 760.0;
const HEIGHT: f32 = 640.0;
/// Terminal's sheet header and footer.
const HEADER: f32 = 52.0;
const FOOTER: f32 = 60.0;
/// The sheet's margin, which is also the room between its columns.
const PAD: f32 = 20.0;
/// A row of an options card: a 30-point control with 5 points about it.
const ROW: f32 = 40.0;
/// A line of the summary.
const FACT: f32 = 32.0;
/// The width of the control at the trailing end of a row, so that a card's
/// controls start together: 20 more than a select in Terminal's settings,
/// which the longest of today's values, "Auto (Apple VideoToolbox)", needs
/// to be shown whole.
const TRAIL: f32 = 200.0;
/// From one card to the label of the next.
const GAP: f32 = 14.0;
/// The tallest a select's list is before it scrolls (spec §4.3.4).
const LIST: f32 = 260.0;
/// A section's label over its card, Terminal's: its words 12 over the card.
const LABEL: f32 = 24.0;
/// A label with the switch that turns its card on: the words as far over
/// the card as any label's, and the 22-point switch 6 clear of the card, as
/// a control is of the edge of its row.
const SWITCHED: f32 = 34.0;

/// Terminal's sheet (`controls::sheet`) under the id the editor has always
/// given it, which the tests and the notices find it by: `elevated`, with a
/// hairline border and the sheet shadow, `width` wide, its rows laid edge to
/// edge. The window behind it is dimmed in the window's own rounded shape
/// rather than egui's square backdrop, fading in as a sheet first shows.
pub(super) fn scaffold<R>(
    ctx: &egui::Context,
    p: &Palette,
    id: Id,
    title: &str,
    width: f32,
    add: impl FnOnce(&mut Ui) -> R,
) -> egui::ModalResponse<R> {
    // Painted first into the sheet's own layer: under the sheet, and over
    // everything it covers, menus included.
    let layer = egui::LayerId::new(egui::Order::Foreground, id);
    controls::scrim(
        &ctx.layer_painter(layer),
        ctx.content_rect(),
        window::corner_radius(ctx),
        *p,
        scrim_opacity(ctx),
    );
    let frame = egui::Frame::new()
        .fill(p.elevated)
        .corner_radius(metrics::SHEET_RADIUS)
        .stroke(Stroke::new(1.0, p.border))
        .shadow(p.sheet_shadow());
    let modal = egui::Modal::new(id)
        .frame(frame)
        .backdrop_color(Color32::TRANSPARENT)
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            add(ui)
        });
    modal
        .response
        .widget_info(|| WidgetInfo::labeled(WidgetType::Window, true, title));
    modal
}

/// How far the scrim has come in: from nothing as the export sheet or the
/// render queue first shows, over the sheet's 0.16 s, and at once under
/// Reduce motion. Going from one of the two to the other it stays.
fn scrim_opacity(ctx: &egui::Context) -> f32 {
    let id = Id::new("export-scrim");
    let pass = ctx.cumulative_pass_nr();
    let now = ctx.input(|input| input.time);
    let since = ctx.data_mut(|data| {
        // Shown in this pass or the one before: the same showing.
        let since = (data.get_temp::<(u64, f64)>(id))
            .filter(|(shown, _)| shown + 1 >= pass)
            .map_or(now, |(_, since)| since);
        data.insert_temp(id, (pass, since));
        since
    });
    if kit::reduced_motion(ctx) {
        return 1.0;
    }
    let t = ((now - since) as f32 / theme::motion::SHEET).clamp(0.0, 1.0);
    if t < 1.0 {
        ctx.request_repaint();
    }
    egui::emath::easing::cubic_out(t)
}

/// A part of a sheet's body that scrolls: `width` of content at the leading
/// edge of `rect`, whose scroll bar floats in the room beside it, clear of
/// what it holds. An edge that content runs on past fades into `ground`.
/// When `afresh`, as the sheet opens, it shows its top again, wherever it
/// was left.
pub(super) fn scrolled(
    ui: &mut Ui,
    ground: Color32,
    rect: Rect,
    width: f32,
    salt: &str,
    afresh: bool,
    add: impl FnOnce(&mut Ui),
) {
    let mut part = ui.new_child(
        UiBuilder::new()
            .id_salt(salt)
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
    );
    part.spacing_mut().scroll.floating_allocated_width = 0.0;
    let mut area = egui::ScrollArea::vertical()
        .id_salt(salt)
        .auto_shrink([false, false])
        .max_height(rect.height());
    if afresh {
        area = area.vertical_scroll_offset(0.0);
    }
    let output = area.show(&mut part, |ui| {
        super::sheets::column(ui, width, |ui| {
            ui.add_space(4.0);
            add(ui);
            ui.add_space(PAD);
        });
    });
    faded(ui, &output, ground);
}

/// Fades into `ground` each edge of what a scroll area shows that its
/// content runs on past, so that a line cut there fades out rather than
/// being sliced.
fn faded<R>(ui: &Ui, output: &egui::scroll_area::ScrollAreaOutput<R>, ground: Color32) {
    let shown = output.inner_rect;
    let above = output.state.offset.y;
    let below = output.content_size.y - above - shown.height();
    let painter = ui.painter_at(shown);
    if above > 0.5 {
        kit::scroll_fade(&painter, shown, kit::Edge::Top, ground);
    }
    if below > 0.5 {
        kit::scroll_fade(&painter, shown, kit::Edge::Bottom, ground);
    }
}

/// A sheet's 52-point title row: the title at its leading edge and, at the
/// trailing one, `close` named `close_name`, with what `more` adds before
/// it, right to left, each 28 points square.
pub(super) fn header(
    ui: &mut Ui,
    p: &Palette,
    title: &str,
    close_name: &str,
    more: impl FnOnce(&mut Ui),
) -> bool {
    let (_, rect) = ui.allocate_space(vec2(ui.available_width(), HEADER));
    let words = ui.painter().text(
        pos2(rect.left() + PAD, rect.center().y),
        Align2::LEFT_CENTER,
        title,
        theme::semibold(15.0),
        p.fg,
    );
    said(ui, words, title);
    // The close control's centre is 26 in from the edge, as in Terminal.
    let controls = Rect::from_min_max(
        pos2(rect.left() + PAD, rect.top()),
        pos2(rect.right() - 12.0, rect.bottom()),
    );
    let mut closed = false;
    controls::place(
        ui,
        controls,
        Layout::right_to_left(Align::Center),
        ("sheet-header", title),
        |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            // What comes before the close control is made first, so that Tab
            // reaches it first.
            let at = ui.cursor();
            let mut before = ui.new_child(
                UiBuilder::new()
                    .max_rect(Rect::from_min_max(
                        controls.left_top(),
                        pos2(at.right() - 28.0 - 4.0, controls.bottom()),
                    ))
                    .layout(Layout::right_to_left(Align::Center)),
            );
            before.spacing_mut().item_spacing.x = 4.0;
            more(&mut before);
            closed = widgets::icon_button(ui, p, Icon::Close, close_name).clicked();
        },
    );
    closed
}

/// Words that are only painted, named for those who do not see them as the
/// labels they replaced were: `text` over `rect`, where it is drawn.
pub(super) fn said(ui: &Ui, rect: Rect, text: &str) {
    ui.interact(rect, ui.auto_id_with(("said", text)), Sense::hover())
        .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, text));
}

/// The label over a card, set in 4 from it as Terminal sets it. Returns
/// its row, for a switch at its trailing end.
fn section(ui: &mut Ui, p: &Palette, text: &str) -> Rect {
    section_of(ui, p, text, LABEL)
}

/// The label over a card in a row `height` tall, as far over the card
/// whatever room the row has over it.
fn section_of(ui: &mut Ui, p: &Palette, text: &str, height: f32) -> Rect {
    let (_, rect) = ui.allocate_space(vec2(ui.available_width(), height));
    let words = ui.painter().text(
        pos2(rect.left() + 4.0, rect.bottom() - LABEL * 0.5),
        Align2::LEFT_CENTER,
        text,
        theme::medium(11.5),
        p.secondary,
    );
    said(ui, words, text);
    rect
}

/// The switch that turns a whole card on, at the end of the card's label
/// where the card's own controls end.
fn section_switch(
    ui: &mut Ui,
    p: &Palette,
    label: Rect,
    on: &mut bool,
    name: &str,
    enabled: bool,
) -> egui::Response {
    let at = Rect::from_min_size(
        pos2(label.right() - 12.0 - 38.0, label.bottom() - 6.0 - 22.0),
        vec2(38.0, 22.0),
    );
    controls::place(
        ui,
        at,
        Layout::left_to_right(Align::Center),
        ("section-switch", name),
        |ui| {
            ui.add_enabled_ui(enabled, |ui| controls::toggle(ui, *p, on, name))
                .inner
        },
    )
}

/// A row of an options card: its label at the leading edge and what `add`
/// makes at the trailing one, given the room the label leaves it.
fn row<R>(
    ui: &mut Ui,
    p: &Palette,
    group: &mut Group,
    label: &str,
    add: impl FnOnce(&mut Ui, f32) -> R,
) -> R {
    let lead = label_width(ui, p, label);
    row_after(ui, p, group, label, lead, add)
}

/// How wide a row's label is.
fn label_width(ui: &Ui, p: &Palette, label: &str) -> f32 {
    (ui.painter())
        .layout_no_wrap(label.to_owned(), theme::regular(13.0), p.fg)
        .size()
        .x
}

/// A row whose control is given the room after `lead`, the widest label of
/// the rows it lines up with, so that their controls start together.
fn row_after<R>(
    ui: &mut Ui,
    p: &Palette,
    group: &mut Group,
    label: &str,
    lead: f32,
    add: impl FnOnce(&mut Ui, f32) -> R,
) -> R {
    let words = label_width(ui, p, label);
    group.row_with_height(ui, label, ROW, |ui| {
        // The label is 2 in from where the controls' room starts.
        let row = ui.max_rect();
        said(
            ui,
            Rect::from_min_size(pos2(row.left() + 2.0, row.top()), vec2(words, row.height())),
            label,
        );
        let room = (ui.available_width() - 2.0 - lead.max(words) - 16.0).max(40.0);
        add(ui, room)
    })
}

/// A row whose control takes the card's width: under its label when it has
/// one, else alone.
fn wide<R>(
    ui: &mut Ui,
    p: &Palette,
    group: &mut Group,
    label: Option<&str>,
    salt: &str,
    add: impl FnOnce(&mut Ui, f32) -> R,
) -> R {
    let height = if label.is_some() { 68.0 } else { ROW };
    let rect = group.custom(ui, height);
    if let Some(label) = label {
        let words = ui.painter().text(
            pos2(rect.left() + 14.0, rect.top() + 20.0),
            Align2::LEFT_CENTER,
            label,
            theme::regular(13.0),
            p.fg,
        );
        said(ui, words, label);
    }
    let control = Rect::from_min_max(
        pos2(rect.left() + 12.0, rect.bottom() - 6.0 - 28.0),
        pos2(rect.right() - 12.0, rect.bottom() - 6.0),
    );
    controls::place(
        ui,
        control,
        Layout::left_to_right(Align::Center),
        ("wide-row", salt),
        |ui| add(ui, control.width()),
    )
}

/// One of a few choices, as a segmented control whose cells Left and Right
/// move among, choosing as they go (Terminal). Whether the choice changed.
fn choice<T: PartialEq + Copy>(
    ui: &mut Ui,
    p: &Palette,
    value: &mut T,
    options: &[(T, &str)],
    width: f32,
) -> bool {
    // The cells' ids, as the segmented control makes them.
    let name = (options.iter())
        .map(|(_, label)| *label)
        .collect::<Vec<_>>()
        .join("|");
    let base = ui.id().with(("segmented", name.as_str()));
    let cells: Vec<Id> = options.iter().map(|(_, label)| base.with(*label)).collect();
    let selected = (options.iter())
        .position(|(option, _)| option == value)
        .unwrap_or(0);
    let mut changed = false;
    if let Some(to) = super::arrow_choice(ui.ctx(), &cells, selected) {
        *value = options[to].0;
        changed = true;
    }
    widgets::segmented(ui, p, value, options, width) || changed
}

/// A select over `options`, the one equal to `current` checked, each offered
/// only when its reason is `None` and otherwise disabled with the reason on
/// hover. Its rows are buttons. The option taken, if one was.
#[allow(clippy::too_many_arguments)]
fn list<T: PartialEq + Copy>(
    ui: &mut Ui,
    p: &Palette,
    id: &str,
    name: &str,
    shown: &str,
    width: f32,
    current: T,
    options: &[(T, String, Option<String>)],
) -> Option<T> {
    let select = kit::select(
        ui,
        p,
        Id::new(id),
        shown,
        name,
        vec2(width, metrics::CONTROL_HEIGHT),
    );
    let mut taken = None;
    egui::Popup::menu(&select).show(|ui| {
        widgets::menu_style(ui, width.max(240.0));
        for (option, text, reason) in options {
            if offer(ui, p, text, *option == current, reason.as_deref()) {
                taken = Some(*option);
            }
        }
    });
    taken
}

/// A row of a select's list, checked when it is the current choice, and
/// disabled with `reason` on hover. Whether it was taken.
fn offer(ui: &mut Ui, p: &Palette, text: &str, current: bool, reason: Option<&str>) -> bool {
    let response = ui
        .add_enabled_ui(reason.is_none(), |ui| choice_row(ui, p, text, current))
        .inner;
    match reason {
        Some(reason) => {
            response.on_disabled_hover_text(reason);
            false
        }
        None => response.clicked(),
    }
}

/// A row of a select's list: a button, checked when it is the current
/// choice and announced as chosen, as the selectable rows it replaced were.
fn choice_row(ui: &mut Ui, p: &Palette, text: &str, current: bool) -> egui::Response {
    let check = current.then_some(Icon::Check);
    let response = kit::menu_row(ui, p, check, text, "", false, None);
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, current, text));
    response
}

/// A field on the control material, `width` wide, its text in a 13-point
/// editor named `name` for accessibility, and `suffix` (a unit or an
/// extension) set quietly inside its trailing end. Text that cannot be used
/// outlines it in the problem colour.
#[allow(clippy::too_many_arguments)]
fn field(
    ui: &mut Ui,
    p: &Palette,
    id: Id,
    text: &mut String,
    width: f32,
    hint: &str,
    name: &str,
    suffix: Option<&str>,
    valid: bool,
) -> egui::Response {
    let (_, rect) = ui.allocate_space(vec2(width, metrics::CONTROL_HEIGHT));
    controls::field_frame(ui, *p, rect, id, valid);
    let unit = suffix.map(|suffix| {
        let galley =
            (ui.painter()).layout_no_wrap(suffix.to_owned(), theme::regular(13.0), p.muted);
        let at = pos2(
            rect.right() - 10.0 - galley.size().x,
            rect.center().y - galley.size().y * 0.5,
        );
        let shown = Rect::from_min_size(at, galley.size());
        ui.painter().galley(at, galley, p.muted);
        (suffix, shown)
    });
    let after = unit.map_or(0.0, |(_, shown)| shown.width() + 4.0);
    let inner = Rect::from_min_max(
        pos2(rect.left() + 10.0, rect.top()),
        pos2(rect.right() - 10.0 - after, rect.bottom()),
    );
    let response = controls::place(
        ui,
        inner,
        Layout::left_to_right(Align::Center),
        ("field", id),
        |ui| controls::bare_text_edit(ui, id, text, hint, 13.0, inner.width()),
    );
    // Read after what is typed, where it is seen.
    if let Some((suffix, shown)) = unit {
        said(ui, shown, suffix);
    }
    response.widget_info(|| WidgetInfo::labeled(WidgetType::TextEdit, ui.is_enabled(), name));
    response
}

/// Supporting words under a card, wrapped.
fn note(ui: &mut Ui, text: &str, color: Color32) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(text)
                    .font(theme::regular(11.5))
                    .color(color),
            )
            .wrap(),
        );
    });
}

/// A line of the summary: what, and its value at the trailing edge, named
/// together for those who do not see them.
fn fact(ui: &mut Ui, p: &Palette, group: &mut Group, label: &str, value: &str) {
    let rect = group.custom(ui, FACT);
    let painter = ui.painter().clone();
    let words = controls::galley_at(
        &painter,
        pos2(rect.left() + 14.0, rect.center().y),
        painter.layout_no_wrap(label.to_owned(), theme::regular(12.5), p.secondary),
    );
    let room = rect.right() - 14.0 - words.right() - 12.0;
    let galley = controls::elided(&painter, value, theme::regular(12.5), p.fg, room);
    painter.galley(
        pos2(
            rect.right() - 14.0 - galley.size().x,
            rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        p.fg,
    );
    ui.interact(rect, ui.id().with(("fact", label)), Sense::hover())
        .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, format!("{label}: {value}")));
}

/// A problem or a caution, led by the warning glyph in `color`: the words
/// of a problem in it too, a caution's in the quieter ink.
fn caution(ui: &mut Ui, p: &Palette, text: &str, color: Color32) {
    ui.add_space(8.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add_space(4.0);
        let (glyph, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        icons::paint(
            ui.painter(),
            glyph.translate(vec2(0.0, 1.0)),
            Icon::Warning,
            color,
        );
        ui.add(
            egui::Label::new(
                egui::RichText::new(text)
                    .font(theme::regular(12.0))
                    .color(if color == p.red { p.red } else { p.secondary }),
            )
            .wrap(),
        );
    });
}

/// Shows the export sheet. Returns what the person asked for, if anything;
/// Escape and the backdrop close it, an Enter pressed afresh exports, and the
/// application keeps `state` to show again.
pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    state: &mut ExportSheetState,
    input: &ExportSheetInput,
) -> Option<ExportAnswer> {
    let mut answer = None;
    let caps = match input.caps {
        CapabilityView::Ready(caps) => Some(caps),
        _ => None,
    };
    if let Some(caps) = caps
        && !state.fitted
    {
        fit_to(&mut state.settings, caps);
        state.fitted = true;
    }
    let range_problem = state.follow_range(input);
    let before = state.settings.clone();
    let named = (state.name.clone(), state.folder.clone());
    let plan = caps.map(|caps| state.settings.plan(input.project, caps));
    let blocker = blocker(state, input, range_problem.as_deref(), plan.as_ref());

    // Export waited for a folder, which has come.
    if let Some(start_now) = state.pending
        && let Some(destination) = state.destination()
    {
        state.pending = None;
        if blocker.is_none() {
            answer = Some(ExportAnswer::Queue {
                settings: state.settings.clone(),
                destination,
                start_now,
            });
        }
    }

    // A field being typed in is the innermost interaction: Escape leaves the
    // field, and only the next one closes the sheet. egui lets go of the
    // focus as the frame begins, so whether one was typed in is kept from
    // the frame before.
    let typing_id = Id::new("export-sheet-typing");
    let typing = ctx.data(|data| data.get_temp(typing_id).unwrap_or(false));
    // Enter exports, as the sheet's one Primary, when it is pressed afresh
    // while nothing has the keyboard: not one carried over from what was
    // there before the sheet, held down, or meant for a field or a button.
    let opening = opening(ctx, Id::new("export-sheet"));
    let enter = fresh_enter(ctx, opening) && ctx.memory(|memory| memory.focused()).is_none();
    let screen = ctx.content_rect();
    let size = vec2(
        WIDTH.min(screen.width() - 24.0).max(240.0),
        HEIGHT
            .min(screen.height() - 24.0)
            .max(HEADER + FOOTER + 120.0),
    );
    let response = scaffold(ctx, p, Id::new("export-sheet"), "Export", size.x, |ui| {
        // Escape gives up a preset name being typed before it closes the
        // sheet.
        if state.naming.is_some()
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            state.naming = None;
        }
        let queue = input.queue_len;
        if header(ui, p, "Export", "Close export", |ui| {
            if queue > 0 {
                let label = match queue {
                    1 => "Render queue (1 job)".to_owned(),
                    n => format!("Render queue ({n} jobs)"),
                };
                if widgets::icon_button(ui, p, Icon::List, &label).clicked() {
                    answer = Some(ExportAnswer::OpenQueue);
                }
            }
        }) {
            answer = Some(ExportAnswer::Close);
        }
        let (_, body) = ui.allocate_space(vec2(size.x, size.y - HEADER - FOOTER));
        let mut sheet = Sheet {
            p,
            input,
            caps,
            plan: plan.as_ref(),
            range_problem: range_problem.as_deref(),
            answer: &mut answer,
            opening,
        };
        sheet.body(ui, body, state);
        let facts = (plan.as_ref())
            .and_then(|plan| plan.as_ref().ok())
            .map(brief);
        footer(
            ui,
            p,
            state,
            input,
            blocker.as_deref(),
            facts.as_deref(),
            &mut answer,
        );
    });
    if state.settings != before {
        // What is shown beside the control that changed was laid out from
        // the settings before it: show it again from the new ones.
        ctx.request_repaint();
        if matched(&state.settings, state.preset.as_deref(), input.presets)
            .is_none_or(|preset| Some(preset.id.as_str()) != state.preset.as_deref())
        {
            state.preset = None;
        }
    }
    let escape = response.is_top_modal
        && !response.any_popup_open
        && !typing
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let focused = ctx.text_edit_focused();
    ctx.data_mut(|data| data.insert_temp(typing_id, focused));
    if answer.is_none() && (response.backdrop_response.clicked() || escape) {
        answer = Some(ExportAnswer::Close);
    }
    if answer.is_none()
        && enter
        && blocker.is_none()
        && state.naming.is_none()
        && response.is_top_modal
        && !response.any_popup_open
    {
        answer = Some(queued(state, true));
    }
    // A refusal answers what was last asked; it goes once the person
    // changes what would be made, or asks again.
    if answer.is_some()
        || state.settings != before
        || (state.name.clone(), state.folder.clone()) != named
    {
        state.refusal = None;
    }
    answer
}

/// Whether the sheet `id` is shown in this frame for the first time since it
/// was last away: the frame whose keys were meant for what was there before.
fn opening(ctx: &egui::Context, id: Id) -> bool {
    let frame = ctx.cumulative_frame_nr();
    let key = id.with("shown");
    let seen = ctx.data(|data| data.get_temp::<(u64, u64)>(key));
    let first = match seen {
        Some((first, last)) if last == frame || last + 1 == frame => first,
        _ => frame,
    };
    ctx.data_mut(|data| data.insert_temp(key, (first, frame)));
    first == frame
}

/// An Enter pressed in this frame, and none that was held down or pressed
/// as the sheet opened.
fn fresh_enter(ctx: &egui::Context, opening: bool) -> bool {
    let (fresh, carried) = ctx.input(|input| {
        (input.events.iter()).fold((false, false), |(fresh, carried), event| match event {
            egui::Event::Key {
                key: egui::Key::Enter,
                pressed: true,
                repeat,
                ..
            } => {
                let old = *repeat || opening;
                (fresh || !old, carried || old)
            }
            _ => (fresh, carried),
        })
    });
    fresh && !carried
}

/// The export as it stands, to start now or to wait in the queue; or, with
/// no folder yet, the question that asks for one, the export kept until it
/// comes.
fn queued(state: &mut ExportSheetState, start_now: bool) -> ExportAnswer {
    match state.destination() {
        Some(destination) => ExportAnswer::Queue {
            settings: state.settings.clone(),
            destination,
            start_now,
        },
        None => {
            state.pending = Some(start_now);
            ExportAnswer::ChooseFolder
        }
    }
}

/// What the file will be, in a few words for the footer, whatever the body
/// is scrolled to: "≈ 1.9 MB · 0:05 · 1280 × 720".
fn brief(plan: &ExportPlan) -> String {
    let mut parts = vec![
        format!(
            "≈ {}",
            timefmt::file_size(ExportSettings::estimate_bytes(plan))
        ),
        timefmt::short(plan.end - plan.start),
    ];
    if let Some(video) = &plan.video {
        let [width, height] = video.geometry.output;
        parts.push(format!("{width} × {height}"));
    }
    parts.join(" · ")
}

/// `text` in 12 points on one line of at most `width`, its figures in the
/// monospace face so that a changing estimate does not shift what follows.
fn figures(text: &str, color: Color32, width: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let figure = |c: char| c.is_ascii_digit() || c == '.' || c == ':';
    let mut start = 0;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (at, (_, c)) in chars.iter().enumerate() {
        let next = chars.get(at + 1);
        if next.is_none_or(|(_, next)| figure(*next) != figure(*c)) {
            let end = next.map_or(text.len(), |(index, _)| *index);
            let font = if figure(*c) {
                theme::mono(12.0)
            } else {
                theme::regular(12.0)
            };
            job.append(
                &text[start..end],
                0.0,
                egui::TextFormat::simple(font, color),
            );
            start = end;
        }
    }
    job.wrap = egui::text::TextWrapping {
        max_width: width.max(0.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    job
}

/// Why the export cannot be queued as it stands.
fn blocker(
    state: &ExportSheetState,
    input: &ExportSheetInput,
    range_problem: Option<&str>,
    plan: Option<&Result<ExportPlan, ExportError>>,
) -> Option<String> {
    match input.caps {
        CapabilityView::Probing => return Some("Checking which encoders work here…".to_owned()),
        CapabilityView::Failed(problem) => {
            return Some(format!("The encoders could not be checked: {problem}"));
        }
        CapabilityView::Ready(_) => {}
    }
    if let Some(problem) = state.typed_problem() {
        return Some(problem);
    }
    if let Some(problem) = range_problem {
        return Some(problem.to_owned());
    }
    if let Some(Err(error)) = plan {
        return Some(error.to_string());
    }
    if input.queue_len >= QUEUE_LIMIT {
        return Some("The render queue is full; remove finished jobs first".to_owned());
    }
    None
}

/// The fixed bar under the body: what will be made, or the name of a preset
/// being kept, at the leading edge, and at the trailing one Cancel, the
/// other actions, then Export, the sheet's one accent.
fn footer(
    ui: &mut Ui,
    p: &Palette,
    state: &mut ExportSheetState,
    input: &ExportSheetInput,
    blocker: Option<&str>,
    facts: Option<&str>,
    answer: &mut Option<ExportAnswer>,
) {
    let bar = controls::sheet_footer(ui, *p).shrink2(vec2(PAD - 16.0, 0.0));
    controls::place(
        ui,
        bar,
        Layout::right_to_left(Align::Center),
        "export-footer",
        |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let ready = matches!(input.caps, CapabilityView::Ready(_));
            let reason = blocker.unwrap_or_default();
            let ok = blocker.is_none();
            // Made from the trailing edge in, the confirming action first,
            // which is also the order Tab takes.
            let export = ui
                .add_enabled_ui(ok, |ui| {
                    controls::button(ui, *p, "Export", ButtonKind::Primary)
                })
                .inner
                .on_hover_text("Queue it to render now, and show the render queue")
                .on_disabled_hover_text(reason);
            if export.clicked() && ok {
                *answer = Some(queued(state, true));
            }
            let add = ui
                .add_enabled_ui(ok, |ui| {
                    controls::button(ui, *p, "Add to queue", ButtonKind::Secondary)
                })
                .inner
                .on_hover_text("Queue it paused, to start from the render queue")
                .on_disabled_hover_text(reason);
            if add.clicked() && ok {
                *answer = Some(queued(state, false));
            }
            // While a preset is being named, its name has the leading edge
            // to itself, away from Cancel, which closes the whole sheet.
            if state.naming.is_none() {
                let own = matched(&state.settings, state.preset.as_deref(), input.presets)
                    .filter(|preset| preset.destination == Destination::Custom);
                if let Some(preset) = own
                    && kit::small_icon_button(
                        ui,
                        p,
                        Icon::Trash,
                        &format!("Delete preset “{}”", preset.name),
                    )
                    .clicked()
                {
                    *answer = Some(ExportAnswer::DeletePreset(preset.id.clone()));
                    state.preset = None;
                }
                let save = ui
                    .add_enabled_ui(ready, |ui| {
                        controls::button(ui, *p, "Save preset…", ButtonKind::Secondary)
                    })
                    .inner
                    .on_hover_text("Keep these settings as a preset of your own")
                    .on_disabled_hover_text("Available once the encoders have been checked");
                if save.clicked() {
                    state.naming = Some(String::new());
                    state.focus_naming = true;
                }
            }
            if controls::button(ui, *p, "Cancel", ButtonKind::Secondary).clicked() {
                *answer = Some(ExportAnswer::Close);
            }
            // What is left at the leading edge says what will be made.
            let room = Rect::from_min_max(
                bar.left_top(),
                pos2(bar.left() + ui.available_width() - 8.0, bar.bottom()),
            );
            if state.naming.is_some() {
                let mut lead = ui.new_child(
                    UiBuilder::new()
                        .max_rect(room)
                        .layout(Layout::left_to_right(Align::Center)),
                );
                naming(&mut lead, p, state, answer);
            } else if let Some(refusal) = state.refusal.as_deref() {
                refusal_line(ui, p, room, refusal);
            } else if matches!(input.caps, CapabilityView::Probing) {
                let glyph = Rect::from_center_size(
                    pos2(room.left() + 7.0, room.center().y),
                    Vec2::splat(14.0),
                );
                kit::spinner(ui, glyph, p.muted);
                let words = ui.painter().layout_job(figures(
                    "Checking encoders…",
                    p.secondary,
                    room.width() - 22.0,
                ));
                let shown = controls::galley_at(
                    ui.painter(),
                    pos2(room.left() + 22.0, room.center().y),
                    words,
                );
                said(ui, shown, "Checking encoders…");
            } else if let Some(facts) = facts {
                let words = (ui.painter()).layout_job(figures(facts, p.secondary, room.width()));
                let shown = controls::galley_at(ui.painter(), room.left_center(), words);
                said(ui, shown, facts);
            }
        },
    );
}

/// Why what a sheet asked for was refused, at its footer's leading edge in
/// the problem colour after the warning glyph, on at most two lines. The
/// whole of it is read out, and shown on hover where it is cut short.
pub(super) fn refusal_line(ui: &Ui, p: &Palette, room: Rect, text: &str) {
    let glyph = Rect::from_center_size(pos2(room.left() + 7.0, room.center().y), Vec2::splat(14.0));
    icons::paint(ui.painter(), glyph, Icon::Warning, p.red);
    let mut job = egui::text::LayoutJob::simple(
        text.to_owned(),
        theme::regular(12.0),
        p.red,
        (room.width() - 22.0).max(0.0),
    );
    job.wrap.max_rows = 2;
    job.wrap.break_anywhere = false;
    job.wrap.overflow_character = Some('…');
    let words = ui.painter().layout_job(job);
    let cut = words.elided;
    let shown = controls::galley_at(
        ui.painter(),
        pos2(room.left() + 22.0, room.center().y),
        words,
    );
    let response = ui.interact(shown, ui.auto_id_with(("refusal", text)), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, text));
    if cut {
        response.on_hover_text(text);
    }
}

/// A preset's name being typed in the footer, with its Save and a way to
/// stop, in reading order.
fn naming(
    ui: &mut Ui,
    p: &Palette,
    state: &mut ExportSheetState,
    answer: &mut Option<ExportAnswer>,
) {
    let save_width = (ui.painter())
        .layout_no_wrap("Save".to_owned(), theme::medium(13.0), p.fg)
        .size()
        .x
        .max(40.0)
        + 32.0;
    let width = 160.0 + 8.0 + save_width + 4.0 + metrics::SMALL_BUTTON;
    ui.allocate_ui_with_layout(
        vec2(width, metrics::CONTROL_HEIGHT),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let Some(name) = state.naming.as_mut() else {
                return;
            };
            let typed = field(
                ui,
                p,
                Id::new("export-preset-name"),
                name,
                160.0,
                "Preset name",
                "Preset name",
                None,
                true,
            );
            if std::mem::take(&mut state.focus_naming) {
                typed.request_focus();
            }
            let entered = typed.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let named = !name.trim().is_empty();
            let save = ui
                .add_enabled_ui(named, |ui| {
                    controls::button(ui, *p, "Save", ButtonKind::Secondary)
                })
                .inner
                .on_hover_text("Keep these settings as a preset")
                .on_disabled_hover_text("Name the preset");
            ui.spacing_mut().item_spacing.x = 4.0;
            if (save.clicked() || entered) && named {
                let mut settings = state.settings.clone();
                // A preset carries neither a range nor a caption track.
                settings.range = ExportRange::Timeline;
                settings.subtitles.track = None;
                *answer = Some(ExportAnswer::SavePreset {
                    name: name.trim().to_owned(),
                    settings,
                });
                state.naming = None;
            } else if kit::small_icon_button(ui, p, Icon::Close, "Stop naming the preset").clicked()
            {
                state.naming = None;
            }
        },
    );
}

/// What both columns read while they are drawn.
struct Sheet<'s, 'a> {
    p: &'s Palette,
    input: &'s ExportSheetInput<'a>,
    caps: Option<&'a Capabilities>,
    plan: Option<&'s Result<ExportPlan, ExportError>>,
    range_problem: Option<&'s str>,
    answer: &'s mut Option<ExportAnswer>,
    /// The sheet shows in this frame for the first time since it was away.
    opening: bool,
}

impl Sheet<'_, '_> {
    fn canvas(&self) -> [u32; 2] {
        let settings = &self.input.project.settings;
        [settings.width, settings.height]
    }

    fn rate(&self) -> FrameRate {
        self.input.project.settings.frame_rate
    }

    /// The body between the header and the footer: from 640 points of
    /// sheet, the picture and what will be made in a column beside the
    /// choices; narrower, one over the other. What does not fit scrolls.
    fn body(&mut self, ui: &mut Ui, body: Rect, state: &mut ExportSheetState) {
        let ground = self.p.elevated;
        // A sheet opens at its first card, however far down it was left.
        let afresh = self.opening;
        // A scrolled part reaches 6 points short of the edge beside it, so
        // that its bar floats in the margin, clear of what it holds.
        if body.width() >= TWO_COLUMNS {
            let split = body.left() + PAD + LEFT;
            let left = Rect::from_min_max(
                pos2(body.left() + PAD, body.top()),
                pos2(split + PAD - 6.0, body.bottom()),
            );
            scrolled(ui, ground, left, LEFT, "export-summary", afresh, |ui| {
                self.left(ui, body.height(), state);
            });
            let right = Rect::from_min_max(
                pos2(split + PAD, body.top()),
                pos2(body.right() - 6.0, body.bottom()),
            );
            let width = body.right() - PAD - right.left();
            scrolled(ui, ground, right, width, "export-options", afresh, |ui| {
                self.right(ui, state);
            });
        } else {
            let all = Rect::from_min_max(
                pos2(body.left() + PAD, body.top()),
                pos2(body.right() - 6.0, body.bottom()),
            );
            let width = body.width() - PAD * 2.0;
            scrolled(ui, ground, all, width, "export-stacked", afresh, |ui| {
                self.top(ui, state);
                ui.add_space(GAP);
                self.right(ui, state);
            });
        }
    }

    /// The column beside the choices: the picture, then what will be made,
    /// in a body `height` tall.
    fn left(&mut self, ui: &mut Ui, height: f32, state: &ExportSheetState) {
        // Shorter in a short sheet, so that the six lines of the summary
        // under it, with their label 14 under it, are seen with the room
        // the column leaves over and under what it holds.
        let under = GAP + LABEL + 6.0 * FACT;
        let most = (height - 4.0 - under - PAD).clamp(72.0, 170.0);
        let width = ui.available_width();
        self.poster(ui, state, width, most);
        ui.add_space(GAP);
        section(ui, self.p, "Summary");
        self.summary(ui, state);
    }

    /// Over the choices in a narrow sheet: the picture beside what will be
    /// made.
    fn top(&mut self, ui: &mut Ui, state: &ExportSheetState) {
        section(ui, self.p, "Summary");
        let width = ui.available_width();
        // 16:9 at its 100 points' height, or less of a narrow column.
        let poster = (width * 0.4).min(178.0);
        ui.horizontal_top(|ui| {
            self.poster(ui, state, poster, 100.0);
            ui.add_space(12.0);
            super::sheets::column(ui, ui.available_width(), |ui| self.summary(ui, state));
        });
    }

    /// The picture as it lands in the output, on the stage: in the output's
    /// shape, with the bars or the crop the fit makes, in a 16:9 well at
    /// most `width` wide and `most` tall, at the leading edge of the
    /// `width` it has, where the card under it starts.
    fn poster(&self, ui: &mut Ui, state: &ExportSheetState, width: f32, most: f32) {
        let p = self.p;
        let canvas = self.canvas();
        let video = state.settings.video.as_ref();
        let output = video.map_or(canvas, |video| video.resolution.size(canvas));
        // Narrowed rather than shortened, so that the bars it shows are the
        // fit's and not the well's.
        let height = (width * 9.0 / 16.0).min(most).round();
        let (_, room) = ui.allocate_space(vec2(width, height));
        let well = Rect::from_min_size(room.min, vec2((height * 16.0 / 9.0).round(), height));
        let response = ui.interact(well, ui.id().with("export-poster"), Sense::hover());
        let name = match (video, self.input.poster) {
            (Some(_), Some(_)) => format!(
                "Output frame at the playhead, {} × {}",
                output[0], output[1]
            ),
            (Some(_), None) => format!("Output frame, {} × {} (no preview)", output[0], output[1]),
            (None, _) => "Sound only: no picture".to_owned(),
        };
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Image, true, &name));
        if video.is_some()
            && self.input.poster.is_none()
            && let Some(problem) = self.input.poster_problem
        {
            response.on_hover_text(format!("No preview of the output: {problem}"));
        }
        if !ui.is_rect_visible(well) {
            return;
        }
        let painter = ui.painter_at(well);
        let radius = CornerRadius::same(metrics::CONTROL_RADIUS);
        painter.rect_filled(well, radius, p.stage());
        // The stage is dark in either appearance; so is what is drawn on it.
        let stage = Palette::stage_palette(Accent::default());
        let Some(video) = video else {
            icons::paint(
                &painter,
                Rect::from_center_size(well.center() - vec2(0.0, 9.0), Vec2::splat(22.0)),
                Icon::Music,
                stage.muted,
            );
            painter.text(
                well.center() + vec2(0.0, 15.0),
                Align2::CENTER_CENTER,
                "Sound only",
                theme::regular(12.0),
                stage.secondary,
            );
            return;
        };
        // The output's shape, as large as the well holds it.
        let ratio = output[0].max(1) as f32 / output[1].max(1) as f32;
        let size = if well.width() > well.height() * ratio {
            vec2(well.height() * ratio, well.height())
        } else {
            vec2(well.width(), well.width() / ratio)
        };
        let frame = Rect::from_center_size(well.center(), size);
        // Rounded with the well where it fills it; set in it, square.
        let round = if frame.width() >= well.width() - 1.0 && frame.height() >= well.height() - 1.0
        {
            radius
        } else {
            CornerRadius::ZERO
        };
        painter.rect_filled(frame, round, Color32::BLACK);
        let geometry = Geometry::new(canvas, output, video.fit);
        let k = frame.width() / output[0].max(1) as f32;
        let drawn = Rect::from_min_size(
            frame.min + vec2(geometry.offset[0] as f32, geometry.offset[1] as f32) * k,
            vec2(geometry.drawn[0] as f32, geometry.drawn[1] as f32) * k,
        );
        match self.input.poster {
            // The output picture itself: its bars and crop are drawn in it.
            Some((texture, _)) => {
                painter.add(
                    egui::epaint::RectShape::filled(frame, round, Color32::WHITE)
                        .with_texture(texture, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))),
                );
            }
            None => {
                let picture = drawn.intersect(frame);
                // Corners are rounded only where the picture reaches them.
                let corners = if geometry.is_exact() || picture == frame {
                    round
                } else {
                    CornerRadius::ZERO
                };
                painter.rect_filled(
                    picture,
                    corners,
                    theme::mix(p.stage(), Color32::WHITE, 0.07),
                );
                icons::paint(
                    &painter,
                    Rect::from_center_size(picture.center(), Vec2::splat(22.0)),
                    Icon::Film,
                    stage.muted,
                );
            }
        }
    }

    /// What will be made, as a card of facts, then what will not come out
    /// as previewed and why the plan fails, if it does.
    fn summary(&self, ui: &mut Ui, state: &ExportSheetState) {
        let p = self.p;
        let settings = &state.settings;
        let canvas = self.canvas();
        let plan = self.plan.and_then(|plan| plan.as_ref().ok());
        let mut facts: Vec<(&str, String)> = Vec::new();
        if let Some(video) = &settings.video {
            let size = plan.and_then(|plan| plan.video.as_ref()).map_or_else(
                || video.resolution.size(canvas),
                |video| video.geometry.output,
            );
            facts.push((
                "Size",
                format!("{} × {} · {}", size[0], size[1], aspect(size)),
            ));
            let rate = video.frame_rate.unwrap_or(self.rate());
            facts.push(("Frame rate", format!("{} fps", rate_label(rate))));
        }
        facts.push((
            "Sound",
            match &settings.audio {
                Some(audio) => {
                    let channels = if audio.channels == 1 {
                        "Mono"
                    } else {
                        "Stereo"
                    };
                    format!(
                        "{} · {} kHz · {channels}",
                        audio.codec.label(),
                        trim(audio.sample_rate as f64 / 1000.0)
                    )
                }
                None => "None".to_owned(),
            },
        ));
        match plan {
            Some(plan) => {
                let length = plan.end - plan.start;
                let frames = plan.video.as_ref().map(|video| video.frames.count);
                facts.push((
                    "Duration",
                    match frames {
                        Some(frames) => format!("{} · {frames} frames", timefmt::short(length)),
                        None => timefmt::short(length),
                    },
                ));
                facts.push((
                    "Estimated size",
                    format!(
                        "≈ {}",
                        timefmt::file_size(ExportSettings::estimate_bytes(plan))
                    ),
                ));
                facts.push((
                    "Encoder",
                    match (&plan.video, &plan.audio) {
                        (Some(video), _) => {
                            encoder_label(video.encoder.hardware, &video.encoder.name)
                        }
                        (None, Some(audio)) => audio.encoder.name.clone(),
                        (None, None) => "None".to_owned(),
                    },
                ));
            }
            None => {
                if let Ok((start, end)) = settings.span(self.input.project) {
                    facts.push(("Duration", timefmt::short(end - start)));
                }
            }
        }
        controls::group(ui, *p, |ui, group| {
            for (label, value) in &facts {
                fact(ui, p, group, label, value);
            }
        });
        for warning in self.input.warnings {
            caution(ui, p, warning, p.legible(p.yellow));
        }
        if let Some(Err(error)) = self.plan {
            caution(ui, p, &error.to_string(), p.red);
        }
    }

    /// The file, the preset and the range, then the picture, the sound and
    /// the captions, as cards in that order. The choices that need to know
    /// what this host encodes wait for it.
    fn right(&mut self, ui: &mut Ui, state: &mut ExportSheetState) {
        let p = self.p;
        self.file(ui, state);
        ui.add_space(GAP);
        let caps = match (self.input.caps, self.caps) {
            (_, Some(caps)) => caps,
            (CapabilityView::Failed(problem), _) => {
                self.range(ui, state);
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    let (glyph, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
                    icons::paint(ui.painter(), glyph, Icon::Warning, p.red);
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("The encoders could not be checked")
                            .font(theme::medium(13.0))
                            .color(p.fg),
                    );
                    ui.add_space(4.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(problem)
                                .font(theme::regular(12.0))
                                .color(p.secondary),
                        )
                        .wrap(),
                    );
                    ui.add_space(12.0);
                    if controls::button(ui, *p, "Try again", ButtonKind::Secondary)
                        .on_hover_text("Ask FFmpeg which encoders work again")
                        .clicked()
                    {
                        *self.answer = Some(ExportAnswer::Reprobe);
                    }
                });
                return;
            }
            _ => {
                self.range(ui, state);
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    let (glyph, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                    kit::spinner(ui, glyph, p.muted);
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("Checking which encoders work on this computer…")
                            .font(theme::regular(12.5))
                            .color(p.secondary),
                    );
                });
                return;
            }
        };
        self.presets(ui, state, caps);
        ui.add_space(GAP);
        self.range(ui, state);
        self.video(ui, state, caps);
        self.audio(ui, state, caps);
        self.captions(ui, state);
    }

    /// The file's name, its extension, and the folder it goes in.
    fn file(&mut self, ui: &mut Ui, state: &mut ExportSheetState) {
        let p = self.p;
        let extension = format!(".{}", state.settings.container.extension());
        // Shown as the render queue shows where a job goes.
        let folder_text = state.folder.as_ref().map_or_else(
            || "Choose a folder…".to_owned(),
            |folder| super::render_queue::short_folder(folder, state.home.as_deref()),
        );
        let tip = state.folder.as_ref().map_or_else(
            || "Choose where the file is saved".to_owned(),
            |folder| format!("Saved in {}", folder.display()),
        );
        let mut chose = false;
        // The field and the folder start together, after the longer label.
        let lead = label_width(ui, p, "File name").max(label_width(ui, p, "Folder"));
        controls::group(ui, *p, |ui, group| {
            row_after(ui, p, group, "File name", lead, |ui, room| {
                let valid = name_problem(&state.name).is_none();
                field(
                    ui,
                    p,
                    Id::new("export-file-name"),
                    &mut state.name,
                    room,
                    "File name",
                    "File name",
                    Some(&extension),
                    valid,
                );
            });
            row_after(ui, p, group, "Folder", lead, |ui, room| {
                let folder = folder_button(ui, p, &folder_text, state.folder.as_deref(), room);
                chose = folder.on_hover_text(tip).clicked();
            });
        });
        if chose {
            *self.answer = Some(ExportAnswer::ChooseFolder);
        }
    }

    /// Which part of the timeline: chips that wrap inside the card, and the
    /// typed start and end of a custom range.
    fn range(&mut self, ui: &mut Ui, state: &mut ExportSheetState) {
        let p = self.p;
        let selection = self.input.selection;
        let rate = self.rate();
        let chips = [
            (RangeChoice::Timeline, "Whole timeline", None),
            (
                RangeChoice::Selection,
                "Selected clips",
                selection
                    .is_none()
                    .then_some("Close Export and select clips on the timeline first"),
            ),
            (RangeChoice::Custom, "Custom range", None),
        ];
        section(ui, p, "Range");
        let mut picked = None;
        controls::group(ui, *p, |ui, group| {
            // Each chip is given the room it takes when chosen, so that a
            // choice does not move the others onto another line.
            let room = ui.available_width() - 24.0;
            let widths: Vec<f32> = (chips.iter())
                .map(|(_, text, _)| chip_width(ui, text))
                .collect();
            let mut lines: Vec<Vec<usize>> = vec![Vec::new()];
            let mut used = 0.0;
            for (index, width) in widths.iter().enumerate() {
                let line = lines.last_mut().expect("a line to fill");
                if !line.is_empty() && used + 6.0 + width > room {
                    lines.push(vec![index]);
                    used = *width;
                } else {
                    used += if line.is_empty() { 0.0 } else { 6.0 } + width;
                    line.push(index);
                }
            }
            let height = 7.0 * 2.0 + lines.len() as f32 * 26.0 + (lines.len() - 1) as f32 * 6.0;
            let rect = group.custom(ui, height);
            for (at, line) in lines.iter().enumerate() {
                let top = rect.top() + 7.0 + at as f32 * 32.0;
                let strip = Rect::from_min_max(
                    pos2(rect.left() + 12.0, top),
                    pos2(rect.right() - 12.0, top + 26.0),
                );
                controls::place(
                    ui,
                    strip,
                    Layout::left_to_right(Align::Center),
                    ("range-chips", at),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        for index in line {
                            let (choice, text, reason) = chips[*index];
                            let chosen = state.range == choice;
                            let chip = ui
                                .add_enabled(reason.is_none(), |ui: &mut Ui| {
                                    kit::chip(ui, p, text, chosen)
                                })
                                .on_disabled_hover_text(reason.unwrap_or_default());
                            if chip.clicked() && reason.is_none() && !chosen {
                                picked = Some(choice);
                            }
                        }
                    },
                );
            }
            if state.range == RangeChoice::Custom {
                let playhead = self.input.playhead;
                let rows = [
                    ("Start", 0, "Start", "Range start", "Start at playhead"),
                    ("End", 1, "End", "Range end", "End at playhead"),
                ];
                // Both fields and both buttons start together: after the
                // longer label, and as wide as the longer button needs.
                let lead = (rows.iter())
                    .map(|(label, ..)| label_width(ui, p, label))
                    .fold(0.0, f32::max);
                let button_width = (rows.iter())
                    .map(|(.., button)| button_width(ui, button))
                    .fold(0.0, f32::max);
                for (label, which, hint, name, button) in rows {
                    row_after(ui, p, group, label, lead, |ui, room| {
                        let width = (room - 8.0 - button_width).clamp(48.0, 132.0);
                        ui.allocate_ui_with_layout(
                            vec2(width + 8.0 + button_width, metrics::CONTROL_HEIGHT),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                let text = if which == 0 {
                                    &mut state.start_text
                                } else {
                                    &mut state.end_text
                                };
                                field(
                                    ui,
                                    p,
                                    Id::new(("export-range", which)),
                                    text,
                                    width,
                                    hint,
                                    name,
                                    None,
                                    true,
                                );
                                if controls::button(ui, *p, button, ButtonKind::Secondary).clicked()
                                {
                                    *text = timefmt::timecode(playhead, rate);
                                }
                            },
                        );
                    });
                }
            }
        });
        if let Some(choice) = picked {
            if choice == RangeChoice::Custom {
                let (start, end) = match (state.range, selection) {
                    (RangeChoice::Selection, Some(span)) => span,
                    _ => (Time::ZERO, self.input.project.duration()),
                };
                state.start_text = timefmt::timecode(start, rate);
                state.end_text = timefmt::timecode(end, rate);
            }
            state.range = choice;
        }
        if let Some(problem) = self.range_problem {
            note(ui, problem, p.red);
        }
    }

    /// The preset: one list grouped by destination, the person's own last,
    /// the most used presets as chips on one row, and what the chosen one is
    /// for under them.
    fn presets(&self, ui: &mut Ui, state: &mut ExportSheetState, caps: &Capabilities) {
        let p = self.p;
        let presets = self.input.presets;
        let current = matched(&state.settings, state.preset.as_deref(), presets);
        let current_id = current.map(|found| found.id.clone());
        // Destinations in the order their presets come, the person's last.
        let mut destinations: Vec<Destination> = Vec::new();
        for preset in presets {
            if preset.destination != Destination::Custom
                && !destinations.contains(&preset.destination)
            {
                destinations.push(preset.destination);
            }
        }
        if presets.iter().any(|p| p.destination == Destination::Custom) {
            destinations.push(Destination::Custom);
        }
        let mut chosen = None;
        let reason = |preset: &ExportPreset| preset_problem(preset, caps);
        controls::group(ui, *p, |ui, group| {
            row(ui, p, group, "Preset", |ui, room| {
                let width = room.min(240.0);
                let select = kit::select(
                    ui,
                    p,
                    Id::new("export-preset"),
                    current.map_or("Custom settings", |preset| preset.name.as_str()),
                    "Preset",
                    vec2(width, metrics::CONTROL_HEIGHT),
                );
                egui::Popup::menu(&select).show(|ui| {
                    widgets::menu_style(ui, width.max(240.0));
                    // A select's long list (spec §4.3.4), which opens under
                    // the select in any window the sheet fits.
                    let output = egui::ScrollArea::vertical()
                        .max_height(LIST)
                        .show(ui, |ui| {
                            for (at, destination) in destinations.iter().enumerate() {
                                if at > 0 {
                                    widgets::menu_separator(ui, p);
                                }
                                kit::menu_heading(ui, p, destination.label());
                                for preset in
                                    presets.iter().filter(|p| p.destination == *destination)
                                {
                                    let problem = reason(preset);
                                    let selected =
                                        current_id.as_deref() == Some(preset.id.as_str());
                                    let response = ui
                                        .add_enabled_ui(problem.is_none(), |ui| {
                                            choice_row(ui, p, &preset.name, selected)
                                        })
                                        .inner;
                                    if offered(preset, response, problem.as_deref()) {
                                        chosen = Some(preset);
                                    }
                                }
                            }
                        });
                    faded(ui, &output, p.elevated);
                });
            });
            // The most used, as many as one row holds.
            let rect = group.custom(ui, ROW);
            let strip = rect.shrink2(vec2(12.0, 7.0));
            controls::place(
                ui,
                strip,
                Layout::left_to_right(Align::Center),
                "quick-presets",
                |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let mut room = strip.width();
                    for id in QUICK {
                        let Some(preset) = presets.iter().find(|preset| preset.id == id) else {
                            continue;
                        };
                        // The room it takes when chosen, so that the row holds
                        // the same chips whichever is.
                        let width = chip_width(ui, &preset.name);
                        if width > room {
                            break;
                        }
                        room -= width + 6.0;
                        let problem = reason(preset);
                        let selected = current_id.as_deref() == Some(preset.id.as_str());
                        let response = ui
                            .add_enabled_ui(problem.is_none(), |ui| {
                                kit::chip(ui, p, &preset.name, selected)
                            })
                            .inner;
                        if offered(preset, response, problem.as_deref()) {
                            chosen = Some(preset);
                        }
                    }
                },
            );
            // Always one line, so that a choice which turns the preset into
            // custom settings does not move the controls under the pointer.
            let said = match current {
                Some(preset) if !preset.note.is_empty() => preset.note.as_str(),
                Some(_) => "A preset of your own",
                None => "Changed from the presets; Save preset… keeps them",
            };
            let rect = group.custom(ui, 34.0);
            let painter = ui.painter().clone();
            let galley = controls::elided(
                &painter,
                said,
                theme::regular(11.5),
                p.secondary,
                rect.width() - 28.0,
            );
            let cut = galley.text() != said;
            controls::galley_at(&painter, pos2(rect.left() + 14.0, rect.center().y), galley);
            let line = ui.interact(rect, ui.id().with("preset-note"), Sense::hover());
            line.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, said));
            if cut {
                // Cut short where it does not fit; the whole line is on hover.
                line.on_hover_text(said);
            }
        });
        if let Some(preset) = chosen {
            state.apply(preset);
        }
    }

    fn video(&self, ui: &mut Ui, state: &mut ExportSheetState, caps: &Capabilities) {
        let p = self.p;
        let canvas = self.canvas();
        let mut on = state.settings.video.is_some();
        ui.add_space(GAP);
        let label = section_of(ui, p, "Video", SWITCHED);
        section_switch(ui, p, label, &mut on, "Video", true).on_hover_text(if on {
            "Turn off to export sound only"
        } else {
            "Turn on to export the picture"
        });
        if on != state.settings.video.is_some() {
            state.set_video(on, caps);
        }
        controls::group(ui, *p, |ui, group| {
            // The file the picture or the sound goes in.
            let pool: &[Container] = if on {
                &Container::VIDEO
            } else {
                &Container::AUDIO
            };
            let offered: Vec<(Container, &str)> = (pool.iter().copied())
                .filter(|c| caps.has_container(*c))
                .map(|c| (c, c.label()))
                .collect();
            row(ui, p, group, "Format", |ui, room| {
                if offered.is_empty() {
                    ui.label(
                        egui::RichText::new("This FFmpeg writes none")
                            .font(theme::regular(12.5))
                            .color(p.red),
                    );
                    return;
                }
                let mut container = state.settings.container;
                let width = (56.0 * offered.len() as f32).max(TRAIL).min(room);
                if choice(ui, p, &mut container, &offered, width) {
                    state.settings.container = container;
                    settle(&mut state.settings, caps);
                }
            });
            if !on {
                group.note(ui, "Sound only: the file has no picture.");
                return;
            }
            let container = state.settings.container;
            let Some(video) = state.settings.video.as_mut() else {
                return;
            };
            let output = video.resolution.size(canvas);

            let mut size = match video.resolution {
                Resolution::Short { pixels } => SizeChoice::Short(pixels),
                Resolution::Project => SizeChoice::Project,
                Resolution::Custom { .. } => SizeChoice::Custom,
            };
            let sizes: Vec<(SizeChoice, &str)> = (Resolution::NAMED.iter())
                .filter_map(|(resolution, label)| match resolution {
                    Resolution::Short { pixels } => Some((SizeChoice::Short(*pixels), *label)),
                    _ => None,
                })
                .chain([
                    (SizeChoice::Project, "Project"),
                    (SizeChoice::Custom, "Custom"),
                ])
                .collect();
            let resized = wide(
                ui,
                p,
                group,
                Some("Resolution"),
                "resolution",
                |ui, width| choice(ui, p, &mut size, &sizes, width),
            );
            if resized {
                video.resolution = match size {
                    SizeChoice::Short(pixels) => Resolution::Short { pixels },
                    SizeChoice::Project => Resolution::Project,
                    SizeChoice::Custom => {
                        state.width_text = output[0].to_string();
                        state.height_text = output[1].to_string();
                        Resolution::Custom {
                            width: output[0],
                            height: output[1],
                        }
                    }
                };
            }
            if let Resolution::Custom { width, height } = &mut video.resolution {
                row(ui, p, group, "Size", |ui, room| {
                    // Laid out in reading order, which is the order Tab
                    // walks: width, then height. The pair is as wide as the
                    // card's other controls, so that it starts with them.
                    let unit = (ui.painter())
                        .layout_no_wrap("×".to_owned(), theme::regular(13.0), p.muted)
                        .size()
                        .x;
                    let each = ((room.min(TRAIL) - unit - 12.0) * 0.5).max(56.0).floor();
                    ui.allocate_ui_with_layout(
                        vec2(each * 2.0 + unit + 12.0, metrics::CONTROL_HEIGHT),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            let valid = state.width_text.trim().parse::<u32>().is_ok();
                            field(
                                ui,
                                p,
                                Id::new("export-width"),
                                &mut state.width_text,
                                each,
                                "Width",
                                "Width in pixels",
                                None,
                                valid,
                            );
                            ui.label(
                                egui::RichText::new("×")
                                    .font(theme::regular(13.0))
                                    .color(p.muted),
                            );
                            let valid = state.height_text.trim().parse::<u32>().is_ok();
                            field(
                                ui,
                                p,
                                Id::new("export-height"),
                                &mut state.height_text,
                                each,
                                "Height",
                                "Height in pixels",
                                Some("px"),
                                valid,
                            );
                        },
                    );
                });
                if let Ok(typed) = state.width_text.trim().parse::<u32>() {
                    *width = typed;
                }
                if let Ok(typed) = state.height_text.trim().parse::<u32>() {
                    *height = typed;
                }
            }
            let output = video.resolution.size(canvas);
            if !Geometry::new(canvas, output, Fit::Letterbox).is_exact() {
                row(ui, p, group, "Fit", |ui, room| {
                    choice(
                        ui,
                        p,
                        &mut video.fit,
                        &[(Fit::Letterbox, "Letterbox"), (Fit::Fill, "Fill")],
                        room.min(TRAIL),
                    );
                });
            }

            row(ui, p, group, "Frame rate", |ui, room| {
                let project = self.rate();
                let shown = match video.frame_rate {
                    None => format!("Project ({})", rate_label(project)),
                    Some(rate) => format!("{} fps", rate_label(rate)),
                };
                let options: Vec<(Option<FrameRate>, String, Option<String>)> =
                    std::iter::once((None, format!("Project ({})", rate_label(project)), None))
                        .chain(RATES.iter().map(|(numerator, denominator, label)| {
                            let rate = FrameRate {
                                numerator: *numerator,
                                denominator: *denominator,
                            };
                            (Some(rate), format!("{label} fps"), None)
                        }))
                        .collect();
                if let Some(rate) = list(
                    ui,
                    p,
                    "export-frame-rate",
                    "Frame rate",
                    &shown,
                    room.min(TRAIL),
                    video.frame_rate,
                    &options,
                ) {
                    video.frame_rate = rate;
                }
            });

            row(ui, p, group, "Codec", |ui, room| {
                let options: Vec<(VideoCodec, String, Option<String>)> = (container.video_codecs())
                    .iter()
                    .map(|offered| {
                        let reason =
                            (!caps.has_video_codec(*offered)).then(|| codec_reason(caps, *offered));
                        (*offered, offered.label().to_owned(), reason)
                    })
                    .collect();
                if let Some(codec) = list(
                    ui,
                    p,
                    "export-codec",
                    "Codec",
                    video.codec.label(),
                    room.min(TRAIL),
                    video.codec,
                    &options,
                ) && codec != video.codec
                {
                    video.codec = codec;
                    if codec == VideoCodec::ProRes
                        && matches!(video.rate_control, RateControl::Bitrate { .. })
                    {
                        video.rate_control = RateControl::default();
                    }
                }
            });

            let fps = video.frame_rate.unwrap_or(self.rate()).fps();
            if video.codec == VideoCodec::ProRes {
                let mut level = match video.rate_control {
                    RateControl::Quality { level } => level,
                    RateControl::Bitrate { .. } => QualityLevel::Recommended,
                };
                let changed = wide(ui, p, group, Some("Profile"), "profile", |ui, width| {
                    choice(
                        ui,
                        p,
                        &mut level,
                        &[
                            (QualityLevel::Lower, "LT"),
                            (QualityLevel::Recommended, "422"),
                            (QualityLevel::Higher, "HQ"),
                            (QualityLevel::Maximum, "4444"),
                        ],
                        width,
                    )
                });
                if changed {
                    video.rate_control = RateControl::Quality { level };
                }
            } else {
                let mut target = match video.rate_control {
                    RateControl::Quality { .. } => Target::Quality,
                    RateControl::Bitrate { .. } => Target::Bitrate,
                };
                row(ui, p, group, "Target", |ui, room| {
                    choice(
                        ui,
                        p,
                        &mut target,
                        &[(Target::Quality, "Quality"), (Target::Bitrate, "Bitrate")],
                        room.min(TRAIL),
                    );
                });
                match (target, video.rate_control) {
                    (Target::Bitrate, RateControl::Quality { level }) => {
                        let kbps = suggested(video.codec, output, fps, level);
                        state.kbps_text = kbps.to_string();
                        video.rate_control = RateControl::Bitrate { kbps };
                    }
                    (Target::Quality, RateControl::Bitrate { .. }) => {
                        video.rate_control = RateControl::default();
                    }
                    _ => {}
                }
                match &mut video.rate_control {
                    RateControl::Quality { level } => {
                        let levels: Vec<(QualityLevel, &str)> = (QualityLevel::ALL.iter())
                            .map(|l| (*l, l.label()))
                            .collect();
                        wide(ui, p, group, None, "quality", |ui, width| {
                            choice(ui, p, level, &levels, width)
                        });
                    }
                    RateControl::Bitrate { kbps } => {
                        let hint = suggested(video.codec, output, fps, QualityLevel::Recommended);
                        let valid = state.kbps_text.trim().parse::<u32>().is_ok();
                        row(ui, p, group, "Bitrate", |ui, room| {
                            field(
                                ui,
                                p,
                                Id::new("export-kbps"),
                                &mut state.kbps_text,
                                room.min(TRAIL),
                                &hint.to_string(),
                                "Bitrate in kbit/s",
                                Some("kbit/s"),
                                valid,
                            )
                            .on_hover_text(format!(
                                "{MIN_VIDEO_KBPS}–{MAX_VIDEO_KBPS} kbit/s; about {hint} suits this picture"
                            ));
                        });
                        if let Ok(typed) = state.kbps_text.trim().parse::<u32>() {
                            *kbps = typed;
                        }
                    }
                }
            }

            group.disclosure(ui, "Encoder and speed", &mut state.encoding_open);
            if state.encoding_open {
                encoder_rows(ui, p, group, video, caps, output);
            }
        });
    }

    fn audio(&self, ui: &mut Ui, state: &mut ExportSheetState, caps: &Capabilities) {
        let p = self.p;
        let video = state.settings.video.is_some();
        let mut on = state.settings.audio.is_some();
        ui.add_space(GAP);
        let label = section_of(ui, p, "Audio", SWITCHED);
        section_switch(ui, p, label, &mut on, "Audio", video)
            .on_disabled_hover_text("A sound-only file needs its sound");
        if on != state.settings.audio.is_some() {
            state.set_audio(on, caps);
        }
        let container = state.settings.container;
        let Some(audio) = state.settings.audio.as_mut() else {
            return;
        };
        controls::group(ui, *p, |ui, group| {
            row(ui, p, group, "Codec", |ui, room| {
                let options: Vec<(AudioCodec, String, Option<String>)> = (container.audio_codecs())
                    .iter()
                    .map(|offered| {
                        let reason = (!caps.has_audio_codec(*offered))
                            .then(|| format!("This FFmpeg has no {} encoder", offered.label()));
                        (*offered, offered.label().to_owned(), reason)
                    })
                    .collect();
                if let Some(codec) = list(
                    ui,
                    p,
                    "export-audio-codec",
                    "Audio codec",
                    audio.codec.label(),
                    room.min(TRAIL),
                    audio.codec,
                    &options,
                ) && codec != audio.codec
                {
                    audio.codec = codec;
                    audio.kbps = codec.default_kbps();
                }
            });
            if audio.codec.has_bitrate() {
                let most = audio.codec.max_kbps(audio.channels);
                let mut offered: Vec<u32> = (AUDIO_KBPS.into_iter())
                    .filter(|kbps| (MIN_AUDIO_KBPS..=most).contains(kbps))
                    .collect();
                if (MIN_AUDIO_KBPS..=MAX_AUDIO_KBPS).contains(&audio.kbps)
                    && !offered.contains(&audio.kbps)
                {
                    offered.push(audio.kbps);
                    offered.sort_unstable();
                }
                row(ui, p, group, "Bitrate", |ui, room| {
                    let options: Vec<(u32, String, Option<String>)> = (offered.iter())
                        .map(|kbps| {
                            let reason = (*kbps > most).then(|| {
                                format!("{} goes up to {most} kbit/s", audio.codec.label())
                            });
                            (*kbps, format!("{kbps} kbit/s"), reason)
                        })
                        .collect();
                    if let Some(kbps) = list(
                        ui,
                        p,
                        "export-audio-kbps",
                        "Audio bitrate",
                        &format!("{} kbit/s", audio.kbps),
                        room.min(TRAIL),
                        audio.kbps,
                        &options,
                    ) {
                        audio.kbps = kbps;
                    }
                });
            }
            let rates: Vec<(u32, String)> = (SAMPLE_RATES.into_iter())
                .filter(|rate| audio.codec.accepts_rate(*rate))
                .map(|rate| (rate, format!("{} kHz", trim(rate as f64 / 1000.0))))
                .collect();
            let rates: Vec<(u32, &str)> = rates.iter().map(|(r, l)| (*r, l.as_str())).collect();
            row(ui, p, group, "Sample rate", |ui, room| {
                let width = (60.0 * rates.len() as f32).max(TRAIL).min(room);
                choice(ui, p, &mut audio.sample_rate, &rates, width);
            });
            row(ui, p, group, "Channels", |ui, room| {
                if choice(
                    ui,
                    p,
                    &mut audio.channels,
                    &[(1, "Mono"), (2, "Stereo")],
                    room.min(TRAIL),
                ) && audio.codec.has_bitrate()
                {
                    audio.kbps = audio.kbps.min(audio.codec.max_kbps(audio.channels));
                }
            });
        });
    }

    fn captions(&self, ui: &mut Ui, state: &mut ExportSheetState) {
        let p = self.p;
        let tracks: Vec<(TrackId, &str)> = export::caption_tracks(self.input.project)
            .map(|track| (track.id, track.name.as_str()))
            .collect();
        if tracks.is_empty() {
            return;
        }
        let container = state.settings.container;
        let video = state.settings.video.is_some();
        let subtitles = &mut state.settings.subtitles;
        ui.add_space(GAP);
        section(ui, p, "Captions");
        controls::group(ui, *p, |ui, group| {
            row(ui, p, group, "Burn into the picture", |ui, _| {
                ui.add_enabled_ui(video, |ui| {
                    controls::toggle(
                        ui,
                        *p,
                        &mut subtitles.burn_in,
                        "Burn captions into the picture",
                    )
                })
                .inner
                .on_disabled_hover_text("A sound-only file has no picture");
            });
            row(ui, p, group, "Subtitle file", |ui, room| {
                choice(
                    ui,
                    p,
                    &mut subtitles.sidecar,
                    &[
                        (None, "None"),
                        (Some(SidecarFormat::Srt), "SRT"),
                        (Some(SidecarFormat::Vtt), "VTT"),
                    ],
                    room.min(TRAIL),
                );
            });
            let refusal = if !video {
                Some("A sound-only file carries no subtitle stream".to_owned())
            } else {
                container
                    .embedded_subtitles()
                    .is_none()
                    .then(|| format!("{} files carry no subtitle stream", container.label()))
            };
            row(ui, p, group, "Subtitle stream", |ui, _| {
                ui.add_enabled_ui(refusal.is_none(), |ui| {
                    controls::toggle(ui, *p, &mut subtitles.embed, "Subtitle stream in the file")
                })
                .inner
                .on_hover_text("Viewers can turn the captions on and off")
                .on_disabled_hover_text(refusal.as_deref().unwrap_or_default());
            });
            if subtitles.sidecar.is_some() || subtitles.embed {
                let chosen = subtitles
                    .track
                    .filter(|id| tracks.iter().any(|(track, _)| track == id))
                    .unwrap_or(tracks[0].0);
                row(ui, p, group, "Track", |ui, room| {
                    let shown = (tracks.iter())
                        .find(|(id, _)| *id == chosen)
                        .map_or("", |(_, name)| name);
                    let options: Vec<(TrackId, String, Option<String>)> = (tracks.iter())
                        .map(|(id, name)| (*id, (*name).to_owned(), None))
                        .collect();
                    if let Some(pick) = list(
                        ui,
                        p,
                        "export-caption-track",
                        "Caption track",
                        shown,
                        room.min(TRAIL),
                        chosen,
                        &options,
                    ) && pick != chosen
                    {
                        subtitles.track = Some(pick);
                    }
                });
            }
        });
    }
}

/// Whether a preset offered in the list or as a chip was taken: one this
/// host cannot make is disabled with why, and one it can says what it is
/// for on hover.
fn offered(preset: &ExportPreset, response: egui::Response, problem: Option<&str>) -> bool {
    let response = match problem {
        Some(problem) => response.on_disabled_hover_text(problem),
        None if preset.note.is_empty() => response,
        None => response.on_hover_text(&preset.note),
    };
    response.clicked() && problem.is_none()
}

/// The room a chip takes when it is chosen, its check before its name.
fn chip_width(ui: &Ui, text: &str) -> f32 {
    let words = (ui.painter())
        .layout_no_wrap(text.to_owned(), theme::medium(12.0), Color32::PLACEHOLDER)
        .size()
        .x;
    10.0 + 16.0 + words + 10.0
}

/// The width of a text button of the sheet.
fn button_width(ui: &Ui, text: &str) -> f32 {
    let words = (ui.painter())
        .layout_no_wrap(text.to_owned(), theme::medium(13.0), Color32::PLACEHOLDER)
        .size()
        .x;
    (words + 32.0).max(72.0)
}

/// The encoder choice and its effort.
fn encoder_rows(
    ui: &mut Ui,
    p: &Palette,
    group: &mut Group,
    video: &mut VideoSettings,
    caps: &Capabilities,
    output: [u32; 2],
) {
    let codec = video.codec;
    // Families that work here, then those FFmpeg lists that failed.
    let mut families = caps.hardware();
    for unavailable in &caps.unavailable {
        if let Some(kind) = hardware_of(&unavailable.name)
            && !families.contains(&kind)
        {
            families.push(kind);
        }
    }
    let label = |choice: EncoderChoice| match choice {
        EncoderChoice::Auto => match caps.video_encoder(codec, EncoderChoice::Auto, output) {
            Some(encoder) => format!(
                "Auto ({})",
                encoder.hardware.map_or("Software", HardwareKind::label)
            ),
            None => "Auto".to_owned(),
        },
        EncoderChoice::Software => "Software".to_owned(),
        EncoderChoice::Hardware(kind) => kind.label().to_owned(),
    };
    row(ui, p, group, "Encoder", |ui, room| {
        let options: Vec<(EncoderChoice, String, Option<String>)> =
            std::iter::once((EncoderChoice::Auto, label(EncoderChoice::Auto), None))
                .chain(
                    [EncoderChoice::Software]
                        .into_iter()
                        .chain(families.iter().map(|kind| EncoderChoice::Hardware(*kind)))
                        .map(|candidate| {
                            (
                                candidate,
                                label(candidate),
                                choice_reason(caps, candidate, codec, output),
                            )
                        }),
                )
                .collect();
        if let Some(choice) = list(
            ui,
            p,
            "export-encoder",
            "Encoder",
            &label(video.encoder),
            room.min(TRAIL),
            video.encoder,
            &options,
        ) {
            video.encoder = choice;
        }
    });
    if codec != VideoCodec::ProRes {
        row(ui, p, group, "Speed", |ui, room| {
            choice(
                ui,
                p,
                &mut video.speed,
                &[
                    (EncodeSpeed::Faster, "Faster"),
                    (EncodeSpeed::Balanced, "Balanced"),
                    (EncodeSpeed::Smaller, "Smaller"),
                ],
                room.min(240.0),
            );
        });
    }
}

/// A bitrate to start from, rounded to a hundred.
fn suggested(codec: VideoCodec, size: [u32; 2], fps: f64, level: QualityLevel) -> u32 {
    let kbps = export::suggested_kbps(codec, size, fps, level);
    ((kbps + 50) / 100 * 100).clamp(MIN_VIDEO_KBPS, MAX_VIDEO_KBPS)
}

/// `44.1`, `48`.
fn trim(value: f64) -> String {
    let text = format!("{value:.1}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// The folder the file goes in, as a Secondary button `width` wide led by
/// the folder glyph, named with the whole path for accessibility.
fn folder_button(
    ui: &mut Ui,
    p: &Palette,
    text: &str,
    folder: Option<&Path>,
    width: f32,
) -> egui::Response {
    let missing = folder.is_none();
    let size = vec2(width, metrics::CONTROL_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| {
        let name = match folder {
            Some(folder) => format!("Folder: {}", folder.display()),
            None => "Folder: none chosen".to_owned(),
        };
        WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), name)
    });
    if ui.is_rect_visible(rect) {
        let radius = metrics::CONTROL_RADIUS;
        let hover = kit::hover_amount(ui, &response);
        let painter = ui.painter();
        painter.rect_filled(rect, radius, p.control);
        if response.is_pointer_button_down_on() {
            painter.rect_filled(rect, radius, p.pressed);
        } else if hover > 0.0 {
            painter.rect_filled(rect, radius, p.hover.gamma_multiply(hover));
        }
        if response.has_focus() {
            controls::focus_ring(painter, rect, radius, *p);
        }
        icons::paint(
            painter,
            Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), Vec2::splat(14.0)),
            Icon::Folder,
            p.secondary,
        );
        let ink = if missing { p.muted } else { p.fg };
        let galley = controls::elided(
            painter,
            text,
            theme::regular(13.0),
            ink,
            rect.width() - 46.0,
        );
        controls::galley_at(painter, pos2(rect.left() + 34.0, rect.center().y), galley);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

// The export sheet and the render queue were the last to call these
// adapters of the old controls, which are another package's to remove
// (packages.md, P10). Named here so that the build stays free of warnings
// until they go; delete this with them.
const _: () = {
    let _ = widgets::chip;
    let _ = widgets::progress;
};

#[cfg(test)]
#[path = "export_sheet_tests.rs"]
mod tests;
