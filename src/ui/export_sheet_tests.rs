//! The export sheet driven as a person drives it.
//!
//! [`Bench`] shows the sheet in real egui passes over a window of a given
//! size, with capabilities written by hand (no FFmpeg is run), and keeps
//! what it answered. A control is found by the name accessibility is given
//! for it, scrolled into view and pressed with the pointer.
//!
//! `FIXTURE:` is a precondition of the test itself and proves nothing about
//! the sheet.
use super::*;
use crate::theme::{Accent, Appearance};
use editor_core::{
    Clip, Track, TrackKind,
    export::{AudioEncoder, Unavailable, VideoEncoder, builtin_presets},
};
use eframe::egui::{
    Event, Key, Modifiers, PointerButton, Pos2, Shape,
    accesskit::{Role, Toggled},
};
use std::sync::Arc;

const WIDE: Vec2 = vec2(1440.0, 900.0);
const NARROW: Vec2 = vec2(760.0, 560.0);

fn seconds(value: f64) -> Time {
    Time::from_seconds(value)
}

fn video(
    name: &str,
    codec: VideoCodec,
    hardware: Option<HardwareKind>,
    max_side: u32,
) -> VideoEncoder {
    VideoEncoder {
        name: name.to_owned(),
        codec,
        hardware,
        max_side,
        min_size: if hardware.is_some() {
            [146, 50]
        } else {
            [16, 16]
        },
        device: None,
    }
}

/// A host with H.264 in software and on NVENC, VP9 and ProRes in software,
/// no H.265 or AV1 (NVENC's H.265 failed its test), and no MP3 encoder or
/// muxer.
fn host() -> Capabilities {
    Capabilities {
        ffmpeg: "ffmpeg version 8.0.1".to_owned(),
        video: vec![
            video("libx264", VideoCodec::H264, None, 8192),
            video("libvpx-vp9", VideoCodec::Vp9, None, 8192),
            video("prores_ks", VideoCodec::ProRes, None, 8192),
            video(
                "h264_nvenc",
                VideoCodec::H264,
                Some(HardwareKind::Nvenc),
                4096,
            ),
        ],
        audio: [
            ("aac", AudioCodec::Aac),
            ("libopus", AudioCodec::Opus),
            ("flac", AudioCodec::Flac),
            ("pcm_s16le", AudioCodec::Pcm16),
            ("pcm_s24le", AudioCodec::Pcm24),
        ]
        .into_iter()
        .map(|(name, codec)| AudioEncoder {
            name: name.to_owned(),
            codec,
        })
        .collect(),
        containers: Container::ALL
            .into_iter()
            .filter(|c| *c != Container::Mp3)
            .collect(),
        unavailable: vec![Unavailable {
            name: "hevc_nvenc".to_owned(),
            reason: "[hevc_nvenc @ 0x5] No capable devices found".to_owned(),
        }],
    }
}

/// A ten-second project: one title from zero.
fn project() -> Project {
    let mut project = Project::new("Trip");
    let text = (project.tracks.iter_mut())
        .find(|track| track.kind == TrackKind::Text)
        .expect("FIXTURE: a new project has a text track");
    Arc::make_mut(text)
        .clips
        .push(Clip::text("Hello", Time::ZERO, seconds(10.0)));
    project
}

/// The project with an English caption track holding one caption.
fn captioned() -> Project {
    let mut project = project();
    let mut track = Track::new("English captions", TrackKind::Text);
    track.language = Some("en".to_owned());
    track
        .clips
        .push(Clip::text("Hi there", seconds(1.0), seconds(2.0)));
    project.tracks.push(Arc::new(track));
    project
}

#[derive(Clone, Debug)]
enum Probe {
    Probing,
    Ready(Capabilities),
    Failed(String),
}

#[derive(Debug)]
struct Seen {
    name: String,
    rect: Rect,
    role: Role,
    disabled: bool,
    toggled: Option<Toggled>,
}

struct Bench {
    ctx: egui::Context,
    palette: Palette,
    size: Vec2,
    /// Physical pixels to a point, as the display gives them.
    scale: f32,
    time: f64,
    state: ExportSheetState,
    project: Project,
    probe: Probe,
    presets: Vec<ExportPreset>,
    selection: Option<(Time, Time)>,
    queue_len: usize,
    poster: Option<(egui::TextureId, Vec2)>,
    seen: Vec<Seen>,
    /// Where the scroll bars are, at their widest.
    bars: Vec<Rect>,
    painted: Vec<(String, Color32)>,
    answers: Vec<ExportAnswer>,
}

impl Bench {
    fn new(size: Vec2) -> Self {
        Self::with(size, project(), Some(PathBuf::from("/exports")))
    }

