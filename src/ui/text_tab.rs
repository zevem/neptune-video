//! The library's Text tab: titles, caption styles, graphic templates,
//! stickers and emoji to add, and the person's own presets and brand
//! colours.
//!
//! A tile shows its look as the title lane draws it, at the tile's size, so
//! what is seen is what is added; while the pointer rests on one it plays
//! its entrance. A tile is added at the playhead with a double click or
//! Enter, or where it is let go on the timeline. A title's look let go on a
//! title gives it that look and keeps its words and their times; on a
//! caption, every caption of its track. Either way the clips are captured
//! when the tile is let go of, and what is made is one command.
mod catalog;

use super::{
    Action, PresetChange, Selection, controls, kit, library,
    widgets::{self, Kind},
};
use crate::{
    presets::Presets,
    rules,
    theme::{self, Palette, metrics},
    titles::Titles,
};
use editor_core::{
    Clip, ClipContent, ClipId, Command, ElementKind, LayerSource, Project, Time, Track, TrackId,
    TrackKind, TransitionEdge, title,
};
use eframe::egui::{
    self, Color32, CornerRadius, Id, Key, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, WidgetInfo,
    WidgetType, ecolor::Hsva, pos2, vec2,
};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
};
use uuid::Uuid;

/// An emoji's least width; every other look is laid out as the library's
/// other cards are, by [`library::card_columns`].
const EMOJI_MIN: f32 = 40.0;
/// What a card has under its well: its name, 6 below it.
const NAME_HEIGHT: f32 = 24.0;
const SWATCH: f32 = 22.0;
/// Round the cards: room for their rings, which stand 6 out from them,
/// and the same on either side (see [`library::cards_width`]).
const MARGIN: egui::Margin = egui::Margin {
    left: library::LEADING as i8,
    right: library::LEADING as i8,
    top: 6,
    bottom: 8,
};
/// How soon after a double click gave a tile another click is part of it.
const AGAIN: f64 = 0.35;
/// How long a tile rests at its full look before it plays again.
const REST: f64 = 1.0;

/// The headings the tab lists looks under, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    /// The person's own looks.
    Yours,
    Titles,
    Captions,
    LowerThirds,
    TitleCards,
    Callouts,
    Stickers,
    Emoji,
}

impl Group {
    pub fn heading(self) -> &'static str {
        match self {
            Self::Yours => "Yours",
            Self::Titles => "Titles",
            Self::Captions => "Captions",
            Self::LowerThirds => "Lower thirds",
            Self::TitleCards => "Title cards",
            Self::Callouts => "Callouts",
            Self::Stickers => "Stickers",
            Self::Emoji => "Emoji",
        }
    }

    /// What adding one is called in the history.
    fn adding(self, title: bool) -> &'static str {
        match self {
            Self::Emoji => "Add emoji",
            Self::Stickers => "Add sticker",
            _ if title => "Add title",
            _ => "Add graphic",
        }
    }
}

/// Where on the canvas a new clip of a look goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anchor {
    Centre,
    /// Low and centred, where captions are read.
    Lower,
    /// In the lower left corner, inside a margin.
    LowerLeft,
    /// Where it was when it was kept, from the canvas's centre.
    At([f32; 2]),
}

impl Anchor {
    /// The position, from the canvas's centre, of a clip whose content is
    /// `size` canvas pixels on a canvas of `canvas`.
    fn position(self, canvas: [f32; 2], size: [f32; 2]) -> [f32; 2] {
        let margin = canvas[0].min(canvas[1]) * 0.07;
        match self {
            Self::Centre => [0.0, 0.0],
            Self::Lower => [0.0, canvas[1] * 0.3],
            Self::LowerLeft => [
                -canvas[0] * 0.5 + margin + size[0] * 0.5,
                canvas[1] * 0.5 - margin * 1.6 - size[1] * 0.5,
            ],
            Self::At(at) => at,
        }
    }
}

/// Something the Text tab adds: a title with its look, or a graphic.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    /// The same in every run: what its tile and its picture are known by.
    pub id: Uuid,
    pub name: String,
    pub group: Group,
    pub anchor: Anchor,
    /// Other words it is found by.
    tags: String,
    pub content: ClipContent,
}

/// An ID from a name: FNV-1a over its bytes, so that a built-in look is
/// known by the same ID in every run.
fn stable_id(group: &str, name: &str) -> Uuid {
    let mut hash: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    for byte in group.bytes().chain([0]).chain(name.bytes()) {
        hash ^= u128::from(byte);
        hash = hash.wrapping_mul(0x0000_0000_0100_0000_0000_0000_0000_013b);
    }
    Uuid::from_u128(hash)
}

