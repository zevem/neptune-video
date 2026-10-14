//! The selected clip's values, a section at a time.
//!
//! Every row reads the draft and writes into it, as the rest of the
//! inspector does; a row whose value keyframes drive reads the value at the
//! playhead and writes the keyframe there instead. What is not a value being
//! held (a keyframe made or removed, a look applied) is an [`Action`] with the
//! clip and the playhead it was made for.
//!
//! A section is a method of [`Pane`] chosen in [`rows`]: one that is not
//! written yet is [`Pane::placeholder`], and filling it is replacing that arm.
use super::{InspectorInput, InspectorState, ProjectDraft, Section, card, fact, title};
use crate::{
    icons::Icon,
    speed::{self, Curve},
    theme::{self, Palette},
    timefmt,
    ui::{
        Action, ClipDraft, EASINGS, icon_of_media, icon_of_track, tone_of_media, tone_of_track,
        viewer::fitted_size,
        widgets::{self, KeyRow, KeyState, Kind, row, section},
        with_key,
    },
};
use editor_core::{
    AnimTarget, AnimTrack, Asset, BlendMode, Clip, ClipContent, Command, Ease, MediaKind, ParamKey,
    Project, Time, Track, TrackKind, Transform, look,
};
use eframe::egui::{
    self, CornerRadius, Key, Rect, Response, Sense, SetOpenCommand, Stroke, StrokeKind, Ui,
    WidgetInfo, WidgetType, vec2,
};
use uuid::Uuid;

/// The width of a field that has a row to itself, and of one of two in a row.
pub(super) const FIELD: f32 = 78.0;
const PAIRED_FIELD: f32 = 62.0;
const PAIRED_NARROW: f32 = 50.0;
/// The gap between the two fields of a row.
const PAIRED_GAP: f32 = 4.0;
/// The room a row of two fields keeps for its name ("Position" whole), the
/// gaps beside it and the frames of its fields.
const PAIRED_LABEL: f32 = 88.0;

/// The values a row shows together, which are keyframed together: a value
/// written at a time gives the others of its row a keyframe there too.
const PAIRS: [[ParamKey; 2]; 2] = [
    [ParamKey::PositionX, ParamKey::PositionY],
    [ParamKey::ScaleX, ParamKey::ScaleY],
];

/// How a stored value is shown in its field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Field {
    /// The shown value is the stored one times this.
    pub scale: f32,
    /// The stored value's range.
    pub min: f32,
    pub max: f32,
    pub suffix: &'static str,
    pub decimals: usize,
    /// Shown units a point of drag moves the value by.
    pub speed: f64,
    pub width: f32,
}

const POSITION: Field = Field {
    scale: 1.0,
    min: -100_000.0,
    max: 100_000.0,
    suffix: " X",
    decimals: 0,
    speed: 1.0,
    width: PAIRED_FIELD,
};
const ROTATION: Field = Field {
    min: -3600.0,
    max: 3600.0,
    suffix: "°",
    decimals: 1,
    speed: 0.5,
    width: FIELD,
    ..POSITION
};
const PERCENT: Field = Field {
    scale: 100.0,
    min: 0.0,
    max: 1.0,
    suffix: " %",
    decimals: 0,
    speed: 0.5,
    width: FIELD,
};

const BLENDS: [(BlendMode, &str); 9] = [
    (BlendMode::Normal, "Normal"),
    (BlendMode::Multiply, "Multiply"),
    (BlendMode::Screen, "Screen"),
    (BlendMode::Overlay, "Overlay"),
    (BlendMode::SoftLight, "Soft light"),
    (BlendMode::Add, "Add"),
    (BlendMode::Darken, "Darken"),
    (BlendMode::Lighten, "Lighten"),
    (BlendMode::Difference, "Difference"),
];

/// The sections a clip is offered: none of them holds a value the project
/// would refuse for it. A clip on an audio track has sound alone, and no
/// sections; a title has no look, colour or speed.
pub(super) fn sections(track: &Track, clip: &Clip) -> &'static [Section] {
    match (track.kind, &clip.content) {
        (TrackKind::Audio, _) => &[],
        (_, ClipContent::Text { .. }) => &[
            Section::Video,
            Section::Text,
            Section::Animate,
            Section::Motion,
        ],
        (_, ClipContent::Graphic { .. }) => &[
            Section::Video,
            Section::Graphic,
            Section::Animate,
            Section::Motion,
        ],
        _ => &[
            Section::Video,
            Section::Look,
            Section::Color,
            Section::Speed,
            Section::Motion,
        ],
    }
}