    fn with(size: Vec2, project: Project, folder: Option<PathBuf>) -> Self {
        let ctx = egui::Context::default();
        let palette = Palette::new(Appearance::Dark, Accent::default());
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette);
        ctx.enable_accesskit();
        let mut bench = Self {
            ctx,
            palette,
            size,
            scale: 1.0,
            time: 0.0,
            state: ExportSheetState::new(ExportSettings::default(), "Trip".to_owned(), folder),
            project,
            probe: Probe::Ready(host()),
            presets: builtin_presets(),
            selection: None,
            queue_len: 0,
            poster: None,
            seen: Vec::new(),
            bars: Vec::new(),
            painted: Vec::new(),
            answers: Vec::new(),
        };
        bench.settle();
        bench
    }

    /// One frame of the application with these events in it.
    fn frame(&mut self, events: Vec<Event>) {
        self.time += 0.05;
        let ctx = self.ctx.clone();
        let mut raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        raw.viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(self.scale);
        let caps = match &self.probe {
            Probe::Probing => CapabilityView::Probing,
            Probe::Ready(caps) => CapabilityView::Ready(caps),
            Probe::Failed(problem) => CapabilityView::Failed(problem),
        };
        let warnings = ["One clip is offline and renders black".to_owned()];
        let input = ExportSheetInput {
            project: &self.project,
            caps,
            presets: &self.presets,
            selection: self.selection,
            playhead: seconds(2.0),
            poster: self.poster,
            poster_problem: None,
            warnings: &warnings,
            queue_len: self.queue_len,
        };
        let mut answer = None;
        let output = ctx.run_ui(raw, |ui| {
            answer = show(ui.ctx(), &self.palette, &mut self.state, &input);
        });
        if let Some(update) = &output.platform_output.accesskit_update {
            self.seen = (update.nodes.iter())
                .filter_map(|(_, node)| {
                    let bounds = node.bounds()?;
                    Some(Seen {
                        name: node.label().or_else(|| node.value())?.to_owned(),
                        rect: Rect::from_min_max(
                            pos2(bounds.x0 as f32, bounds.y0 as f32),
                            pos2(bounds.x1 as f32, bounds.y1 as f32),
                        ),
                        role: node.role(),
                        disabled: node.is_disabled(),
                        toggled: node.toggled(),
                    })
                })
                .collect();
            self.bars = (update.nodes.iter())
                .filter(|(_, node)| node.role() == Role::ScrollBar)
                .filter_map(|(_, node)| node.bounds())
                .map(|b| {
                    Rect::from_min_max(
                        pos2(b.x0 as f32, b.y0 as f32),
                        pos2(b.x1 as f32, b.y1 as f32),
                    )
                })
                .collect();
        }
        self.painted.clear();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut self.painted);
        }
        output.drop_without_applying_deltas();
        self.answers.extend(answer);
    }

    /// Frames until nothing is still arriving; nothing may be answered.
    fn settle(&mut self) {
        for _ in 0..4 {
            self.frame(Vec::new());
        }
        assert!(
            self.answers.is_empty(),
            "FIXTURE: a sheet nobody touched answered {:?}",
            self.answers
        );
    }

    fn find(&self, name: &str) -> Option<&Seen> {
        (self.seen.iter())
            // Spec §5.9.1: the sheet itself is a Window named by its title,
            // "Export", which is not the button of that name.
            .find(|seen| {
                seen.name == name
                    && !matches!(seen.role, Role::Label | Role::TextRun | Role::Window)
            })
    }

    fn get(&self, name: &str) -> &Seen {
        self.find(name).unwrap_or_else(|| {
            let names: Vec<&str> = self.seen.iter().map(|seen| seen.name.as_str()).collect();
            panic!("FIXTURE: no control is named \"{name}\" among {names:?}")
        })
    }

    /// The preset chip `name`, a button, and not the quality level or the
    /// list row of the same name.
    fn chip(&self, name: &str) -> &Seen {
        (self.seen.iter())
            .find(|seen| seen.name == name && seen.role == Role::Button)
            .unwrap_or_else(|| panic!("FIXTURE: no chip is named \"{name}\""))
    }

    fn says(&self, text: &str) -> bool {
        self.seen.iter().any(|seen| seen.name.contains(text))
            || self
                .painted
                .iter()
                .any(|(painted, _)| painted.contains(text))
    }

    fn painted_in(&self, text: &str) -> Option<Color32> {
        (self.painted.iter())
            .find(|(painted, _)| painted.contains(text))
            .map(|(_, color)| *color)
    }

    /// The band the sheet's scrolled columns show: under the header's close
    /// button and over the footer's buttons.
    fn band(&self) -> (f32, f32) {
        (
            self.get("Close export").rect.bottom(),
            self.get("Cancel").rect.top(),
        )
    }

    /// Scrolls the column holding `name` until it is in sight.
    fn reveal(&mut self, name: &str) {
        for _ in 0..8 {
            self.frame(Vec::new());
            let (top, bottom) = self.band();
            let rect = self.get(name).rect;
            // The header's and the footer's controls do not scroll.
            let chrome = [
                "Close export",
                "Export",
                "Add to queue",
                "Cancel",
                "Save preset…",
            ]
            .contains(&name)
                || [
                    "Render queue (",
                    "Delete preset",
                    "Preset name",
                    "Save",
                    "Stop naming",
                ]
                .iter()
                .any(|start| name.starts_with(start));
            if chrome || (rect.top() >= top && rect.bottom() <= bottom) {
                return;
            }
            let middle = (top + bottom) * 0.5;
            let at = pos2(rect.center().x, middle);
            self.frame(vec![Event::PointerMoved(at)]);
            self.frame(vec![Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, (middle - rect.center().y).clamp(-400.0, 400.0)),
                phase: egui::TouchPhase::Move,
                modifiers: Modifiers::NONE,
            }]);
            for _ in 0..12 {
                self.frame(Vec::new());
            }
        }
        panic!("FIXTURE: \"{name}\" could not be scrolled into sight");
    }

    fn click_at(&mut self, at: Pos2) {
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]);
        self.frame(Vec::new());
    }

    fn click(&mut self, name: &str) {
        self.reveal(name);
        let at = self.get(name).rect.center();
        self.click_at(at);
    }

    fn key(&mut self, key: Key) {
        self.frame(vec![Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        self.frame(Vec::new());
    }

    /// Replaces what the field `name` holds with `text`.
    fn retype(&mut self, name: &str, text: &str) {
        self.click(name);
        self.frame(vec![Event::Key {
            key: Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        }]);
        self.frame(vec![Event::Text(text.to_owned())]);
        self.frame(Vec::new());
    }

    fn take(&mut self) -> Vec<ExportAnswer> {
        std::mem::take(&mut self.answers)
    }

    /// Rests the pointer on `name` until its tooltip shows.
    fn hover(&mut self, name: &str) {
        self.reveal(name);
        let at = self.get(name).rect.center();
        self.frame(vec![Event::PointerMoved(at)]);
        for _ in 0..30 {
            self.frame(Vec::new());
        }
    }

    /// Opens the preset list.
    fn open_presets(&mut self) {
        self.click("Preset");
        assert!(
            egui::Popup::is_any_open(&self.ctx),
            "FIXTURE: the preset list opens"
        );
    }

    /// Whether `name` is shown whole between the header and the footer.
    fn in_sight(&self, name: &str) -> bool {
        let (top, bottom) = self.band();
        let rect = self.get(name).rect;
        rect.top() >= top && rect.bottom() <= bottom
    }
}

fn collect(shape: &Shape, into: &mut Vec<(String, Color32)>) {
    match shape {
        Shape::Vec(shapes) => {
            for shape in shapes {
                collect(shape, into);
            }
        }
        Shape::Text(text) => {
            let color = (text.galley.job.sections.first())
                .map_or(text.fallback_color, |section| section.format.color);
            let color = if color == Color32::PLACEHOLDER {
                text.fallback_color
            } else {
                color
            };
            into.push((text.galley.text().to_owned(), color));
        }
        _ => {}
    }
}

// ---- Layout -----------------------------------------------------------------

#[test]
fn the_sheet_shows_presets_and_every_section_at_both_sizes() {
    for size in [WIDE, NARROW] {
        let bench = Bench::new(size);
        for name in [
            "Preset",
            "Recommended",
            "YouTube 1080p",
            "TikTok",
            "File name",
            "Folder: /exports",
            "Whole timeline",
            "Video",
            "Codec",
            "Frame rate",
            "Audio",
            "Audio codec",
            "Export",
            "Add to queue",
            "Save preset…",
        ] {
            assert!(bench.find(name).is_some(), "{name} is offered at {size:?}");
        }
        assert!(bench.says("Estimated size") && bench.says("NVIDIA NVENC · h264_nvenc"));
        assert!(bench.says("1920 × 1080 · 16:9"));
        // The person's own settings and a preset both start as Recommended.
        assert_eq!(bench.chip("Recommended").toggled, Some(Toggled::True));
        assert!(bench.says("Recommended"), "the preset list shows it chosen");

        // Spec §5.9.3: two columns, the picture on the left and the file
        // card first among the choices on the right, before the presets;
        // the footer inside the window.
        let file = bench.get("File name").rect;
        let preset = bench.get("YouTube 1080p").rect;
        assert!(file.bottom() <= preset.top(), "{file:?} before {preset:?}");
        let poster = (bench.seen.iter())
            .find(|seen| seen.name.starts_with("Output frame"))
            .expect("the poster is there")
            .rect;
        assert!(poster.right() < file.left(), "{poster:?} beside {file:?}");
        let export = bench.get("Export").rect;
        assert!(
            export.right() <= size.x && export.bottom() <= size.y,
            "{export:?} is inside {size:?}"
        );
        // Captions are offered only to a project with a caption track.
        assert!(bench.find("Burn captions into the picture").is_none());
    }
}

#[test]
fn a_window_too_narrow_for_two_columns_stacks_them() {
    let mut bench = Bench::new(vec2(640.0, 560.0));
    let file = bench.get("File name").rect;
    bench.reveal("YouTube 1080p");
    let preset = bench.get("YouTube 1080p").rect;
    assert!(
        preset.left() < file.right(),
        "the presets come under the file: {file:?}, {preset:?}"
    );
    let export = bench.get("Export").rect;
    assert!(export.right() <= 640.0 && export.bottom() <= 560.0);
}

// ---- Presets and options ----------------------------------------------------

#[test]
fn clicking_a_preset_changes_the_settings_until_one_is_changed() {
    let mut bench = Bench::new(WIDE);
    bench.click("YouTube 1080p");
    let settings = bench.state.settings().clone();
    let video = settings.video.expect("a picture");
    assert_eq!(video.resolution, Resolution::P1080);
    assert_eq!(
        video.rate_control,
        RateControl::Quality {
            level: QualityLevel::Higher
        }
    );
    assert_eq!(settings.audio.map(|a| a.kbps), Some(384));
    assert_eq!(bench.state.preset(), Some("youtube-1080"));
    assert_eq!(bench.chip("YouTube 1080p").toggled, Some(Toggled::True));
    assert_eq!(bench.chip("Recommended").toggled, Some(Toggled::False));
    assert!(bench.take().is_empty(), "a preset is not an answer");

    // Another size is no longer the preset, and the control clicked stays
    // under the pointer: the preset's line is kept, saying so.
    bench.reveal("720p");
    let before = bench.get("720p").rect;
    bench.click("720p");
    assert_eq!(
        bench.state.settings().video.unwrap().resolution,
        Resolution::P720
    );
    assert_eq!(bench.state.preset(), None);
    assert_eq!(bench.get("YouTube 1080p").toggled, Some(Toggled::False));
    assert!(bench.says("Custom settings"));
    assert!(bench.says("Changed from the presets"));
    assert!(bench.says("1280 × 720"));
    assert_eq!(
        bench.get("720p").rect,
        before,
        "nothing moved under the click"
    );
}

#[test]
fn what_this_host_cannot_encode_is_disabled_with_the_reason() {
    let mut bench = Bench::new(WIDE);
    // Presets that need H.265 or MP3, listed and disabled.
    bench.open_presets();
    for name in ["YouTube 4K", "Small file", "Audio (MP3)"] {
        assert!(bench.get(name).disabled, "{name} is disabled");
    }
    assert!(!bench.get("Web (VP9)").disabled);
    bench.key(Key::Escape);
    assert!(bench.take().is_empty(), "Escape closes the list first");

    // The codec list holds what MP4 carries; what this host lacks is there
    // but cannot be chosen.
    bench.click("Codec");
    for (codec, disabled) in [("H.264", false), ("H.265 (HEVC)", true), ("AV1", true)] {
        let seen = (bench.seen.iter())
            .find(|seen| seen.name == codec && seen.role == Role::Button)
            .unwrap_or_else(|| panic!("{codec} is listed"));
        assert_eq!(seen.disabled, disabled, "{codec}");
    }
    let hevc = (bench.seen.iter())
        .find(|seen| seen.name == "H.265 (HEVC)" && seen.role == Role::Button)
        .unwrap()
        .rect
        .center();
    bench.click_at(hevc);
    assert_eq!(
        bench.state.settings().video.unwrap().codec,
        VideoCodec::H264,
        "a disabled codec is not taken"
    );
    if egui::Popup::is_any_open(&bench.ctx) {
        bench.key(Key::Escape);
        assert!(
            bench.take().is_empty(),
            "Escape closes the open list first, not the sheet"
        );
    }

    // NVENC is offered, and refused for a size it cannot encode.
    bench.click("Encoder and speed");
    bench.click("Encoder");
    assert!(bench.says("Auto (NVIDIA NVENC)"));
    let nvenc = (bench.seen.iter())
        .find(|seen| seen.name == "NVIDIA NVENC" && seen.role == Role::Button)
        .expect("NVENC is offered");
    assert!(!nvenc.disabled);
}

#[test]
fn turning_the_picture_off_makes_a_sound_only_file_and_back() {
    let mut bench = Bench::new(WIDE);
    bench.click("Video");
    let settings = bench.state.settings().clone();
    assert_eq!(settings.video, None);
    assert_eq!(settings.container, Container::M4a, "AAC goes in an M4A");
    assert!(settings.audio.is_some());
    for format in ["M4A", "WAV", "FLAC"] {
        assert!(bench.find(format).is_some(), "{format} is offered");
    }
    assert!(bench.find("MP3").is_none(), "this host writes no MP3");
    assert!(bench.find("MP4").is_none());
    assert!(bench.get("Audio").disabled, "sound-only keeps its sound");
    assert!(bench.says("Sound only"));
    assert!(bench.state.destination().unwrap().ends_with("Trip.m4a"));

    bench.click("WAV");
    let settings = bench.state.settings().clone();
    assert_eq!(settings.container, Container::Wav);
    assert_eq!(settings.audio.unwrap().codec, AudioCodec::Pcm16);

    bench.click("Video");
    let settings = bench.state.settings().clone();
    assert_eq!(settings.container, Container::Mp4);
    assert_eq!(settings.video.unwrap().codec, VideoCodec::H264);
    assert_eq!(
        settings.audio.unwrap().codec,
        AudioCodec::Aac,
        "MP4 carries AAC"
    );
}

#[test]
fn a_format_change_keeps_only_codecs_it_carries() {
    let mut bench = Bench::new(WIDE);
    bench.click("WebM");
    let settings = bench.state.settings().clone();
    assert_eq!(settings.container, Container::Webm);
    assert_eq!(settings.video.unwrap().codec, VideoCodec::Vp9);
    assert_eq!(settings.audio.unwrap().codec, AudioCodec::Opus);
    assert_eq!(settings.audio.unwrap().sample_rate, 48_000);
    assert!(bench.state.destination().unwrap().ends_with("Trip.webm"));
}

#[test]
fn a_custom_size_is_typed_and_fit_is_offered_for_another_aspect() {
    let mut bench = Bench::new(WIDE);
    assert!(
        bench.find("Letterbox").is_none(),
        "the project's aspect needs no fit"
    );
    bench.click("Custom");
    bench.retype("Width in pixels", "1080");
    bench.retype("Height in pixels", "1920");
    assert_eq!(
        bench.state.settings().video.unwrap().resolution,
        Resolution::Custom {
            width: 1080,
            height: 1920
        }
    );
    assert!(bench.find("Letterbox").is_some() && bench.find("Fill").is_some());
    bench.click("Fill");
    assert_eq!(bench.state.settings().video.unwrap().fit, Fit::Fill);

    bench.retype("Width in pixels", "wide");
    assert!(bench.get("Export").disabled);
}

// ---- Queueing ---------------------------------------------------------------

#[test]
fn export_queues_the_file_in_the_folder_with_its_extension() {
    let mut bench = Bench::new(WIDE);
    bench.click("Export");
    let answers = bench.take();
    let Some(ExportAnswer::Queue {
        settings,
        destination,
        start_now,
    }) = answers.first()
    else {
        panic!("Export queues: {answers:?}");
    };
    assert_eq!(destination, &PathBuf::from("/exports/Trip.mp4"));
    assert!(start_now);
    assert_eq!(settings, bench.state.settings());

    bench.click("MOV");
    bench.click("Add to queue");
    let answers = bench.take();
    assert!(
        matches!(
            answers.as_slice(),
            [ExportAnswer::Queue { destination, start_now: false, .. }]
                if destination == &PathBuf::from("/exports/Trip.mov")
        ),
        "{answers:?}"
    );
}

#[test]
fn a_typed_extension_is_not_doubled_and_a_bad_name_is_refused() {
    let mut bench = Bench::new(WIDE);
    bench.retype("File name", "Holiday.mp4");
    assert_eq!(
        bench.state.destination(),
        Some(PathBuf::from("/exports/Holiday.mp4"))
    );
    bench.retype("File name", "a/b");
    assert!(bench.get("Export").disabled);
    bench.click("Export");
    assert!(bench.take().is_empty());
    bench.retype("File name", "  ");
    assert!(bench.get("Export").disabled);
}

#[test]
fn export_without_a_folder_asks_for_one_and_then_queues() {
    let mut bench = Bench::with(WIDE, project(), None);
    assert!(bench.says("Choose a folder…"));
    bench.click("Export");
    assert_eq!(bench.take(), [ExportAnswer::ChooseFolder]);

    // The dialog was cancelled: nothing is queued.
    bench.state.folder_chosen(None);
    bench.frame(Vec::new());
    assert!(bench.take().is_empty());

    bench.click("Export");
    assert_eq!(bench.take(), [ExportAnswer::ChooseFolder]);
    bench.state.folder_chosen(Some(PathBuf::from("/films")));
    bench.frame(Vec::new());
    let answers = bench.take();
    assert!(
        matches!(
            answers.as_slice(),
            [ExportAnswer::Queue { destination, start_now: true, .. }]
                if destination == &PathBuf::from("/films/Trip.mp4")
        ),
        "{answers:?}"
    );
    bench.frame(Vec::new());
    assert!(bench.take().is_empty(), "it is queued once");

    // The folder button asks too; it is named with the folder.
    bench.click("Folder: /films");
    assert_eq!(bench.take(), [ExportAnswer::ChooseFolder]);
}

#[test]
fn a_plan_that_fails_disables_export_and_says_why() {
    let mut bench = Bench::with(WIDE, Project::new("Empty"), Some(PathBuf::from("/exports")));
    assert!(bench.get("Export").disabled && bench.get("Add to queue").disabled);
    assert!(bench.says("The timeline is empty"));
    assert_eq!(
        bench.painted_in("The timeline is empty"),
        Some(bench.palette.red)
    );
    bench.click("Export");
    assert!(bench.take().is_empty());
}

#[test]
fn the_range_follows_the_selection_or_typed_times() {
    let mut bench = Bench::new(WIDE);
    assert!(bench.get("Selected clips").disabled, "nothing is selected");
    bench.selection = Some((seconds(2.0), seconds(4.0)));
    bench.click("Selected clips");
    assert_eq!(bench.state.range(), RangeChoice::Selection);
    assert!(bench.says("0:02 · 60 frames"));
    bench.click("Export");
    let answers = bench.take();
    assert!(
        matches!(
            answers.as_slice(),
            [ExportAnswer::Queue { settings, .. }]
                if settings.range == ExportRange::Span { start: seconds(2.0), end: seconds(4.0) }
        ),
        "{answers:?}"
    );

    // Custom starts from the selection and is typed.
    bench.click("Custom range");
    assert_eq!(bench.state.range(), RangeChoice::Custom);
    bench.retype("Range end", "8");
    bench.frame(Vec::new());
    assert_eq!(
        bench.state.settings().range,
        ExportRange::Span {
            start: seconds(2.0),
            end: seconds(8.0)
        }
    );
    bench.retype("Range end", "1");
    assert!(bench.says("The range ends before it starts"));
    assert!(bench.get("Export").disabled);
}

// ---- Encoders being checked -------------------------------------------------

#[test]
fn while_encoders_are_checked_or_failed_nothing_is_queued() {
    let mut bench = Bench::new(NARROW);
    bench.probe = Probe::Probing;
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    assert!(bench.get("Export").disabled);
    assert!(bench.says("Checking encoders…"));
    assert!(
        bench.find("Codec").is_none(),
        "nothing is offered unchecked"
    );

    bench.probe = Probe::Failed("ffmpeg was not found".to_owned());
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    assert!(bench.get("Export").disabled);
    assert!(bench.says("ffmpeg was not found"));
    bench.click("Try again");
    assert_eq!(bench.take(), [ExportAnswer::Reprobe]);
}

#[test]
fn settings_this_host_cannot_make_are_moved_when_it_is_known() {
    let mut settings = ExportSettings::default();
    settings.video.as_mut().unwrap().codec = VideoCodec::H265;
    let mut state = ExportSheetState::new(settings, "Trip".to_owned(), None);
    let mut bench = Bench::new(WIDE);
    std::mem::swap(&mut bench.state, &mut state);
    bench.frame(Vec::new());
    assert_eq!(
        bench.state.settings().video.unwrap().codec,
        VideoCodec::H264,
        "no H.265 encoder works here"
    );
}

// ---- Captions, presets of one's own, closing ----------------------------------

#[test]
fn captions_are_offered_with_a_caption_track() {
    let mut bench = Bench::with(WIDE, captioned(), Some(PathBuf::from("/exports")));
    assert!(bench.find("Burn captions into the picture").is_some());
    assert!(
        bench.find("Caption track").is_none(),
        "no file or stream yet"
    );
    bench.click("SRT");
    assert_eq!(
        bench.state.settings().subtitles.sidecar,
        Some(SidecarFormat::Srt)
    );
    assert!(bench.find("Caption track").is_some());
    bench.click("Subtitle stream in the file");
    assert!(bench.state.settings().subtitles.embed);

    // WAV carries no subtitle stream.
    bench.click("Video");
    assert!(!bench.state.settings().subtitles.embed);
    assert!(bench.get("Subtitle stream in the file").disabled);
}

#[test]
fn a_preset_is_saved_by_name_and_ones_own_can_be_deleted() {
    let mut bench = Bench::new(WIDE);
    bench.click("720p");
    bench.click("Save preset…");
    bench.frame(vec![Event::Text("Draft".to_owned())]);
    bench.click("Save");
    let answers = bench.take();
    let [ExportAnswer::SavePreset { name, settings }] = answers.as_slice() else {
        panic!("{answers:?}");
    };
    assert_eq!(name, "Draft");
    assert_eq!(settings.video.unwrap().resolution, Resolution::P720);
    assert_eq!(settings.range, ExportRange::Timeline);

    // Kept by the application, it comes back as the person's.
    bench.presets.push(ExportPreset {
        id: "mine".to_owned(),
        name: name.clone(),
        destination: Destination::Custom,
        note: String::new(),
        settings: settings.clone(),
    });
    bench.frame(Vec::new());
    assert!(bench.says("Draft"), "the preset list shows it chosen");
    bench.open_presets();
    assert!(
        bench.says("Saved"),
        "the person's own are listed under Saved"
    );
    assert_eq!(bench.get("Draft").role, Role::Button);
    bench.key(Key::Escape);
    bench.click("Delete preset “Draft”");
    assert_eq!(
        bench.take(),
        [ExportAnswer::DeletePreset("mine".to_owned())]
    );
}

#[test]
fn escape_gives_up_a_preset_name_then_closes() {
    let mut bench = Bench::new(WIDE);
    bench.click("Save preset…");
    assert!(bench.find("Preset name").is_some());
    // The name is typed at the footer's leading edge, away from Cancel,
    // which closes the whole sheet and stays beside Add to queue.
    let (cancel, add) = (bench.get("Cancel").rect, bench.get("Add to queue").rect);
    assert!(
        (add.left() - cancel.right() - 8.0).abs() < 0.5,
        "Cancel {cancel:?} is beside Add to queue {add:?}"
    );
    let stop = bench.get("Stop naming the preset").rect;
    assert!(
        stop.right() + 40.0 < cancel.left(),
        "the naming {stop:?} keeps away from Cancel {cancel:?}"
    );
    bench.key(Key::Escape);
    assert!(bench.find("Preset name").is_none());
    assert!(bench.take().is_empty());
    bench.key(Key::Escape);
    assert_eq!(bench.take(), [ExportAnswer::Close]);

    bench.click("Close export");
    assert_eq!(bench.take(), [ExportAnswer::Close]);

    // Escape in the file name leaves the field; the next one closes.
    bench.click("File name");
    bench.key(Key::Escape);
    assert!(bench.take().is_empty(), "the field is left first");
    bench.key(Key::Escape);
    assert_eq!(bench.take(), [ExportAnswer::Close]);
}

#[test]
fn the_queue_is_one_press_away_once_it_has_jobs() {
    let mut bench = Bench::new(WIDE);
    assert!(bench.find("Render queue (2 jobs)").is_none());
    bench.queue_len = 2;
    bench.frame(Vec::new());
    bench.click("Render queue (2 jobs)");
    assert_eq!(bench.take(), [ExportAnswer::OpenQueue]);

    bench.queue_len = QUEUE_LIMIT;
    bench.frame(Vec::new());
    assert!(bench.get("Export").disabled, "a full queue takes no more");
}

#[test]
fn names_extensions_and_aspects_read_as_written() {
    assert_eq!(stem_of(" Trip.MP4 "), "Trip");
    assert_eq!(stem_of("Trip.final"), "Trip.final");
    assert_eq!(stem_of(".mp4"), ".mp4");
    assert_eq!(
        name_problem("a:b"),
        Some("A file name cannot contain / \\ : * ? \" < > |")
    );
    assert_eq!(name_problem(""), Some("Name the file"));
    assert_eq!(name_problem("Trip"), None);
    assert_eq!(aspect([1920, 1080]), "16:9");
    assert_eq!(aspect([1080, 1350]), "4:5");
    assert_eq!(aspect([1998, 1080]), "1.85:1");
    assert_eq!(
        rate_label(FrameRate {
            numerator: 30000,
            denominator: 1001
        }),
        "29.97"
    );
    assert_eq!(
        rate_label(FrameRate {
            numerator: 48,
            denominator: 1
        }),
        "48"
    );
}

#[test]
fn the_sheet_stays_inside_a_small_window() {
    // The range chips wrap inside the summary's column rather than widen it,
    // so the sheet fits the narrowest window the toolbar is laid out for.
    for size in [vec2(720.0, 520.0), vec2(680.0, 520.0), WIDE] {
        let bench = Bench::new(size);
        for name in [
            "Close export",
            "Save preset…",
            "Cancel",
            "Add to queue",
            "Export",
        ] {
            let rect = bench.get(name).rect;
            assert!(
                rect.left() >= 0.0 && rect.right() <= size.x && rect.bottom() <= size.y,
                "{name} at {rect:?} is inside {size:?}"
            );
        }
        for name in [
            "File name",
            "Folder: /exports",
            "Custom range",
            "Preset",
            "Encoder and speed",
        ] {
            let rect = bench.get(name).rect;
            assert!(
                rect.left() >= 0.0 && rect.right() <= size.x,
                "{name} at {rect:?} is across {size:?}"
            );
        }
        let folder = bench.get("Folder: /exports").rect;
        let custom = bench.get("Custom range").rect;
        assert!(
            custom.right() <= folder.right() + 0.5,
            "the range chips keep to the column: {custom:?}, {folder:?}"
        );
    }
}

// ---- Room, presets remembered, reasons, the poster --------------------------

#[test]
fn the_settings_are_in_sight_without_scrolling() {
    // Spec §5.9.3: the file, the preset and the range come before the
    // picture's cards now, so the sound's are under the fold. At a common
    // desktop size the file, the preset, the range and the picture's
    // settings down to its codec are in sight as the sheet opens.
    let bench = Bench::new(WIDE);
    for name in [
        "File name",
        "Folder: /exports",
        "Preset",
        "YouTube 1080p",
        "Whole timeline",
        "Video",
        "1080p",
        "Frame rate",
        "Codec",
    ] {
        assert!(bench.in_sight(name), "{name} is in sight at {WIDE:?}");
    }
    let (top, bottom) = bench.band();
    let chip = bench.chip("Recommended").rect;
    assert!(
        chip.top() >= top && chip.bottom() <= bottom,
        "the Recommended chip is in sight at {WIDE:?}"
    );
    // At the smaller sizes the file, the preset and the range are, and what
    // the file will be is always said in the footer. The Video switch is at
    // 760 × 560; in the least window the sheet keeps the title bar clear, as
    // Terminal's do (§5.9.1), and the switch is a scroll away.
    for size in [NARROW, vec2(720.0, 520.0)] {
        let bench = Bench::new(size);
        for name in ["File name", "Preset", "YouTube 1080p", "Whole timeline"] {
            assert!(bench.in_sight(name), "{name} is in sight at {size:?}");
        }
        if size == NARROW {
            assert!(bench.in_sight("Video"), "Video is in sight at {size:?}");
        }
        let (top, bottom) = bench.band();
        let chip = bench.chip("Recommended").rect;
        assert!(
            chip.top() >= top && chip.bottom() <= bottom,
            "the Recommended chip is in sight at {size:?}"
        );
        let facts = " · 0:10 · 1920 × 1080";
        assert!(
            bench.says("≈ ") && bench.says(facts),
            "the footer says what will be made at {size:?}"
        );
        // Spec §7.1: and says it to those who do not see it.
        assert!(
            (bench.seen.iter()).any(|seen| seen.role == Role::Label && seen.name.ends_with(facts)),
            "the footer's words are a label at {size:?}"
        );
        // Spec §5.9.3: beside the choices the poster stays 16:9, shorter
        // in a short sheet so that the whole summary under it is in sight.
        let poster = (bench.seen.iter())
            .find(|seen| seen.name.starts_with("Output frame"))
            .expect("the poster is there")
            .rect;
        assert!(
            (poster.width() / poster.height() - 16.0 / 9.0).abs() < 0.02,
            "the poster at {size:?} is 16:9: {poster:?}"
        );
        let last = (bench.seen.iter())
            .find(|seen| seen.name.starts_with("Encoder: "))
            .expect("the summary names the encoder")
            .rect;
        // Narrowed, it is centred over the summary rather than left short
        // of the column's trailing edge.
        let first = (bench.seen.iter())
            .find(|seen| seen.name.starts_with("Size: "))
            .expect("the summary gives the size")
            .rect;
        assert!(
            (poster.center().x - first.center().x).abs() <= 1.0,
            "the poster {poster:?} is centred over the summary {first:?} at {size:?}"
        );
        assert!(
            last.bottom() <= bench.band().1,
            "the summary is in sight at {size:?}"
        );
    }
    // Stacked, the poster sits beside the summary at most 100 tall.
    let bench = Bench::new(vec2(640.0, 560.0));
    let poster = (bench.seen.iter())
        .find(|seen| seen.name.starts_with("Output frame"))
        .expect("the poster is there");
    assert!(
        poster.rect.height() <= 100.0,
        "a narrow sheet's poster is short"
    );
}

#[test]
fn the_painted_words_are_said_as_the_labels_they_were() {
    // Spec §7.1: the title, the cards' labels, their rows' labels and the
    // extension inside the file field are painted, and still said.
    let bench = Bench::new(WIDE);
    for text in ["Export", "Summary", "Range", "Video", "Format", ".mp4"] {
        assert!(
            (bench.seen.iter()).any(|seen| seen.role == Role::Label && seen.name == text),
            "\"{text}\" is a label"
        );
    }
}

#[test]
fn the_sheet_opens_again_at_its_first_card() {
    let mut bench = Bench::new(NARROW);
    bench.reveal("Audio codec");
    assert!(
        !bench.in_sight("File name"),
        "FIXTURE: scrolled past the file"
    );
    // Closed for a while, then shown again.
    for _ in 0..2 {
        bench.time += 0.05;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, bench.size)),
            time: Some(bench.time),
            ..Default::default()
        };
        let _ = bench.ctx.run_ui(raw, |_| {});
    }
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    assert!(bench.in_sight("File name"), "the sheet opens at its top");
}