impl Look {
    fn builtin(group: Group, anchor: Anchor, name: &str, tags: &str, content: ClipContent) -> Self {
        Self {
            id: stable_id(group.heading(), name),
            name: name.to_owned(),
            group,
            anchor,
            tags: tags.to_owned(),
            content,
        }
    }

    pub fn saved(saved: &crate::presets::SavedLook) -> Self {
        Self {
            id: saved.id,
            name: saved.name.clone(),
            group: Group::Yours,
            anchor: saved.position.map_or(Anchor::Centre, Anchor::At),
            tags: String::new(),
            content: saved.content.clone(),
        }
    }

    /// What the T key adds: the plain title a new one always was.
    pub fn typed() -> &'static Self {
        static TYPED: LazyLock<Look> = LazyLock::new(|| {
            Look::builtin(
                Group::Titles,
                Anchor::Centre,
                "Text",
                "",
                Clip::text("Text", Time::ZERO, Time(1)).content,
            )
        });
        &TYPED
    }

    /// The look of a title already on the timeline, by its name: what it
    /// gives is its size, colour, style and animations.
    pub fn of_title(name: &str, content: ClipContent) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.to_owned(),
            group: Group::Captions,
            anchor: Anchor::Lower,
            tags: String::new(),
            content,
        }
    }

    pub fn is_title(&self) -> bool {
        matches!(self.content, ClipContent::Text { .. })
    }

    fn matches(&self, query: &str) -> bool {
        query.is_empty()
            || [self.name.as_str(), self.group.heading(), self.tags.as_str()]
                .iter()
                .any(|text| text.to_lowercase().contains(query))
    }

    /// How long a clip of it is made: three seconds, or what its entrances
    /// and exits need when that is more.
    pub fn duration(&self, rate: editor_core::FrameRate) -> Time {
        let span = |animation: &Option<editor_core::TextAnimation>| {
            animation
                .as_ref()
                .map_or(Time::ZERO, |animation| animation.span())
        };
        let needed = match &self.content {
            ClipContent::Text { enter, exit, .. } => span(enter) + span(exit),
            ClipContent::Graphic { elements, .. } => (elements.iter())
                .map(|element| span(&element.enter) + span(&element.exit))
                .max()
                .unwrap_or(Time::ZERO),
            ClipContent::Media { .. } => Time::ZERO,
        };
        let frames = rate.frame_at(needed) + 1;
        Time::from_seconds(3.0).max(rate.frame_time(frames))
    }

    /// A new clip of this look at `start`, for `project`'s canvas: its sizes
    /// scaled from a canvas 1080 pixels on its short side, placed by its
    /// anchor, and its elements given IDs of their own.
    pub fn clip(&self, start: Time, project: &Project) -> Clip {
        let settings = &project.settings;
        let canvas = [settings.width as f32, settings.height as f32];
        let factor = match self.anchor {
            Anchor::At(_) => 1.0,
            _ => canvas[0].min(canvas[1]) / 1080.0,
        };
        let content = scaled(&self.content, factor);
        let duration = self.duration(settings.frame_rate);
        let mut clip = match content {
            ClipContent::Graphic { size, elements } => {
                let elements = (elements.into_iter())
                    .map(|mut element| {
                        element.id = Uuid::new_v4();
                        element
                    })
                    .collect();
                Clip::graphic(self.name.clone(), size, elements, start, duration)
            }
            content => {
                let text = match &content {
                    ClipContent::Text { text, .. } => text.clone(),
                    _ => self.name.clone(),
                };
                let mut clip = Clip::text(text, start, duration);
                clip.content = content;
                clip
            }
        };
        let size = match &clip.content {
            ClipContent::Graphic { size, .. } => *size,
            _ => [0.0, 0.0],
        };
        clip.transform.position = self.anchor.position(canvas, size);
        clip
    }
}

/// `content` with its sizes in canvas pixels made `factor` times as large.
/// A style's lengths are in ems and grow with its size.
fn scaled(content: &ClipContent, factor: f32) -> ClipContent {
    let mut content = content.clone();
    if !factor.is_finite() || (factor - 1.0).abs() < 1.0e-3 {
        return content;
    }
    let size_of = |size: f32| (size * factor).clamp(1.0, 4096.0);
    match &mut content {
        ClipContent::Text {
            font_size, style, ..
        } => {
            *font_size = size_of(*font_size);
            style.wrap = (style.wrap * factor).min(16_384.0);
        }
        ClipContent::Graphic { size, elements } => {
            *size = size.map(|side| (side * factor).clamp(1.0, 16_384.0));
            for element in elements {
                element.frame = element.frame.map(|value| value * factor);
                match &mut element.kind {
                    ElementKind::Text {
                        font_size, style, ..
                    } => {
                        *font_size = size_of(*font_size);
                        style.wrap = (style.wrap * factor).min(16_384.0);
                    }
                    ElementKind::Rect { radius, stroke, .. } => {
                        *radius *= factor;
                        if let Some(stroke) = stroke {
                            stroke.width = (stroke.width * factor).clamp(0.5, 1000.0);
                        }
                    }
                    ElementKind::Ellipse { stroke, .. } => {
                        if let Some(stroke) = stroke {
                            stroke.width = (stroke.width * factor).clamp(0.5, 1000.0);
                        }
                    }
                    ElementKind::Line { width, .. } => {
                        *width = (*width * factor).clamp(0.5, 1000.0)
                    }
                }
            }
        }
        ClipContent::Media { .. } => {}
    }
    content
}