/// One of the clip's own values, by the key it is animated under.
fn clip_value(draft: &ClipDraft, key: ParamKey) -> f32 {
    let t = &draft.transform;
    match key {
        ParamKey::PositionX => t.position[0],
        ParamKey::PositionY => t.position[1],
        ParamKey::ScaleX => t.scale[0],
        ParamKey::ScaleY => t.scale[1],
        ParamKey::Rotation => t.rotation,
        ParamKey::Opacity => t.opacity,
        ParamKey::CropLeft => t.crop[0],
        ParamKey::CropTop => t.crop[1],
        ParamKey::CropRight => t.crop[2],
        ParamKey::CropBottom => t.crop[3],
        ParamKey::Gain => draft.gain,
        _ => 0.0,
    }
}

fn set_clip_value(draft: &mut ClipDraft, key: ParamKey, value: f32) {
    let t = &mut draft.transform;
    match key {
        ParamKey::PositionX => t.position[0] = value,
        ParamKey::PositionY => t.position[1] = value,
        ParamKey::ScaleX => t.scale[0] = value,
        ParamKey::ScaleY => t.scale[1] = value,
        ParamKey::Rotation => t.rotation = value,
        ParamKey::Opacity => t.opacity = value,
        ParamKey::CropLeft => t.crop[0] = value,
        ParamKey::CropTop => t.crop[1] = value,
        ParamKey::CropRight => t.crop[2] = value,
        ParamKey::CropBottom => t.crop[3] = value,
        ParamKey::Gain => draft.gain = value,
        _ => {}
    }
}

/// Where a picture-in-picture layout puts a clip on the canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Layout {
    Fill,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Left,
    Right,
}

/// The scale of an inset picture, and its margin as a part of the canvas's
/// height.
const INSET_SCALE: f32 = 0.3;
const INSET_MARGIN: f32 = 0.04;

impl Layout {
    pub(super) const ALL: [Self; 7] = [
        Self::Fill,
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
        Self::Left,
        Self::Right,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Fill => "Fill the canvas",
            Self::TopLeft => "Inset, top left",
            Self::TopRight => "Inset, top right",
            Self::BottomLeft => "Inset, bottom left",
            Self::BottomRight => "Inset, bottom right",
            Self::Left => "Left half",
            Self::Right => "Right half",
        }
    }

    /// The side of the canvas an inset sits against on each axis.
    fn corner(self) -> Option<[f32; 2]> {
        match self {
            Self::TopLeft => Some([-1.0, -1.0]),
            Self::TopRight => Some([1.0, -1.0]),
            Self::BottomLeft => Some([-1.0, 1.0]),
            Self::BottomRight => Some([1.0, 1.0]),
            Self::Fill | Self::Left | Self::Right => None,
        }
    }

    /// `from` laid out this way: a layout sets position, scale and crop, and
    /// leaves the turn, the flips and the opacity alone. `source` is the
    /// picture's size and `canvas` the canvas's, in pixels.
    pub(super) fn apply(self, from: &Transform, source: [f32; 2], canvas: [f32; 2]) -> Transform {
        let mut to = Transform {
            position: [0.0, 0.0],
            scale: [1.0, 1.0],
            crop: [0.0; 4],
            ..from.clone()
        };
        let [width, height] = canvas;
        if let Some(side) = self.corner() {
            let fitted = fitted_size(source.into(), to.crop, canvas.into()) * INSET_SCALE;
            let margin = INSET_MARGIN * height;
            to.scale = [INSET_SCALE; 2];
            to.position = [
                (side[0] * (width - fitted.x) * 0.5 - side[0] * margin).round(),
                (side[1] * (height - fitted.y) * 0.5 - side[1] * margin).round(),
            ];
        } else if self != Self::Fill {
            // Half the canvas, filled: what of the picture does not fit the
            // half is cropped away evenly from both sides.
            let wanted = (width * 0.5) / height.max(1.0);
            let shape = source[0] / source[1].max(1.0);
            if shape > wanted {
                let cut = ((1.0 - wanted / shape) * 0.5).clamp(0.0, 0.49);
                to.crop = [cut, 0.0, cut, 0.0];
            } else {
                let cut = ((1.0 - shape / wanted) * 0.5).clamp(0.0, 0.49);
                to.crop = [0.0, cut, 0.0, cut];
            }
            let side = if self == Self::Left { -1.0 } else { 1.0 };
            to.position = [(side * width * 0.25).round(), 0.0];
        }
        to
    }

    /// The part of the canvas the layout fills, as the glyph draws it: left,
    /// top, width and height in parts of the canvas.
    fn cell(self) -> [f32; 4] {
        match self {
            Self::Fill => [0.0, 0.0, 1.0, 1.0],
            Self::Left => [0.0, 0.0, 0.5, 1.0],
            Self::Right => [0.5, 0.0, 0.5, 1.0],
            _ => {
                let [x, y] = self.corner().unwrap_or([0.0, 0.0]);
                [
                    if x < 0.0 { 0.08 } else { 0.5 },
                    if y < 0.0 { 0.12 } else { 0.46 },
                    0.42,
                    0.42,
                ]
            }
        }
    }
}

