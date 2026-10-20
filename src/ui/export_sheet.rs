//! The export dialog: which file to make, from which part of the timeline,
//! and how. It edits only its own copy of the settings and answers what the
//! person asked for; the application queues the job, asks for a folder and
//! keeps presets. Nothing offered is something this host cannot encode:
//! choices come from the probed [`Capabilities`], and the rest are absent or
//! disabled with the reason on hover.
use super::{
    sheets,
    widgets::{self, Kind, icon_button, row, section},
};
use crate::{
    icons::{self, Icon},
    theme::{self, Palette, metrics},
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
    self, Align, Color32, CornerRadius, Id, Layout, Rect, RichText, Sense, Ui, Vec2, WidgetInfo,
    WidgetType, pos2, vec2,
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

fn note(ui: &mut Ui, text: &str, color: Color32) {
    ui.add(egui::Label::new(RichText::new(text).font(theme::regular(11.5)).color(color)).wrap());
}

/// A line of the summary: what, and its value at the trailing edge.
fn fact(ui: &mut Ui, p: &Palette, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.set_min_height(20.0);
        ui.label(
            RichText::new(label)
                .font(theme::regular(12.0))
                .color(p.secondary),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add(
                egui::Label::new(RichText::new(value).font(theme::regular(12.0)).color(p.fg))
                    .truncate(),
            );
        });
    });
}

/// A problem or a caution, led by the warning glyph.
fn caution(ui: &mut Ui, p: &Palette, text: &str, color: Color32) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (glyph, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        icons::paint(
            ui.painter(),
            glyph.translate(vec2(0.0, 1.0)),
            Icon::Warning,
            color,
        );
        ui.add(
            egui::Label::new(
                RichText::new(text)
                    .font(theme::regular(12.0))
                    .color(if color == p.red { p.red } else { p.secondary }),
            )
            .wrap(),
        );
    });
}

/// A field on the control material, named for accessibility.
fn field(
    ui: &mut Ui,
    p: &Palette,
    text: &mut String,
    width: f32,
    hint: &str,
    name: &str,
) -> egui::Response {
    egui::Frame::NONE
        .fill(p.control)
        .corner_radius(CornerRadius::same(metrics::CONTROL_RADIUS))
        .inner_margin(egui::Margin::symmetric(8, 5))
        .show(ui, |ui| {
            let response = ui.add(
                egui::TextEdit::singleline(text)
                    .hint_text(hint)
                    .frame(egui::Frame::NONE)
                    .desired_width(width)
                    .font(theme::regular(12.5)),
            );
            response
                .widget_info(|| WidgetInfo::labeled(WidgetType::TextEdit, ui.is_enabled(), name));
            response
        })
        .inner
}

/// A choice in a dropdown that is offered only when `reason` is `None`.
fn option<T: PartialEq>(ui: &mut Ui, value: &mut T, choice: T, text: &str, reason: Option<&str>) {
    let enabled = reason.is_none();
    let response = ui
        .add_enabled_ui(enabled, |ui| ui.selectable_value(value, choice, text))
        .inner;
    if let Some(reason) = reason {
        response.on_disabled_hover_text(reason);
    }
}

/// Shows the export sheet. Returns what the person asked for, if anything;
/// Escape and the backdrop close it, and the application keeps `state` to
/// show again.
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
    let response = egui::Modal::new(Id::new("export-sheet"))
        .backdrop_color(p.scrim)
        .frame(sheets::sheet(p))
        .show(ctx, |ui| {
            let screen = ui.ctx().content_rect();
            let width = 760.0f32.min(screen.width() - 64.0).max(240.0);
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 6.0;
            reserve_scroll_bar(ui);
            // Escape gives up a preset name being typed before it closes the
            // sheet.
            if state.naming.is_some()
                && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
            {
                state.naming = None;
            }
            header(ui, p, input, &mut answer);

            let room = (screen.height() - 190.0).clamp(160.0, 600.0);
            let mut sheet = Sheet {
                p,
                input,
                caps,
                plan: plan.as_ref(),
                range_problem: range_problem.as_deref(),
                answer: &mut answer,
            };
            if width >= TWO_COLUMNS {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    sheets::column(ui, LEFT, |ui| {
                        ui.set_min_height(room);
                        egui::ScrollArea::vertical()
                            .id_salt("export-summary")
                            .max_height(room)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 6.0;
                                sheet.left(ui, state);
                            });
                    });
                    ui.add_space(10.0);
                    let (line, _) = ui.allocate_exact_size(vec2(1.0, room), Sense::hover());
                    ui.painter()
                        .vline(line.center().x, line.y_range(), p.hairline());
                    ui.add_space(10.0);
                    sheets::column(ui, ui.available_width(), |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("export-options")
                            .max_height(room)
                            .min_scrolled_height(room)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 6.0;
                                sheet.right(ui, state);
                            });
                    });
                });
            } else {
                sheets::column(ui, width, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("export-stacked")
                        .max_height(room)
                        .min_scrolled_height(room)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 6.0;
                            sheet.left(ui, state);
                            ui.add_space(6.0);
                            sheet.right(ui, state);
                        });
                });
            }
            ui.add_space(4.0);
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
    answer
}