/// Every look Neptune Video comes with, in the order the tab lists them.
pub fn builtin() -> &'static [Arc<Look>] {
    catalog::builtin()
}

/// The look a caption is first given when its track has none to copy.
pub fn caption_look() -> &'static Arc<Look> {
    (builtin().iter())
        .find(|look| look.group == Group::Captions)
        .expect("the catalog has caption looks")
}

/// The payload of a tile carried toward the timeline.
#[derive(Clone, Debug)]
pub struct LookDrag(pub Arc<Look>);

/// Where a new clip of `look` goes, and the one edit that puts it there:
/// on `track` at the first room from `at` when that is an unlocked text
/// track; otherwise at `at` on the first plain text track with room there,
/// or on a new text track. Hands back the new clip's ID.
pub fn adding(
    project: &Project,
    look: &Look,
    track: Option<TrackId>,
    at: Time,
) -> (Command, ClipId) {
    adding_for(project, look, track, at, None)
}

/// `adding`, with the clip `duration` long rather than its look's own
/// length: its animations are fitted to it and its place is found for it.
pub fn adding_for(
    project: &Project,
    look: &Look,
    track: Option<TrackId>,
    at: Time,
    duration: Option<Time>,
) -> (Command, ClipId) {
    let at = rules::quantize(at, project.settings.frame_rate);
    let mut clip = look.clip(at, project);
    if let Some(duration) = duration {
        let frame = project.settings.frame_rate.frame_time(1);
        clip.duration = duration;
        match &mut clip.content {
            ClipContent::Text { enter, exit, .. } => {
                (*enter, *exit) =
                    editor_core::title::fit_animations(enter.take(), exit.take(), frame, duration);
            }
            ClipContent::Graphic { elements, .. } => {
                for element in elements {
                    (element.enter, element.exit) = editor_core::title::fit_animations(
                        element.enter.take(),
                        element.exit.take(),
                        frame,
                        duration,
                    );
                }
            }
            ClipContent::Media { .. } => {}
        }
    }
    let id = clip.id;
    let label = look.group.adding(look.is_title()).to_owned();
    let usable = |track: &Track| track.kind == TrackKind::Text && !track.locked;
    let chosen = (track.and_then(|id| project.tracks.iter().find(|track| track.id == id)))
        .filter(|track| usable(track));
    let commands = if let Some(track) = chosen {
        clip.start = rules::free_start(track, at, clip.duration, &[]);
        vec![Command::AddClip {
            track_id: track.id,
            clip,
        }]
    } else if let Some(track) = project.tracks.iter().find(|track| {
        usable(track)
            && track.language.is_none()
            && rules::free_start(track, at, clip.duration, &[]) == at
    }) {
        vec![Command::AddClip {
            track_id: track.id,
            clip,
        }]
    } else {
        let count = (project.tracks.iter())
            .filter(|track| track.kind == TrackKind::Text)
            .count();
        let track = Track::new(format!("Text {}", count + 1), TrackKind::Text);
        let track_id = track.id;
        vec![
            Command::AddTrack { track },
            Command::AddClip { track_id, clip },
        ]
    };
    (Command::Batch { label, commands }, id)
}

/// The titles a title's look let go on `clip` is given: every title of its
/// track when that is a caption track, as captions share a look; else the
/// clip alone.
pub fn restyled_with(project: &Project, clip: ClipId) -> Vec<ClipId> {
    match project.clip(clip) {
        Some((track, _)) if track.language.is_some() => (track.clips.iter())
            .filter(|clip| matches!(clip.content, ClipContent::Text { .. }))
            .map(|clip| clip.id)
            .collect(),
        _ => vec![clip],
    }
}