/// A layout's button: the canvas with the part the layout fills.
fn layout_button(ui: &mut Ui, p: &Palette, layout: Layout, on: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(30.0, 26.0), Sense::click());
    let enabled = ui.is_enabled();
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, enabled, on, layout.label()));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let radius = CornerRadius::same(6);
        if on {
            painter.rect_filled(rect, radius, theme::tint(p.accent, 0.16));
        } else if response.hovered() && enabled {
            painter.rect_filled(rect, radius, p.hover);
        }
        if response.has_focus() {
            painter.rect_stroke(rect, radius, Stroke::new(1.5, p.accent), StrokeKind::Inside);
        }
        let ink = if !enabled {
            p.muted.gamma_multiply(0.5)
        } else if on {
            p.accent
        } else {
            p.secondary
        };
        let canvas = Rect::from_center_size(rect.center(), vec2(20.0, 13.0));
        painter.rect_stroke(
            canvas,
            CornerRadius::same(2),
            Stroke::new(1.0, ink.gamma_multiply(0.7)),
            StrokeKind::Inside,
        );
        let [x, y, width, height] = layout.cell();
        let inner = canvas.shrink(2.0);
        painter.rect_filled(
            Rect::from_min_size(
                inner.min + vec2(x * inner.width(), y * inner.height()),
                vec2(width * inner.width(), height * inner.height()),
            ),
            CornerRadius::same(1),
            ink,
        );
    }
    response.on_hover_text(layout.label())
}

/// What a section's rows are drawn with: the clip, the draft they write, and
/// where the playhead is in the clip.
pub(super) struct Pane<'a> {
    pub p: &'a Palette,
    pub project: &'a Project,
    pub track: &'a Track,
    pub clip: &'a Clip,
    /// The font families a title may name, once they are known.
    pub families: Option<&'a [String]>,
    /// The person's brand colours, offered in every colour.
    pub brand: &'a [[u8; 4]],
    pub asset: Option<&'a Asset>,
    /// The playhead, which may be off the clip.
    pub playhead: Time,
    /// The playhead from the clip's start, held inside the clip: where a
    /// keyframe made in this frame is put.
    pub local: Time,
    /// Half a canvas frame: the playhead within this of a keyframe is on it.
    pub reach: Time,
    pub next: &'a mut ClipDraft,
    /// The canvas's values being edited, which are the project's.
    pub canvas: &'a mut ProjectDraft,
    pub state: &'a mut InspectorState,
    pub actions: &'a mut Vec<Action>,
}

/// What stays above a clip's rows: its name, and the sections it is offered
/// when it has more than one. Hands back the section whose rows are shown.
pub(super) fn head(
    ui: &mut Ui,
    input: &InspectorInput,
    track: &Track,
    clip: &Clip,
    state: &mut InspectorState,
    compact: bool,
) -> Option<Section> {
    let (p, project) = (input.p, input.project);
    let rate = project.settings.frame_rate;
    let asset = clip.asset_id().and_then(|id| project.assets.get(&id));
    let detail = format!("{} · {}", track.name, timefmt::timecode(clip.start, rate));
    if compact {
        super::compact_title(ui, p, &clip.name, &detail);
    } else {
        title(
            ui,
            p,
            asset.map_or(icon_of_track(track.kind), |asset| icon_of_media(asset.kind)),
            p.tone(asset.map_or(tone_of_track(track.kind), |asset| tone_of_media(asset.kind))),
            &clip.name,
            &detail,
        );
    }
    if track.locked {
        ui.label(
            egui::RichText::new("This clip's track is locked.")
                .font(theme::regular(12.0))
                .color(p.muted),
        );
    }

    // The section last chosen stays chosen for the clips that have it.
    let offered = sections(track, clip);
    let mut section = if offered.contains(&state.section) {
        state.section
    } else {
        *offered.first()?
    };
    if offered.len() > 1 {
        let mut options = [(Section::Video, ""); 5];
        for (option, section) in options.iter_mut().zip(offered) {
            *option = (*section, section.label());
        }
        let width = ui.available_width();
        if widgets::segmented(ui, p, &mut section, &options[..offered.len()], width) {
            state.section = section;
        }
    }
    Some(section)
}