#[test]
fn presets_with_the_same_settings_keep_the_one_chosen() {
    // YouTube Shorts and TikTok make the same file: the one picked is the
    // one shown, whichever comes first.
    let presets = builtin_presets();
    let tiktok = (presets.iter())
        .find(|preset| preset.id == "tiktok")
        .expect("FIXTURE: a TikTok preset");
    let shorts = (presets.iter())
        .find(|preset| preset.id == "youtube-shorts")
        .expect("FIXTURE: a Shorts preset");
    assert_eq!(
        export::apply_preset(&ExportSettings::default(), tiktok),
        export::apply_preset(&ExportSettings::default(), shorts),
        "FIXTURE: the two presets are the same settings"
    );
    let settings = export::apply_preset(&ExportSettings::default(), tiktok);
    let mut bench = Bench::new(WIDE);
    bench.state = ExportSheetState::new(settings.clone(), "Trip".to_owned(), None)
        .with_preset(Some("tiktok".to_owned()));
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    assert_eq!(bench.get("TikTok").toggled, Some(Toggled::True));
    assert_eq!(bench.state.preset(), Some("tiktok"));
    assert!(
        !bench.says("YouTube Shorts"),
        "the list shows TikTok chosen"
    );
    bench.open_presets();
    let chosen: Vec<&str> = (bench.seen.iter())
        .filter(|seen| seen.name == "TikTok" || seen.name == "YouTube Shorts")
        .filter(|seen| seen.toggled == Some(Toggled::True))
        .map(|seen| seen.name.as_str())
        .collect();
    assert!(
        chosen.iter().all(|name| *name == "TikTok"),
        "only TikTok is chosen: {chosen:?}"
    );
    // Spec §7.1: the list's rows say which is chosen, the chip and the
    // list's TikTok both, and its Shorts not.
    assert_eq!(chosen.len(), 2, "the chip and the list row: {chosen:?}");
    assert!(
        (bench.seen.iter()).any(|seen| seen.name == "YouTube Shorts"
            && seen.role == Role::Button
            && seen.toggled == Some(Toggled::False)),
        "the list's Shorts is a button, not chosen"
    );

    // A preset of one's own the same as a built-in one is shown as one's
    // own when it was the one chosen, and can be deleted.
    let mut bench = Bench::new(WIDE);
    bench.presets.push(ExportPreset {
        id: "mine".to_owned(),
        name: "Vertical".to_owned(),
        destination: Destination::Custom,
        note: String::new(),
        settings: settings.clone(),
    });
    bench.state = ExportSheetState::new(settings, "Trip".to_owned(), None)
        .with_preset(Some("mine".to_owned()));
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    assert!(bench.find("Delete preset “Vertical”").is_some());
}