/// What giving the look of `look`, a title, to the titles among `clips`
/// asks: one edit that keeps each title's words and their times and holds
/// the look's entrance and exit inside it; or a line that says why there is
/// nothing to give it to.
pub fn restyle(project: &Project, look: &Look, clips: &[ClipId]) -> Action {
    let ClipContent::Text {
        font_size,
        color,
        style,
        enter,
        exit,
        ..
    } = &look.content
    else {
        return Action::Notify("Only a title's look can be given to a title.".into());
    };
    let frame = rules::frame(project.settings.frame_rate);
    let mut commands = Vec::new();
    let mut titles = 0;
    for (track, clip) in clips.iter().filter_map(|id| project.clip(*id)) {
        let ClipContent::Text {
            text,
            font_size: had_size,
            color: had_color,
            style: had_style,
            enter: had_enter,
            exit: had_exit,
            ..
        } = &clip.content
        else {
            continue;
        };
        if track.locked {
            continue;
        }
        titles += 1;
        if (had_size, had_color) != (font_size, color) {
            commands.push(Command::SetText {
                clip_id: clip.id,
                text: text.clone(),
                font_size: *font_size,
                color: *color,
            });
        }
        if had_style != style {
            commands.push(Command::SetTextStyle {
                clip_id: clip.id,
                style: style.clone(),
            });
        }
        let (enter, exit) = title::fit_animations(*enter, *exit, frame, clip.duration);
        for (edge, had, animation) in [
            (TransitionEdge::In, had_enter, enter),
            (TransitionEdge::Out, had_exit, exit),
        ] {
            if *had != animation {
                commands.push(Command::SetTextAnimation {
                    clip_id: clip.id,
                    edge,
                    animation,
                });
            }
        }
    }
    match titles {
        0 => Action::Notify("Select a title to give this look to, or drag it onto one.".to_owned()),
        _ if commands.is_empty() => {
            Action::Notify(format!("They already look like “{}”.", look.name))
        }
        1 => Action::Edit(Command::Batch {
            label: "Style title".into(),
            commands,
        }),
        _ => Action::Edit(Command::Batch {
            label: "Style titles".into(),
            commands,
        }),
    }
}

/// A look as its tile shows it: at rest, or `local` into its entrance; a
/// caption's highlight on a word in the middle, or moving through them.
fn preview(look: &Look, local: Option<Time>) -> LayerSource {
    match &look.content {
        ClipContent::Text {
            text,
            font_size,
            color,
            style,
            enter,
            ..
        } => {
            let words = style.highlight.as_ref().map(|highlight| {
                let span = Time::from_seconds(2.4);
                let times = title::spread(text, Time::ZERO, span);
                let at = local.map_or(Time::from_seconds(0.9), |local| Time(local.0 % span.0));
                title::word_state(&times, highlight, at)
            });
            LayerSource::Text {
                text: text.clone(),
                font_size: *font_size,
                color: *color,
                style: style.clone(),
                enter: local.and_then(|local| title::entering(enter.as_ref(), local)),
                exit: None,
                words,
            }
        }
        ClipContent::Graphic { size, elements } => LayerSource::Graphic {
            size: *size,
            elements: (elements.iter())
                .map(|element| editor_core::title::ElementLayer {
                    element: element.clone(),
                    enter: local.and_then(|local| title::entering(element.enter.as_ref(), local)),
                    exit: None,
                })
                .collect(),
        },
        ClipContent::Media { .. } => LayerSource::Text {
            text: look.name.clone(),
            font_size: 64.0,
            color: [255; 4],
            style: Default::default(),
            enter: None,
            exit: None,
            words: None,
        },
    }
}

/// How long a tile plays before it rests: its entrance, or a caption's
/// words, and a moment at its full look.
fn cycle(look: &Look) -> f64 {
    let span = |animation: &Option<editor_core::TextAnimation>| {
        animation
            .as_ref()
            .map_or(0.0, |animation| animation.span().as_seconds())
    };
    let playing = match &look.content {
        ClipContent::Text { enter, style, .. } => {
            let words = if style.highlight.is_some() { 2.4 } else { 0.0 };
            span(enter).max(words)
        }
        ClipContent::Graphic { elements, .. } => (elements.iter())
            .map(|element| span(&element.enter))
            .fold(0.0, f64::max),
        ClipContent::Media { .. } => 0.0,
    };
    playing + REST
}

/// The canvas pixels a look is guessed to cover before its first picture
/// says, around its anchor.
fn guessed(look: &Look) -> Rect {
    let size = match &look.content {
        ClipContent::Text {
            text,
            font_size,
            style,
            ..
        } => {
            let longest = text
                .lines()
                .map(|line| line.chars().count())
                .max()
                .unwrap_or(1);
            let mut width = longest.max(1) as f32 * font_size * 0.56;
            let mut lines = text.lines().count().max(1) as f32;
            if style.wrap > 0.0 && width > style.wrap {
                lines *= (width / style.wrap).ceil();
                width = style.wrap;
            }
            vec2(width, lines * font_size * style.line_height)
        }
        ClipContent::Graphic { size, .. } => vec2(size[0], size[1]),
        ClipContent::Media { .. } => vec2(100.0, 100.0),
    };
    Rect::from_center_size(egui::Pos2::ZERO, size)
}