/// The rows of one of a clip's sections; of a clip that has no sections,
/// its sound's.
#[expect(clippy::too_many_arguments, reason = "one call site in the frame")]
pub(super) fn rows(
    ui: &mut Ui,
    input: &InspectorInput,
    track: &Track,
    clip: &Clip,
    section: Option<Section>,
    state: &mut InspectorState,
    draft: &mut Option<ClipDraft>,
    project_draft: &mut ProjectDraft,
    actions: &mut Vec<Action>,
) {
    let (p, project) = (input.p, input.project);
    let rate = project.settings.frame_rate;
    let before = draft
        .clone()
        .filter(|draft| draft.clip == clip.id)
        .unwrap_or_else(|| ClipDraft::of(clip));
    let mut next = before.clone();
    let mut pane = Pane {
        p,
        project,
        track,
        clip,
        families: input.families,
        brand: input.brand,
        asset: clip.asset_id().and_then(|id| project.assets.get(&id)),
        playhead: input.playhead,
        local: (input.playhead - clip.start).clamp(Time::ZERO, clip.duration),
        reach: Time(rate.frame_time(1).0 / 2),
        next: &mut next,
        canvas: project_draft,
        state: &mut *state,
        actions,
    };
    ui.add_enabled_ui(!track.locked, |ui| match section {
        None => pane.sound(ui),
        Some(Section::Video) => pane.video(ui),
        Some(Section::Look) => pane.look(ui),
        Some(Section::Speed) => pane.speed(ui),
        Some(Section::Color) => pane.color(ui),
        Some(Section::Motion) => pane.motion(ui),
        Some(Section::Text) => pane.text(ui),
        Some(Section::Graphic) => pane.graphic(ui),
        Some(Section::Animate) => pane.animate(ui),
    });

    if matches!(section, None | Some(Section::Video)) {
        section_timing(ui, p, clip, rate);
    }
    // What an interaction an Escape gave up still writes is dropped.
    if next != before && !state.cancelled {
        *draft = Some(next);
    }
}

fn section_timing(ui: &mut Ui, p: &Palette, clip: &Clip, rate: editor_core::FrameRate) {
    section(ui, p, "Timing");
    card(ui, p, |ui| {
        fact(ui, p, "Start", timefmt::timecode(clip.start, rate));
        fact(ui, p, "End", timefmt::timecode(clip.end(), rate));
        fact(ui, p, "Duration", timefmt::timecode(clip.duration, rate));
        if clip.linked_id.is_some() {
            fact(ui, p, "Linked", "Yes");
        }
        if clip.group_id.is_some() {
            fact(ui, p, "Grouped", "Yes");
        }
    });
}

impl<'a> Pane<'a> {
    // ---- Keyframes --------------------------------------------------------