#[test]
fn disabled_controls_say_why_and_the_folder_is_named_with_its_path() {
    let mut bench = Bench::new(WIDE);
    bench.probe = Probe::Probing;
    bench.frame(Vec::new());
    assert!(bench.get("Save preset…").disabled);
    bench.hover("Save preset…");
    assert!(
        bench.says("Available once the encoders have been checked"),
        "a disabled Save preset… says why"
    );

    let mut bench = Bench::new(WIDE);
    bench.hover("Selected clips");
    assert!(bench.says("Close Export and select clips on the timeline first"));
    assert!(bench.find("Folder: /exports").is_some());
    let bench = Bench::with(WIDE, project(), None);
    assert!(bench.find("Folder: none chosen").is_some());
}

#[test]
fn the_poster_shows_the_output_frame_when_one_is_drawn() {
    let mut bench = Bench::new(WIDE);
    assert!(
        bench
            .find("Output frame, 1920 × 1080 (no preview)")
            .is_some(),
        "without a picture the frame says so"
    );
    let texture = bench.ctx.load_texture(
        "poster",
        egui::ColorImage::filled([32, 18], Color32::RED),
        egui::TextureOptions::LINEAR,
    );
    bench.poster = Some((texture.id(), texture.size_vec2()));
    bench.frame(Vec::new());
    let poster = (bench.seen.iter())
        .find(|seen| seen.name == "Output frame at the playhead, 1920 × 1080")
        .expect("the poster is named for what it shows");
    assert_eq!(poster.role, Role::Image);
    assert!(poster.rect.width() > 200.0);
}