/// The canvas pixels a picture covers, around its anchor.
fn covered(picture: &editor_media::text::TitlePicture) -> Rect {
    let [width, height] = picture.size();
    Rect::from_min_size(
        pos2(picture.origin[0], picture.origin[1]),
        vec2(width, height),
    )
}

#[derive(Default)]
pub struct TextTabState {
    /// The tile last clicked: the one Enter adds.
    pub picked: Option<Uuid>,
    /// The tile playing its entrance, and since when.
    playing: Option<(Uuid, f64)>,
    /// The scale each tile's picture was last asked at: a picture answering
    /// it does not ask again for a scale only a little different.
    scales: HashMap<Uuid, f32>,
    /// The preset being renamed and the name being typed, and whether its
    /// field is still to take the keyboard.
    renaming: Option<(Uuid, String, bool)>,
    /// The colour about to be added to the brand colours.
    new_color: Option<[u8; 4]>,
    /// The person's looks, made once for each list of presets.
    saved: Option<(Arc<Presets>, Vec<Arc<Look>>)>,
    /// When a double click last gave a tile, in the frame's time: the
    /// triple click that follows it gives none.
    given_at: Option<f64>,
}

impl TextTabState {
    fn saved_looks(&mut self, presets: &Arc<Presets>) -> Vec<Arc<Look>> {
        match &self.saved {
            Some((held, looks)) if Arc::ptr_eq(held, presets) => looks.clone(),
            _ => {
                let looks: Vec<Arc<Look>> = (presets.looks.iter())
                    .map(|saved| Arc::new(Look::saved(saved)))
                    .collect();
                self.saved = Some((Arc::clone(presets), looks.clone()));
                looks
            }
        }
    }
}

pub struct TextTabInput<'a> {
    pub p: &'a Palette,
    pub project: &'a Project,
    pub presets: &'a Arc<Presets>,
}

/// The tiles of the tab in `rect`, filtered by `search`.
#[expect(clippy::too_many_arguments, reason = "the library's one call")]
pub fn show(
    ui: &mut Ui,
    rect: Rect,
    search: &str,
    state: &mut TextTabState,
    selection: &mut Selection,
    titles: &mut Titles,
    input: &TextTabInput,
    actions: &mut Vec<Action>,
) {
    let p = input.p;
    let query = search.trim().to_lowercase();
    let saved = state.saved_looks(input.presets);
    let mut groups: Vec<(Group, Vec<Arc<Look>>)> = Vec::new();
    let yours: Vec<Arc<Look>> = saved
        .into_iter()
        .filter(|look| look.matches(&query))
        .collect();
    // The person's own come first once there are any.
    let yours_first = !input.presets.looks.is_empty() || !input.presets.colors.is_empty();
    if yours_first {
        groups.push((Group::Yours, yours.clone()));
    }
    for look in builtin().iter().filter(|look| look.matches(&query)) {
        match groups.last_mut() {
            Some((group, looks)) if *group == look.group => looks.push(Arc::clone(look)),
            _ => groups.push((look.group, vec![Arc::clone(look)])),
        }
    }
    if !yours_first {
        groups.push((Group::Yours, yours));
    }
    let any = groups.iter().any(|(_, looks)| !looks.is_empty());
    if !any && !query.is_empty() {
        kit::empty_state(
            ui,
            p,
            library::inset(rect),
            crate::icons::Icon::Search,
            "No matches",
            "Nothing here has that name.",
            None,
        );
        return;
    }

    let mut body = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    body.set_clip_rect(rect.intersect(ui.clip_rect()));
    body.spacing_mut().item_spacing = Vec2::ZERO;
    let width = library::cards_width(rect.width());
    let enter = ui.input(|i| i.key_pressed(Key::Enter));
    let mut hovered = None;
    let output = egui::ScrollArea::vertical()
        .id_salt("text-tab")
        .auto_shrink([false, false])
        .show(&mut body, |ui| {
            egui::Frame::NONE.inner_margin(MARGIN).show(ui, |ui| {
                ui.set_width(width);
                for (group, looks) in &groups {
                    let mine = *group == Group::Yours;
                    if looks.is_empty() && !(mine && query.is_empty()) {
                        continue;
                    }
                    library::heading(ui, p, group.heading());
                    ui.add_space(4.0);
                    if mine && query.is_empty() {
                        brand_colors(ui, p, state, input.presets, width, actions);
                        if looks.is_empty() {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(
                                        "Keep a title's look or a graphic from the inspector to find it here.",
                                    )
                                    .font(theme::regular(11.5))
                                    .color(p.secondary),
                                )
                                .wrap(),
                            );
                            ui.add_space(12.0);
                            continue;
                        }
                    }
                    // Emoji are small squares; every other look is a card,
                    // as many across as on the library's other tabs.
                    let compact = *group == Group::Emoji;
                    let (columns, tile_width) = if compact {
                        let gap = library::CARD_GAP;
                        let columns = ((width + gap) / (EMOJI_MIN + gap)).floor().max(1.0) as usize;
                        (columns, (width - gap * (columns as f32 - 1.0)) / columns as f32)
                    } else {
                        library::card_columns(width)
                    };
                    let tile_height = if compact {
                        tile_width
                    } else {
                        tile_width * 9.0 / 16.0 + NAME_HEIGHT
                    };
                    let mut row = Rect::NOTHING;
                    for (index, look) in looks.iter().enumerate() {
                        let column = index % columns;
                        if column == 0 {
                            (row, _) = ui.allocate_exact_size(
                                vec2(width, tile_height + library::CARD_ROW_GAP),
                                Sense::hover(),
                            );
                        }
                        let cell = Rect::from_min_size(
                            pos2(
                                row.left() + column as f32 * (tile_width + library::CARD_GAP),
                                row.top(),
                            ),
                            vec2(tile_width, tile_height),
                        );
                        if !ui.is_rect_visible(cell.expand(6.0)) {
                            continue;
                        }
                        let shown = Tile {
                            look,
                            compact,
                            rect: cell,
                        };
                        if shown.show(ui, state, selection, titles, input, enter, actions) {
                            hovered = Some(look.id);
                        }
                    }
                    ui.add_space(6.0);
                }
            });
        });
    kit::scroll_fades(ui, &output, p.chrome);
    // The tile under the pointer plays; the others rest.
    let now = ui.input(|i| i.time);
    state.playing = match (hovered, state.playing) {
        (Some(id), Some((playing, since))) if playing == id => Some((id, since)),
        (Some(id), _) => Some((id, now)),
        (None, _) => None,
    };
}

