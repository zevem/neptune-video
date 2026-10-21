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
use super::{
    FIELD, Ink, InspectorInput, InspectorState, PAIRED_GAP, ProjectDraft, SWITCH, Section, Thumb,
    button, button_row, card, fact, in_reading_order, keyed_card, keyed_row, note, number,
    number_with, object_header, paired_width, row, section, section_switch, select_list, tall_row,
    time_fact,
};
use crate::{
    icons::Icon,
    normalize,
    speed::{self, Curve},
    theme::Palette,
    timefmt,
    ui::{
        Action, ClipDraft, EASINGS,
        fades::{self, FadeEdge},
        icon_of_media, icon_of_track, kit, tone_of_media, tone_of_track,
        viewer::fitted_size,
        widgets::{self, KeyRow, KeyState, Kind},
        with_key,
    },
};
use editor_core::{
    AnimTarget, AnimTrack, Asset, BlendMode, Clip, ClipContent, Command, Ease, MediaKind, ParamKey,
    Project, Time, Track, TrackKind, Transform, look, sound,
};
use eframe::egui::{
    self, Key, Rect, Response, Sense, SetOpenCommand, Ui, Vec2, WidgetInfo, WidgetType, pos2, vec2,
};
use uuid::Uuid;

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
    /// The axis, before the value, where the row's name says it.
    pub prefix: Option<&'static str>,
    pub suffix: &'static str,
    pub decimals: usize,
    /// Shown units a point of drag moves the value by.
    pub speed: f64,
    pub width: f32,
    /// Dragged by a part of the value rather than by `speed`, so that each
    /// point of drag is the same step to the ear: an octave takes as long
    /// at 100 Hz as at 10 kHz.
    pub log: bool,
}

const POSITION: Field = Field {
    scale: 1.0,
    min: -100_000.0,
    max: 100_000.0,
    prefix: Some("X"),
    suffix: "",
    decimals: 0,
    speed: 1.0,
    width: super::PAIRED_FIELD,
    log: false,
};
const ROTATION: Field = Field {
    min: -3600.0,
    max: 3600.0,
    prefix: None,
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
    prefix: None,
    suffix: " %",
    decimals: 0,
    speed: 0.5,
    width: FIELD,
    log: false,
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
/// would refuse for it. A clip on an audio track has sound alone, in one
/// section; a title has no look, colour or speed.
pub(super) fn sections(track: &Track, clip: &Clip) -> &'static [Section] {
    match (track.kind, &clip.content) {
        (TrackKind::Audio, _) => &[Section::Audio],
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

    /// The icon for this layout's position in the canvas.
    fn icon(self) -> Icon {
        match self {
            Self::Fill => Icon::LayoutFill,
            Self::TopLeft => Icon::LayoutTopLeft,
            Self::TopRight => Icon::LayoutTopRight,
            Self::BottomLeft => Icon::LayoutBottomLeft,
            Self::BottomRight => Icon::LayoutBottomRight,
            Self::Left => Icon::LayoutLeft,
            Self::Right => Icon::LayoutRight,
        }
    }
}

/// A layout's button: the canvas with the part the layout fills, pressed
/// while it is the clip's layout.
fn layout_button(ui: &mut Ui, p: &Palette, layout: Layout, on: bool) -> Response {
    let (_, response) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    let enabled = ui.is_enabled();
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, enabled, on, layout.label()));
    kit::paint_icon_button(ui, p, &response, layout.icon(), on, None);
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
    /// The audio kinds the session's FFmpeg does not run; `None` until its
    /// probe has finished.
    pub missing: Option<&'a [editor_core::EffectKind]>,
    /// The canvas's values being edited, which are the project's.
    pub canvas: &'a mut ProjectDraft,
    pub state: &'a mut InspectorState,
    pub actions: &'a mut Vec<Action>,
}