#[test]
fn a_sheet_opened_again_keeps_what_was_set_but_not_a_waiting_export() {
    let owner = Uuid::new_v4();
    let mut bench = Bench::with(WIDE, project(), None);
    bench.state =
        ExportSheetState::new(ExportSettings::default(), "Trip".to_owned(), None).with_owner(owner);
    bench.frame(Vec::new());
    bench.click("720p");
    bench.retype("File name", "Holiday");
    bench.click("Export");
    assert_eq!(bench.take(), [ExportAnswer::ChooseFolder]);
    bench.key(Key::Escape);
    assert!(bench.take().contains(&ExportAnswer::Close));
    // Closed, then shown again for the same project.
    assert_eq!(bench.state.owner(), Some(owner));
    bench.state.reopened();
    bench.state.folder_chosen(Some(PathBuf::from("/films")));
    bench.frame(Vec::new());
    assert!(
        bench.take().is_empty(),
        "an Export that waited for a folder is not queued on reopening"
    );
    assert_eq!(
        bench.state.settings().video.unwrap().resolution,
        Resolution::P720
    );
    assert_eq!(
        bench.state.destination(),
        Some(PathBuf::from("/films/Holiday.mp4"))
    );
}

#[test]
fn a_scroll_bar_widened_under_the_pointer_covers_nothing() {
    // At the small size the options column scrolls. With the pointer over
    // it (spec §5.9.3: the folder is in it now) its bar is drawn at its
    // widest, where the controls end: room is kept for it.
    let mut bench = Bench::new(NARROW);
    let over = bench.get("Folder: /exports").rect.center();
    for _ in 0..12 {
        bench.frame(vec![Event::PointerMoved(over)]);
    }
    assert!(
        bench.bars.iter().any(|bar| bar.left() > over.x),
        "FIXTURE: the options column scrolls at {NARROW:?}: {:?}",
        bench.bars
    );
    for bar in &bench.bars {
        let covered: Vec<&str> = (bench.seen.iter())
            .filter(|seen| seen.role != Role::ScrollBar && seen.rect.width() < 400.0)
            .filter(|seen| {
                seen.rect.right() > bar.left() + 0.5
                    && seen.rect.left() < bar.right()
                    && seen.rect.bottom() > bar.top()
                    && seen.rect.top() < bar.bottom()
            })
            .map(|seen| seen.name.as_str())
            .collect();
        assert!(covered.is_empty(), "the bar at {bar:?} covers {covered:?}");
    }
    assert!(bench.says("1920 × 1080 · 16:9"), "the summary is there");
}