/// Keeps room beside a scrolled column for its scroll bar at its widest, so
/// that the bar, widened under the pointer, covers nothing: the summary's
/// values and the options' trailing controls end at the column's edge.
fn reserve_scroll_bar(ui: &mut Ui) {
    let scroll = &mut ui.spacing_mut().scroll;
    scroll.floating_allocated_width = scroll.bar_width + scroll.bar_inner_margin;
}

/// What the file will be, in a few words for the footer, whatever either
/// column is scrolled to: "≈ 1.9 MB · 0:05 · 1280 × 720".
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

fn header(ui: &mut Ui, p: &Palette, input: &ExportSheetInput, answer: &mut Option<ExportAnswer>) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Export")
                .font(theme::semibold(15.0))
                .color(p.fg),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if icon_button(ui, p, Icon::Close, "Close export").clicked() {
                *answer = Some(ExportAnswer::Close);
            }
            if input.queue_len > 0 {
                let label = match input.queue_len {
                    1 => "Render queue (1 job)".to_owned(),
                    n => format!("Render queue ({n} jobs)"),
                };
                if icon_button(ui, p, Icon::List, &label).clicked() {
                    *answer = Some(ExportAnswer::OpenQueue);
                }
            }
        });
    });
}

fn footer(
    ui: &mut Ui,
    p: &Palette,
    state: &mut ExportSheetState,
    input: &ExportSheetInput,
    blocker: Option<&str>,
    facts: Option<&str>,
    answer: &mut Option<ExportAnswer>,
) {
    ui.horizontal(|ui| {
        ui.set_min_height(metrics::legacy::CONTROL_HEIGHT);
        let ready = matches!(input.caps, CapabilityView::Ready(_));
        if let Some(name) = state.naming.as_mut() {
            let typed = field(ui, p, name, 150.0, "Preset name", "Preset name");
            if std::mem::take(&mut state.focus_naming) {
                typed.request_focus();
            }
            let entered = typed.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let named = !name.trim().is_empty();
            let save = ui
                .add_enabled_ui(named, |ui| {
                    widgets::button(ui, p, None, "Save", Kind::Plain)
                })
                .inner
                .on_hover_text("Keep these settings as a preset")
                .on_disabled_hover_text("Name the preset");
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
            } else if widgets::small_icon_button(ui, p, Icon::Close, "Stop naming the preset")
                .clicked()
            {
                state.naming = None;
            }
        } else {
            let save = ui
                .add_enabled_ui(ready, |ui| {
                    widgets::button(ui, p, None, "Save preset…", Kind::Plain)
                })
                .inner
                .on_hover_text("Keep these settings as a preset of your own")
                .on_disabled_hover_text("Available once the encoders have been checked");
            if save.clicked() {
                state.naming = Some(String::new());
                state.focus_naming = true;
            }
            let own = matched(&state.settings, state.preset.as_deref(), input.presets)
                .filter(|preset| preset.destination == Destination::Custom);
            if let Some(preset) = own
                && widgets::small_icon_button(
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
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let reason = blocker.unwrap_or_default();
            let ok = blocker.is_none();
            let mut queue = |start_now: bool| match state.destination() {
                Some(destination) => Some(ExportAnswer::Queue {
                    settings: state.settings.clone(),
                    destination,
                    start_now,
                }),
                None => {
                    state.pending = Some(start_now);
                    Some(ExportAnswer::ChooseFolder)
                }
            };
            let export = ui
                .add_enabled_ui(ok, |ui| {
                    widgets::button(ui, p, Some(Icon::Export), "Export", Kind::Primary)
                })
                .inner
                .on_hover_text("Queue it to render now, and show the render queue")
                .on_disabled_hover_text(reason);
            if export.clicked() && ok {
                *answer = queue(true);
            }
            let add = ui
                .add_enabled_ui(ok, |ui| {
                    widgets::button(ui, p, None, "Add to queue", Kind::Plain)
                })
                .inner
                .on_hover_text("Queue it paused, to start from the render queue")
                .on_disabled_hover_text(reason);
            if add.clicked() && ok {
                *answer = queue(false);
            }
            if widgets::button(ui, p, None, "Cancel", Kind::Plain).clicked() {
                *answer = Some(ExportAnswer::Close);
            }
            if matches!(input.caps, CapabilityView::Probing) {
                ui.label(
                    RichText::new("Checking encoders…")
                        .font(theme::regular(12.0))
                        .color(p.muted),
                );
                ui.add(egui::Spinner::new().size(12.0).color(p.muted));
            } else if let Some(facts) = facts
                && state.naming.is_none()
            {
                ui.add_space(6.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(facts)
                            .font(theme::regular(11.5))
                            .color(p.muted),
                    )
                    .truncate(),
                );
            }
        });
    });
}