/// What stays above a clip's rows in `rect`: its header, and under it the
/// sections it is offered when it has more than one. Hands back the section
/// whose rows are shown and where what stays above them ends.
pub(super) fn head(
    ui: &mut Ui,
    input: &InspectorInput,
    rect: Rect,
    track: &Track,
    clip: &Clip,
    state: &mut InspectorState,
    compact: bool,
) -> (Option<Section>, f32) {
    let (p, project) = (input.p, input.project);
    let rate = project.settings.frame_rate;
    let asset = clip.asset_id().and_then(|id| project.assets.get(&id));
    let icon = asset.map_or(icon_of_track(track.kind), |asset| icon_of_media(asset.kind));
    let tone =
        Ink::Tone(asset.map_or(tone_of_track(track.kind), |asset| tone_of_media(asset.kind)));
    // A picture shows its poster; a sound its wave, and a title its glyph.
    let thumb = match asset.map(|asset| asset.kind) {
        Some(MediaKind::Video | MediaKind::Image | MediaKind::Gif) => Thumb::Picture(
            asset.and_then(|asset| input.textures.poster(asset.id)),
            icon,
            tone,
        ),
        Some(MediaKind::Audio) => Thumb::Glyph(Icon::Music, Ink::Wave),
        _ if track.kind == TrackKind::Audio => Thumb::Glyph(Icon::Music, Ink::Wave),
        _ => Thumb::Glyph(icon, tone),
    };
    let words = format!("{} · ", track.name);
    let time = timefmt::timecode(clip.start, rate);
    object_header(ui, p, rect, thumb, &clip.name, (&words, &time), compact);

    // The section last chosen stays chosen for the clips that have it.
    let offered = sections(track, clip);
    let Some(first) = offered.first() else {
        return (None, rect.bottom());
    };
    let mut section = if offered.contains(&state.section) {
        state.section
    } else {
        *first
    };
    if offered.len() < 2 {
        return (Some(section), rect.bottom());
    }
    let switch = Rect::from_min_size(
        pos2(rect.left(), rect.bottom() + 4.0),
        vec2(rect.width(), SWITCH),
    );
    if section_switch(ui, p, switch, offered, &mut section) {
        state.section = section;
    }
    (Some(section), switch.bottom())
}