#[test]
fn the_folder_is_named_as_the_render_queue_names_it() {
    let folder = PathBuf::from("/home/someone/Documents/Projects/trip/out");
    let mut bench = Bench::new(WIDE);
    bench.state = ExportSheetState::new(
        ExportSettings::default(),
        "Trip".to_owned(),
        Some(folder.clone()),
    )
    .with_home(Some(PathBuf::from("/home/someone")));
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    let short = super::super::render_queue::short_folder(&folder, Some(Path::new("/home/someone")));
    if std::path::MAIN_SEPARATOR == '/' {
        assert_eq!(short, "~/…/trip/out", "FIXTURE: the queue's short form");
    }
    assert!(
        bench.painted.iter().any(|(text, _)| *text == short),
        "the button shows {short}"
    );
    // The whole path is its accessible name and its tooltip.
    let name = format!("Folder: {}", folder.display());
    assert!(bench.find(&name).is_some());
    bench.hover(&name);
    assert!(bench.says(&format!("Saved in {}", folder.display())));
}

#[test]
fn the_sheet_holds_still_under_a_moving_pointer_at_a_fractional_scale() {
    // 1200 × 750 at 1.5 is a 1800 × 1125 window at 125% with the interface
    // at 120%: the sheet shook there by a pixel at every repaint.
    let windows = [vec2(1200.0, 750.0), WIDE, NARROW, vec2(1013.0, 707.0)];
    for size in windows {
        for scale in [1.25, 1.5, 1.75] {
            let mut bench = Bench::new(size);
            bench.scale = scale;
            bench.settle();
            let mut seen = Vec::new();
            for step in 0..6 {
                bench.frame(vec![Event::PointerMoved(pos2(
                    size.x * 0.3 + step as f32 * 9.0,
                    size.y * 0.5,
                ))]);
                seen.push(bench.ctx.memory(|m| m.area_rect(Id::new("export-sheet"))));
            }
            assert!(
                seen.windows(2).all(|pair| pair[0] == pair[1]),
                "at {size:?} and {scale}× the sheet moved: {seen:?}"
            );
        }
    }
}