/// The brand colours, each a swatch, and the button that adds one.
fn brand_colors(
    ui: &mut Ui,
    p: &Palette,
    state: &mut TextTabState,
    presets: &Presets,
    width: f32,
    actions: &mut Vec<Action>,
) {
    let per_row = ((width + 6.0) / (SWATCH + 6.0)).floor().max(1.0) as usize;
    let count = presets.colors.len() + 1;
    let rows = count.div_ceil(per_row);
    let (strip, _) =
        ui.allocate_exact_size(vec2(width, rows as f32 * (SWATCH + 6.0)), Sense::hover());
    let place = |index: usize| {
        Rect::from_min_size(
            pos2(
                strip.left() + (index % per_row) as f32 * (SWATCH + 6.0),
                strip.top() + (index / per_row) as f32 * (SWATCH + 6.0),
            ),
            Vec2::splat(SWATCH),
        )
    };
    for (index, color) in presets.colors.iter().enumerate() {
        let rect = place(index);
        let name = format!("Brand colour {}", hex(*color));
        let response = ui.interact(rect, Id::new(("brand-color", index)), Sense::click());
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &name));
        let [r, g, b, a] = *color;
        egui::widgets::color_picker::show_color_at(
            ui.painter(),
            Color32::from_rgba_unmultiplied(r, g, b, a),
            rect.shrink(1.0),
        );
        let ring = if response.hovered() {
            Stroke::new(1.5, p.secondary)
        } else {
            Stroke::new(1.0, p.border)
        };
        ui.painter()
            .rect_stroke(rect, CornerRadius::same(4), ring, StrokeKind::Inside);
        if response.has_focus() {
            controls::focus_ring(ui.painter(), rect, 4, *p);
        }
        let response = response.on_hover_text(format!(
            "{} is offered in every colour of the inspector. Right-click to remove it.",
            hex(*color)
        ));
        response.context_menu(|ui| {
            widgets::menu_style(ui, 180.0);
            if widgets::menu_item_destructive(
                ui,
                p,
                Some(crate::icons::Icon::Trash),
                "Remove colour",
                "",
            )
            .clicked()
            {
                actions.push(Action::Presets(PresetChange::RemoveColor(*color)));
                ui.close();
            }
        });
    }
    let rect = place(presets.colors.len());
    let add = ui.interact(rect, Id::new("brand-color-add"), Sense::click());
    add.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Add brand colour"));
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(4),
        Stroke::new(1.0, if add.hovered() { p.secondary } else { p.border }),
        StrokeKind::Inside,
    );
    crate::icons::paint(
        ui.painter(),
        rect.shrink(5.0),
        crate::icons::Icon::Plus,
        if add.hovered() { p.fg } else { p.secondary },
    );
    if add.has_focus() {
        controls::focus_ring(ui.painter(), rect, 4, *p);
    }
    let add = add.on_hover_text("Add a brand colour");
    egui::Popup::menu(&add)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(240.0);
            ui.spacing_mut().slider_width = 220.0;
            let color = state.new_color.get_or_insert([255, 214, 0, 255]);
            let mut hsva = Hsva::from_srgba_unmultiplied(*color);
            if egui::widgets::color_picker::color_picker_hsva_2d(
                ui,
                &mut hsva,
                egui::widgets::color_picker::Alpha::OnlyBlend,
            ) {
                *color = hsva.to_srgba_unmultiplied();
            }
            ui.add_space(6.0);
            if widgets::button(ui, p, None, "Add colour", Kind::Primary).clicked() {
                actions.push(Action::Presets(PresetChange::AddColor(*color)));
                ui.close();
            }
        });
    ui.add_space(8.0);
}