/// The rows of one of a clip's sections; of a clip that has none, its
/// sound's.
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
        missing: input.missing,
        canvas: project_draft,
        state: &mut *state,
        actions,
    };
    if track.locked {
        note(ui, p, "This clip's track is locked.");
    }
    ui.add_enabled_ui(!track.locked, |ui| match section {
        None | Some(Section::Audio) => pane.audio(ui),
        Some(Section::Video) => pane.video(ui),
        Some(Section::Look) => pane.look(ui),
        Some(Section::Speed) => pane.speed(ui),
        Some(Section::Color) => pane.color(ui),
        Some(Section::Motion) => pane.motion(ui),
        Some(Section::Text) => pane.text(ui),
        Some(Section::Graphic) => pane.graphic(ui),
        Some(Section::Animate) => pane.animate(ui),
    });

    if matches!(section, None | Some(Section::Video | Section::Audio)) {
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
        time_fact(ui, p, "Start", timefmt::timecode(clip.start, rate));
        time_fact(ui, p, "End", timefmt::timecode(clip.end(), rate));
        time_fact(ui, p, "Duration", timefmt::timecode(clip.duration, rate));
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
        // Read again each frame of a drag, so the value grows by a part of
        // itself: the whole range in the three hundred points a field's
        // drag takes.
        let speed = if field.log && field.min > 0.0 {
            f64::from(shown.max(field.min * field.scale) * (field.max / field.min).ln() / 300.0)
        } else {
            field.speed
        };
        let changed = number_with(
            ui,
            self.p,
            &mut shown,
            &range,
            speed,
            (field.prefix, field.suffix),
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
            kit::menu_heading(
                ui,
                p,
                if easing.is_some() {
                    "Easing from this keyframe"
                } else {
                    "No keyframe follows to ease to"
                },
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
            widgets::menu_separator(ui, p);
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
        let row = keyed_row(ui, p, label, &what, state, |ui| fields(self, ui), |_| {});
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
        card(ui, self.p, |ui| note(ui, self.p, text));
    }

    fn video(&mut self, ui: &mut Ui) {
        let p = self.p;
        if self.asset.is_some() {
            self.layouts(ui);
        }

        section(ui, p, "Transform");
        keyed_card(ui, p, |ui| {
            self.clip_row(
                ui,
                "Position",
                &[ParamKey::PositionX, ParamKey::PositionY],
                |pane, ui| {
                    // A narrow card has narrower fields, and the name is
                    // left whole.
                    let x = Field {
                        width: paired_width(ui),
                        ..POSITION
                    };
                    let y = Field {
                        prefix: Some("Y"),
                        ..x
                    };
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
                    let field = number(
                        ui,
                        pane.p,
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
                in_reading_order(ui, 2.0 * widgets::SMALL_BUTTON + 2.0, |ui| {
                    let (icon, on) = (Icon::FlipHorizontal, t.flip_x);
                    if kit::small_icon_toggle(ui, p, icon, on, "Flip horizontally").clicked() {
                        t.flip_x = !t.flip_x;
                    }
                    let (icon, on) = (Icon::FlipVertical, t.flip_y);
                    if kit::small_icon_toggle(ui, p, icon, on, "Flip vertically").clicked() {
                        t.flip_y = !t.flip_y;
                    }
                });
            });
            if self.next.transform != Transform::default() {
                button_row(ui, p, |ui| {
                    if button(ui, p, Some(Icon::Rotate), "Reset transform", Kind::Plain).clicked() {
                        self.next.transform = Transform::default();
                    }
                });
            }
        });

        if self.asset.is_some() {
            section(ui, p, "Crop");
            keyed_card(ui, p, |ui| {
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

        // A title is drawn over the composition: it takes no blend or mask.
        if self.asset.is_some() {
            section(ui, p, "Compositing");
            card(ui, p, |ui| {
                tall_row(ui, p, "Blend", |ui| {
                    let width = ui.available_width().min(132.0);
                    select_list(
                        ui,
                        p,
                        "blend-mode",
                        "Blend mode",
                        &mut self.next.blend,
                        &BLENDS,
                        width,
                    );
                });
            });
            self.masks(ui);
        }
        if self.asset.is_some_and(|asset| asset.has_audio) {
            section(ui, p, "Sound");
            keyed_card(ui, p, |ui| self.sound_rows(ui));
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
        card(ui, p, |ui| {
            ui.add_enabled_ui(free, |ui| {
                button_row(ui, p, |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
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
        });
    }

    /// How a clip on an audio track plays: it carries sound and nothing
    /// that is drawn.
    pub(super) fn sound(&mut self, ui: &mut Ui) {
        let p = self.p;
        let timed = self
            .asset
            .is_some_and(|asset| asset.kind != MediaKind::Image);
        let audible = self.asset.is_some_and(|asset| asset.has_audio);
        if !(timed || audible) {
            return;
        }
        section(ui, p, "Playback");
        keyed_card(ui, p, |ui| {
            if timed {
                self.speed_row(ui);
                if audible {
                    self.pitch_row(ui);
                }
            }
            if audible {
                self.sound_rows(ui);
            }
        });
    }

    /// The volume and the fades of the clip's sound. A picture clip whose
    /// sound its linked audio clip owns has one line in their place that
    /// says where the sound is: a value set here would enter the history
    /// and never be heard. A clip that is only muted, soloed out or at
    /// volume 0 keeps its rows.
    fn sound_rows(&mut self, ui: &mut Ui) {
        if sound::owned(self.project, self.clip) {
            note(ui, self.p, fades::OWNED);
            return;
        }
        self.volume_row(ui);
        for edge in fades::EDGES {
            self.fade_row(ui, edge);
        }
        self.normalize_row(ui);
    }

    /// Measures the clip's loudness and sets its volume from it, as the
    /// clip menu does. Shown and not to be chosen, with the reason on
    /// hover, for a clip that is not heard.
    fn normalize_row(&mut self, ui: &mut Ui) {
        let p = self.p;
        let refused = normalize::refusal(self.project, self.clip.id);
        button_row(ui, p, |ui| {
            ui.add_enabled_ui(refused.is_none(), |ui| {
                let button = button(ui, p, Some(Icon::Volume), normalize::LABEL, Kind::Plain)
                    .on_hover_text(format!(
                        "Set the volume so the clip reaches {} LUFS, its peak at most {} dBTP",
                        normalize::TARGET,
                        normalize::CEILING
                    ))
                    .on_disabled_hover_text(refused.as_deref().unwrap_or_default());
                if button.clicked() {
                    self.actions.push(Action::Normalize(vec![self.clip.id]));
                }
            });
        });
    }

    /// How long one edge of the clip fades, in seconds, held to the room
    /// the other fade leaves. The field reads hundredths of a second and a
    /// fade may be held finer, as one a handle left on a frame is: a field
    /// left reading what it read has changed nothing.
    fn fade_row(&mut self, ui: &mut Ui, edge: FadeEdge) {
        let p = self.p;
        let (held, other) = match edge {
            FadeEdge::In => (self.next.fade_in, self.next.fade_out),
            FadeEdge::Out => (self.next.fade_out, self.next.fade_in),
        };
        let room = (self.clip.duration - other).max(Time::ZERO);
        let name = fades::name(edge);
        let mut shown = held.as_seconds() as f32;
        let read = |seconds: f32| format!("{:.2}", f64::from(seconds));
        let before = read(shown);
        let range = 0.0..=room.as_seconds() as f32;
        let changed = row(ui, p, name, |ui| {
            number(ui, p, &mut shown, range, 0.01, " s", 2, FIELD, name).changed()
        });
        // The field takes what it reads for typed when the focus leaves it.
        if changed && read(shown) != before {
            // The field holds a rounded number of seconds: the room is exact.
            let length = Time::from_seconds(f64::from(shown)).clamp(Time::ZERO, room);
            match edge {
                FadeEdge::In => self.next.fade_in = length,
                FadeEdge::Out => self.next.fade_out = length,
            }
        }
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
                if number(ui, p, &mut shown, range, 0.01, " ×", 2, FIELD, "Speed").changed() {
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
            if number(ui, p, &mut shown, range, 0.01, " ×", 2, FIELD, "Speed").changed() {
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