#[test]
fn a_fresh_enter_exports_when_nothing_has_the_keyboard() {
    // Spec §5.9.1: Enter gives the sheet's one Primary, Export, but only
    // pressed afresh: not while a field is typed in, and not as the sheet
    // opens.
    let enter = |pressed| Event::Key {
        key: Key::Enter,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    };
    // Pressed and let go, as a person strikes it.
    let strike = |bench: &mut Bench| {
        bench.frame(vec![enter(true)]);
        bench.frame(vec![enter(false)]);
    };
    let mut bench = Bench::new(WIDE);
    bench.retype("File name", "Holiday");
    strike(&mut bench);
    assert!(bench.take().is_empty(), "the field's Enter is the field's");

    strike(&mut bench);
    let answers = bench.take();
    assert!(
        matches!(
            answers.as_slice(),
            [ExportAnswer::Queue {
                start_now: true,
                ..
            }]
        ),
        "{answers:?}"
    );

    // Away for a while, then shown again with an Enter in its first frame:
    // that one was meant for what was there before.
    let ctx = bench.ctx.clone();
    for _ in 0..2 {
        bench.time += 0.05;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, bench.size)),
            time: Some(bench.time),
            ..Default::default()
        };
        let _ = ctx.run_ui(raw, |_| {});
    }
    bench.frame(vec![enter(true)]);
    assert!(bench.take().is_empty(), "an Enter carried into the sheet");
}