    /// The keyframes that drive one value, when any do.
    pub(super) fn track(&self, target: Option<Uuid>, key: ParamKey) -> Option<&'a AnimTrack> {
        (self.clip.animation.iter())
            .find(|track| track.target == target && track.key == key && !track.keys.is_empty())
    }

    /// The time of the keyframe the playhead sits on, or the playhead's own
    /// when it is on none.
    fn key_time(&self, track: &AnimTrack) -> (Time, bool) {
        (track.keys.iter())
            .find(|key| (key.at.0 - self.local.0).abs() <= self.reach.0)
            .map_or((self.local, false), |key| (key.at, true))
    }

    /// The value a row shows: the one being typed or dragged, the one its
    /// keyframes give at the playhead, or the stored one.
    pub(super) fn shown(&self, target: Option<Uuid>, key: ParamKey, stored: f32) -> f32 {
        if let Some(held) = self.next.key(target, key) {
            return held.value;
        }
        self.track(target, key)
            .map_or(stored, |track| track.sample(self.key_time(track).0))
    }

    /// Takes a value a row was given. One that keyframes drive is held as the
    /// keyframe at the playhead; otherwise it is the stored value's to take
    /// and is handed back.
    pub(super) fn write(&mut self, target: Option<Uuid>, key: ParamKey, value: f32) -> Option<f32> {
        let Some(track) = self.track(target, key) else {
            return Some(value);
        };
        let (at, _) = self.key_time(track);
        self.next.set_key(target, key, at, value);
        // The other value of the row is keyed with it, at what it is there,
        // so the row's diamond speaks for both.
        let partners = PAIRS
            .iter()
            .filter(|pair| target.is_none() && pair.contains(&key));
        for partner in partners.flatten().filter(|partner| **partner != key) {
            if self.next.key(target, *partner).is_none()
                && let Some(track) = self.track(target, *partner)
            {
                self.next.set_key(target, *partner, at, track.sample(at));
            }
        }
        None
    }

    /// A field for one value. Hands back the new stored value when that is
    /// what changed.
    pub(super) fn field(
        &mut self,
        ui: &mut Ui,
        target: Option<Uuid>,
        key: ParamKey,
        stored: f32,
        field: &Field,
        name: &str,
    ) -> Option<f32> {
        let mut shown = self.shown(target, key, stored) * field.scale;
        let range = field.min * field.scale..=field.max * field.scale;
        let changed = widgets::named_number(
            ui,
            &mut shown,
            range,
            field.speed,
            field.suffix,
            field.decimals,
            field.width,
            name,
        )
        .changed();
        if !changed {
            return None;
        }
        self.write(
            target,
            key,
            (shown / field.scale).clamp(field.min, field.max),
        )
    }

    pub(super) fn key_state(&self, target: Option<Uuid>, keys: &[ParamKey]) -> KeyState {
        let (low, high) = (self.local.0 - self.reach.0, self.local.0 + self.reach.0);
        let mut state = None;
        for track in keys.iter().filter_map(|key| self.track(target, *key)) {
            // The row is on a keyframe where any of its values has one: the
            // diamond then takes it away from all of them.
            let (on_key, previous, next) = state.unwrap_or((false, false, false));
            state = Some((
                on_key || self.key_time(track).1,
                previous || track.keys.iter().any(|key| key.at.0 < low),
                next || track.keys.iter().any(|key| key.at.0 > high),
            ));
        }
        state.map_or(KeyState::Off, |(on_key, previous, next)| {
            KeyState::Animated {
                on_key,
                previous,
                next,
            }
        })
    }

    /// One command for a keyframe edit, undone as one step.
    fn edit(&mut self, label: &str, mut commands: Vec<Command>) {
        let command = match commands.len() {
            0 => return,
            1 => commands.remove(0),
            _ => Command::Batch {
                label: label.to_owned(),
                commands,
            },
        };
        self.actions.push(Action::Edit(command));
    }

    /// What the diamond and the steps beside a row's fields do. `stored`
    /// reads the stored value of one of `keys` out of the draft.
    pub(super) fn key_controls(
        &mut self,
        ui: &mut Ui,
        row: &KeyRow,
        target: Option<Uuid>,
        anim: AnimTarget,
        keys: &[ParamKey],
        stored: &dyn Fn(&ClipDraft, ParamKey) -> f32,
    ) {
        let clip = self.clip;
        let p = self.p;
        let (low, high) = (self.local.0 - self.reach.0, self.local.0 + self.reach.0);
        let tracks = || {
            keys.iter()
                .filter_map(|key| Some((*key, self.track(target, *key)?)))
        };
        let times = || tracks().flat_map(|(_, track)| track.keys.iter().map(|key| key.at));
        let clicked =
            |response: &Option<Response>| response.as_ref().is_some_and(Response::clicked);
        let step = if clicked(&row.previous) {
            times().filter(|at| at.0 < low).max()
        } else if clicked(&row.next) {
            times().filter(|at| at.0 > high).min()
        } else {
            None
        };
        // The key whose easing shapes the stretch the playhead is in. The
        // last key of a track leaves no stretch: an easing set on it would
        // be a step in the history that changes nothing.
        let leaving = |track: &AnimTrack| {
            (track.keys.iter())
                .rposition(|key| key.at.0 <= high)
                .filter(|index| index + 1 < track.keys.len())
        };
        let easing = tracks().find_map(|(_, track)| Some(track.keys[leaving(track)?].ease));
        let animated = tracks().next().is_some();
        // A keyframe goes where the playhead is, and only on the clip: off
        // it, the diamond says so instead of keying the clip's nearer end.
        let outside = self.playhead < clip.start || self.playhead > clip.end();

        // Shift+Enter on the focused diamond opens its menu, as a secondary
        // click does; a plain Enter or a click is the diamond's own.
        // The key is read with the modifiers of its own event, as the
        // application's shortcuts are.
        let menu_key = row.toggle.has_focus()
            && ui.input(|i| {
                i.events.iter().any(|event| {
                    matches!(
                        event,
                        egui::Event::Key {
                            key: Key::Enter,
                            pressed: true,
                            modifiers,
                            ..
                        } if modifiers.shift
                    )
                })
            });
        let mut toggled = row.toggle.clicked() && !menu_key;
        let open = if row.toggle.secondary_clicked() || menu_key {
            Some(SetOpenCommand::Bool(true))
        } else if row.toggle.clicked() {
            Some(SetOpenCommand::Bool(false))
        } else {
            None
        };
        let mut eased: Option<Ease> = None;
        let mut removed = false;
        let menu_id = egui::Popup::default_response_id(&row.toggle);
        let was_open = egui::Popup::is_id_open(ui.ctx(), menu_id) && open.is_none();
        // A menu the keyboard opened puts the keyboard on its first choice:
        // the rows are laid out after the whole pane, a long way down the
        // Tab order from the diamond. The frame a menu is measured on has
        // nothing enabled; the request waits for the one that shows it.
        let reach_id = row.toggle.id.with("reach");
        if menu_key {
            ui.data_mut(|data| data.insert_temp(reach_id, true));
        }
        let reach = |response: &egui::Response, enabled: bool| {
            let ctx = &response.ctx;
            if enabled && ctx.data(|data| data.get_temp(reach_id).unwrap_or(false)) {
                response.request_focus();
                ctx.data_mut(|data| data.remove::<bool>(reach_id));
            }
        };
        egui::Popup::menu(&row.toggle).open_memory(open).show(|ui| {
            widgets::menu_style(ui, 196.0);
            if !animated {
                let add = widgets::menu_item(ui, p, None, "Add keyframe at playhead", "");
                reach(&add, ui.is_enabled());
                if add.clicked() {
                    toggled = true;
                    ui.close();
                }
                return;
            }
            ui.label(
                egui::RichText::new(if easing.is_some() {
                    "Easing from this keyframe"
                } else {
                    "No keyframe follows to ease to"
                })
                .font(theme::medium(11.0))
                .color(p.muted),
            );
            ui.add_enabled_ui(easing.is_some(), |ui| {
                for (name, ease) in EASINGS {
                    let chosen = (easing == Some(ease)).then_some(Icon::Check);
                    let row = widgets::menu_item(ui, p, chosen, name, "");
                    reach(&row, ui.is_enabled());
                    if row.clicked() {
                        eased = Some(ease);
                        ui.close();
                    }
                }
            });
            widgets::hairline(ui, p);
            let remove = widgets::menu_item(ui, p, Some(Icon::Trash), "Remove animation", "");
            reach(&remove, ui.is_enabled());
            if remove.clicked() {
                removed = true;
                ui.close();
            }
        });
        // A menu the keyboard closed, with a choice or an Escape, hands the
        // keyboard back to the diamond it was opened from.
        let by_key = ui.input(|i| {
            i.key_pressed(Key::Escape) || i.key_pressed(Key::Enter) || i.key_pressed(Key::Space)
        });
        if was_open && by_key && !egui::Popup::is_id_open(ui.ctx(), menu_id) {
            row.toggle.request_focus();
        }
        let refused = toggled && outside;
        toggled &= !outside;

        let keyframes = |key: ParamKey, keys| Command::SetKeyframes {
            clip_id: clip.id,
            target,
            key,
            keys,
        };
        let (label, commands): (&str, Vec<Command>) = if removed {
            (
                "Remove animation",
                tracks()
                    .map(|(key, _)| keyframes(key, Vec::new()))
                    .collect(),
            )
        } else if let Some(ease) = eased {
            (
                "Change easing",
                tracks()
                    .filter_map(|(key, track)| {
                        let mut keys = track.keys.clone();
                        keys[leaving(track)?].ease = ease;
                        Some(keyframes(key, keys))
                    })
                    .collect(),
            )
        } else if toggled
            && matches!(
                self.key_state(target, keys),
                KeyState::Animated { on_key: true, .. }
            )
        {
            (
                "Remove keyframe",
                tracks()
                    .map(|(key, track)| {
                        let (at, _) = self.key_time(track);
                        let keys = track.keys.iter().filter(|key| key.at != at).copied();
                        keyframes(key, keys.collect())
                    })
                    .collect(),
            )
        } else if toggled {
            // A keyframe at the playhead for each value of the row, holding
            // what the row shows there.
            (
                "Add keyframe",
                keys.iter()
                    .map(|key| {
                        let value = self.shown(target, *key, stored(self.next, *key));
                        let value = look::param_spec(anim, *key)
                            .map_or(value, |spec| value.clamp(spec.min, spec.max));
                        let (at, existing) = match self.track(target, *key) {
                            Some(track) => (self.key_time(track).0, track.keys.as_slice()),
                            None => (self.local, [].as_slice()),
                        };
                        keyframes(*key, with_key(existing, at, value))
                    })
                    .collect(),
            )
        } else {
            ("", Vec::new())
        };
        self.edit(label, commands);
        if refused {
            (self.actions).push(Action::Notify(
                "Move the playhead onto the clip to set a keyframe.".to_owned(),
            ));
        }
        if let Some(at) = step {
            self.actions.push(Action::Seek(clip.start + at));
        }
    }

    /// A row of the clip's own values, keyframed under `keys`.
    fn clip_row(
        &mut self,
        ui: &mut Ui,
        label: &str,
        keys: &[ParamKey],
        fields: impl FnOnce(&mut Self, &mut Ui),
    ) {
        let state = self.key_state(None, keys);
        let p = self.p;
        let what = label.to_lowercase();
        let row = widgets::keyed_row(ui, p, label, &what, state, |ui| fields(self, ui));
        self.key_controls(ui, &row, None, AnimTarget::Clip, keys, &clip_value);
    }

    /// A field for one of the clip's own values.
    fn clip_field(&mut self, ui: &mut Ui, key: ParamKey, field: &Field, name: &str) {
        let stored = clip_value(self.next, key);
        if let Some(value) = self.field(ui, None, key, stored, field, name) {
            set_clip_value(self.next, key, value);
        }
    }

    // ---- Sections ---------------------------------------------------------

    /// A section nothing is edited in yet.
    pub(super) fn placeholder(&mut self, ui: &mut Ui, text: &str) {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(text)
                .font(theme::regular(12.0))
                .color(self.p.muted),
        );
    }

    fn video(&mut self, ui: &mut Ui) {
        let p = self.p;
        if self.asset.is_some() {
            self.layouts(ui);
        }

        section(ui, p, "Transform");
        card(ui, p, |ui| {
            // Two fields give way before the row's other controls do: a
            // narrow panel has them narrower, the same whether or not the
            // steps between keyframes are shown, and the name is left whole.
            let spare = ui.available_width() - 3.0 * widgets::SMALL_BUTTON - PAIRED_LABEL;
            self.clip_row(
                ui,
                "Position",
                &[ParamKey::PositionX, ParamKey::PositionY],
                |pane, ui| {
                    let x = Field {
                        width: (spare * 0.5).clamp(PAIRED_NARROW, PAIRED_FIELD),
                        ..POSITION
                    };
                    let y = Field { suffix: " Y", ..x };
                    ui.spacing_mut().item_spacing.x = PAIRED_GAP;
                    pane.clip_field(ui, ParamKey::PositionX, &x, "Position X");
                    pane.clip_field(ui, ParamKey::PositionY, &y, "Position Y");
                },
            );
            self.clip_row(
                ui,
                "Scale",
                &[ParamKey::ScaleX, ParamKey::ScaleY],
                |pane, ui| {
                    // One field scales both axes and keeps their ratio.
                    let [x, y] = pane.next.transform.scale;
                    let x = pane.shown(None, ParamKey::ScaleX, x);
                    let y = pane.shown(None, ParamKey::ScaleY, y);
                    let mut percent = x * 100.0;
                    let field = widgets::named_number(
                        ui,
                        &mut percent,
                        1.0..=10_000.0,
                        0.5,
                        " %",
                        0,
                        FIELD,
                        "Scale",
                    );
                    if field.changed() {
                        let ratio = y / x.max(0.001);
                        let x = (percent / 100.0).clamp(0.001, 100.0);
                        let y = (x * ratio).clamp(0.001, 100.0);
                        if let Some(x) = pane.write(None, ParamKey::ScaleX, x) {
                            pane.next.transform.scale[0] = x;
                        }
                        if let Some(y) = pane.write(None, ParamKey::ScaleY, y) {
                            pane.next.transform.scale[1] = y;
                        }
                    }
                },
            );
            self.clip_row(ui, "Rotation", &[ParamKey::Rotation], |pane, ui| {
                pane.clip_field(ui, ParamKey::Rotation, &ROTATION, "Rotation");
            });
            self.clip_row(ui, "Opacity", &[ParamKey::Opacity], |pane, ui| {
                pane.clip_field(ui, ParamKey::Opacity, &PERCENT, "Opacity");
            });
            let t = &mut self.next.transform;
            row(ui, p, "Flip", |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if widgets::icon_toggle(ui, p, Icon::FlipVertical, t.flip_y, "Flip vertically")
                    .clicked()
                {
                    t.flip_y = !t.flip_y;
                }
                if widgets::icon_toggle(ui, p, Icon::FlipHorizontal, t.flip_x, "Flip horizontally")
                    .clicked()
                {
                    t.flip_x = !t.flip_x;
                }
            });
        });

        if self.asset.is_some() {
            section(ui, p, "Crop");
            card(ui, p, |ui| {
                const SIDES: [(ParamKey, &str); 4] = [
                    (ParamKey::CropLeft, "Left"),
                    (ParamKey::CropTop, "Top"),
                    (ParamKey::CropRight, "Right"),
                    (ParamKey::CropBottom, "Bottom"),
                ];
                for (index, (key, label)) in SIDES.into_iter().enumerate() {
                    self.clip_row(ui, label, &[key], |pane, ui| {
                        // Opposite sides together must leave some picture.
                        let (other, _) = SIDES[(index + 2) % 4];
                        let other = pane.shown(None, other, clip_value(pane.next, other));
                        let field = Field {
                            max: (0.99 - other).max(0.0),
                            speed: 0.25,
                            ..PERCENT
                        };
                        pane.clip_field(ui, key, &field, label);
                    });
                }
            });
        }
        if self.next.transform != Transform::default() {
            ui.horizontal(|ui| {
                if widgets::button(ui, p, Some(Icon::Rotate), "Reset transform", Kind::Plain)
                    .clicked()
                {
                    self.next.transform = Transform::default();
                }
            });
        }

        // A title is drawn over the composition: it takes no blend or mask.
        if self.asset.is_some() {
            section(ui, p, "Compositing");
            card(ui, p, |ui| {
                widgets::fitted_row(ui, p, "Blend", |ui| {
                    let blend = &mut self.next.blend;
                    let name = |mode: BlendMode| {
                        BLENDS
                            .iter()
                            .find(|(listed, _)| *listed == mode)
                            .map_or("Normal", |(_, name)| name)
                    };
                    egui::ComboBox::from_id_salt("blend-mode")
                        .selected_text(name(*blend))
                        .width(118.0)
                        // Every mode at once: nine rows are few enough that
                        // none should be found by scrolling.
                        .height(BLENDS.len() as f32 * 40.0)
                        .show_ui(ui, |ui| {
                            for (mode, name) in BLENDS {
                                ui.selectable_value(blend, mode, name);
                            }
                        })
                        .response
                        .widget_info(|| {
                            WidgetInfo::labeled(WidgetType::ComboBox, true, "Blend mode")
                        });
                });
            });
            self.masks(ui);
        }
        if self.asset.is_some_and(|asset| asset.has_audio) {
            section(ui, p, "Sound");
            card(ui, p, |ui| self.volume_row(ui));
        }
    }

    /// The picture-in-picture layouts: each sets the clip's place, scale and
    /// crop in one step.
    fn layouts(&mut self, ui: &mut Ui) {
        let Some(asset) = self.asset else {
            return;
        };
        let p = self.p;
        let settings = &self.project.settings;
        let canvas = [settings.width as f32, settings.height as f32];
        let source = if asset.width > 0 && asset.height > 0 {
            [asset.width as f32, asset.height as f32]
        } else {
            canvas
        };
        // A layout writes stored values, which show only where no keyframe
        // drives them.
        const WRITTEN: [ParamKey; 8] = [
            ParamKey::PositionX,
            ParamKey::PositionY,
            ParamKey::ScaleX,
            ParamKey::ScaleY,
            ParamKey::CropLeft,
            ParamKey::CropTop,
            ParamKey::CropRight,
            ParamKey::CropBottom,
        ];
        let free = self.key_state(None, &WRITTEN) == KeyState::Off;
        section(ui, p, "Layout");
        ui.add_enabled_ui(free, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                for layout in Layout::ALL {
                    let laid = layout.apply(&self.next.transform, source, canvas);
                    let on = free && laid == self.next.transform;
                    let button = layout_button(ui, p, layout, on)
                        .on_disabled_hover_text("Position, scale or crop is animated");
                    if button.clicked() {
                        self.next.transform = laid;
                    }
                }
            });
        });
    }

    /// A clip on an audio track: it carries sound and nothing that is drawn.
    fn sound(&mut self, ui: &mut Ui) {
        let p = self.p;
        let timed = self
            .asset
            .is_some_and(|asset| asset.kind != MediaKind::Image);
        let audible = self.asset.is_some_and(|asset| asset.has_audio);
        if !(timed || audible) {
            return;
        }
        section(ui, p, "Playback");
        card(ui, p, |ui| {
            if timed {
                self.speed_row(ui);
            }
            if audible {
                self.volume_row(ui);
            }
        });
    }

    pub(super) fn speed_row(&mut self, ui: &mut Ui) {
        let p = self.p;
        let (slowest, fastest) = (speed::SLOWEST as f32, speed::FASTEST as f32);
        let map = match (&self.next.retime, &self.clip.retime) {
            (Some((map, _)), _) | (None, Some(map)) => Some(Curve::of_map(map)),
            (None, None) => None,
        };
        let Some(curve) = map else {
            row(ui, p, "Speed", |ui| {
                let mut shown = self.next.rate as f32;
                let range = slowest..=fastest;
                if widgets::named_number(ui, &mut shown, range, 0.01, " ×", 2, FIELD, "Speed")
                    .changed()
                {
                    self.next.rate = f64::from(shown).clamp(speed::SLOWEST, speed::FASTEST);
                }
            });
            return;
        };
        // The rate of a mapped clip is fixed at 1 and its speed is the map:
        // the field shows the speed it has on the whole, and a change of
        // that plays the whole map as much faster or slower.
        let Some(mean) = curve.mean() else {
            fact(ui, p, "Speed", "Held frame");
            return;
        };
        let label = if curve.steady().is_some() {
            "Speed"
        } else {
            "Average speed"
        };
        row(ui, p, label, |ui| {
            let mut shown = (mean * self.next.rate) as f32;
            let range = (mean as f32 * slowest)..=(mean as f32 * fastest);
            if widgets::named_number(ui, &mut shown, range, 0.01, " ×", 2, FIELD, "Speed").changed()
            {
                self.next.rate = (f64::from(shown) / mean).clamp(speed::SLOWEST, speed::FASTEST);
            }
        });
    }

    fn volume_row(&mut self, ui: &mut Ui) {
        const VOLUME: Field = Field {
            max: 8.0,
            speed: 1.0,
            ..PERCENT
        };
        self.clip_row(ui, "Volume", &[ParamKey::Gain], |pane, ui| {
            pane.clip_field(ui, ParamKey::Gain, &VOLUME, "Volume");
        });
    }
}