pub fn hex(color: [u8; 4]) -> String {
    let [r, g, b, a] = color;
    if a == 255 {
        format!("#{r:02X}{g:02X}{b:02X}")
    } else {
        format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
    }
}

/// One tile, where it is drawn.
struct Tile<'a> {
    look: &'a Arc<Look>,
    /// An emoji's: a square with no name under it.
    compact: bool,
    rect: Rect,
}

impl Tile<'_> {
    /// Draws the tile and does what was done to it. Says whether the
    /// pointer is on it.
    #[expect(clippy::too_many_arguments, reason = "one call site per tile")]
    fn show(
        &self,
        ui: &mut Ui,
        state: &mut TextTabState,
        selection: &mut Selection,
        titles: &mut Titles,
        input: &TextTabInput,
        enter: bool,
        actions: &mut Vec<Action>,
    ) -> bool {
        let p = input.p;
        let look = self.look;
        let rect = self.rect;
        let response = ui.interact(
            rect,
            Id::new(("text-tile", look.id)),
            Sense::click_and_drag(),
        );
        let enabled = ui.is_enabled();
        let picked = state.picked == Some(look.id);
        response
            .widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, picked, &look.name));

        let picture = if self.compact {
            rect
        } else {
            Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - NAME_HEIGHT))
        };
        let radius = CornerRadius::same(metrics::CONTROL_RADIUS);
        // Titles are mostly light and are made to be seen on pictures: they
        // sit on the stage's dark in both appearances.
        library::card_states(ui, p, rect, picture, &response, picked);
        ui.painter().rect_filled(picture, radius, p.stage());
        let playing = (state.playing)
            .filter(|(id, _)| *id == look.id && response.hovered())
            .map(|(_, since)| {
                let elapsed = (ui.input(|i| i.time) - since).max(0.0);
                Time::from_seconds(elapsed % cycle(look))
            });
        if playing.is_some() {
            ui.ctx().request_repaint();
        }
        self.picture(
            ui,
            picture.shrink(if self.compact { 4.0 } else { 8.0 }),
            state,
            titles,
            playing,
        );
        let renaming = matches!(&state.renaming, Some((id, ..)) if *id == look.id);
        if renaming {
            self.rename_field(ui, state, actions);
        } else if !self.compact {
            library::card_name(ui, p, picture, &look.name, None);
        }

        if response.clicked() || response.drag_started() {
            state.picked = Some(look.id);
            // Return is this tile's from here on, not a library item's.
            selection.asset = None;
            response.request_focus();
        }
        let now = ui.input(|i| i.time);
        let again = response.double_clicked() || response.triple_clicked();
        let twice = again && state.given_at.is_none_or(|at| now - at > AGAIN);
        if twice {
            state.given_at = Some(now);
        }
        if twice || (response.has_focus() && enter && !renaming) {
            actions.push(Action::AddLook {
                look: Arc::clone(look),
                track: None,
                at: None,
            });
        }
        if response.drag_started() {
            egui::DragAndDrop::set_payload(ui.ctx(), LookDrag(Arc::clone(look)));
        }
        // What is being carried follows the pointer: its picture, lifted.
        if response.dragged()
            && let Some(pos) = ui.ctx().pointer_latest_pos()
        {
            let layer = egui::LayerId::new(egui::Order::Tooltip, Id::new("text-carry"));
            let mut carry = ui.ctx().layer_painter(layer);
            let lifted = Rect::from_min_size(pos + vec2(12.0, 10.0), vec2(64.0, 36.0));
            carry.add(p.popup_shadow().as_shape(lifted, 6));
            carry.set_opacity(0.85);
            carry.rect_filled(lifted, 6, p.stage());
            if let Some((picture, texture)) = titles.picture(look.id) {
                let covers = covered(picture);
                let inner = lifted.shrink(4.0);
                let fit = (inner.width() / covers.width().max(1.0))
                    .min(inner.height() / covers.height().max(1.0));
                carry.image(
                    texture,
                    Rect::from_center_size(inner.center(), covers.size() * fit),
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        }
        let titled = (selection.clips.iter())
            .filter_map(|id| input.project.clip(*id))
            .any(|(_, clip)| matches!(clip.content, ClipContent::Text { .. }));
        response.context_menu(|ui| {
            widgets::menu_style(ui, 220.0);
            if widgets::menu_item(
                ui,
                p,
                Some(crate::icons::Icon::Plus),
                "Add at playhead",
                "Return",
            )
            .clicked()
            {
                actions.push(Action::AddLook {
                    look: Arc::clone(look),
                    track: None,
                    at: None,
                });
                ui.close();
            }
            if look.is_title() {
                let give = ui.add_enabled_ui(titled, |ui| {
                    widgets::menu_item(
                        ui,
                        p,
                        Some(crate::icons::Icon::Text),
                        "Style selected titles",
                        "",
                    )
                });
                if give.inner.clicked() {
                    actions.push(restyle(input.project, look, &selection.clips));
                    ui.close();
                }
            }
            if look.group == Group::Yours {
                widgets::menu_separator(ui, p);
                if widgets::menu_item(ui, p, None, "Rename…", "").clicked() {
                    state.renaming = Some((look.id, look.name.clone(), true));
                    ui.close();
                }
                if widgets::menu_item_destructive(
                    ui,
                    p,
                    Some(crate::icons::Icon::Trash),
                    "Remove preset",
                    "",
                )
                .clicked()
                {
                    actions.push(Action::Presets(PresetChange::Remove(look.id)));
                    ui.close();
                }
            }
        });
        let hint = if look.is_title() {
            "Double-click to add at the playhead, or drag onto the timeline. Let go on a title to give it this look."
        } else {
            "Double-click to add at the playhead, or drag onto the timeline."
        };
        let response = if self.compact {
            response.on_hover_text(format!("{}\n{hint}", look.name))
        } else {
            response.on_hover_text(hint)
        };
        response.hovered()
    }

    /// The look as the title lane draws it, fitted into `inner`; its name
    /// in the interface's face until the first picture comes.
    fn picture(
        &self,
        ui: &Ui,
        inner: Rect,
        state: &mut TextTabState,
        titles: &mut Titles,
        playing: Option<Time>,
    ) {
        let look = self.look;
        let fit = |covers: Rect| {
            (inner.width() / covers.width().max(1.0)).min(inner.height() / covers.height().max(1.0))
        };
        let shown = titles.picture(look.id).map(|(picture, _)| covered(picture));
        let wanted = fit(shown.unwrap_or_else(|| guessed(look))) * ui.ctx().pixels_per_point();
        // A small change of the scale keeps the picture there is.
        let scale = match state.scales.get(&look.id) {
            Some(asked) if shown.is_some() && (wanted / asked - 1.0).abs() < 0.15 => *asked,
            _ => wanted.clamp(0.01, 4.0),
        };
        state.scales.insert(look.id, scale);
        titles.want(look.id, &preview(look, playing), scale);
        match titles.picture(look.id) {
            Some((picture, texture)) => {
                let covers = covered(picture);
                let size = covers.size() * fit(covers);
                ui.painter().image(
                    texture,
                    Rect::from_center_size(inner.center(), size),
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => {
                let words = match &look.content {
                    ClipContent::Text { text, .. } => text.as_str(),
                    _ => look.name.as_str(),
                };
                let label = widgets::truncated(
                    ui,
                    words,
                    theme::medium(if self.compact { 18.0 } else { 12.0 }),
                    Color32::from_gray(230),
                    inner.width(),
                );
                kit::on_dark(ui.painter(), || {
                    ui.painter().galley(
                        inner.center() - label.size() * 0.5,
                        label,
                        Color32::from_gray(230),
                    );
                });
            }
        }
    }

    /// The field a kept look's new name is typed in, over its name.
    fn rename_field(&self, ui: &mut Ui, state: &mut TextTabState, actions: &mut Vec<Action>) {
        let Some((id, name, focus)) = &mut state.renaming else {
            return;
        };
        let field = Rect::from_min_max(
            pos2(self.rect.left(), self.rect.bottom() - NAME_HEIGHT + 2.0),
            self.rect.max,
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(field));
        let response = child.add(
            egui::TextEdit::singleline(name)
                .desired_width(field.width())
                .font(theme::regular(11.5)),
        );
        if std::mem::take(focus) {
            response.request_focus();
        }
        // Return keeps the name; leaving the field any other way does not.
        if response.lost_focus() {
            if ui.input(|i| i.key_pressed(Key::Enter)) {
                actions.push(Action::Presets(PresetChange::Rename(*id, name.clone())));
            }
            state.renaming = None;
        }
    }
}

#[cfg(test)]
#[path = "text_tab_tests.rs"]
mod tests;