#[test]
fn a_refusal_is_said_in_the_footer_of_a_short_window_until_something_changes() {
    // Spec §5.8: a toast waits under a sheet that leaves it no room, so the
    // queue's refusal of an Export is said inside the sheet.
    let why = "The render queue holds 64 jobs; remove finished jobs first";
    let mut bench = Bench::new(NARROW);
    bench.click("Export");
    assert!(matches!(
        bench.take().as_slice(),
        [ExportAnswer::Queue { .. }]
    ));
    bench.state.refuse(why.to_owned());
    bench.frame(Vec::new());
    let said = (bench.seen.iter())
        .find(|seen| seen.name == why && seen.role == Role::Label)
        .unwrap_or_else(|| panic!("the refusal is read out"))
        .rect;
    let export = bench.get("Export").rect;
    let cancel = bench.get("Cancel").rect;
    assert!(
        said.bottom() <= NARROW.y && said.top() >= cancel.top() - 12.0,
        "in the footer, in sight: {said:?}, {cancel:?}"
    );
    assert!(
        said.right() <= cancel.left(),
        "beside the buttons: {said:?}"
    );
    assert!(!said.intersects(export));
    assert_eq!(
        bench.painted_in("render queue holds"),
        Some(bench.palette.red)
    );

    // Asking again answers it afresh.
    bench.click("Export");
    assert!(matches!(
        bench.take().as_slice(),
        [ExportAnswer::Queue { .. }]
    ));
    assert_eq!(bench.state.refusal(), None);

    // So does changing what would be made.
    bench.state.refuse(why.to_owned());
    bench.click("MOV");
    assert_eq!(bench.state.refusal(), None);
    assert!(!bench.says("render queue holds"));
}

#[test]
fn a_short_window_keeps_the_sheet_clear_of_the_title_bar() {
    // §5.9.1: as tall as Preferences in a window under about 600 tall, so
    // that the traffic lights, the title and the toolbar stay in sight.
    for size in [NARROW, vec2(720.0, 520.0)] {
        let bench = Bench::new(size);
        let sheet = (bench.seen.iter())
            .find(|seen| seen.name == "Export" && seen.role == Role::Window)
            .expect("the sheet is a window")
            .rect;
        assert!(
            sheet.top() >= crate::theme::metrics::TOOLBAR_HEIGHT + 3.0,
            "{size:?}: {sheet:?}"
        );
        assert!(
            (sheet.center().y - size.y * 0.5).abs() <= 1.0,
            "{size:?}: {sheet:?}"
        );
    }
}

#[test]
fn a_card_switch_shares_a_line_with_its_name() {
    // §4.5: the switch that turns the picture or the sound on is the first
    // row of its card, its name at the leading edge on one line with it.
    for size in [WIDE, NARROW] {
        let mut bench = Bench::new(size);
        for name in ["Video", "Audio"] {
            bench.reveal(name);
            let switch = bench.get(name).rect;
            let words = (bench.seen.iter())
                .find(|seen| seen.name == name && seen.role == Role::Label)
                .expect("the row's name is a label")
                .rect;
            assert!(
                (words.center().y - switch.center().y).abs() < 0.5,
                "{name} at {size:?}: {words:?} and {switch:?}"
            );
        }
    }
}

#[test]
fn the_trailing_controls_start_together() {
    // Spec §5.9.3: one trailing width, so that the Preset select over the
    // first card and the Speed control under the encoder start where the
    // other controls do.
    for size in [WIDE, NARROW] {
        let mut bench = Bench::new(size);
        bench.click("Encoder and speed");
        let rect = |bench: &Bench, name: &str| bench.get(name).rect;
        let select = rect(&bench, "Frame rate");
        for name in ["Preset", "Codec", "Encoder"] {
            let at = rect(&bench, name);
            assert!(
                (at.left() - select.left()).abs() < 0.5
                    && (at.width() - select.width()).abs() < 0.5,
                "{name} at {size:?}: {at:?}, against Frame rate at {select:?}"
            );
        }
        let (speed, format) = (rect(&bench, "Faster"), rect(&bench, "MP4"));
        assert!(
            (speed.left() - format.left()).abs() < 0.5,
            "Speed at {size:?} starts at {speed:?}, against the format at {format:?}"
        );
    }
}

#[test]
fn a_summary_value_cut_short_is_shown_whole_on_hover() {
    // FIXTURE: a Mac's H.264, whose family and FFmpeg name together are
    // longer than the summary's column.
    let mut caps = host();
    caps.video
        .retain(|encoder| encoder.hardware != Some(HardwareKind::Nvenc));
    caps.video.push(video(
        "h264_videotoolbox",
        VideoCodec::H264,
        Some(HardwareKind::VideoToolbox),
        4096,
    ));
    let mut bench = Bench::new(WIDE);
    bench.probe = Probe::Ready(caps);
    bench.settle();
    // A text shape keeps its whole text, cut or not, so a value is counted
    // where it is painted: once in the summary, and again in a tooltip.
    let rest_on = |bench: &mut Bench, label: &str, value: &str| {
        let fact = (bench.seen.iter())
            .find(|seen| seen.name == format!("{label}: {value}"))
            .unwrap_or_else(|| panic!("FIXTURE: the summary says {label}: {value}"))
            .rect;
        bench.frame(vec![Event::PointerMoved(fact.center())]);
        for _ in 0..30 {
            bench.frame(Vec::new());
        }
        (bench.painted.iter())
            .filter(|(text, _)| text == value)
            .count()
    };
    assert_eq!(
        rest_on(
            &mut bench,
            "Encoder",
            "Apple VideoToolbox · h264_videotoolbox"
        ),
        2,
        "the encoder, cut short, is shown whole on hover"
    );
    assert_eq!(
        rest_on(&mut bench, "Size", "1920 × 1080 · 16:9"),
        1,
        "a size that fits has no tooltip"
    );
}