/// What both columns read while they are drawn.
struct Sheet<'s, 'a> {
    p: &'s Palette,
    input: &'s ExportSheetInput<'a>,
    caps: Option<&'a Capabilities>,
    plan: Option<&'s Result<ExportPlan, ExportError>>,
    range_problem: Option<&'s str>,
    answer: &'s mut Option<ExportAnswer>,
}

impl Sheet<'_, '_> {
    fn canvas(&self) -> [u32; 2] {
        let settings = &self.input.project.settings;
        [settings.width, settings.height]
    }

    fn rate(&self) -> FrameRate {
        self.input.project.settings.frame_rate
    }

    /// The picture, the file, what will be made and the range.
    fn left(&mut self, ui: &mut Ui, state: &mut ExportSheetState) {
        let p = self.p;
        self.poster(ui, state);
        ui.add_space(2.0);

        // The file: its name, its extension and the folder it goes in.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let extension = format!(".{}", state.settings.container.extension());
            let galley =
                ui.painter()
                    .layout_no_wrap(extension.clone(), theme::regular(12.5), p.muted);
            let room = (ui.available_width() - galley.size().x - 22.0).max(40.0);
            field(ui, p, &mut state.name, room, "File name", "File name");
            ui.label(
                RichText::new(extension)
                    .font(theme::regular(12.5))
                    .color(p.muted),
            );
        });
        // Shown as the render queue shows where a job goes.
        let folder_text = state.folder.as_ref().map_or_else(
            || "Choose a folder…".to_owned(),
            |folder| super::render_queue::short_folder(folder, state.home.as_deref()),
        );
        let tip = state.folder.as_ref().map_or_else(
            || "Choose where the file is saved".to_owned(),
            |folder| format!("Saved in {}", folder.display()),
        );
        let folder = folder_button(ui, p, &folder_text, state.folder.as_deref()).on_hover_text(tip);
        if folder.clicked() {
            *self.answer = Some(ExportAnswer::ChooseFolder);
        }

        section(ui, p, "Summary");
        self.summary(ui, state);

        section(ui, p, "Range");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            let selection = self.input.selection;
            for (choice, text, reason) in [
                (RangeChoice::Timeline, "Whole timeline", None),
                (
                    RangeChoice::Selection,
                    "Selected clips",
                    selection
                        .is_none()
                        .then_some("Close Export and select clips on the timeline first"),
                ),
                (RangeChoice::Custom, "Custom range", None),
            ] {
                // Added in place rather than in a child, so the row wraps
                // inside the column instead of widening it.
                let chosen = state.range == choice;
                let chip = ui
                    .add_enabled(reason.is_none(), |ui: &mut Ui| {
                        widgets::chip(ui, p, text, chosen)
                    })
                    .on_disabled_hover_text(reason.unwrap_or_default());
                if chip.clicked() && reason.is_none() && state.range != choice {
                    if choice == RangeChoice::Custom {
                        let (start, end) = match (state.range, selection) {
                            (RangeChoice::Selection, Some(span)) => span,
                            _ => (Time::ZERO, self.input.project.duration()),
                        };
                        state.start_text = timefmt::timecode(start, self.rate());
                        state.end_text = timefmt::timecode(end, self.rate());
                    }
                    state.range = choice;
                }
            }
        });
        if state.range == RangeChoice::Custom {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let width = ((ui.available_width() - 40.0) * 0.5 - 16.0).max(48.0);
                field(ui, p, &mut state.start_text, width, "Start", "Range start");
                ui.label(
                    RichText::new("to")
                        .font(theme::regular(12.0))
                        .color(p.muted),
                );
                field(ui, p, &mut state.end_text, width, "End", "Range end");
            });
            let playhead = self.input.playhead;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if widgets::button(ui, p, None, "Start at playhead", Kind::Plain).clicked() {
                    state.start_text = timefmt::timecode(playhead, self.rate());
                }
                if widgets::button(ui, p, None, "End at playhead", Kind::Plain).clicked() {
                    state.end_text = timefmt::timecode(playhead, self.rate());
                }
            });
        }
        if let Some(problem) = self.range_problem {
            note(ui, problem, p.red);
        }
    }

    /// The picture as it lands in the output: in the output's shape, with
    /// the bars or the crop the fit makes.
    fn poster(&self, ui: &mut Ui, state: &ExportSheetState) {
        let p = self.p;
        let canvas = self.canvas();
        let video = state.settings.video.as_ref();
        let output = video.map_or(canvas, |video| video.resolution.size(canvas));
        let ratio = output[1].max(1) as f32 / output[0].max(1) as f32;
        let width = ui.available_width();
        // Shorter on a short screen, so that the summary under it is seen.
        let most = match ui.ctx().content_rect().height() {
            high if high >= 760.0 => 170.0,
            high if high >= 600.0 => 130.0,
            _ => 96.0,
        };
        let size = if width * ratio > most {
            vec2(most / ratio, most)
        } else {
            vec2(width, width * ratio)
        };
        let (row, response) = ui.allocate_exact_size(vec2(width, size.y), Sense::hover());
        let frame = Rect::from_center_size(row.center(), size);
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
        if !ui.is_rect_visible(row) {
            return;
        }
        let painter = ui.painter_at(frame);
        let radius = CornerRadius::same(metrics::CONTROL_RADIUS);
        let Some(video) = video else {
            painter.rect_filled(frame, radius, p.sunken);
            icons::paint(
                &painter,
                Rect::from_center_size(frame.center() - vec2(0.0, 8.0), Vec2::splat(22.0)),
                Icon::Music,
                p.muted,
            );
            painter.text(
                frame.center() + vec2(0.0, 16.0),
                egui::Align2::CENTER_CENTER,
                "Sound only",
                theme::regular(12.0),
                p.muted,
            );
            return;
        };
        painter.rect_filled(frame, radius, Color32::BLACK);
        let geometry = Geometry::new(canvas, output, video.fit);
        let k = frame.width() / output[0].max(1) as f32;
        let drawn = Rect::from_min_size(
            frame.min + vec2(geometry.offset[0] as f32, geometry.offset[1] as f32) * k,
            vec2(geometry.drawn[0] as f32, geometry.drawn[1] as f32) * k,
        );
        // Corners are rounded only where the picture reaches them.
        let round = if drawn.contains_rect(frame.shrink(0.5)) || geometry.is_exact() {
            radius
        } else {
            CornerRadius::ZERO
        };
        match self.input.poster {
            // The output picture itself: its bars and crop are drawn in it.
            Some((texture, _)) => {
                painter.add(
                    egui::epaint::RectShape::filled(frame, radius, Color32::WHITE)
                        .with_texture(texture, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))),
                );
            }
            None => {
                painter.rect_filled(drawn, round, theme::mix(p.sunken, p.elevated, 0.6));
                icons::paint(
                    &painter,
                    Rect::from_center_size(drawn.intersect(frame).center(), Vec2::splat(22.0)),
                    Icon::Film,
                    p.muted,
                );
            }
        }
    }

    fn summary(&self, ui: &mut Ui, state: &ExportSheetState) {
        let p = self.p;
        let settings = &state.settings;
        let canvas = self.canvas();
        ui.spacing_mut().item_spacing.y = 0.0;
        let plan = self.plan.and_then(|plan| plan.as_ref().ok());
        if let Some(video) = &settings.video {
            let size = plan.and_then(|plan| plan.video.as_ref()).map_or_else(
                || video.resolution.size(canvas),
                |video| video.geometry.output,
            );
            fact(
                ui,
                p,
                "Size",
                &format!("{} × {} · {}", size[0], size[1], aspect(size)),
            );
            let rate = video.frame_rate.unwrap_or(self.rate());
            fact(ui, p, "Frame rate", &format!("{} fps", rate_label(rate)));
        }
        if let Some(audio) = &settings.audio {
            let channels = if audio.channels == 1 {
                "Mono"
            } else {
                "Stereo"
            };
            fact(
                ui,
                p,
                "Sound",
                &format!(
                    "{} · {} kHz · {channels}",
                    audio.codec.label(),
                    trim(audio.sample_rate as f64 / 1000.0)
                ),
            );
        } else {
            fact(ui, p, "Sound", "None");
        }
        match plan {
            Some(plan) => {
                let length = plan.end - plan.start;
                let frames = plan.video.as_ref().map(|video| video.frames.count);
                let duration = match frames {
                    Some(frames) => format!("{} · {frames} frames", timefmt::short(length)),
                    None => timefmt::short(length),
                };
                fact(ui, p, "Duration", &duration);
                fact(
                    ui,
                    p,
                    "Estimated size",
                    &format!(
                        "≈ {}",
                        timefmt::file_size(ExportSettings::estimate_bytes(plan))
                    ),
                );
                let encoder = match (&plan.video, &plan.audio) {
                    (Some(video), _) => encoder_label(video.encoder.hardware, &video.encoder.name),
                    (None, Some(audio)) => audio.encoder.name.clone(),
                    (None, None) => "None".to_owned(),
                };
                fact(ui, p, "Encoder", &encoder);
            }
            None => {
                if let Ok((start, end)) = settings.span(self.input.project) {
                    fact(ui, p, "Duration", &timefmt::short(end - start));
                }
            }
        }
        ui.spacing_mut().item_spacing.y = 6.0;
        for warning in self.input.warnings {
            caution(ui, p, warning, p.yellow);
        }
        if let Some(Err(error)) = self.plan {
            caution(ui, p, &error.to_string(), p.red);
        }
    }

    /// Presets, then the picture, the sound and the captions.
    fn right(&mut self, ui: &mut Ui, state: &mut ExportSheetState) {
        let p = self.p;
        let caps = match (self.input.caps, self.caps) {
            (_, Some(caps)) => caps,
            (CapabilityView::Failed(problem), _) => {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    let (glyph, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
                    icons::paint(ui.painter(), glyph, Icon::Warning, p.red);
                    ui.label(
                        RichText::new("The encoders could not be checked")
                            .font(theme::medium(13.0))
                            .color(p.fg),
                    );
                    note(ui, problem, p.secondary);
                    ui.add_space(4.0);
                    if widgets::button(ui, p, Some(Icon::Loop), "Try again", Kind::Plain)
                        .on_hover_text("Ask FFmpeg which encoders work again")
                        .clicked()
                    {
                        *self.answer = Some(ExportAnswer::Reprobe);
                    }
                });
                return;
            }
            _ => {
                ui.add_space(48.0);
                ui.vertical_centered(|ui| {
                    ui.add(egui::Spinner::new().size(18.0).color(p.muted));
                    ui.label(
                        RichText::new("Checking which encoders work on this computer…")
                            .font(theme::regular(12.5))
                            .color(p.secondary),
                    );
                });
                return;
            }
        };
        self.presets(ui, state, caps);
        self.video(ui, state, caps);
        self.audio(ui, state, caps);
        self.captions(ui, state);
    }

    /// The preset: one list grouped by destination, the person's own last,
    /// with what the chosen one is for under it, and the most used presets
    /// as chips on one row.
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
        // A preset this host cannot make is listed, disabled with why.
        // Whether it was chosen.
        let offer = |preset: &ExportPreset, response: egui::Response| {
            let problem = preset_problem(preset, caps);
            let response = match &problem {
                Some(problem) => response.on_disabled_hover_text(problem),
                None if preset.note.is_empty() => response,
                None => response.on_hover_text(&preset.note),
            };
            response.clicked() && problem.is_none()
        };
        row(ui, p, "Preset", |ui| {
            let room = ui.ctx().content_rect().height() - 160.0;
            egui::ComboBox::from_id_salt("export-preset")
                .icon(widgets::combo_icon)
                .selected_text(current.map_or("Custom settings", |preset| preset.name.as_str()))
                .width(ui.available_width().min(240.0))
                .height(room.max(160.0))
                .show_ui(ui, |ui| {
                    for destination in &destinations {
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(destination.label())
                                .font(theme::medium(11.0))
                                .color(p.muted),
                        );
                        for preset in presets.iter().filter(|p| p.destination == *destination) {
                            let enabled = preset_problem(preset, caps).is_none();
                            let selected = current_id.as_deref() == Some(preset.id.as_str());
                            let response = ui
                                .add_enabled_ui(enabled, |ui| {
                                    ui.selectable_label(selected, &preset.name)
                                })
                                .inner;
                            if offer(preset, response) {
                                chosen = Some(preset);
                            }
                        }
                    }
                })
                .response
                .widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, true, "Preset"));
        });
        // Always one line, so that a choice which turns the preset into
        // custom settings does not move the controls under the pointer.
        let said = match current {
            Some(preset) if !preset.note.is_empty() => preset.note.as_str(),
            Some(_) => "A preset of your own",
            None => "Changed from the presets; Save preset… keeps them",
        };
        // Cut short where it does not fit; the whole line is on hover.
        ui.add(
            egui::Label::new(
                RichText::new(said)
                    .font(theme::regular(11.5))
                    .color(p.muted),
            )
            .truncate(),
        );
        // The most used, as many as one row holds.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let mut room = ui.available_width();
            for id in QUICK {
                let Some(preset) = presets.iter().find(|preset| preset.id == id) else {
                    continue;
                };
                let width = ui
                    .painter()
                    .layout_no_wrap(preset.name.clone(), theme::medium(12.0), p.fg)
                    .size()
                    .x
                    + 22.0;
                if width > room {
                    break;
                }
                room -= width + 6.0;
                let enabled = preset_problem(preset, caps).is_none();
                let selected = current_id.as_deref() == Some(preset.id.as_str());
                let response = ui
                    .add_enabled_ui(enabled, |ui| widgets::chip(ui, p, &preset.name, selected))
                    .inner;
                if offer(preset, response) {
                    chosen = Some(preset);
                }
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
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            section(ui, p, "Video");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::switch(ui, p, on, "Video")
                    .on_hover_text(if on {
                        "Turn off to export sound only"
                    } else {
                        "Turn on to export the picture"
                    })
                    .clicked()
                {
                    on = !on;
                }
            });
        });
        if on != state.settings.video.is_some() {
            state.set_video(on, caps);
        }
        sheets::group(ui, p, |ui| {
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
            row(ui, p, "Format", |ui| {
                if offered.is_empty() {
                    ui.label(
                        RichText::new("This FFmpeg writes none")
                            .font(theme::regular(12.5))
                            .color(p.red),
                    );
                    return;
                }
                let mut container = state.settings.container;
                let width = (56.0 * offered.len() as f32).min(ui.available_width());
                if widgets::segmented(ui, p, &mut container, &offered, width) {
                    state.settings.container = container;
                    settle(&mut state.settings, caps);
                }
            });
            if !on {
                note(ui, "Sound only: the file has no picture.", p.muted);
                return;
            }
            let container = state.settings.container;
            let Some(video) = state.settings.video.as_mut() else {
                return;
            };
            let output = video.resolution.size(canvas);

            stacked(ui, p, "Resolution");
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
            let full = ui.available_width();
            if widgets::segmented(ui, p, &mut size, &sizes, full) {
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
                row(ui, p, "Size", |ui| {
                    // Laid out in reading order, which is the order Tab
                    // walks: width, then height.
                    ui.allocate_ui_with_layout(
                        vec2(160.0, metrics::legacy::CONTROL_HEIGHT),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            field(
                                ui,
                                p,
                                &mut state.width_text,
                                44.0,
                                "Width",
                                "Width in pixels",
                            );
                            ui.label(RichText::new("×").font(theme::regular(12.5)).color(p.muted));
                            field(
                                ui,
                                p,
                                &mut state.height_text,
                                44.0,
                                "Height",
                                "Height in pixels",
                            );
                            ui.label(
                                RichText::new("px")
                                    .font(theme::regular(12.0))
                                    .color(p.muted),
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
                row(ui, p, "Fit", |ui| {
                    widgets::segmented(
                        ui,
                        p,
                        &mut video.fit,
                        &[(Fit::Letterbox, "Letterbox"), (Fit::Fill, "Fill")],
                        170.0,
                    );
                });
            }

            row(ui, p, "Frame rate", |ui| {
                let project = self.rate();
                let shown = match video.frame_rate {
                    None => format!("Project ({})", rate_label(project)),
                    Some(rate) => format!("{} fps", rate_label(rate)),
                };
                egui::ComboBox::from_id_salt("export-frame-rate")
                    .icon(widgets::combo_icon)
                    .selected_text(shown)
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut video.frame_rate,
                            None,
                            format!("Project ({})", rate_label(project)),
                        );
                        for (numerator, denominator, label) in RATES {
                            let rate = FrameRate {
                                numerator,
                                denominator,
                            };
                            ui.selectable_value(
                                &mut video.frame_rate,
                                Some(rate),
                                format!("{label} fps"),
                            );
                        }
                    })
                    .response
                    .widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, true, "Frame rate"));
            });

            row(ui, p, "Codec", |ui| {
                let mut codec = video.codec;
                egui::ComboBox::from_id_salt("export-codec")
                    .icon(widgets::combo_icon)
                    .selected_text(codec.label())
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        for offered in container.video_codecs() {
                            let reason = (!caps.has_video_codec(*offered))
                                .then(|| codec_reason(caps, *offered));
                            option(ui, &mut codec, *offered, offered.label(), reason.as_deref());
                        }
                    })
                    .response
                    .widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, true, "Codec"));
                if codec != video.codec {
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
                stacked(ui, p, "Profile");
                let mut level = match video.rate_control {
                    RateControl::Quality { level } => level,
                    RateControl::Bitrate { .. } => QualityLevel::Recommended,
                };
                let full = ui.available_width();
                if widgets::segmented(
                    ui,
                    p,
                    &mut level,
                    &[
                        (QualityLevel::Lower, "LT"),
                        (QualityLevel::Recommended, "422"),
                        (QualityLevel::Higher, "HQ"),
                        (QualityLevel::Maximum, "4444"),
                    ],
                    full,
                ) {
                    video.rate_control = RateControl::Quality { level };
                }
            } else {
                let mut target = match video.rate_control {
                    RateControl::Quality { .. } => Target::Quality,
                    RateControl::Bitrate { .. } => Target::Bitrate,
                };
                row(ui, p, "Target", |ui| {
                    widgets::segmented(
                        ui,
                        p,
                        &mut target,
                        &[(Target::Quality, "Quality"), (Target::Bitrate, "Bitrate")],
                        170.0,
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
                        let full = ui.available_width();
                        widgets::segmented(ui, p, level, &levels, full);
                    }
                    RateControl::Bitrate { kbps } => {
                        let hint = suggested(video.codec, output, fps, QualityLevel::Recommended);
                        row(ui, p, "Bitrate", |ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.label(
                                RichText::new("kbit/s")
                                    .font(theme::regular(12.0))
                                    .color(p.muted),
                            );
                            field(
                                ui,
                                p,
                                &mut state.kbps_text,
                                64.0,
                                &hint.to_string(),
                                "Bitrate in kbit/s",
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

            ui.add_space(2.0);
            if widgets::disclosure(ui, p, state.encoding_open, "Encoder and speed", false).clicked()
            {
                state.encoding_open = !state.encoding_open;
            }
            if state.encoding_open {
                encoder_rows(ui, p, video, caps, output);
            }
        });
    }

    fn audio(&self, ui: &mut Ui, state: &mut ExportSheetState, caps: &Capabilities) {
        let p = self.p;
        let video = state.settings.video.is_some();
        let mut on = state.settings.audio.is_some();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            section(ui, p, "Audio");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let switch = ui
                    .add_enabled_ui(video, |ui| widgets::switch(ui, p, on, "Audio"))
                    .inner
                    .on_disabled_hover_text("A sound-only file needs its sound");
                if switch.clicked() && video {
                    on = !on;
                }
            });
        });
        if on != state.settings.audio.is_some() {
            state.set_audio(on, caps);
        }
        let container = state.settings.container;
        let Some(audio) = state.settings.audio.as_mut() else {
            return;
        };
        sheets::group(ui, p, |ui| {
            row(ui, p, "Codec", |ui| {
                let mut codec = audio.codec;
                egui::ComboBox::from_id_salt("export-audio-codec")
                    .icon(widgets::combo_icon)
                    .selected_text(codec.label())
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        for offered in container.audio_codecs() {
                            let reason = (!caps.has_audio_codec(*offered))
                                .then(|| format!("This FFmpeg has no {} encoder", offered.label()));
                            option(ui, &mut codec, *offered, offered.label(), reason.as_deref());
                        }
                    })
                    .response
                    .widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, true, "Audio codec"));
                if codec != audio.codec {
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
                row(ui, p, "Bitrate", |ui| {
                    egui::ComboBox::from_id_salt("export-audio-kbps")
                        .icon(widgets::combo_icon)
                        .selected_text(format!("{} kbit/s", audio.kbps))
                        .width(150.0)
                        .show_ui(ui, |ui| {
                            for kbps in offered {
                                let reason = (kbps > most).then(|| {
                                    format!("{} goes up to {most} kbit/s", audio.codec.label())
                                });
                                option(
                                    ui,
                                    &mut audio.kbps,
                                    kbps,
                                    &format!("{kbps} kbit/s"),
                                    reason.as_deref(),
                                );
                            }
                        })
                        .response
                        .widget_info(|| {
                            WidgetInfo::labeled(WidgetType::ComboBox, true, "Audio bitrate")
                        });
                });
            }
            let rates: Vec<(u32, String)> = (SAMPLE_RATES.into_iter())
                .filter(|rate| audio.codec.accepts_rate(*rate))
                .map(|rate| (rate, format!("{} kHz", trim(rate as f64 / 1000.0))))
                .collect();
            let rates: Vec<(u32, &str)> = rates.iter().map(|(r, l)| (*r, l.as_str())).collect();
            row(ui, p, "Sample rate", |ui| {
                let width = (72.0 * rates.len() as f32).min(ui.available_width());
                widgets::segmented(ui, p, &mut audio.sample_rate, &rates, width);
            });
            row(ui, p, "Channels", |ui| {
                if widgets::segmented(
                    ui,
                    p,
                    &mut audio.channels,
                    &[(1, "Mono"), (2, "Stereo")],
                    150.0,
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
        ui.add_space(4.0);
        section(ui, p, "Captions");
        sheets::group(ui, p, |ui| {
            row(ui, p, "Burn into the picture", |ui| {
                let switch = ui
                    .add_enabled_ui(video, |ui| {
                        widgets::switch(ui, p, subtitles.burn_in, "Burn captions into the picture")
                    })
                    .inner
                    .on_disabled_hover_text("A sound-only file has no picture");
                if switch.clicked() && video {
                    subtitles.burn_in = !subtitles.burn_in;
                }
            });
            row(ui, p, "Subtitle file", |ui| {
                widgets::segmented(
                    ui,
                    p,
                    &mut subtitles.sidecar,
                    &[
                        (None, "None"),
                        (Some(SidecarFormat::Srt), "SRT"),
                        (Some(SidecarFormat::Vtt), "VTT"),
                    ],
                    170.0,
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
            row(ui, p, "Subtitle stream", |ui| {
                let switch = ui
                    .add_enabled_ui(refusal.is_none(), |ui| {
                        widgets::switch(ui, p, subtitles.embed, "Subtitle stream in the file")
                    })
                    .inner
                    .on_hover_text("Viewers can turn the captions on and off")
                    .on_disabled_hover_text(refusal.as_deref().unwrap_or_default());
                if switch.clicked() && refusal.is_none() {
                    subtitles.embed = !subtitles.embed;
                }
            });
            if subtitles.sidecar.is_some() || subtitles.embed {
                let chosen = subtitles
                    .track
                    .filter(|id| tracks.iter().any(|(track, _)| track == id))
                    .unwrap_or(tracks[0].0);
                let mut pick = chosen;
                row(ui, p, "Track", |ui| {
                    let shown = (tracks.iter())
                        .find(|(id, _)| *id == chosen)
                        .map_or("", |(_, name)| name);
                    egui::ComboBox::from_id_salt("export-caption-track")
                        .icon(widgets::combo_icon)
                        .selected_text(shown)
                        .width(150.0)
                        .show_ui(ui, |ui| {
                            for (id, name) in &tracks {
                                ui.selectable_value(&mut pick, *id, *name);
                            }
                        })
                        .response
                        .widget_info(|| {
                            WidgetInfo::labeled(WidgetType::ComboBox, true, "Caption track")
                        });
                });
                if pick != chosen {
                    subtitles.track = Some(pick);
                }
            }
        });
    }
}

/// A row's label alone, over a control as wide as the row.
fn stacked(ui: &mut Ui, p: &Palette, label: &str) {
    ui.add_space(2.0);
    ui.label(
        RichText::new(label)
            .font(theme::regular(12.5))
            .color(p.secondary),
    );
}

/// The encoder choice and its effort.
fn encoder_rows(
    ui: &mut Ui,
    p: &Palette,
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
    row(ui, p, "Encoder", |ui| {
        let mut choice = video.encoder;
        egui::ComboBox::from_id_salt("export-encoder")
            .icon(widgets::combo_icon)
            .selected_text(label(choice))
            .width(170.0)
            .show_ui(ui, |ui| {
                option(
                    ui,
                    &mut choice,
                    EncoderChoice::Auto,
                    &label(EncoderChoice::Auto),
                    None,
                );
                for candidate in [EncoderChoice::Software]
                    .into_iter()
                    .chain(families.iter().map(|kind| EncoderChoice::Hardware(*kind)))
                {
                    let reason = choice_reason(caps, candidate, codec, output);
                    option(
                        ui,
                        &mut choice,
                        candidate,
                        &label(candidate),
                        reason.as_deref(),
                    );
                }
            })
            .response
            .widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, true, "Encoder"));
        video.encoder = choice;
    });
    if codec != VideoCodec::ProRes {
        row(ui, p, "Speed", |ui| {
            widgets::segmented(
                ui,
                p,
                &mut video.speed,
                &[
                    (EncodeSpeed::Faster, "Faster"),
                    (EncodeSpeed::Balanced, "Balanced"),
                    (EncodeSpeed::Smaller, "Smaller"),
                ],
                210.0,
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

/// The folder the file goes in, as a field-like button, named with the
/// whole path for accessibility.
fn folder_button(ui: &mut Ui, p: &Palette, text: &str, folder: Option<&Path>) -> egui::Response {
    let missing = folder.is_none();
    let size = vec2(ui.available_width(), metrics::legacy::CONTROL_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| {
        let name = match folder {
            Some(folder) => format!("Folder: {}", folder.display()),
            None => "Folder: none chosen".to_owned(),
        };
        WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), name)
    });
    if ui.is_rect_visible(rect) {
        let radius = CornerRadius::same(metrics::CONTROL_RADIUS);
        let fill = if response.is_pointer_button_down_on() {
            p.pressed
        } else if response.hovered() {
            p.hover
        } else {
            p.control
        };
        let painter = ui.painter();
        painter.rect_filled(rect, radius, fill);
        if response.has_focus() {
            painter.rect_stroke(
                rect,
                radius,
                egui::Stroke::new(1.5, p.accent),
                egui::StrokeKind::Inside,
            );
        }
        icons::paint(
            painter,
            Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), Vec2::splat(14.0)),
            Icon::Folder,
            p.secondary,
        );
        let ink = if missing { p.muted } else { p.fg };
        let galley = widgets::truncated(ui, text, theme::regular(12.5), ink, rect.width() - 40.0);
        ui.painter().galley(
            pos2(rect.left() + 30.0, rect.center().y - galley.size().y * 0.5),
            galley,
            ink,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::Default)
}

#[cfg(test)]
#[path = "export_sheet_tests.rs"]
mod tests;
